use std::error::Error;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use tri_sync::canonical_json::to_canonical_string;
use tri_sync::digest::sha256_hex;
use tri_sync::event::Event;
use tri_sync::event_log::AppendOnlyEventLog;
use tri_sync::replay::ReplayEngine;

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
        tenant: String,
        #[arg(long)]
        key: String,
        #[arg(long)]
        value: String,
    },
    Delete {
        #[arg(long)]
        log: PathBuf,
        #[arg(long)]
        tenant: String,
        #[arg(long)]
        key: String,
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
            tenant,
            key,
            value,
        } => {
            let log = AppendOnlyEventLog::open(log);
            let sequence = log.next_sequence()?;
            let event = Event::new_set(sequence, tenant, key, value.as_bytes());
            log.append(&event)?;
            println!("appended set event at sequence {}", event.sequence);
        }
        Commands::Delete { log, tenant, key } => {
            let log = AppendOnlyEventLog::open(log);
            let sequence = log.next_sequence()?;
            let event = Event::new_delete(sequence, tenant, key);
            log.append(&event)?;
            println!("appended delete event at sequence {}", event.sequence);
        }
        Commands::Replay { log } => {
            let log = AppendOnlyEventLog::open(log);
            let events = log.load()?;
            let state = ReplayEngine::replay(&events).map_err(|err| std::io::Error::other(err))?;
            println!("{}", to_canonical_string(&state.to_nested_hex_json())?);
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
    log.append(&Event::new_set(0, "tenant-a", "job", b"queued"))?;
    log.append(&Event::new_set(1, "tenant-a", "job", b"running"))?;
    log.append(&Event::new_set(2, "tenant-b", "job", b"queued"))?;
    log.append(&Event::new_delete(3, "tenant-b", "job"))?;

    let state = ReplayEngine::replay(&log.load()?).map_err(std::io::Error::other)?;
    println!("log={}", log.path().display());
    println!(
        "state={}",
        to_canonical_string(&state.to_nested_hex_json())?
    );

    Ok(())
}
