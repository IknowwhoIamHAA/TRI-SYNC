use std::collections::HashSet;

use crate::event::{BatchOpType, Event, EventType, event_value_to_bsm, ZERO_DIGEST_HEX};
use crate::key::validate_key;
use crate::state_map::{BinaryStateMap, BsmValue, StateSnapshot};
use crate::hex::encode_hex;

use super::ProtocolViolationError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayOutcome {
    pub state: BinaryStateMap,
    pub warnings: Vec<String>,
}

pub struct ReplayEngine;

impl ReplayEngine {
    pub fn replay(events: &[Event]) -> Result<BinaryStateMap, ProtocolViolationError> {
        Ok(Self::replay_with_snapshot(events, None)?.state)
    }

    pub fn replay_with_snapshot(
        events: &[Event],
        snapshot: Option<StateSnapshot>,
    ) -> Result<ReplayOutcome, ProtocolViolationError> {
        // Snapshot boundary: seq continuity
        if snapshot.is_none() && events.first().is_some_and(|e| e.seq != 0) {
            return Err(ProtocolViolationError::SequenceGap {
                expected: 0,
                found: events.first().unwrap().seq,
                namespace: events.first().unwrap().namespace.clone(),
                tick: events.first().unwrap().tick,
            });
        }

        let has_snapshot = snapshot.is_some();
        let snapshot_ns = snapshot.as_ref().map(|s| s.namespace.clone());
        let snapshot_prev_digest = snapshot
            .as_ref()
            .map(|s| encode_hex(&s.seal_digest));

        let mut state = snapshot
            .as_ref()
            .map(|s| s.state.clone())
            .unwrap_or_else(BinaryStateMap::new);

        let mut warnings = Vec::new();

        let mut expected_seq = if has_snapshot {
            snapshot.as_ref().unwrap().seal_seq + 1
        } else {
            0
        };

        let mut expected_prev_digest = if has_snapshot {
            snapshot_prev_digest.unwrap()
        } else {
            ZERO_DIGEST_HEX.to_string()
        };

        let mut seen_digests = HashSet::new();
        let mut expected_namespace =
            snapshot_ns.or_else(|| events.first().map(|e| e.namespace.clone()));

        let mut last_seal_timestamp_ms =
            snapshot.as_ref().map(|s| s.seal_timestamp_ms);

        let mut max_tick_seen =
            snapshot.as_ref().map_or(0, |s| s.tick);

        for event in events {
            // Namespace isolation
            if let Some(ns) = &expected_namespace {
                if &event.namespace != ns {
                    return Err(ProtocolViolationError::NamespaceBreach {
                        expected: ns.clone(),
                        found: event.namespace.clone(),
                        seq: event.seq,
                        tick: event.tick,
                    });
                }
            } else {
                expected_namespace = Some(event.namespace.clone());
            }

            // Sequence continuity
            if event.seq != expected_seq {
                return Err(ProtocolViolationError::SequenceGap {
                    expected: expected_seq,
                    found: event.seq,
                    namespace: event.namespace.clone(),
                    tick: event.tick,
                });
            }

            // Digest continuity
            if event.prev_digest != expected_prev_digest {
                return Err(ProtocolViolationError::DigestMismatch {
                    expected: expected_prev_digest.clone(),
                    found: event.prev_digest.clone(),
                    namespace: event.namespace.clone(),
                    seq: event.seq,
                    tick: event.tick,
                });
            }

            // Digest correctness
            if let Err(_) = event.validate_digest() {
                return Err(ProtocolViolationError::DigestMismatch {
                    expected: "<valid digest>".into(),
                    found: event.digest.clone(),
                    namespace: event.namespace.clone(),
                    seq: event.seq,
                    tick: event.tick,
                });
            }

            // Duplicate detection
            if !seen_digests.insert(event.digest.clone()) {
                if event.is_idempotent() {
                    warnings.push(format!(
                        "WARN_DUPLICATE: skipped duplicate idempotent event {}",
                        event.seq
                    ));
                    expected_prev_digest = event.digest.clone();
                    expected_seq += 1;
                    continue;
                }

                return Err(ProtocolViolationError::DuplicateEvent {
                    digest: event.digest.clone(),
                    seq: event.seq,
                    namespace: event.namespace.clone(),
                    tick: event.tick,
                });
            }

            // TICK_SEAL timestamp monotonicity
            if event.event_type == EventType::TickSeal {
                let ts = event.timestamp_ms.ok_or_else(|| {
                    ProtocolViolationError::InvalidEventFormat {
                        detail: format!("TICK_SEAL missing timestamp_ms at seq {}", event.seq),
                        seq: event.seq,
                        namespace: event.namespace.clone(),
                        tick: event.tick,
                    }
                })?;

                if let Some(prev_ts) = last_seal_timestamp_ms {
                    if ts < prev_ts {
                        return Err(ProtocolViolationError::TimestampRegression {
                            previous: prev_ts,
                            current: ts,
                            seq: event.seq,
                            namespace: event.namespace.clone(),
                            tick: event.tick,
                        });
                    }
                }

                last_seal_timestamp_ms = Some(ts);
            }

            // Tick regression
            if event.tick > 0 {
                if event.tick < max_tick_seen {
                    return Err(ProtocolViolationError::TickRegression {
                        previous: max_tick_seen,
                        current: event.tick,
                        seq: event.seq,
                        namespace: event.namespace.clone(),
                        tick: event.tick,
                    });
                }
                max_tick_seen = event.tick;
            }

            // Apply event
            Self::apply_event(&mut state, event)?;

            expected_prev_digest = event.digest.clone();
            expected_seq += 1;
        }

        Ok(ReplayOutcome { state, warnings })
    }

    fn apply_event(
        state: &mut BinaryStateMap,
        event: &Event,
    ) -> Result<(), ProtocolViolationError> {
        match event.event_type {
            EventType::StateWrite => {
                let key = event.key.as_deref().ok_or_else(|| {
                    ProtocolViolationError::InvalidEventFormat {
                        detail: "STATE_WRITE missing key".into(),
                        seq: event.seq,
                        namespace: event.namespace.clone(),
                        tick: event.tick,
                    }
                })?;

                validate_key(&event.namespace, key).map_err(|err| {
                    ProtocolViolationError::InvalidEventFormat {
                        detail: err.to_string(),
                        seq: event.seq,
                        namespace: event.namespace.clone(),
                        tick: event.tick,
                    }
                })?;

                let value = event
                    .state_write_value()
                    .map_err(|err| ProtocolViolationError::InvalidEventFormat {
                        detail: err.to_string(),
                        seq: event.seq,
                        namespace: event.namespace.clone(),
                        tick: event.tick,
                    })?
                    .ok_or_else(|| ProtocolViolationError::InvalidEventFormat {
                        detail: "STATE_WRITE missing value payload".into(),
                        seq: event.seq,
                        namespace: event.namespace.clone(),
                        tick: event.tick,
                    })?;

                state.set_validated(key.to_string(), value).map_err(|err| {
                    ProtocolViolationError::StateMismatch {
                        expected: "<valid state>".into(),
                        found: err.to_string(),
                        seq: event.seq,
                        namespace: event.namespace.clone(),
                        tick: event.tick,
                    }
                })?;
            }

            EventType::StateDelete => {
                let key = event.key.as_deref().ok_or_else(|| {
                    ProtocolViolationError::InvalidEventFormat {
                        detail: "STATE_DELETE missing key".into(),
                        seq: event.seq,
                        namespace: event.namespace.clone(),
                        tick: event.tick,
                    }
                })?;

                validate_key(&event.namespace, key).map_err(|err| {
                    ProtocolViolationError::InvalidEventFormat {
                        detail: err.to_string(),
                        seq: event.seq,
                        namespace: event.namespace.clone(),
                        tick: event.tick,
                    }
                })?;

                if state.get(key).is_none() && !event.idempotent.unwrap_or(false) {
                    return Err(ProtocolViolationError::StateMismatch {
                        expected: "existing key".into(),
                        found: "missing key".into(),
                        seq: event.seq,
                        namespace: event.namespace.clone(),
                        tick: event.tick,
                    });
                }

                state.delete(&event.namespace, key).map_err(|err| {
                    ProtocolViolationError::StateMismatch {
                        expected: "<valid delete>".into(),
                        found: err.to_string(),
                        seq: event.seq,
                        namespace: event.namespace.clone(),
                        tick: event.tick,
                    }
                })?;
            }

            EventType::StateBatch => {
                let ops = event.ops.as_ref().ok_or_else(|| {
                    ProtocolViolationError::InvalidEventFormat {
                        detail: "STATE_BATCH missing ops".into(),
                        seq: event.seq,
                        namespace: event.namespace.clone(),
                        tick: event.tick,
                    }
                })?;

                let mut staged = state.clone();

                for op in ops {
                    validate_key(&event.namespace, &op.key).map_err(|err| {
                        ProtocolViolationError::InvalidEventFormat {
                            detail: err.to_string(),
                            seq: event.seq,
                            namespace: event.namespace.clone(),
                            tick: event.tick,
                        }
                    })?;

                    match op.op_type {
                        BatchOpType::StateWrite => {
                            let value_type = op.value_type.ok_or_else(|| {
                                ProtocolViolationError::InvalidEventFormat {
                                    detail: "STATE_WRITE op missing value_type".into(),
                                    seq: event.seq,
                                    namespace: event.namespace.clone(),
                                    tick: event.tick,
                                }
                            })?;

                            let raw = op.value.as_ref().ok_or_else(|| {
                                ProtocolViolationError::InvalidEventFormat {
                                    detail: "STATE_WRITE op missing value".into(),
                                    seq: event.seq,
                                    namespace: event.namespace.clone(),
                                    tick: event.tick,
                                }
                            })?;

                            let value = event_value_to_bsm(value_type, raw).map_err(|err| {
                                ProtocolViolationError::InvalidEventFormat {
                                    detail: err.to_string(),
                                    seq: event.seq,
                                    namespace: event.namespace.clone(),
                                    tick: event.tick,
                                }
                            })?;

                            staged.set_validated(op.key.clone(), value).map_err(|err| {
                                ProtocolViolationError::StateMismatch {
                                    expected: "<valid state>".into(),
                                    found: err.to_string(),
                                    seq: event.seq,
                                    namespace: event.namespace.clone(),
                                    tick: event.tick,
                                }
                            })?;
                        }

                        BatchOpType::StateDelete => {
                            if staged.get(&op.key).is_none() && !op.idempotent {
                                return Err(ProtocolViolationError::StateMismatch {
                                    expected: "existing key".into(),
                                    found: "missing key".into(),
                                    seq: event.seq,
                                    namespace: event.namespace.clone(),
                                    tick: event.tick,
                                });
                            }

                            staged.delete(&event.namespace, &op.key).map_err(|err| {
                                ProtocolViolationError::StateMismatch {
                                    expected: "<valid delete>".into(),
                                    found: err.to_string(),
                                    seq: event.seq,
                                    namespace: event.namespace.clone(),
                                    tick: event.tick,
                                }
                            })?;
                        }
                    }
                }

                *state = staged;
            }

            EventType::TickSeal => {
                let expected_root = event.root_digest.as_ref().ok_or_else(|| {
                    ProtocolViolationError::InvalidEventFormat {
                        detail: "TICK_SEAL missing root_digest".into(),
                        seq: event.seq,
                        namespace: event.namespace.clone(),
                        tick: event.tick,
                    }
                })?;

                let current_root = state.root_digest_hex().map_err(|err| {
                    ProtocolViolationError::StateMismatch {
                        expected: expected_root.clone(),
                        found: err.to_string(),
                        seq: event.seq,
                        namespace: event.namespace.clone(),
                        tick: event.tick,
                    }
                })?;

                if &current_root != expected_root {
                    return Err(ProtocolViolationError::StateMismatch {
                        expected: expected_root.clone(),
                        found: current_root,
                        seq: event.seq,
                        namespace: event.namespace.clone(),
                        tick: event.tick,
                    });
                }
            }

            EventType::Compact => {
                let snapshot_digest = event.snapshot_digest.as_deref().ok_or_else(|| {
                    ProtocolViolationError::InvalidEventFormat {
                        detail: "COMPACT missing snapshot_digest".into(),
                        seq: event.seq,
                        namespace: event.namespace.clone(),
                        tick: event.tick,
                    }
                })?;

                let current_root = state.root_digest_hex().map_err(|err| {
                    ProtocolViolationError::CompactMismatch {
                        expected: snapshot_digest.to_string(),
                        found: err.to_string(),
                        seq: event.seq,
                        namespace: event.namespace.clone(),
                        tick: event.tick,
                    }
                })?;

                if current_root != snapshot_digest {
                    return Err(ProtocolViolationError::CompactMismatch {
                        expected: snapshot_digest.to_string(),
                        found: current_root,
                        seq: event.seq,
                        namespace: event.namespace.clone(),
                        tick: event.tick,
                    });
                }
            }

            EventType::ProtocolError => {
                return Err(ProtocolViolationError::ProtocolErrorEvent {
                    code: event.error_code.clone().unwrap_or("UNKNOWN_PROTOCOL_ERROR".into()),
                    detail: event.detail.clone(),
                    seq: event.seq,
                    namespace: event.namespace.clone(),
                    tick: event.tick,
                });
            }
        }

        Ok(())
    }
}
