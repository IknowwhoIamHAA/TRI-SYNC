use std::env;
use std::error::Error;
use std::io;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use tri_sync::backend::{EventLogBackend, FileSystemBackend};
use tri_sync::event::{Event, ZERO_DIGEST_HEX};
use tri_sync::replay::verify_events;
use tri_sync::state_map::BsmValue;

const DEFAULT_EVENTS: usize = 100_000;
const MIN_EVENTS: usize = 10_000;
const MAX_EVENTS: usize = 100_000;

fn benchmark_path() -> PathBuf {
    let mut path = env::temp_dir();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    path.push(format!("tri-sync-benchmark-ledger-{timestamp}.jsonl"));
    path
}

fn parse_event_count() -> Result<usize, Box<dyn Error>> {
    let Some(raw) = env::args().nth(1) else {
        return Ok(DEFAULT_EVENTS);
    };

    let parsed = raw
        .parse::<usize>()
        .map_err(|err| io::Error::other(format!("invalid event count '{raw}': {err}")))?;

    if !(MIN_EVENTS..=MAX_EVENTS).contains(&parsed) {
        return Err(io::Error::other(format!(
            "event count must be between {MIN_EVENTS} and {MAX_EVENTS}"
        ))
        .into());
    }

    Ok(parsed)
}

fn main() -> Result<(), Box<dyn Error>> {
    let event_count = parse_event_count()?;
    let log_path = benchmark_path();
    let backend = FileSystemBackend::open(&log_path);

    let namespace = "bench-tenant";
    let mut prev_digest = ZERO_DIGEST_HEX.to_string();
    let mut batch = Vec::with_capacity(event_count);

    let ingest_start = Instant::now();
    for seq in 0..event_count {
        let key = format!("{namespace}:txn:{}", seq % 1_000);
        let value = BsmValue::Integer((seq % 50_000) as i64);
        let event = Event::state_write(
            seq as u64,
            seq as u64,
            namespace,
            key,
            value,
            false,
            prev_digest.clone(),
            None,
        )
        .map_err(io::Error::other)?;

        prev_digest = event.digest.clone();
        batch.push(event);
    }
    backend.append_batch(&batch)?;
    let ingest_elapsed = ingest_start.elapsed();

    let load_start = Instant::now();
    let events = backend.load()?;
    let load_elapsed = load_start.elapsed();

    if events.len() != event_count {
        return Err(io::Error::other(format!(
            "event count mismatch: expected {event_count}, loaded {}",
            events.len()
        ))
        .into());
    }

    let verify_start = Instant::now();
    verify_events(&events, None)?;
    let verify_elapsed = verify_start.elapsed();

    let throughput = event_count as f64 / ingest_elapsed.as_secs_f64();

    println!("TRI-SYNC benchmark complete");
    println!("  log_path: {}", log_path.display());
    println!("  events_ingested: {event_count}");
    println!(
        "  ingest_time_ms: {:.3}",
        ingest_elapsed.as_secs_f64() * 1_000.0
    );
    println!("  throughput_events_per_sec: {:.2}", throughput);
    println!(
        "  load_time_ms: {:.3}",
        load_elapsed.as_secs_f64() * 1_000.0
    );
    println!(
        "  verify_time_ms: {:.3}",
        verify_elapsed.as_secs_f64() * 1_000.0
    );
    println!("  sha256_chain_verified: true");

    Ok(())
}
