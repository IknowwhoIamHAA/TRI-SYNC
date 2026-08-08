use std::error::Error;

use serde::{Deserialize, Serialize};

use crate::event::Event;
use crate::event_log::AppendOnlyEventLog;
use crate::hex::encode_hex;
use crate::replay::ReplayEngine;
use crate::state_map::BinaryStateMap;

/// A workflow loaded from a JSON file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    #[serde(default)]
    pub description: String,
    pub steps: Vec<Step>,
}

/// One step in a workflow. A step belongs to a single tenant and contains
/// one or more operations that are each appended as events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    pub tenant: String,
    pub ops: Vec<Op>,
}

/// A deterministic operation that produces a single event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// Write a string value to a key.
    Set { key: String, value: String },
    /// Remove a key.
    Delete { key: String },
    /// Add `operand` to the current numeric value of `key` (default 0.0).
    Add { key: String, operand: f64 },
    /// Multiply the current numeric value of `key` by `operand` (default 0.0).
    Multiply { key: String, operand: f64 },
}

/// Executes a workflow against an append-only event log.
pub struct WorkflowRunner;

impl WorkflowRunner {
    /// Run `workflow` and append all produced events to `log`.
    /// Returns the final state after all steps have been applied.
    pub fn run(
        workflow: &Workflow,
        log: &AppendOnlyEventLog,
    ) -> Result<BinaryStateMap, Box<dyn Error>> {
        let mut events = log.load()?;
        let mut sequence = events.last().map_or(0, |e| e.sequence + 1);
        let mut state = ReplayEngine::replay(&events).map_err(std::io::Error::other)?;

        for step in &workflow.steps {
            for op in &step.ops {
                let event = Self::op_to_event(op, sequence, &step.tenant, &state)?;
                log.append(&event)?;
                // Apply to in-memory state so subsequent ops see the update.
                match event.kind {
                    crate::event::EventKind::Set => {
                        let value = event
                            .value_bytes()?
                            .expect("set event always has value_bytes");
                        state.set_tenant_key(event.tenant_key(), value);
                    }
                    crate::event::EventKind::Delete => {
                        state.delete_tenant_key(&event.tenant_key());
                    }
                }
                events.push(event);
                sequence += 1;
            }
        }

        Ok(state)
    }

    fn op_to_event(
        op: &Op,
        sequence: u64,
        tenant: &str,
        state: &BinaryStateMap,
    ) -> Result<Event, Box<dyn Error>> {
        match op {
            Op::Set { key, value } => {
                Ok(Event::new_set(sequence, tenant, key, value.as_bytes()))
            }
            Op::Delete { key } => Ok(Event::new_delete(sequence, tenant, key)),
            Op::Add { key, operand } => {
                let current = read_f64(state, tenant, key);
                let result = current + operand;
                Ok(Event::new_set(
                    sequence,
                    tenant,
                    key,
                    encode_hex(&result.to_bits().to_be_bytes()).as_bytes(),
                ))
            }
            Op::Multiply { key, operand } => {
                let current = read_f64(state, tenant, key);
                let result = current * operand;
                Ok(Event::new_set(
                    sequence,
                    tenant,
                    key,
                    encode_hex(&result.to_bits().to_be_bytes()).as_bytes(),
                ))
            }
        }
    }
}

/// Read a key as an F64 big-endian value, returning 0.0 if absent or invalid.
fn read_f64(state: &BinaryStateMap, tenant: &str, key: &str) -> f64 {
    state
        .get(tenant, key)
        .and_then(|bytes| {
            // Accept either 8 raw bytes (big-endian F64) or a hex string
            // produced by a previous `add`/`multiply` op.
            if bytes.len() == 8 {
                let arr: [u8; 8] = bytes.try_into().ok()?;
                Some(f64::from_bits(u64::from_be_bytes(arr)))
            } else {
                // Might be a UTF-8 string like "3.14" from a `set` op.
                std::str::from_utf8(bytes).ok()?.parse::<f64>().ok()
            }
        })
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::event_log::AppendOnlyEventLog;

    use super::{Op, Step, Workflow, WorkflowRunner};

    fn tmp_log() -> AppendOnlyEventLog {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tri-sync-wf-{unique}.log"));
        AppendOnlyEventLog::open(path)
    }

    #[test]
    fn run_set_delete_workflow() {
        let log = tmp_log();
        let workflow = Workflow {
            id: "test".into(),
            description: "unit test".into(),
            steps: vec![
                Step {
                    id: "s1".into(),
                    tenant: "t1".into(),
                    ops: vec![Op::Set {
                        key: "x".into(),
                        value: "hello".into(),
                    }],
                },
                Step {
                    id: "s2".into(),
                    tenant: "t1".into(),
                    ops: vec![Op::Delete { key: "x".into() }],
                },
            ],
        };

        let state = WorkflowRunner::run(&workflow, &log).expect("run should succeed");
        assert_eq!(state.get("t1", "x"), None);
        let _ = fs::remove_file(log.path());
    }

    #[test]
    fn run_add_multiply_workflow() {
        let log = tmp_log();
        let workflow = Workflow {
            id: "math".into(),
            description: "numeric ops".into(),
            steps: vec![Step {
                id: "s1".into(),
                tenant: "t1".into(),
                ops: vec![
                    Op::Add {
                        key: "counter".into(),
                        operand: 5.0,
                    },
                    Op::Add {
                        key: "counter".into(),
                        operand: 3.0,
                    },
                    Op::Multiply {
                        key: "counter".into(),
                        operand: 2.0,
                    },
                ],
            }],
        };

        WorkflowRunner::run(&workflow, &log).expect("run should succeed");

        // Reload from log to confirm persistence.
        let events = log.load().expect("load should succeed");
        assert_eq!(events.len(), 3);
        let _ = fs::remove_file(log.path());
    }
}
