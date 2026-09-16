use std::time::Instant;

use tempfile::tempdir;
use tri_sync::backend::{EventLogBackend, FileSystemBackend};
use tri_sync::event::{Event, ZERO_DIGEST_HEX};
use tri_sync::state_map::BsmValue;

const EVENT_COUNT: usize = 10_000;
const MIN_BATCH_THROUGHPUT_EVENTS_PER_SEC: f64 = 200.0;

#[test]
fn batch_append_throughput_stays_above_floor() {
    let temp = tempdir().expect("tempdir");
    let log_path = temp.path().join("events.jsonl");
    let backend = FileSystemBackend::open(&log_path);

    let mut prev_digest = ZERO_DIGEST_HEX.to_string();
    let mut events = Vec::with_capacity(EVENT_COUNT);

    for seq in 0..EVENT_COUNT {
        let event = Event::state_write(
            seq as u64,
            seq as u64,
            "tenant-a",
            format!("tenant-a:key:{}", seq % 1000),
            BsmValue::Integer((seq % 50000) as i64),
            false,
            prev_digest.clone(),
            None,
        )
        .expect("event");
        prev_digest = event.digest.clone();
        events.push(event);
    }

    let started = Instant::now();
    backend.append_batch(&events).expect("append batch");
    let elapsed = started.elapsed();
    let throughput = EVENT_COUNT as f64 / elapsed.as_secs_f64();

    assert!(
        throughput >= MIN_BATCH_THROUGHPUT_EVENTS_PER_SEC,
        "batch throughput {:.2} events/sec fell below floor {:.2} events/sec",
        throughput,
        MIN_BATCH_THROUGHPUT_EVENTS_PER_SEC
    );
}
