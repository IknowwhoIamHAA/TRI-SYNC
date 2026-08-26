use std::error::Error;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use tri_sync::canonical_json::to_canonical_string;
use tri_sync::digest::sha256_hex;
use tri_sync::event::{Event, ZERO_DIGEST_HEX};
use tri_sync::event_log::AppendOnlyEventLog;
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
    },
    Replay {
        #[arg(long)]
        log: PathBuf,
    },
    Digest {
        #[arg(long)]
        input: String,
    },
    Example {
        #[arg(long)]
        log: PathBuf,
    },
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Apply {
            log,
            namespace,
            key,
            value,
            tick,
        } => {
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
        } => {
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
        Commands::Digest { input } => {
            println!("{}", sha256_hex(input.as_bytes()));
        }
        Commands::Example { log } => {
            run_example(log)?;
        }
    }

    Ok(())
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

fn namespaced_key(namespace: &str, key: &str) -> String {
    let expected = format!("{namespace}:");
    if key.starts_with(&expected) {
        key.to_string()
    } else {
        format!("{expected}{key}")
    }
}
