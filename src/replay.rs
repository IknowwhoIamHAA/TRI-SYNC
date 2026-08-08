use crate::event::{Event, EventKind};
use crate::hex::encode_hex;
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

    /// Replay events with verbose output: per-event summary, state diffs,
    /// digest chain, and tenant boundary markers.
    pub fn replay_verbose(events: &[Event]) -> Result<BinaryStateMap, String> {
        let mut state = BinaryStateMap::new();
        let mut expected_sequence = 0_u64;
        let mut prev_tenant: Option<&str> = None;

        for event in events {
            if event.sequence != expected_sequence {
                return Err(format!(
                    "non-deterministic sequence: expected {}, got {}",
                    expected_sequence, event.sequence
                ));
            }
            event.validate_digest()?;

            // Tenant boundary marker.
            if prev_tenant.map_or(false, |t| t != event.tenant) {
                println!(
                    "── tenant boundary: {} → {} ──",
                    prev_tenant.unwrap_or(""),
                    event.tenant
                );
            }

            // Snapshot state before the event.
            let before: std::collections::BTreeMap<_, _> = state
                .entries()
                .filter(|(k, _)| k.tenant == event.tenant)
                .map(|(k, v)| (k.key.clone(), encode_hex(v)))
                .collect();

            // Apply the event.
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

            // Snapshot state after the event.
            let after: std::collections::BTreeMap<_, _> = state
                .entries()
                .filter(|(k, _)| k.tenant == event.tenant)
                .map(|(k, v)| (k.key.clone(), encode_hex(v)))
                .collect();

            // Print event header.
            println!(
                "[seq={} tenant={} key={} kind={:?}]",
                event.sequence, event.tenant, event.key, event.kind
            );
            println!("  digest: {}", event.payload_sha256);

            // Print state diff.
            let all_keys: std::collections::BTreeSet<_> =
                before.keys().chain(after.keys()).cloned().collect();
            for key in &all_keys {
                match (before.get(key), after.get(key)) {
                    (None, Some(v)) => println!("  + {key} = {v}"),
                    (Some(_), None) => println!("  - {key}"),
                    (Some(old), Some(new)) if old != new => {
                        println!("  ~ {key}: {old} → {new}")
                    }
                    _ => {}
                }
            }

            expected_sequence += 1;
            prev_tenant = Some(&event.tenant);
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
