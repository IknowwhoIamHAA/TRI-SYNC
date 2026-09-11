use std::error::Error;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use tri_sync::canonical_json::to_canonical_string;
use tri_sync::digest::sha256_hex;
use tri_sync::event::{BatchOpType, Event, EventType, ZERO_DIGEST_HEX};
use tri_sync::event_log::AppendOnlyEventLog;
use tri_sync::license;
use tri_sync::replay::ReplayEngine;
use tri_sync::state_map::BsmValue;

#[derive(Parser)]
#[command(name = "tri-sync")]
#[command(about = "Deterministic runtime with append-only replayable state transitions")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Apply {
        #[arg(long)]
        log: PathBuf,
        #[arg(long)]
        namespace: String,
        #[arg(long)]
        key: String,
        #[arg(long)]
        value: String,
        /// Logical tick (monotonic epoch counter) for this event. Defaults to 0.
        #[arg(long, default_value_t = 0)]
        tick: u64,
        /// Mark this write as commercial production use. Requires an enterprise license.
        #[arg(long)]
        production: bool,
    },
    Delete {
        #[arg(long)]
        log: PathBuf,
        #[arg(long)]
        namespace: String,
        #[arg(long)]
        key: String,
        /// Logical tick (monotonic epoch counter) for this event. Defaults to 0.
        #[arg(long, default_value_t = 0)]
        tick: u64,
        /// Mark this deletion as commercial production use. Requires an enterprise license.
        #[arg(long)]
        production: bool,
    },
    Replay {
        #[arg(long)]
        log: PathBuf,
    },
    /// Verify an event log: replay from genesis, print the final root digest,
    /// and exit 0 on success or 1 if the log is invalid.
    Verify {
        #[arg(long)]
        log: PathBuf,
        /// After a successful verify, append a TICK_SEAL checkpoint event to the
        /// log.  The seal records the current root digest and event count so that
        /// subsequent verify runs can confirm no events were added or modified.
        #[arg(long)]
        seal: bool,
        /// Logical tick to use for the appended TICK_SEAL (only used with --seal).
        #[arg(long, default_value_t = 0)]
        tick: u64,
        /// Namespace for the appended TICK_SEAL (only used with --seal).
        #[arg(long, default_value = "")]
        namespace: String,
    },
    /// Export the full event log as a JSON array to stdout.
    Export {
        #[arg(long)]
        log: PathBuf,
        /// Output format.  Currently only "json" is supported.
        #[arg(long, default_value = "json")]
        format: String,
    },
    Digest {
        #[arg(long)]
        input: String,
    },
    Example {
        #[arg(long)]
        log: PathBuf,
    },
    /// Print a human-readable summary of every event in a log file.
    Inspect {
        #[arg(long)]
        log: PathBuf,
    },
    /// Print a one-line status summary of a log file: event count, head digest,
    /// sealed/unsealed, and whether replay passes.
    Status {
        #[arg(long)]
        log: PathBuf,
    },
    /// Generate a machine-readable compliance report for a verified event log.
    Report {
        #[arg(long)]
        log: PathBuf,
    },
}

fn main() {
    if let Err(err) = run() {
        if let Some(violation) = err.downcast_ref::<tri_sync::error::ProtocolViolationError>() {
            eprintln!("{}", violation.to_json_line());
            std::process::exit(violation.exit_code);
        }
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Apply {
            log,
            namespace,
            key,
            value,
            tick,
            production,
        } => {
            if production {
                license::require_enterprise("Commercial production execution")?;
            }
            let log = AppendOnlyEventLog::open(log);
            let events = log.load()?;
            let seq = log.next_sequence()?;
            let prev = events
                .last()
                .map_or(ZERO_DIGEST_HEX.to_string(), |event| event.digest.clone());
            let key = namespaced_key(&namespace, &key);
            let event = Event::state_write(
                seq,
                tick,
                namespace,
                key,
                BsmValue::Bytes(value.into_bytes()),
                false,
                prev,
                None,
            )?;
            log.append(&event)?;
            println!("appended STATE_WRITE at seq {}", event.seq);
        }
        Commands::Delete {
            log,
            namespace,
            key,
            tick,
            production,
        } => {
            if production {
                license::require_enterprise("Commercial production execution")?;
            }
            let log = AppendOnlyEventLog::open(log);
            let events = log.load()?;
            let seq = log.next_sequence()?;
            let prev = events
                .last()
                .map_or(ZERO_DIGEST_HEX.to_string(), |event| event.digest.clone());
            let key = namespaced_key(&namespace, &key);
            let event = Event::state_delete(seq, tick, namespace, key, None, true, prev)?;
            log.append(&event)?;
            println!("appended STATE_DELETE at seq {}", event.seq);
        }
        Commands::Replay { log } => {
            let log = AppendOnlyEventLog::open(log);
            let events = log.load()?;
            let state = ReplayEngine::replay(&events).map_err(std::io::Error::other)?;
            let json_value = serde_json::to_value(state.to_json_value())?;
            println!("{}", to_canonical_string(&json_value)?);
        }
        Commands::Verify {
            log,
            seal,
            tick,
            namespace,
        } => {
            let log_path = log.clone();
            let log = AppendOnlyEventLog::open(log);
            let events = log.load().map_err(|err| {
                eprintln!(
                    "VERIFY FAILED: could not load log {}: {err}",
                    log_path.display()
                );
                std::process::exit(1);
            })?;

            // Warn when the log does not end with a TICK_SEAL checkpoint.
            let ends_with_seal = events
                .last()
                .map(|e| e.event_type == tri_sync::event::EventType::TickSeal)
                .unwrap_or(false);
            if !events.is_empty() && !ends_with_seal {
                eprintln!("WARNING: log does not end with a TICK_SEAL checkpoint");
            }

            match ReplayEngine::replay(&events) {
                Ok(state) => match state.root_digest_hex() {
                    Ok(digest) => {
                        println!("OK");
                        println!("log={}", log_path.display());
                        println!("events={}", events.len());
                        println!("root_digest={digest}");

                        if seal {
                            let ns = if namespace.is_empty() {
                                events
                                    .first()
                                    .map(|e| e.namespace.clone())
                                    .unwrap_or_else(|| "trisync-system".to_string())
                            } else {
                                namespace
                            };
                            let seq = log.next_sequence()?;
                            let prev = events
                                .last()
                                .map_or(ZERO_DIGEST_HEX.to_string(), |e| e.digest.clone());
                            let now_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_millis() as u64)
                                .unwrap_or(0);
                            let seal_event = Event::tick_seal(
                                seq,
                                tick,
                                ns,
                                events.len() as u32,
                                digest.clone(),
                                prev,
                                now_ms,
                            )?;
                            log.append(&seal_event)?;
                            println!("seal_seq={}", seal_event.seq);
                            println!("seal_digest={}", seal_event.digest);
                        }
                    }
                    Err(err) => {
                        eprintln!("VERIFY FAILED: root digest error: {err}");
                        std::process::exit(1);
                    }
                },
                Err(err) => {
                    eprintln!("VERIFY FAILED: replay error: {err}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Export { log, format } => {
            if format != "json" {
                eprintln!("Unsupported format '{}'. Only 'json' is supported.", format);
                std::process::exit(1);
            }
            let log = AppendOnlyEventLog::open(log);
            let events = log.load()?;
            let arr: Vec<serde_json::Value> = events
                .iter()
                .map(serde_json::to_value)
                .collect::<Result<_, _>>()?;
            let json = serde_json::to_value(arr)?;
            println!("{}", to_canonical_string(&json)?);
        }
        Commands::Digest { input } => {
            println!("{}", sha256_hex(input.as_bytes()));
        }
        Commands::Example { log } => {
            run_example(log)?;
        }
        Commands::Inspect { log } => {
            let event_log = AppendOnlyEventLog::open(log);
            let events = event_log.load()?;
            if events.is_empty() {
                println!("(empty log)");
            } else {
                println!(
                    "{:<6} {:<16} {:<20} {:<32} digest",
                    "seq", "type", "namespace", "key"
                );
                println!("{}", "-".repeat(100));
                for event in &events {
                    println!(
                        "{:<6} {:<16} {:<20} {:<32} {}",
                        event.seq,
                        format!("{:?}", event.event_type),
                        event.namespace,
                        event.key.as_deref().unwrap_or("-"),
                        event.digest.get(..16).unwrap_or(&event.digest),
                    );
                }
                println!("{}", "-".repeat(100));
                println!("total: {} events", events.len());
            }
        }
        Commands::Status { log } => {
            let log_path = log.clone();
            let event_log = AppendOnlyEventLog::open(log);
            let events = event_log.load()?;
            let count = events.len();
            let head_digest = events
                .last()
                .map(|e| e.digest.as_str())
                .unwrap_or(ZERO_DIGEST_HEX);
            let sealed = events
                .last()
                .map(|e| e.event_type == tri_sync::event::EventType::TickSeal)
                .unwrap_or(false);
            let replay_ok = ReplayEngine::replay(&events).is_ok();
            println!("log={}", log_path.display());
            println!("events={count}");
            println!("head_digest={head_digest}");
            println!("sealed={sealed}");
            println!("replay_ok={replay_ok}");
        }
        Commands::Report { log } => {
            license::require_enterprise("Automated compliance reporting")?;
            let log_path = log.clone();
            let event_log = AppendOnlyEventLog::open(log);
            let events = event_log.load()?;
            let state = ReplayEngine::replay(&events).map_err(std::io::Error::other)?;
            let namespaces: std::collections::BTreeSet<&str> = events
                .iter()
                .map(|event| event.namespace.as_str())
                .collect();
            let (write_count, delete_count) = state_operation_counts(&events);
            let report = serde_json::json!({
                "schema_version": "1.0",
                "log": log_path,
                "verification": "passed",
                "event_count": events.len(),
                "namespaces": namespaces,
                "state_writes": write_count,
                "state_deletes": delete_count,
                "root_digest": state.root_digest_hex().map_err(std::io::Error::other)?,
                "digest_algorithm": "SHA-256"
            });
            println!("{}", to_canonical_string(&report)?);
        }
    }

    Ok(())
}

fn state_operation_counts(events: &[Event]) -> (usize, usize) {
    let mut writes = 0usize;
    let mut deletes = 0usize;

    for event in events {
        match event.event_type {
            EventType::StateWrite => writes += 1,
            EventType::StateDelete => deletes += 1,
            EventType::StateBatch => {
                if let Some(ops) = &event.ops {
                    for op in ops {
                        match op.op_type {
                            BatchOpType::StateWrite => writes += 1,
                            BatchOpType::StateDelete => deletes += 1,
                        }
                    }
                }
            }
            _ => {}
        }
    }

    (writes, deletes)
}

fn run_example(log_path: PathBuf) -> Result<(), Box<dyn Error>> {
    if log_path.exists() {
        std::fs::remove_file(&log_path)?;
    }

    let log = AppendOnlyEventLog::open(&log_path);

    let first = Event::state_write(
        0,
        0,
        "tenant-a",
        "tenant-a:job",
        BsmValue::String("queued".to_string()),
        false,
        ZERO_DIGEST_HEX,
        None,
    )?;
    log.append(&first)?;

    let second = Event::state_write(
        1,
        0,
        "tenant-a",
        "tenant-a:job",
        BsmValue::String("running".to_string()),
        false,
        first.digest.clone(),
        None,
    )?;
    log.append(&second)?;

    let seal_state = ReplayEngine::replay(&log.load()?).map_err(std::io::Error::other)?;
    let root_digest = seal_state
        .root_digest_hex()
        .map_err(std::io::Error::other)?;
    let seal = Event::tick_seal(2, 0, "tenant-a", 2, root_digest, second.digest.clone(), 0)?;
    log.append(&seal)?;

    let state = ReplayEngine::replay(&log.load()?).map_err(std::io::Error::other)?;
    let state_json = serde_json::to_value(state.to_json_value())?;

    println!("log={}", log.path().display());
    println!("state={}", to_canonical_string(&state_json)?);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::state_operation_counts;
    use tri_sync::event::{Event, ZERO_DIGEST_HEX, delete_op, write_op};
    use tri_sync::state_map::BsmValue;

    #[test]
    fn state_operation_counts_include_batch_ops() {
        let write = Event::state_write(
            0,
            0,
            "tenant",
            "tenant:key-a",
            BsmValue::String("one".to_string()),
            false,
            ZERO_DIGEST_HEX,
            None,
        )
        .expect("state write should build");
        let delete = Event::state_delete(
            1,
            0,
            "tenant",
            "tenant:key-a",
            None,
            true,
            write.digest.clone(),
        )
        .expect("state delete should build");
        let batch = Event::state_batch(
            2,
            0,
            "tenant",
            vec![
                write_op("tenant:key-b", BsmValue::String("two".to_string()), false)
                    .expect("batch write op should build"),
                delete_op("tenant:key-b", None, true),
            ],
            delete.digest.clone(),
        )
        .expect("state batch should build");

        let (writes, deletes) = state_operation_counts(&[write, delete, batch]);
        assert_eq!(writes, 2);
        assert_eq!(deletes, 2);
    }
}

fn namespaced_key(namespace: &str, key: &str) -> String {
    let expected = format!("{namespace}:");
    if key.starts_with(&expected) {
        key.to_string()
    } else {
        format!("{expected}{key}")
    }
}
