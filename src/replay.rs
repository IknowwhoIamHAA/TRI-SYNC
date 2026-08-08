use crate::event::{Event, EventKind};
use crate::state_map::BinaryStateMap;

pub struct ReplayEngine;

impl ReplayEngine {
    pub fn replay(events: &[Event]) -> Result<BinaryStateMap, String> {
        let mut state = BinaryStateMap::new();
        let mut expected_sequence = 0_u64;

        for event in events {
            if event.sequence != expected_sequence {
                return Err(format!(
                    "non-deterministic sequence: expected {}, got {}",
                    expected_sequence, event.sequence
                ));
            }
            event.validate_digest()?;

            let tenant_key = event.tenant_key();
            match event.kind {
                EventKind::Set => {
                    let value = event
                        .value_bytes()?
                        .ok_or_else(|| format!("set event {} missing value_hex", event.sequence))?;
                    state.set_tenant_key(tenant_key, value);
                }
                EventKind::Delete => {
                    state.delete_tenant_key(&tenant_key);
                }
            }

            expected_sequence += 1;
        }

        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::event::Event;
    use crate::event_log::AppendOnlyEventLog;

    use super::ReplayEngine;

    #[test]
    fn replays_append_only_log_deterministically() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("current time should be later than epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tri-sync-{unique}.log"));
        let log = AppendOnlyEventLog::open(&path);

        log.append(&Event::new_set(0, "tenant-a", "counter", b"1"))
            .expect("append set event should succeed");
        log.append(&Event::new_set(1, "tenant-b", "flag", b"enabled"))
            .expect("append set event should succeed");
        log.append(&Event::new_delete(2, "tenant-b", "flag"))
            .expect("append delete event should succeed");

        let state = ReplayEngine::replay(&log.load().expect("loading log should succeed"))
            .expect("replay should succeed");

        assert_eq!(state.get("tenant-a", "counter"), Some("1".as_bytes()));
        assert_eq!(state.get("tenant-b", "flag"), None);

        let _ = fs::remove_file(path);
    }
}
