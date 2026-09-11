use std::error::Error;
use std::path::Path;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use serde_json::json;
use tri_sync::backend::{EventLogBackend, FileSystemBackend};
use tri_sync::canonical_json::to_canonical_string;
use tri_sync::digest::sha256_hex;
use tri_sync::error::ProtocolViolationError;
use tri_sync::event::{Event, EventType, ZERO_DIGEST_HEX};
use tri_sync::hex::decode_hex;
use tri_sync::license::{self, LicenseError};
use tri_sync::replay::{ReplayCheckpoint, ReplayEngine};
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
        /// Resume verification from a trusted prior TICK_SEAL root digest
        /// checkpoint. The matching verified snapshot is loaded from the local
        /// checkpoint cache instead of replaying from genesis.
        #[arg(long)]
        checkpoint_root: Option<String>,
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

#[derive(Debug)]
enum CliError {
    Protocol(ProtocolViolationError),
    License(LicenseError),
    Message(String),
}

impl CliError {
    fn exit_code(&self) -> i32 {
        match self {
            Self::Protocol(error) => error.exit_code(),
            Self::License(error) => error.exit_code(),
            Self::Message(_) => 1,
        }
    }

    fn stderr_json(&self) -> String {
        match self {
            Self::Protocol(error) => error.to_stderr_json(),
            Self::License(error) => error.to_stderr_json(),
            Self::Message(message) => to_canonical_string(&json!({
                "code": "COMMAND_ERROR",
                "error_type": "CommandError",
                "exit_code": 1,
                "message": message,
            }))
            .unwrap_or_else(|_| format!(r#"{{"code":"COMMAND_ERROR","message":"{message}"}}"#)),
        }
    }
}

impl From<ProtocolViolationError> for CliError {
    fn from(value: ProtocolViolationError) -> Self {
        Self::Protocol(value)
    }
}

impl From<LicenseError> for CliError {
    fn from(value: LicenseError) -> Self {
        Self::License(value)
    }
}

impl From<String> for CliError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    if let Err(error) = run() {
        eprintln!("{}", error.stderr_json());
        std::process::exit(error.exit_code());
    }

    Ok(())
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
                license::require_enterprise_detailed("Commercial production execution")?;
            }
            let log = FileSystemBackend::open(log);
            let key = namespaced_key(&namespace, &key);
            let events = log.load()?;
            let seq = log.next_sequence()?;
            let prev = events
                .last()
                .map_or(ZERO_DIGEST_HEX.to_string(), |event| event.digest.clone());
            let event = Event::state_write(
                seq,
                tick,
                namespace,
                key,
                BsmValue::Bytes(value.into_bytes()),
                false,
                prev,
                None,
            )
            .map_err(ProtocolViolationError::from_message)?;
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
                license::require_enterprise_detailed("Commercial production execution")?;
            }
            let log = FileSystemBackend::open(log);
            let events = log.load()?;
            let seq = log.next_sequence()?;
            let prev = events
                .last()
                .map_or(ZERO_DIGEST_HEX.to_string(), |event| event.digest.clone());
            let key = namespaced_key(&namespace, &key);
            let event = Event::state_delete(seq, tick, namespace, key, None, true, prev)
                .map_err(ProtocolViolationError::from_message)?;
            log.append(&event)?;
            println!("appended STATE_DELETE at seq {}", event.seq);
        }
        Commands::Replay { log } => {
            let log = FileSystemBackend::open(log);
            let events = log.load()?;
            let state = ReplayEngine::replay(&events)?;
            let json_value = serde_json::to_value(state.to_json_value())
                .map_err(|err| CliError::Message(err.to_string()))?;
            println!(
                "{}",
                to_canonical_string(&json_value).map_err(CliError::Message)?
            );
        }
        Commands::Verify {
            log,
            checkpoint_root,
            seal,
            tick,
            namespace,
        } => {
            let log = FileSystemBackend::open(log);
            let log_path = log.path().to_path_buf();
            let events = log.load()?;

            // Warn when the log does not end with a TICK_SEAL checkpoint.
            let ends_with_seal = events
                .last()
                .map(|e| e.event_type == tri_sync::event::EventType::TickSeal)
                .unwrap_or(false);
            let (state, verified_events) = if let Some(checkpoint_root) = checkpoint_root.as_deref()
            {
                let (checkpoint_index, checkpoint) =
                    load_replay_checkpoint(log.path(), &events, checkpoint_root)?;
                let outcome = ReplayEngine::replay_with_checkpoint(
                    &events[checkpoint_index + 1..],
                    Some(checkpoint),
                )?;
                (
                    outcome.state,
                    events.len().saturating_sub(checkpoint_index + 1),
                )
            } else {
                let outcome = ReplayEngine::replay(&events)?;
                (outcome, events.len())
            };
            let digest = state
                .root_digest_hex()
                .map_err(ProtocolViolationError::from_message)?;
            println!("OK");
            println!("log={}", log_path.display());
            println!("events={}", events.len());
            println!("root_digest={digest}");
            if let Some(checkpoint_root) = checkpoint_root.as_deref() {
                println!("checkpoint_root={checkpoint_root}");
                println!("verified_events={verified_events}");
            }
            if !events.is_empty() && !ends_with_seal {
                eprintln!("WARNING: log does not end with a TICK_SEAL checkpoint");
            }

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
                )
                .map_err(ProtocolViolationError::from_message)?;
                log.append(&seal_event)?;
                persist_checkpoint_snapshot(log.path(), &seal_event, &state)?;
                println!("seal_seq={}", seal_event.seq);
                println!("seal_digest={}", seal_event.digest);
            } else if let Some(last_event) = events.last() {
                if last_event.event_type == EventType::TickSeal {
                    persist_checkpoint_snapshot(log.path(), last_event, &state)?;
                }
            }
        }
        Commands::Export { log, format } => {
            if format != "json" {
                return Err(CliError::Message(format!(
                    "Unsupported format '{}'. Only 'json' is supported.",
                    format
                )));
            }
            let log = FileSystemBackend::open(log);
            let events = log.load()?;
            let arr: Vec<serde_json::Value> = events
                .iter()
                .map(serde_json::to_value)
                .collect::<Result<_, _>>()
                .map_err(|err| CliError::Message(err.to_string()))?;
            let json =
                serde_json::to_value(arr).map_err(|err| CliError::Message(err.to_string()))?;
            println!("{}", to_canonical_string(&json).map_err(CliError::Message)?);
        }
        Commands::Digest { input } => {
            println!("{}", sha256_hex(input.as_bytes()));
        }
        Commands::Example { log } => {
            run_example(log)?;
        }
        Commands::Inspect { log } => {
            let event_log = FileSystemBackend::open(log);
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
            let event_log = FileSystemBackend::open(log);
            let log_path = event_log.path().to_path_buf();
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
            license::require_enterprise_detailed("Automated compliance reporting")?;
            let event_log = FileSystemBackend::open(log);
            let log_path = event_log.path().to_path_buf();
            let events = event_log.load()?;
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
                "root_digest": state.root_digest_hex().map_err(ProtocolViolationError::from_message)?,
                "digest_algorithm": "SHA-256"
            });
            println!(
                "{}",
                to_canonical_string(&report).map_err(CliError::Message)?
            );
        }
    }

    Ok(())
}

fn run_example(log_path: PathBuf) -> Result<(), CliError> {
    cleanup_log_artifacts(&log_path).map_err(|err| CliError::Message(err.to_string()))?;

    let log = FileSystemBackend::open(&log_path);

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
    .map_err(ProtocolViolationError::from_message)?;
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
    )
    .map_err(ProtocolViolationError::from_message)?;
    log.append(&second)?;

    let seal_state = ReplayEngine::replay(&log.load()?)?;
    let root_digest = seal_state
        .root_digest_hex()
        .map_err(ProtocolViolationError::from_message)?;
    let seal = Event::tick_seal(2, 0, "tenant-a", 2, root_digest, second.digest.clone(), 0)
        .map_err(ProtocolViolationError::from_message)?;
    log.append(&seal)?;
    persist_checkpoint_snapshot(log.path(), &seal, &seal_state)?;

    let state = ReplayEngine::replay(&log.load()?)?;
    let state_json = serde_json::to_value(state.to_json_value())
        .map_err(|err| CliError::Message(err.to_string()))?;

    println!("log={}", log.path().display());
    println!(
        "state={}",
        to_canonical_string(&state_json).map_err(CliError::Message)?
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

fn load_replay_checkpoint(
    log_path: &Path,
    events: &[Event],
    checkpoint_root: &str,
) -> Result<(usize, ReplayCheckpoint), CliError> {
    let matching_seals = events
        .iter()
        .enumerate()
        .filter(|(_, event)| {
            event.event_type == EventType::TickSeal
                && event.root_digest.as_deref() == Some(checkpoint_root)
        })
        .collect::<Vec<_>>();

    if matching_seals.is_empty() {
        return Err(ProtocolViolationError::missing_tick_seal(
            Some(checkpoint_root.to_string()),
            format!(
                "MISSING_TICK_SEAL: no TICK_SEAL with root_digest {} was found in {}",
                checkpoint_root,
                log_path.display()
            ),
        )
        .into());
    }

    if matching_seals.len() > 1 {
        return Err(ProtocolViolationError::state_mismatch(
            None,
            Some(checkpoint_root.to_string()),
            None,
            Some(checkpoint_root.to_string()),
            format!(
                "STATE_MISMATCH: checkpoint root {} is ambiguous; found {} matching TICK_SEAL events in {}",
                checkpoint_root,
                matching_seals.len(),
                log_path.display()
            ),
        )
        .into());
    }

    let (index, seal_event) = matching_seals.into_iter().next().ok_or_else(|| {
        ProtocolViolationError::missing_tick_seal(
            Some(checkpoint_root.to_string()),
            format!(
                "MISSING_TICK_SEAL: no TICK_SEAL with root_digest {} was found in {}",
                checkpoint_root,
                log_path.display()
            ),
        )
    })?;

    seal_event
        .validate_digest()
        .map_err(ProtocolViolationError::from_message)?;

    let expected_prev_digest = if index == 0 {
        ZERO_DIGEST_HEX.to_string()
    } else {
        events[index - 1].digest.clone()
    };
    seal_event
        .validate_prev_digest(&expected_prev_digest)
        .map_err(ProtocolViolationError::from_message)?;

    if seal_event.seq != index as u64 {
        return Err(ProtocolViolationError::state_mismatch(
            Some(seal_event.seq),
            Some(checkpoint_root.to_string()),
            None,
            Some(checkpoint_root.to_string()),
            format!(
                "STATE_MISMATCH: checkpoint seal seq {} does not match its log position {}",
                seal_event.seq, index
            ),
        )
        .into());
    }

    if seal_event.event_count != Some(index as u32) {
        return Err(ProtocolViolationError::invalid_event_format(
            Some(seal_event.seq),
            format!(
                "TICK_SEAL at seq {} reports event_count {:?}, expected {}",
                seal_event.seq, seal_event.event_count, index
            ),
        )
        .into());
    }

    let snapshot_path = checkpoint_snapshot_path(log_path, checkpoint_root);
    let bytes = std::fs::read(&snapshot_path).map_err(|err| {
        ProtocolViolationError::state_mismatch(
            Some(seal_event.seq),
            Some(checkpoint_root.to_string()),
            None,
            Some(checkpoint_root.to_string()),
            format!(
                "STATE_MISMATCH: no verified snapshot cache found at {} for checkpoint {}: {err}",
                snapshot_path.display(),
                checkpoint_root
            ),
        )
    })?;
    let snapshot =
        StateSnapshot::from_binary(&bytes).map_err(ProtocolViolationError::from_message)?;
    let checkpoint = ReplayCheckpoint::from_tick_seal(seal_event, snapshot)?;
    Ok((index, checkpoint))
}

fn persist_checkpoint_snapshot(
    log_path: &Path,
    seal_event: &Event,
    state: &BinaryStateMap,
) -> Result<(), CliError> {
    if seal_event.event_type != EventType::TickSeal {
        return Ok(());
    }

    let root_digest = seal_event.root_digest.as_ref().ok_or_else(|| {
        ProtocolViolationError::invalid_event_format(
            Some(seal_event.seq),
            format!("TICK_SEAL missing root_digest at seq {}", seal_event.seq),
        )
    })?;
    let root_bytes = decode_hex(root_digest).map_err(ProtocolViolationError::from_message)?;
    let root_array: [u8; 32] = root_bytes.try_into().map_err(|_| {
        ProtocolViolationError::invalid_event_format(
            Some(seal_event.seq),
            format!("root digest {} must decode to 32 bytes", root_digest),
        )
    })?;
    let seal_digest_bytes =
        decode_hex(&seal_event.digest).map_err(ProtocolViolationError::from_message)?;
    let seal_digest: [u8; 32] = seal_digest_bytes.try_into().map_err(|_| {
        ProtocolViolationError::invalid_event_format(
            Some(seal_event.seq),
            format!("seal digest {} must decode to 32 bytes", seal_event.digest),
        )
    })?;
    let seal_timestamp_ms = seal_event.timestamp_ms.ok_or_else(|| {
        ProtocolViolationError::invalid_event_format(
            Some(seal_event.seq),
            format!("TICK_SEAL missing timestamp_ms at seq {}", seal_event.seq),
        )
    })?;

    let snapshot = StateSnapshot {
        namespace: seal_event.namespace.clone(),
        tick: seal_event.tick,
        seal_seq: seal_event.seq,
        seal_timestamp_ms,
        root_digest: root_array,
        seal_digest,
        state: state.clone(),
    };
    let bytes = snapshot
        .to_binary()
        .map_err(ProtocolViolationError::from_message)?;
    let snapshot_path = checkpoint_snapshot_path(log_path, root_digest);
    if let Some(parent) = snapshot_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| CliError::Message(err.to_string()))?;
    }
    std::fs::write(&snapshot_path, bytes).map_err(|err| CliError::Message(err.to_string()))?;
    Ok(())
}

fn checkpoint_snapshot_path(log_path: &Path, checkpoint_root: &str) -> PathBuf {
    PathBuf::from(format!("{}.snapshots", log_path.display()))
        .join(format!("{checkpoint_root}.bsm"))
}

fn cleanup_log_artifacts(log_path: &Path) -> Result<(), std::io::Error> {
    let artifacts = [
        log_path.to_path_buf(),
        PathBuf::from(format!("{}.catalog.json", log_path.display())),
        PathBuf::from(format!("{}.catalog.tmp", log_path.display())),
        PathBuf::from(format!("{}.lock", log_path.display())),
        PathBuf::from(format!("{}.legacy.jsonl", log_path.display())),
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
        if let Err(err) = std::fs::remove_dir_all(&segments_dir) {
            if err.kind() != std::io::ErrorKind::NotFound {
                return Err(err);
            }
        }
    }

    let snapshots_dir = PathBuf::from(format!("{}.snapshots", log_path.display()));
    if snapshots_dir.exists() {
        if let Err(err) = std::fs::remove_dir_all(&snapshots_dir) {
            if err.kind() != std::io::ErrorKind::NotFound {
                return Err(err);
            }
        }
    }

    Ok(())
}
