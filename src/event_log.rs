use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::canonical_json::to_canonical_string;
use crate::digest::sha256_hex;
use crate::error::ProtocolViolationError;
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

    /// Append an event to the log.
    ///
    /// An exclusive OS-level file lock is held for the entire duration of the
    /// operation, preventing concurrent writers from interleaving events or
    /// corrupting the chain.  The `SegmentHeader`'s `seq_end` field is updated
    /// to reflect the new last sequence number after every successful append.
    pub fn append(&self, event: &Event) -> Result<(), ProtocolViolationError> {
        self.with_write_lock(|log| log.append_under_lock(event))
    }

    pub fn lock_for_write(&self) -> Result<(), ProtocolViolationError> {
        self.with_write_lock(|_| Ok(()))
    }

    fn with_write_lock<T>(
        &self,
        operation: impl FnOnce(&Self) -> Result<T, ProtocolViolationError>,
    ) -> Result<T, ProtocolViolationError> {
        let lock_path = self.path.with_extension("lock");
        let lock_file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|err| {
                ProtocolViolationError::invalid_event_format(
                    None,
                    format!("failed to open lock file {}: {err}", lock_path.display()),
                )
            })?;
        lock_file.lock_exclusive().map_err(|err| {
            ProtocolViolationError::invalid_event_format(
                None,
                format!("failed to acquire lock for {}: {err}", self.path.display()),
            )
        })?;

        let result = operation(self);
        drop(lock_file);
        result
    }

    fn append_under_lock(&self, event: &Event) -> Result<(), ProtocolViolationError> {
        let last_event = self.load_last_event()?;

        let expected_seq = last_event.as_ref().map_or(0, |last| last.seq + 1);
        if event.seq != expected_seq {
            return Err(ProtocolViolationError::SequenceGap {
                expected_seq,
                actual_seq: event.seq,
            });
        }

        let expected_prev = last_event
            .as_ref()
            .map_or(ZERO_DIGEST_HEX.to_string(), |last| last.digest.clone());
        event
            .validate_prev_digest(&expected_prev)
            .map_err(ProtocolViolationError::from_message)?;
        event
            .validate_digest()
            .map_err(ProtocolViolationError::from_message)?;

        if let Some(last) = &last_event {
            if last.namespace != event.namespace {
                return Err(ProtocolViolationError::namespace_breach(
                    Some(last.namespace.clone()),
                    Some(event.namespace.clone()),
                    event.key.clone(),
                    format!(
                        "NAMESPACE_LEAK: mixed namespaces in one log file (expected {}, got {})",
                        last.namespace, event.namespace
                    ),
                ));
            }
        }

        if last_event.is_none() {
            self.write_header(event)?;
        }

        let value = serde_json::to_value(event).map_err(|err| {
            ProtocolViolationError::invalid_event_format(
                Some(event.seq),
                format!("failed to serialize event at seq {}: {err}", event.seq),
            )
        })?;
        let canonical = to_canonical_string(&value)
            .map_err(|err| ProtocolViolationError::invalid_event_format(Some(event.seq), err))?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{canonical}")?;
        drop(file);

        // Fix 8: update seq_end in the segment header after every successful append.
        self.update_header_seq_end(event.seq)?;

        Ok(())
    }

    pub fn load(&self) -> Result<Vec<Event>, ProtocolViolationError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&self.path).map_err(|err| {
            ProtocolViolationError::invalid_event_format(
                None,
                format!("failed to open log {}: {err}", self.path.display()),
            )
        })?;
        let mut events = Vec::new();
        for line in BufReader::new(file).lines() {
            let line = line.map_err(|err| {
                ProtocolViolationError::invalid_event_format(
                    None,
                    format!("failed to read log {}: {err}", self.path.display()),
                )
            })?;
            let line = line.trim();
            if line.is_empty() || line.starts_with(SEGMENT_PREFIX) {
                continue;
            }
            let event = serde_json::from_str::<Event>(line).map_err(|err| {
                ProtocolViolationError::invalid_event_format(
                    None,
                    format!("failed to parse event log line as JSON event: {err}"),
                )
            })?;
            events.push(event);
        }
        Ok(events)
    }

    pub fn load_header(&self) -> Result<Option<SegmentHeader>, ProtocolViolationError> {
        if !self.path.exists() {
            return Ok(None);
        }

        let file = File::open(&self.path).map_err(|err| {
            ProtocolViolationError::invalid_event_format(
                None,
                format!("failed to open log header {}: {err}", self.path.display()),
            )
        })?;
        for line in BufReader::new(file).lines() {
            let line = line.map_err(|err| {
                ProtocolViolationError::invalid_event_format(
                    None,
                    format!("failed to read log header {}: {err}", self.path.display()),
                )
            })?;
            if let Some(payload) = line.strip_prefix(SEGMENT_PREFIX) {
                let header = serde_json::from_str(payload).map_err(|err| {
                    ProtocolViolationError::invalid_event_format(
                        None,
                        format!("failed to parse segment header: {err}"),
                    )
                })?;
                return Ok(Some(header));
            }
            if !line.trim().is_empty() {
                break;
            }
        }
        Ok(None)
    }

    pub fn next_sequence(&self) -> Result<u64, ProtocolViolationError> {
        Ok(self.load_last_event()?.map_or(0, |event| event.seq + 1))
    }

    fn load_last_event(&self) -> Result<Option<Event>, ProtocolViolationError> {
        if !self.path.exists() {
            return Ok(None);
        }

        let file = File::open(&self.path).map_err(|err| {
            ProtocolViolationError::invalid_event_format(
                None,
                format!("failed to open log {}: {err}", self.path.display()),
            )
        })?;
        let mut last_event = None;
        for line in BufReader::new(file).lines() {
            let line = line.map_err(|err| {
                ProtocolViolationError::invalid_event_format(
                    None,
                    format!("failed to read log {}: {err}", self.path.display()),
                )
            })?;
            let line = line.trim();
            if line.is_empty() || line.starts_with(SEGMENT_PREFIX) {
                continue;
            }
            last_event = Some(serde_json::from_str::<Event>(line).map_err(|err| {
                ProtocolViolationError::invalid_event_format(
                    None,
                    format!("failed to parse event log line as JSON event: {err}"),
                )
            })?);
        }
        Ok(last_event)
    }

    fn write_header(&self, first_event: &Event) -> Result<(), ProtocolViolationError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|err| {
                ProtocolViolationError::invalid_event_format(
                    Some(first_event.seq),
                    format!("system clock error while writing header: {err}"),
                )
            })?;
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

        let header_value = serde_json::to_value(&header).map_err(|err| {
            ProtocolViolationError::invalid_event_format(
                Some(first_event.seq),
                format!("failed to serialize segment header: {err}"),
            )
        })?;
        let header_canonical = to_canonical_string(&header_value).map_err(|err| {
            ProtocolViolationError::invalid_event_format(Some(first_event.seq), err)
        })?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{SEGMENT_PREFIX}{header_canonical}")?;

        Ok(())
    }

    /// Fix 8: rewrite the `#SEGMENT` header line with the updated `seq_end`.
    ///
    /// Reads the entire file, replaces the first `#SEGMENT` line with a new
    /// header containing the updated `seq_end`, then atomically rewrites the
    /// file via a temporary sibling.
    fn update_header_seq_end(&self, new_seq_end: u64) -> Result<(), ProtocolViolationError> {
        if !self.path.exists() {
            return Err(ProtocolViolationError::invalid_event_format(
                Some(new_seq_end),
                format!(
                    "log file {} disappeared after append; seq_end not updated",
                    self.path.display()
                ),
            ));
        }

        let content = std::fs::read_to_string(&self.path).map_err(|err| {
            ProtocolViolationError::invalid_event_format(
                Some(new_seq_end),
                format!(
                    "failed to read segment header file {}: {err}",
                    self.path.display()
                ),
            )
        })?;
        let mut new_content = String::with_capacity(content.len());
        let mut updated = false;

        for line in content.lines() {
            if !updated {
                if let Some(payload) = line.strip_prefix(SEGMENT_PREFIX) {
                    let mut header: SegmentHeader =
                        serde_json::from_str(payload).map_err(|err| {
                            ProtocolViolationError::invalid_event_format(
                                Some(new_seq_end),
                                format!("failed to parse segment header: {err}"),
                            )
                        })?;
                    header.seq_end = new_seq_end;
                    let header_value = serde_json::to_value(&header).map_err(|err| {
                        ProtocolViolationError::invalid_event_format(
                            Some(new_seq_end),
                            format!("failed to serialize segment header: {err}"),
                        )
                    })?;
                    let header_canonical = to_canonical_string(&header_value).map_err(|err| {
                        ProtocolViolationError::invalid_event_format(Some(new_seq_end), err)
                    })?;
                    new_content.push_str(SEGMENT_PREFIX);
                    new_content.push_str(&header_canonical);
                    new_content.push('\n');
                    updated = true;
                    continue;
                }
            }
            new_content.push_str(line);
            new_content.push('\n');
        }

        if updated {
            // Write to a temp file then rename for atomic replacement.
            let tmp_path = self.path.with_extension("tmp");
            std::fs::write(&tmp_path, &new_content).map_err(|err| {
                ProtocolViolationError::invalid_event_format(
                    Some(new_seq_end),
                    format!(
                        "failed to write temp segment header {}: {err}",
                        tmp_path.display()
                    ),
                )
            })?;
            std::fs::rename(&tmp_path, &self.path).map_err(|err| {
                ProtocolViolationError::invalid_event_format(
                    Some(new_seq_end),
                    format!(
                        "failed to atomically replace segment header {}: {err}",
                        self.path.display()
                    ),
                )
            })?;
        }

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

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("lock"));
    }

    // Fix 8: seq_end must be updated after every append.
    #[test]
    fn updates_seq_end_in_segment_header() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tri-sync-seqend-{unique}.jsonl"));

        let log = AppendOnlyEventLog::open(&path);

        let first = Event::state_write(
            0,
            0,
            "tenant-a",
            "tenant-a:k",
            BsmValue::Integer(1),
            false,
            ZERO_DIGEST_HEX,
            None,
        )
        .expect("first");
        log.append(&first).expect("append first");

        let header_after_first = log
            .load_header()
            .expect("load header")
            .expect("header exists");
        assert_eq!(header_after_first.seq_end, 0);

        let second = Event::state_write(
            1,
            0,
            "tenant-a",
            "tenant-a:k",
            BsmValue::Integer(2),
            false,
            first.digest.clone(),
            None,
        )
        .expect("second");
        log.append(&second).expect("append second");

        let header_after_second = log
            .load_header()
            .expect("load header")
            .expect("header exists");
        assert_eq!(
            header_after_second.seq_end, 1,
            "seq_end must be updated to 1 after second append"
        );

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("lock"));
    }
}
