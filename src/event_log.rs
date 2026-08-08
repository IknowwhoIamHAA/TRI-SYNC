use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::canonical_json::to_canonical_string;
use crate::event::Event;

#[derive(Debug, Clone)]
pub struct AppendOnlyEventLog {
    path: PathBuf,
}

impl AppendOnlyEventLog {
    pub fn open(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, event: &Event) -> Result<(), Box<dyn Error>> {
        let value = serde_json::to_value(event)?;
        let canonical = to_canonical_string(&value)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{canonical}")?;
        Ok(())
    }

    pub fn load(&self) -> Result<Vec<Event>, Box<dyn Error>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&self.path)?;
        let mut events = Vec::new();
        for line in BufReader::new(file).lines() {
            let line = line?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            events.push(serde_json::from_str::<Event>(line)?);
        }
        Ok(events)
    }

    pub fn next_sequence(&self) -> Result<u64, Box<dyn Error>> {
        Ok(self.load()?.last().map_or(0, |event| event.sequence + 1))
    }
}
