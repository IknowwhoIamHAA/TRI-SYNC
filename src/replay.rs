use std::collections::HashSet;

use crate::event::{BatchOpType, Event, EventType, event_value_to_bsm};
use crate::key::validate_key;
use crate::state_map::{BinaryStateMap, BsmValue, StateSnapshot};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayOutcome {
    pub state: BinaryStateMap,
    pub warnings: Vec<String>,
}

pub struct ReplayEngine;

impl ReplayEngine {
    pub fn replay(events: &[Event]) -> Result<BinaryStateMap, String> {
        Ok(Self::replay_with_snapshot(events, None)?.state)
    }

    pub fn replay_with_snapshot(
        events: &[Event],
        snapshot: Option<StateSnapshot>,
    ) -> Result<ReplayOutcome, String> {
        let has_snapshot = snapshot.is_some();
        let mut state = if let Some(snapshot) = snapshot {
            snapshot.state
        } else {
            BinaryStateMap::new()
        };

        let mut warnings = Vec::new();
        let mut expected_seq = if has_snapshot {
            events.first().map_or(0, |first| first.seq)
        } else {
            0
        };

        let mut expected_prev_digest = if has_snapshot {
            events
                .first()
                .map_or(crate::event::ZERO_DIGEST_HEX.to_string(), |first| {
                    first.prev_digest.clone()
                })
        } else if let Some(first) = events.first() {
            if first.seq == 0 {
                crate::event::ZERO_DIGEST_HEX.to_string()
            } else {
                crate::event::ZERO_DIGEST_HEX.to_string()
            }
        } else {
            crate::event::ZERO_DIGEST_HEX.to_string()
        };

        let mut seen_digests = HashSet::new();

        for event in events {
            if event.seq != expected_seq {
                return Err(format!(
                    "SEQ_GAP: expected seq {}, got {}",
                    expected_seq, event.seq
                ));
            }

            event.validate_prev_digest(&expected_prev_digest)?;
            event.validate_digest()?;

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

                warnings.push(format!(
                    "DUPLICATE_EVENT: skipped non-idempotent duplicate event {}",
                    event.seq
                ));
                expected_prev_digest = event.digest.clone();
                expected_seq += 1;
                continue;
            }

            Self::apply_event(&mut state, event)?;

            expected_prev_digest = event.digest.clone();
            expected_seq += 1;
        }

        Ok(ReplayOutcome { state, warnings })
    }

    fn apply_event(state: &mut BinaryStateMap, event: &Event) -> Result<(), String> {
        match event.event_type {
            EventType::StateWrite => {
                let key = event
                    .key
                    .as_deref()
                    .ok_or_else(|| "STATE_WRITE missing key".to_string())?;
                validate_key(&event.namespace, key)?;

                let value = event
                    .state_write_value()?
                    .ok_or_else(|| "STATE_WRITE missing value payload".to_string())?;

                if let Some(current) = state.get(key)
                    && current.type_tag() != value.type_tag()
                {
                    return Err(format!(
                        "TYPE_MISMATCH: key {} expected type 0x{:02x}, got 0x{:02x}",
                        key,
                        current.type_tag(),
                        value.type_tag()
                    ));
                }

                state.set_validated(key.to_string(), value);
                Ok(())
            }
            EventType::StateDelete => {
                let key = event
                    .key
                    .as_deref()
                    .ok_or_else(|| "STATE_DELETE missing key".to_string())?;
                validate_key(&event.namespace, key)?;

                if state.get(key).is_none() && !event.idempotent.unwrap_or(false) {
                    return Err(format!("KEY_NOT_FOUND: {}", key));
                }

                state.delete(&event.namespace, key)?;
                Ok(())
            }
            EventType::StateBatch => {
                let ops = event
                    .ops
                    .as_ref()
                    .ok_or_else(|| "STATE_BATCH missing ops".to_string())?;
                let mut staged = state.clone();
                for op in ops {
                    validate_key(&event.namespace, &op.key)?;
                    match op.op_type {
                        BatchOpType::StateWrite => {
                            let value_type = op
                                .value_type
                                .ok_or_else(|| "STATE_WRITE op missing value_type".to_string())?;
                            let raw = op
                                .value
                                .as_ref()
                                .ok_or_else(|| "STATE_WRITE op missing value".to_string())?;
                            let value = event_value_to_bsm(value_type, raw)?;
                            if let Some(current) = staged.get(&op.key)
                                && current.type_tag() != value.type_tag()
                            {
                                return Err(format!(
                                    "TYPE_MISMATCH: key {} expected type 0x{:02x}, got 0x{:02x}",
                                    op.key,
                                    current.type_tag(),
                                    value.type_tag()
                                ));
                            }
                            staged.set_validated(op.key.clone(), value);
                        }
                        BatchOpType::StateDelete => {
                            if staged.get(&op.key).is_none() && !op.idempotent {
                                return Err(format!("KEY_NOT_FOUND: {}", op.key));
                            }
                            staged.delete(&event.namespace, &op.key)?;
                        }
                    }
                }
                *state = staged;
                Ok(())
            }
            EventType::TickSeal => {
                let expected_root = event
                    .root_digest
                    .as_ref()
                    .ok_or_else(|| "TICK_SEAL missing root_digest".to_string())?;
                let current_root = state.root_digest_hex()?;
                if &current_root != expected_root {
                    return Err(format!(
                        "TICK_SEAL_FAIL: expected root {}, got {}",
                        expected_root, current_root
                    ));
                }
                Ok(())
            }
            EventType::Compact | EventType::ProtocolError => Ok(()),
        }
    }

    pub fn reconstruct_value_digest(
        state: &BinaryStateMap,
        key: &str,
    ) -> Result<Option<String>, String> {
        let digest = match state.get(key) {
            Some(BsmValue::Null) => Some(BinaryStateMap::value_digest_hex(&BsmValue::Null)?),
            Some(value) => Some(BinaryStateMap::value_digest_hex(value)?),
            None => None,
        };
        Ok(digest)
    }
}

#[cfg(test)]
mod tests {
    use crate::event::{Event, ZERO_DIGEST_HEX};
    use crate::state_map::BsmValue;

    use super::ReplayEngine;

    #[test]
    fn replays_events_and_verifies_tick_seal() {
        let write = Event::state_write(
            0,
            0,
            "tenant-a",
            "tenant-a:counter",
            BsmValue::Integer(1),
            false,
            ZERO_DIGEST_HEX,
            None,
        )
        .expect("state write create");

        let mut state = crate::state_map::BinaryStateMap::new();
        state
            .set("tenant-a", "tenant-a:counter", BsmValue::Integer(1))
            .expect("set");
        let root_digest = state.root_digest_hex().expect("root digest");

        let seal = Event::tick_seal(1, 0, "tenant-a", 1, root_digest, write.digest.clone(), 0)
            .expect("tick seal create");

        let replayed = ReplayEngine::replay(&[write, seal]).expect("replay should succeed");
        assert_eq!(
            replayed.get("tenant-a:counter"),
            Some(&BsmValue::Integer(1))
        );
    }

    #[test]
    fn fails_on_sequence_gap() {
        let event = Event::state_write(
            1,
            0,
            "tenant-a",
            "tenant-a:key",
            BsmValue::String("x".to_string()),
            false,
            ZERO_DIGEST_HEX,
            None,
        )
        .expect("state write create");

        let err = ReplayEngine::replay(&[event]).expect_err("replay should fail");
        assert!(err.contains("SEQ_GAP"));
    }
}
