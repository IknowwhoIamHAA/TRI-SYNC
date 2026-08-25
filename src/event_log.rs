use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::canonical_json::to_canonical_string;
use crate::digest::sha256_hex;
use crate::event::{Event, ZERO_DIGEST_HEX};

const SEGMENT_PREFIX: &str = "#SEGMENT ";
const PROTOCOL_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentHeader {
    pub segment_id: String,
    pub namespace: String,
    pub seq_start: u64,
    pub seq_end: u64,
    pub first_digest: String,
    pub prev_segment: Option<String>,
    pub created_at: u64,
    pub protocol_ver: String,
}

impl SegmentHeader {
    pub fn digest_hex(&self) -> Result<String, String> {
        let value = serde_json::to_value(self).map_err(|err| err.to_string())?;
        let canonical = to_canonical_string(&value)?;
        Ok(sha256_hex(canonical.as_bytes()))
    }
}

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
        let last_event = self.load_last_event()?;

        let expected_seq = last_event.as_ref().map_or(0, |last| last.seq + 1);
        if event.seq != expected_seq {
            return Err(
                format!("SEQ_GAP: expected seq {}, got {}", expected_seq, event.seq).into(),
            );
        }

        let expected_prev = last_event
            .as_ref()
            .map_or(ZERO_DIGEST_HEX.to_string(), |last| last.digest.clone());
        event.validate_prev_digest(&expected_prev)?;
        event.validate_digest()?;

        if let Some(last) = &last_event
            && last.namespace != event.namespace
        {
            return Err("NAMESPACE_LEAK: mixed namespaces in one log file".into());
        }

        if last_event.is_none() {
            self.write_header(event)?;
        }

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
            if line.is_empty() || line.starts_with(SEGMENT_PREFIX) {
                continue;
            }
            events.push(serde_json::from_str::<Event>(line)?);
        }
        Ok(events)
    }

    pub fn load_header(&self) -> Result<Option<SegmentHeader>, Box<dyn Error>> {
        if !self.path.exists() {
            return Ok(None);
        }

        let file = File::open(&self.path)?;
        for line in BufReader::new(file).lines() {
            let line = line?;
            if let Some(payload) = line.strip_prefix(SEGMENT_PREFIX) {
                return Ok(Some(serde_json::from_str(payload)?));
            }
            if !line.trim().is_empty() {
                break;
            }
        }
        Ok(None)
    }

    pub fn next_sequence(&self) -> Result<u64, Box<dyn Error>> {
        Ok(self.load_last_event()?.map_or(0, |event| event.seq + 1))
    }

    fn load_last_event(&self) -> Result<Option<Event>, Box<dyn Error>> {
        if !self.path.exists() {
            return Ok(None);
        }

        let file = File::open(&self.path)?;
        let mut last_event = None;
        for line in BufReader::new(file).lines() {
            let line = line?;
            let line = line.trim();
            if line.is_empty() || line.starts_with(SEGMENT_PREFIX) {
                continue;
            }
            last_event = Some(serde_json::from_str::<Event>(line)?);
        }
        Ok(last_event)
    }

    fn write_header(&self, first_event: &Event) -> Result<(), Box<dyn Error>> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|err| err.to_string())?;
        let header = SegmentHeader {
            segment_id: format!("seg-{}-{}", first_event.seq, now.as_nanos()),
            namespace: first_event.namespace.clone(),
            seq_start: first_event.seq,
            seq_end: first_event.seq,
            first_digest: first_event.digest.clone(),
            prev_segment: None,
            created_at: now.as_millis() as u64,
            protocol_ver: PROTOCOL_VERSION.to_string(),
        };

        let header_value = serde_json::to_value(&header)?;
        let header_canonical = to_canonical_string(&header_value)?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{SEGMENT_PREFIX}{header_canonical}")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::event::{Event, ZERO_DIGEST_HEX};
    use crate::state_map::BsmValue;

    use super::AppendOnlyEventLog;

    #[test]
    fn enforces_append_only_sequence_and_chain() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("current time should be later than epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tri-sync-log-{unique}.jsonl"));

        let log = AppendOnlyEventLog::open(&path);

        let first = Event::state_write(
            0,
            0,
            "tenant-a",
            "tenant-a:key",
            BsmValue::String("v1".to_string()),
            false,
            ZERO_DIGEST_HEX,
            None,
        )
        .expect("event create");
        log.append(&first).expect("append first should succeed");

        let second = Event::state_delete(
            1,
            0,
            "tenant-a",
            "tenant-a:key",
            None,
            true,
            first.digest.clone(),
        )
        .expect("event create");
        log.append(&second).expect("append second should succeed");

        let mut bad = second.clone();
        bad.seq = 3;
        bad.refresh_digest().expect("digest refresh");
        assert!(log.append(&bad).is_err());

        let _ = fs::remove_file(path);
    }
}
