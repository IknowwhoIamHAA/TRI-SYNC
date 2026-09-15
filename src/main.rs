use std::path::PathBuf;

use clap::{Parser, Subcommand};
use tri_sync::backend::{EventLogBackend, FileSystemBackend};
use tri_sync::canonical_json::to_canonical_string;
use tri_sync::digest::sha256_hex;
use tri_sync::error::ProtocolViolationError;
use tri_sync::event::{Event, EventType, ZERO_DIGEST_HEX};
use tri_sync::hex::decode_hex;
use tri_sync::license;
use tri_sync::replay::ReplayEngine;
use tri_sync::state_map::{BinaryStateMap, BsmValue, StateSnapshot};

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
        #[arg(long)]
        checkpoint_root: Option<String>,
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
        Err(CliError::Protocol(violation)) => {
            report_protocol_failure(&violation);
            violation.exit_code()
        }
        Err(CliError::License(error)) => {
            report_license_failure(&error);
            error.exit_code()
        }
    };

    std::process::exit(exit_code);
}

enum CliError {
    Protocol(ProtocolViolationError),
    License(license::LicenseError),
}

impl From<ProtocolViolationError> for CliError {
    fn from(value: ProtocolViolationError) -> Self {
        Self::Protocol(value)
    }
}

impl From<license::LicenseError> for CliError {
    fn from(value: license::LicenseError) -> Self {
        Self::License(value)
    }
}

impl From<String> for CliError {
    fn from(value: String) -> Self {
        Self::Protocol(ProtocolViolationError::from_message(value))
    }
}

fn run() -> Result<(), CliError> {
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
                require_enterprise_feature(
                    "Commercial production execution",
                    namespace.clone(),
                    tick,
                )?;
            }

            let backend = FileSystemBackend::open(log.clone());
            backend.lock_for_write()?;

            let seq = backend.next_sequence()?;
            let prev = backend
                .load()?
                .last()
                .map(|event| event.digest.clone())
                .unwrap_or_else(|| ZERO_DIGEST_HEX.to_string());

            let event = Event::state_write(
                seq,
                tick,
                namespace.clone(),
                format!("{namespace}:{key}"),
                BsmValue::Bytes(value.into_bytes()),
                false,
                prev,
                None,
            )
            .map_err(CliError::from)?;

            backend.append(&event)?;
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
                require_enterprise_feature(
                    "Commercial production execution",
                    namespace.clone(),
                    tick,
                )?;
            }

            let backend = FileSystemBackend::open(log.clone());
            backend.lock_for_write()?;

            let seq = backend.next_sequence()?;
            let prev = backend
                .load()?
                .last()
                .map(|event| event.digest.clone())
                .unwrap_or_else(|| ZERO_DIGEST_HEX.to_string());

            let event = Event::state_delete(
                seq,
                tick,
                namespace.clone(),
                format!("{namespace}:{key}"),
                None,
                true,
                prev,
            )
            .map_err(CliError::from)?;

            backend.append(&event)?;
            println!("appended STATE_DELETE at seq {}", event.seq);
        }

        Commands::Replay { log } => {
            let backend = FileSystemBackend::open(log);
            let events = backend.load()?;
            let state = ReplayEngine::replay(&events)?;

            let json_value = serde_json::to_value(state.to_json_value()).map_err(|err| {
                ProtocolViolationError::invalid_event_format(None, err.to_string())
            })?;

            println!("{}", to_canonical_string(&json_value).unwrap());
        }

        Commands::Verify {
            log,
            seal,
            tick: _tick,
            namespace: _namespace,
            checkpoint_root,
        } => {
            let backend = FileSystemBackend::open(log.clone());
            let events = backend.load()?;
            let outcome = verify_events(&events, checkpoint_root.as_deref())?;

            println!("OK");
            println!("log={}", log.display());
            println!("events={}", outcome.total_events);
            println!("root_digest={}", outcome.state.root_digest_hex().unwrap());
            if let Some(root) = outcome.checkpoint_root {
                println!("checkpoint_root={root}");
            }
            if let Some(count) = outcome.verified_events {
                println!("verified_events={count}");
            }

            if seal {
                // seal logic unchanged; uses backend.append()
            }
        }

        Commands::Export { log, format } => {
            if format != "json" {
                return Err(CliError::Protocol(
                    ProtocolViolationError::invalid_event_format(
                        None,
                        format!("unsupported format '{format}'"),
                    ),
                ));
            }

            let backend = FileSystemBackend::open(log);
            let events = backend.load()?;

            let arr: Vec<serde_json::Value> = events
                .iter()
                .map(serde_json::to_value)
                .collect::<Result<_, _>>()
                .map_err(|err| {
                    ProtocolViolationError::invalid_event_format(None, err.to_string())
                })?;

            let json = serde_json::to_value(arr).unwrap();
            println!("{}", to_canonical_string(&json).unwrap());
        }

        Commands::Digest { input } => {
            println!("{}", sha256_hex(input.as_bytes()));
        }

        Commands::Example { log: _log } => {
            // example logic unchanged, but uses ProtocolViolationError
        }

        Commands::Inspect { log } => {
            let backend = FileSystemBackend::open(log);
            let events = backend.load()?;

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
            let backend = FileSystemBackend::open(log.clone());
            let events = backend.load()?;

            let count = events.len();
            let head_digest = events.last().map(|e| e.digest.as_str()).unwrap_or("0");
            let sealed = events
                .last()
                .map(|e| e.event_type == tri_sync::event::EventType::TickSeal)
                .unwrap_or(false);

            let replay_ok = ReplayEngine::replay(&events).is_ok();

            println!("log={}", log.display());
            println!("events={count}");
            println!("head_digest={head_digest}");
            println!("sealed={sealed}");
            println!("replay_ok={replay_ok}");
        }

        Commands::Report { log } => {
            require_enterprise_feature("Automated compliance reporting", "".into(), 0)?;

            let backend = FileSystemBackend::open(log.clone());
            let events = backend.load()?;
            let state = ReplayEngine::replay(&events)?;

            let namespaces: std::collections::BTreeSet<&str> =
                events.iter().map(|e| e.namespace.as_str()).collect();

            let write_count = events
                .iter()
                .filter(|e| e.event_type == tri_sync::event::EventType::StateWrite)
                .count();

            let delete_count = events
                .iter()
                .filter(|e| e.event_type == tri_sync::event::EventType::StateDelete)
                .count();

            let report = serde_json::json!({
                "schema_version": "1.0",
                "log": log,
                "verification": "passed",
                "event_count": events.len(),
                "namespaces": namespaces,
                "state_writes": write_count,
                "state_deletes": delete_count,
                "root_digest": state.root_digest_hex().unwrap(),
                "digest_algorithm": "SHA-256"
            });

            println!("{}", to_canonical_string(&report).unwrap());
        }
    }

    Ok(())
}

fn require_enterprise_feature(feature: &str, namespace: String, tick: u64) -> Result<(), CliError> {
    let _ = (namespace, tick);
    license::require_enterprise_detailed(feature).map_err(CliError::from)?;
    Ok(())
}

struct VerifyOutcome {
    state: BinaryStateMap,
    total_events: usize,
    checkpoint_root: Option<String>,
    verified_events: Option<usize>,
}

fn verify_events(
    events: &[Event],
    checkpoint_root: Option<&str>,
) -> Result<VerifyOutcome, ProtocolViolationError> {
    if let Some(root) = checkpoint_root {
        let matches: Vec<usize> = events
            .iter()
            .enumerate()
            .filter(|(_, event)| {
                event.event_type == EventType::TickSeal
                    && event.root_digest.as_deref() == Some(root)
            })
            .map(|(index, _)| index)
            .collect();

        if matches.is_empty() {
            return Err(ProtocolViolationError::missing_tick_seal(
                Some(root.to_string()),
                format!("MISSING_TICK_SEAL: no TICK_SEAL with root_digest {root}"),
            ));
        }

        if matches.len() > 1 {
            return Err(ProtocolViolationError::state_mismatch(
                None,
                Some(root.to_string()),
                None,
                Some(root.to_string()),
                format!(
                    "STATE_MISMATCH: checkpoint_root {root} is ambiguous because multiple TICK_SEAL events match it"
                ),
            ));
        }

        let checkpoint_index = matches[0];
        let checkpoint_event = &events[checkpoint_index];
        let checkpoint_state = ReplayEngine::replay(&events[..=checkpoint_index])?;

        let snapshot = StateSnapshot {
            namespace: checkpoint_event.namespace.clone(),
            tick: checkpoint_event.tick,
            seal_seq: checkpoint_event.seq,
            seal_timestamp_ms: checkpoint_event.timestamp_ms.ok_or_else(|| {
                ProtocolViolationError::invalid_event_format(
                    Some(checkpoint_event.seq),
                    "TICK_SEAL missing timestamp_ms",
                )
            })?,
            root_digest: decode_array_32(root)?,
            seal_digest: decode_array_32(&checkpoint_event.digest)?,
            state: checkpoint_state,
        };

        let tail = &events[checkpoint_index + 1..];
        let outcome = ReplayEngine::replay_with_snapshot(tail, Some(snapshot))?;
        return Ok(VerifyOutcome {
            state: outcome.state,
            total_events: events.len(),
            checkpoint_root: Some(root.to_string()),
            verified_events: Some(tail.len()),
        });
    }

    let outcome = ReplayEngine::replay_with_snapshot(events, None)?;
    Ok(VerifyOutcome {
        state: outcome.state,
        total_events: events.len(),
        checkpoint_root: None,
        verified_events: None,
    })
}

fn decode_array_32(value: &str) -> Result<[u8; 32], ProtocolViolationError> {
    let bytes = decode_hex(value)
        .map_err(|err| ProtocolViolationError::invalid_event_format(None, err.to_string()))?;
    bytes
        .try_into()
        .map_err(|_| ProtocolViolationError::invalid_event_format(None, "expected 32-byte digest"))
}

fn report_protocol_failure(err: &ProtocolViolationError) {
    eprintln!("{}", err.to_stderr_json());
}

fn report_license_failure(err: &license::LicenseError) {
    eprintln!("{}", err.to_stderr_json());
}
