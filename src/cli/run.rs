use std::path::PathBuf;
use anyhow::{Result, Context};
use crate::workflow::{Workflow, Step, Op};
use crate::event::Event;
use crate::event_log::AppendOnlyEventLog;
use crate::replay::ReplayEngine;
use crate::state_map::BinaryStateMap;

pub fn run_workflow(path: PathBuf, log_path: PathBuf) -> Result<()> {
    // 1. Load existing event log
    let mut log = AppendOnlyEventLog::open(log_path.clone())
        .context("Failed to open event log")?;

    let events = log.load_events()
        .context("Failed to load events")?;

    // 2. Replay deterministically
    let mut replay = ReplayEngine::new();
    replay.apply_all(&events)
        .context("Replay failed")?;

    let mut state = replay.into_state();

    // 3. Load workflow JSON
    let workflow: Workflow = {
        let data = std::fs::read_to_string(&path)
            .context("Failed to read workflow file")?;
        serde_json::from_str(&data)
            .context("Failed to parse workflow JSON")?
    };

    // 4. Execute steps → events
    for step in workflow.steps {
        for op in step.ops {
            let event = op.to_event(step.tenant.clone(), &state)
                .context("Failed to convert op to event")?;

            // 5. Append event to log
            log.append(&event)
                .context("Failed to append event")?;

            // 6. Apply event to state
            state.apply_event(&event)
                .context("Failed to apply event")?;
        }
    }

    // 7. Emit final state snapshot
    let snapshot = state.to_json();
    println!("{}", snapshot);

    Ok(())
}
