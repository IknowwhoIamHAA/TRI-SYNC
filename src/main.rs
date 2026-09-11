use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use tri_sync::canonical_json::to_canonical_string;
use tri_sync::digest::sha256_hex;
use tri_sync::errors::{
    ProtocolAction, ProtocolError, ProtocolErrorReason, ProtocolPhase, ProtocolResult,
};
use tri_sync::event::{Event, EventType, ZERO_DIGEST_HEX};
use tri_sync::event_log::AppendOnlyEventLog;
use tri_sync::key::{RESERVED_SYSTEM_NAMESPACE, validate_runtime_namespace};
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
        #[arg(long, default_value_t = 0)]
        tick: u64,
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
        #[arg(long, default_value_t = 0)]
        tick: u64,
        #[arg(long)]
        production: bool,
    },
    Replay {
        #[arg(long)]
        log: PathBuf,
    },
    Verify {
        #[arg(long)]
        log: PathBuf,
        #[arg(long)]
        seal: bool,
        #[arg(long, default_value_t = 0)]
        tick: u64,
        #[arg(long, default_value = "")]
        namespace: String,
    },
    Export {
        #[arg(long)]
        log: PathBuf,
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
    Inspect {
        #[arg(long)]
        log: PathBuf,
    },
    Status {
        #[arg(long)]
        log: PathBuf,
    },
    Report {
        #[arg(long)]
        log: PathBuf,
    },
}

fn main() {
    let exit_code = match run() {
        Ok(()) => 0,
        Err(err) => {
            report_failure(&err);
            match err.code() {
                "LICENSE_REQUIRED" => 3,
                "UNSUPPORTED_FORMAT" | "IO_ERROR" => 2,
                _ => 1,
            }
        }
    };

    std::process::exit(exit_code);
}

#[allow(clippy::result_large_err)]
fn run() -> ProtocolResult<()> {
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
                license::require_enterprise("Commercial production execution")
                    .map_err(|err| license_error("Commercial production execution", err))?;
            }

            let log = AppendOnlyEventLog::open(log);
            let tail = log
                .tail_metadata()
                .map_err(|err| io_error(ProtocolPhase::Append, err))?;
            let seq = tail.next_seq;
            let prev = tail.prev_digest.clone();
            let key = namespaced_key(&namespace, &key);
            let event = Event::state_write(
                seq,
                tick,
                namespace.clone(),
                key,
                BsmValue::Bytes(value.into_bytes()),
                false,
                prev,
                None,
            )
            .map_err(|err| {
                ProtocolError::from_message(ProtocolPhase::Append, ProtocolAction::Reject, err)
                    .with_namespace(namespace.clone())
                    .with_seq(seq)
                    .with_tick(tick)
            })?;

            if let Err(err) = log.append(&event) {
                let protocol_error = ProtocolError::from_message(
                    ProtocolPhase::Append,
                    ProtocolAction::Reject,
                    err.to_string(),
                )
                .with_namespace(namespace.clone())
                .with_seq(seq)
                .with_tick(tick);
                emit_protocol_error(&log, &namespace, tick, &protocol_error);
                return Err(protocol_error);
            }

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
                license::require_enterprise("Commercial production execution")
                    .map_err(|err| license_error("Commercial production execution", err))?;
            }

            let log = AppendOnlyEventLog::open(log);
            let tail = log
                .tail_metadata()
                .map_err(|err| io_error(ProtocolPhase::Append, err))?;
            let seq = tail.next_seq;
            let prev = tail.prev_digest.clone();
            let key = namespaced_key(&namespace, &key);
            let event = Event::state_delete(seq, tick, namespace.clone(), key, None, true, prev)
                .map_err(|err| {
                    ProtocolError::from_message(ProtocolPhase::Append, ProtocolAction::Reject, err)
                        .with_namespace(namespace.clone())
                        .with_seq(seq)
                        .with_tick(tick)
                })?;

            if let Err(err) = log.append(&event) {
                let protocol_error = ProtocolError::from_message(
                    ProtocolPhase::Append,
                    ProtocolAction::Reject,
                    err.to_string(),
                )
                .with_namespace(namespace.clone())
                .with_seq(seq)
                .with_tick(tick);
                emit_protocol_error(&log, &namespace, tick, &protocol_error);
                return Err(protocol_error);
            }

            println!("appended STATE_DELETE at seq {}", event.seq);
        }
        Commands::Replay { log } => {
            let log = AppendOnlyEventLog::open(log);
            let events = log
                .load()
                .map_err(|err| io_error(ProtocolPhase::Replay, err))?;
            let state = ReplayEngine::replay(&events)?;
            let json_value = serde_json::to_value(state.to_json_value()).map_err(|err| {
                ProtocolError::new(
                    ProtocolErrorReason::IoError,
                    ProtocolPhase::Replay,
                    ProtocolAction::Halt,
                    err.to_string(),
                )
            })?;
            println!(
                "{}",
                to_canonical_string(&json_value).map_err(|err| {
                    ProtocolError::new(
                        ProtocolErrorReason::IoError,
                        ProtocolPhase::Replay,
                        ProtocolAction::Halt,
                        err,
                    )
                })?
            );
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
                io_error(
                    ProtocolPhase::Verify,
                    format!("could not load log {}: {err}", log_path.display()),
                )
            })?;

            let ends_with_seal = events
                .last()
                .map(|e| e.event_type == tri_sync::event::EventType::TickSeal)
                .unwrap_or(false);
            if !events.is_empty() && !ends_with_seal {
                eprintln!("WARNING: log does not end with a TICK_SEAL checkpoint");
            }

            let state = ReplayEngine::replay(&events).map_err(|err| {
                err.with_namespace(
                    events
                        .first()
                        .map(|event| event.namespace.clone())
                        .unwrap_or_else(|| RESERVED_SYSTEM_NAMESPACE.to_string()),
                )
            })?;
            let digest = state.root_digest_hex().map_err(|err| {
                ProtocolError::from_message(ProtocolPhase::Verify, ProtocolAction::Halt, err)
            })?;
            println!("OK");
            println!("log={}", log_path.display());
            println!("events={}", events.len());
            println!("root_digest={digest}");

            if seal {
                let ns = if namespace.is_empty() {
                    events
                        .first()
                        .map(|e| e.namespace.clone())
                        .unwrap_or_else(|| RESERVED_SYSTEM_NAMESPACE.to_string())
                } else {
                    namespace
                };
                let tail = log
                    .tail_metadata()
                    .map_err(|err| io_error(ProtocolPhase::Verify, err))?;
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let seal_event = Event::tick_seal(
                    tail.next_seq,
                    tick,
                    ns,
                    u32::try_from(events.len()).map_err(|_| {
                        ProtocolError::new(
                            ProtocolErrorReason::InvalidSegment,
                            ProtocolPhase::Verify,
                            ProtocolAction::Reject,
                            format!(
                                "event_count {} exceeds maximum TICK_SEAL field width",
                                events.len()
                            ),
                        )
                        .with_tick(tick)
                    })?,
                    digest.clone(),
                    tail.prev_digest,
                    now_ms,
                )
                .map_err(|err| {
                    ProtocolError::from_message(ProtocolPhase::Verify, ProtocolAction::Reject, err)
                        .with_tick(tick)
                })?;
                log.append(&seal_event)
                    .map_err(|err| io_error(ProtocolPhase::Verify, err))?;
                println!("seal_seq={}", seal_event.seq);
                println!("seal_digest={}", seal_event.digest);
            }
        }
        Commands::Export { log, format } => {
            if format != "json" {
                return Err(ProtocolError::new(
                    ProtocolErrorReason::UnsupportedFormat,
                    ProtocolPhase::Input,
                    ProtocolAction::Reject,
                    format!("unsupported format '{format}'. Only 'json' is supported."),
                )
                .with_actual(format));
            }
            let log = AppendOnlyEventLog::open(log);
            let events = log
                .load()
                .map_err(|err| io_error(ProtocolPhase::Report, err))?;
            let arr: Vec<serde_json::Value> = events
                .iter()
                .map(serde_json::to_value)
                .collect::<Result<_, _>>()
                .map_err(|err| {
                    ProtocolError::new(
                        ProtocolErrorReason::IoError,
                        ProtocolPhase::Report,
                        ProtocolAction::Halt,
                        err.to_string(),
                    )
                })?;
            let json = serde_json::to_value(arr).map_err(|err| {
                ProtocolError::new(
                    ProtocolErrorReason::IoError,
                    ProtocolPhase::Report,
                    ProtocolAction::Halt,
                    err.to_string(),
                )
            })?;
            println!(
                "{}",
                to_canonical_string(&json).map_err(|err| {
                    ProtocolError::new(
                        ProtocolErrorReason::IoError,
                        ProtocolPhase::Report,
                        ProtocolAction::Halt,
                        err,
                    )
                })?
            );
        }
        Commands::Digest { input } => {
            println!("{}", sha256_hex(input.as_bytes()));
        }
        Commands::Example { log } => {
            run_example(log)?;
        }
        Commands::Inspect { log } => {
            let event_log = AppendOnlyEventLog::open(log);
            let events = event_log
                .load()
                .map_err(|err| io_error(ProtocolPhase::Report, err))?;
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
            let events = event_log
                .load()
                .map_err(|err| io_error(ProtocolPhase::Report, err))?;
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
            license::require_enterprise("Automated compliance reporting")
                .map_err(|err| license_error("Automated compliance reporting", err))?;
            let log_path = log.clone();
            let event_log = AppendOnlyEventLog::open(log);
            let events = event_log
                .load()
                .map_err(|err| io_error(ProtocolPhase::Report, err))?;
            let state = ReplayEngine::replay(&events)?;
            let namespaces: std::collections::BTreeSet<&str> = events
                .iter()
                .map(|event| event.namespace.as_str())
                .collect();
            let write_count = events
                .iter()
                .filter(|event| event.event_type == EventType::StateWrite)
                .count();
            let delete_count = events
                .iter()
                .filter(|event| event.event_type == EventType::StateDelete)
                .count();
            let report = serde_json::json!({
                "schema_version": "1.0",
                "log": log_path,
                "verification": "passed",
                "event_count": events.len(),
                "namespaces": namespaces,
                "state_writes": write_count,
                "state_deletes": delete_count,
                "root_digest": state.root_digest_hex().map_err(|err| ProtocolError::from_message(ProtocolPhase::Report, ProtocolAction::Halt, err))?,
                "digest_algorithm": "SHA-256"
            });
            println!(
                "{}",
                to_canonical_string(&report).map_err(|err| {
                    ProtocolError::new(
                        ProtocolErrorReason::IoError,
                        ProtocolPhase::Report,
                        ProtocolAction::Halt,
                        err,
                    )
                })?
            );
        }
    }

    Ok(())
}

#[allow(clippy::result_large_err)]
fn run_example(log_path: PathBuf) -> ProtocolResult<()> {
    cleanup_log_artifacts(&log_path).map_err(|err| io_error(ProtocolPhase::Append, err))?;

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
    )
    .map_err(|err| {
        ProtocolError::from_message(ProtocolPhase::Append, ProtocolAction::Reject, err)
    })?;
    log.append(&first)
        .map_err(|err| io_error(ProtocolPhase::Append, err))?;

    let second = Event::state_write(
        1,
        0,
        "tenant-a",
        "tenant-a:job",
        BsmValue::String("running".to_string()),
        false,
        first.digest.clone(),
        None,
    )
    .map_err(|err| {
        ProtocolError::from_message(ProtocolPhase::Append, ProtocolAction::Reject, err)
    })?;
    log.append(&second)
        .map_err(|err| io_error(ProtocolPhase::Append, err))?;

    let seal_state = ReplayEngine::replay(
        &log.load()
            .map_err(|err| io_error(ProtocolPhase::Replay, err))?,
    )?;
    let root_digest = seal_state.root_digest_hex().map_err(|err| {
        ProtocolError::from_message(ProtocolPhase::Replay, ProtocolAction::Halt, err)
    })?;
    let seal = Event::tick_seal(2, 0, "tenant-a", 2, root_digest, second.digest.clone(), 0)
        .map_err(|err| {
            ProtocolError::from_message(ProtocolPhase::Append, ProtocolAction::Reject, err)
        })?;
    log.append(&seal)
        .map_err(|err| io_error(ProtocolPhase::Append, err))?;

    let state = ReplayEngine::replay(
        &log.load()
            .map_err(|err| io_error(ProtocolPhase::Replay, err))?,
    )?;
    let state_json = serde_json::to_value(state.to_json_value()).map_err(|err| {
        ProtocolError::new(
            ProtocolErrorReason::IoError,
            ProtocolPhase::Replay,
            ProtocolAction::Halt,
            err.to_string(),
        )
    })?;

    println!("log={}", log.path().display());
    println!(
        "state={}",
        to_canonical_string(&state_json).map_err(|err| {
            ProtocolError::new(
                ProtocolErrorReason::IoError,
                ProtocolPhase::Replay,
                ProtocolAction::Halt,
                err,
            )
        })?
    );

    Ok(())
}

fn namespaced_key(namespace: &str, key: &str) -> String {
    let expected = format!("{namespace}:");
    if key.starts_with(&expected) {
        key.to_string()
    } else {
        format!("{expected}{key}")
    }
}

fn report_failure(err: &ProtocolError) {
    let value = err.to_json_value();
    if let Ok(rendered) = to_canonical_string(&value) {
        eprintln!("{rendered}");
    } else {
        eprintln!("{err}");
    }
}

fn io_error(phase: ProtocolPhase, err: impl ToString) -> ProtocolError {
    ProtocolError::new(
        ProtocolErrorReason::IoError,
        phase,
        ProtocolAction::Halt,
        err.to_string(),
    )
}

fn license_error(feature: &str, err: impl ToString) -> ProtocolError {
    ProtocolError::new(
        ProtocolErrorReason::LicenseRequired,
        ProtocolPhase::License,
        ProtocolAction::Reject,
        format!("{feature}: {}", err.to_string()),
    )
}

fn emit_protocol_error(
    log: &AppendOnlyEventLog,
    namespace_hint: &str,
    tick: u64,
    err: &ProtocolError,
) {
    let Ok(tail) = log.tail_metadata() else {
        return;
    };

    let namespace = if validate_runtime_namespace(namespace_hint).is_ok() {
        namespace_hint.to_string()
    } else if let Some(existing) = tail.namespace {
        existing
    } else {
        RESERVED_SYSTEM_NAMESPACE.to_string()
    };

    let detail = Some(err.message.clone());
    let offending_seq = err.offending_seq.or(err.seq);
    let event = Event::protocol_error(
        tail.next_seq,
        tick,
        namespace,
        err.code().to_string(),
        offending_seq,
        detail,
        tail.prev_digest,
    );
    if let Ok(event) = event {
        let _ = log.append(&event);
    }
}

fn cleanup_log_artifacts(log_path: &Path) -> Result<(), std::io::Error> {
    let artifacts = [
        log_path.to_path_buf(),
        PathBuf::from(format!("{}.catalog.json", log_path.display())),
        PathBuf::from(format!("{}.catalog.tmp", log_path.display())),
        PathBuf::from(format!("{}.lock", log_path.display())),
    ];

    for artifact in artifacts {
        if artifact.exists() {
            if let Err(err) = std::fs::remove_file(artifact) {
                if err.kind() != std::io::ErrorKind::NotFound {
                    return Err(err);
                }
            }
        }
    }

    let segments_dir = PathBuf::from(format!("{}.segments", log_path.display()));
    if segments_dir.exists() {
        if let Err(err) = std::fs::remove_dir_all(segments_dir) {
            if err.kind() != std::io::ErrorKind::NotFound {
                return Err(err);
            }
        }
    }

    Ok(())
}
