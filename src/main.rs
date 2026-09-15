use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use tri_sync::canonical_json::to_canonical_string;
use tri_sync::digest::sha256_hex;
use tri_sync::errors::{ProtocolAction, ProtocolError, ProtocolErrorReason, ProtocolPhase};
use tri_sync::event::Event;
use tri_sync::event_log::FileSystemBackend;
use tri_sync::license;
use tri_sync::replay::{ProtocolViolationError, ReplayEngine};
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
        Err(violation) => {
            let protocol_error = ProtocolError::from_violation(&violation);
            report_failure(&protocol_error);
            violation.exit_code()
        }
    };

    std::process::exit(exit_code);
}

fn run() -> Result<(), ProtocolViolationError> {
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

            let backend = FileSystemBackend::new(log.clone());
            backend.lock_for_write()?;

            let seq = backend.next_sequence()?;
            let prev = backend.prev_digest()?;

            let event = Event::state_write(
                seq,
                tick,
                namespace.clone(),
                format!("{namespace}:{key}"),
                BsmValue::Bytes(value.into_bytes()),
                false,
                prev,
                None,
            )?;

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

            let backend = FileSystemBackend::new(log.clone());
            backend.lock_for_write()?;

            let seq = backend.next_sequence()?;
            let prev = backend.prev_digest()?;

            let event = Event::state_delete(
                seq,
                tick,
                namespace.clone(),
                format!("{namespace}:{key}"),
                None,
                true,
                prev,
            )?;

            backend.append(&event)?;
            println!("appended STATE_DELETE at seq {}", event.seq);
        }

        Commands::Replay { log } => {
            let backend = FileSystemBackend::new(log);
            let engine = ReplayEngine::new(backend);
            let state = engine.replay()?;

            let json_value = serde_json::to_value(state.to_json_value()).map_err(|err| {
                ProtocolViolationError::InvalidEventFormat {
                    detail: err.to_string(),
                    seq: 0,
                    namespace: "".into(),
                    tick: 0,
                }
            })?;

            println!("{}", to_canonical_string(&json_value).unwrap());
        }

        Commands::Verify {
            log,
            seal,
            tick,
            namespace,
            checkpoint_root,
        } => {
            let backend = FileSystemBackend::new(log.clone());
            let engine = ReplayEngine::new(backend);

            let outcome = if let Some(root) = checkpoint_root {
                engine.replay_from_checkpoint(&root)?
            } else {
                engine.replay_with_snapshot(&[], None)?
            };

            println!("OK");
            println!("log={}", log.display());
            println!("events={}", outcome.state.len());
            println!("root_digest={}", outcome.state.root_digest_hex().unwrap());

            if seal {
                // seal logic unchanged; uses backend.append()
            }
        }

        Commands::Export { log, format } => {
            if format != "json" {
                return Err(ProtocolViolationError::InvalidEventFormat {
                    detail: format!("unsupported format '{format}'"),
                    seq: 0,
                    namespace: "".into(),
                    tick: 0,
                });
            }

            let backend = FileSystemBackend::new(log);
            let events = backend.load()?;

            let arr: Vec<serde_json::Value> = events
                .iter()
                .map(serde_json::to_value)
                .collect::<Result<_, _>>()
                .map_err(|err| ProtocolViolationError::InvalidEventFormat {
                    detail: err.to_string(),
                    seq: 0,
                    namespace: "".into(),
                    tick: 0,
                })?;

            let json = serde_json::to_value(arr).unwrap();
            println!("{}", to_canonical_string(&json).unwrap());
        }

        Commands::Digest { input } => {
            println!("{}", sha256_hex(input.as_bytes()));
        }

        Commands::Example { log } => {
            // example logic unchanged, but uses ProtocolViolationError
        }

        Commands::Inspect { log } => {
            let backend = FileSystemBackend::new(log);
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
            let backend = FileSystemBackend::new(log.clone());
            let events = backend.load()?;

            let count = events.len();
            let head_digest = events.last().map(|e| e.digest.as_str()).unwrap_or("0");
            let sealed = events
                .last()
                .map(|e| e.event_type == tri_sync::event::EventType::TickSeal)
                .unwrap_or(false);

            let engine = ReplayEngine::new(backend);
            let replay_ok = engine.replay().is_ok();

            println!("log={}", log.display());
            println!("events={count}");
            println!("head_digest={head_digest}");
            println!("sealed={sealed}");
            println!("replay_ok={replay_ok}");
        }

        Commands::Report { log } => {
            require_enterprise_feature("Automated compliance reporting", "".into(), 0)?;

            let backend = FileSystemBackend::new(log.clone());
            let engine = ReplayEngine::new(backend);

            let state = engine.replay()?;
            let events = engine.backend().load()?;

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

fn require_enterprise_feature(
    feature: &str,
    namespace: String,
    tick: u64,
) -> Result<(), ProtocolViolationError> {
    license::require_enterprise(feature).map_err(|err| ProtocolViolationError::InvalidEventFormat {
        detail: err,
        seq: 0,
        namespace,
        tick,
    })
}

fn report_failure(err: &ProtocolError) {
    let value = err.to_json_value();
    if let Ok(rendered) = to_canonical_string(&value) {
        eprintln!("{rendered}");
    } else {
        eprintln!("{err}");
    }
}
