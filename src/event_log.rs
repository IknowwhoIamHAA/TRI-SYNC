use std::error::Error;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::canonical_json::to_canonical_string;
use crate::digest::sha256_hex;
use crate::error::ProtocolViolationError;
use crate::event::{Event, EventType, ZERO_DIGEST_HEX};
use crate::hex::decode_hex;
use crate::replay::ReplayEngine;
use crate::state_map::StateSnapshot;

const SEGMENT_PREFIX: &str = "#SEGMENT ";
const PROTOCOL_VERSION: &str = "1.0.0";
const CATALOG_VERSION: &str = "1.0.0";
const DEFAULT_MAX_EVENTS_PER_SEGMENT: u64 = 10_000;
const DEFAULT_MAX_BYTES_PER_SEGMENT: u64 = 4 * 1024 * 1024;
const SNAPSHOT_FILE_SUFFIX: &str = ".snapshot.bin";

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SegmentCatalogEntry {
    segment_id: String,
    file_name: String,
    namespace: String,
    seq_start: u64,
    seq_end: u64,
    first_digest: String,
    last_digest: String,
    prev_segment: Option<String>,
    created_at: u64,
    protocol_ver: String,
    sealed: bool,
    event_count: u64,
    size_bytes: u64,
}

impl SegmentCatalogEntry {
    fn header(&self) -> SegmentHeader {
        SegmentHeader {
            segment_id: self.segment_id.clone(),
            namespace: self.namespace.clone(),
            seq_start: self.seq_start,
            seq_end: self.seq_end,
            first_digest: self.first_digest.clone(),
            prev_segment: self.prev_segment.clone(),
            created_at: self.created_at,
            protocol_ver: self.protocol_ver.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SegmentCatalog {
    catalog_version: String,
    namespace: Option<String>,
    next_seq: u64,
    head_digest: String,
    active_segment_id: Option<String>,
    max_events_per_segment: u64,
    max_bytes_per_segment: u64,
    segments: Vec<SegmentCatalogEntry>,
}

impl SegmentCatalog {
    fn new(max_events_per_segment: u64, max_bytes_per_segment: u64) -> Self {
        Self {
            catalog_version: CATALOG_VERSION.to_string(),
            namespace: None,
            next_seq: 0,
            head_digest: ZERO_DIGEST_HEX.to_string(),
            active_segment_id: None,
            max_events_per_segment,
            max_bytes_per_segment,
            segments: Vec::new(),
        }
    }

    fn active_segment_mut(&mut self) -> Option<&mut SegmentCatalogEntry> {
        let active = self.active_segment_id.as_deref()?;
        self.segments
            .iter_mut()
            .find(|segment| segment.segment_id == active)
    }

    fn active_segment(&self) -> Option<&SegmentCatalogEntry> {
        let active = self.active_segment_id.as_deref()?;
        self.segments
            .iter()
            .find(|segment| segment.segment_id == active)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogTailMetadata {
    pub next_seq: u64,
    pub prev_digest: String,
    pub namespace: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AppendOnlyEventLog {
    path: PathBuf,
    max_events_per_segment: u64,
    max_bytes_per_segment: u64,
}

impl AppendOnlyEventLog {
    pub fn open(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            max_events_per_segment: DEFAULT_MAX_EVENTS_PER_SEGMENT,
            max_bytes_per_segment: DEFAULT_MAX_BYTES_PER_SEGMENT,
        }
    }

    #[cfg(test)]
    fn open_with_limits(
        path: impl AsRef<Path>,
        max_events_per_segment: u64,
        max_bytes_per_segment: u64,
    ) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            max_events_per_segment,
            max_bytes_per_segment,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, event: &Event) -> Result<(), ProtocolViolationError> {
        self.append_batch(std::slice::from_ref(event))
    }

    /// Append a batch of events while holding the filesystem lock once.
    ///
    /// Durability boundary: segment lines are flushed before the catalog write.
    /// If the process exits before catalog persistence, `read_catalog_reconciled`
    /// recovers tail metadata from the active segment on next open.
    pub fn append_batch(&self, events: &[Event]) -> Result<(), ProtocolViolationError> {
        if events.is_empty() {
            return Ok(());
        }

        let lock_path = self.lock_path();
        let lock_file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)?;
        lock_file
            .lock_exclusive()
            .map_err(|err| ProtocolViolationError::invalid_event_format(None, err.to_string()))?;
        let result = self.append_batch_under_lock(events);
        drop(lock_file);
        let sealed_roots = result?;
        for root in sealed_roots {
            self.persist_snapshot_for_root(&root)?;
        }
        Ok(())
    }

    pub fn lock_for_write(&self) -> Result<(), Box<dyn Error>> {
        let lock_path = self.lock_path();
        let lock_file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)?;
        lock_file.lock_exclusive()?;
        drop(lock_file);
        Ok(())
    }

    pub fn load(&self) -> Result<Vec<Event>, Box<dyn Error>> {
        if self.catalog_path().exists() {
            let catalog = self.read_catalog_reconciled()?;
            let mut events = Vec::new();
            for segment in self.ordered_segments(&catalog)? {
                events.extend(self.load_segment_events(segment)?);
            }
            return Ok(events);
        }

        self.load_legacy_events()
    }

    pub fn load_header(&self) -> Result<Option<SegmentHeader>, Box<dyn Error>> {
        if self.catalog_path().exists() {
            let catalog = self.read_catalog_reconciled()?;
            return Ok(catalog.active_segment().map(|segment| segment.header()));
        }

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
        Ok(self.tail_metadata()?.next_seq)
    }

    pub fn tail_metadata(&self) -> Result<LogTailMetadata, Box<dyn Error>> {
        if self.catalog_path().exists() {
            let catalog = self.read_catalog_reconciled()?;
            return Ok(LogTailMetadata {
                next_seq: catalog.next_seq,
                prev_digest: catalog.head_digest,
                namespace: catalog.namespace,
            });
        }

        Ok(match self.load_last_legacy_event()? {
            Some(event) => LogTailMetadata {
                next_seq: event.seq + 1,
                prev_digest: event.digest,
                namespace: Some(event.namespace),
            },
            None => LogTailMetadata {
                next_seq: 0,
                prev_digest: ZERO_DIGEST_HEX.to_string(),
                namespace: None,
            },
        })
    }

    pub fn load_snapshot_for_root(
        &self,
        checkpoint_root: &str,
    ) -> Result<Option<StateSnapshot>, Box<dyn Error>> {
        let path = self.snapshot_path_for_root(checkpoint_root);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(path)?;
        let snapshot = StateSnapshot::from_binary(&bytes)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        Ok(Some(snapshot))
    }

    fn append_batch_under_lock(
        &self,
        events: &[Event],
    ) -> Result<Vec<String>, ProtocolViolationError> {
        if self.catalog_path().exists() {
            return self.append_segmented_under_lock(events);
        }

        if self.path.exists() && self.path.metadata().map_err(ProtocolViolationError::from)?.len() > 0 {
            self.migrate_legacy_log_under_lock()
                .map_err(|err| ProtocolViolationError::invalid_event_format(None, err.to_string()))?;
            return self.append_segmented_under_lock(events);
        }

        self.append_segmented_under_lock(events)
    }

    fn append_segmented_under_lock(
        &self,
        events: &[Event],
    ) -> Result<Vec<String>, ProtocolViolationError> {
        let mut catalog = self
            .read_catalog_reconciled_optional()
            .map_err(|err| ProtocolViolationError::invalid_event_format(None, err.to_string()))?
            .unwrap_or_else(|| {
            SegmentCatalog::new(self.max_events_per_segment, self.max_bytes_per_segment)
        });
        self.ensure_segment_storage()
            .map_err(|err| ProtocolViolationError::invalid_event_format(None, err.to_string()))?;
        let mut open_segment_name: Option<String> = None;
        let mut writer: Option<BufWriter<File>> = None;
        let mut catalog_dirty = false;
        let mut sealed_roots = Vec::new();

        for event in events {
            let expected_seq = catalog.next_seq;
            if event.seq != expected_seq {
                return Err(if event.seq < expected_seq {
                    ProtocolViolationError::sequence_collision(
                        Some(event.namespace.clone()),
                        event.seq,
                        format!(
                            "SEQUENCE_COLLISION: namespace {} already contains seq {}",
                            event.namespace, event.seq
                        ),
                    )
                } else {
                    ProtocolViolationError::SequenceGap {
                        expected_seq,
                        actual_seq: event.seq,
                    }
                });
            }

            event
                .validate_prev_digest(&catalog.head_digest)
                .map_err(ProtocolViolationError::from_message)?;
            event
                .validate_digest()
                .map_err(ProtocolViolationError::from_message)?;

            if let Some(namespace) = &catalog.namespace {
                if namespace != &event.namespace {
                    return Err(ProtocolViolationError::namespace_breach(
                        Some(namespace.clone()),
                        Some(event.namespace.clone()),
                        event.key.clone(),
                        "NAMESPACE_LEAK: mixed namespaces in one log file".to_string(),
                    ));
                }
            }

            let line = to_canonical_string(&serde_json::to_value(event).map_err(ProtocolViolationError::from)?)?;
            let line_len = line.len() as u64 + 1;
            let previous_active = catalog.active_segment_id.clone();
            self.roll_segment_if_needed(&mut catalog, line_len, event)
                .map_err(|err| ProtocolViolationError::invalid_event_format(None, err.to_string()))?;
            if catalog.active_segment_id != previous_active {
                if let Some(writer) = writer.as_mut() {
                    writer.flush().map_err(ProtocolViolationError::from)?;
                }
                writer = None;
                open_segment_name = None;
                self.write_catalog(&catalog)
                    .map_err(|err| ProtocolViolationError::invalid_event_format(None, err.to_string()))?;
            }

            let file_name = catalog
                .active_segment()
                .ok_or_else(|| {
                    ProtocolViolationError::invalid_event_format(
                        None,
                        "INVALID_SEGMENT: missing active segment after initialization",
                    )
                })?
                .file_name
                .clone();
            if open_segment_name.as_deref() != Some(file_name.as_str()) {
                if let Some(writer) = writer.as_mut() {
                    writer.flush().map_err(ProtocolViolationError::from)?;
                }
                let segment_path = self.segment_path(&file_name);
                let file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&segment_path)
                    .map_err(ProtocolViolationError::from)?;
                writer = Some(BufWriter::new(file));
                open_segment_name = Some(file_name.clone());
            }

            let active_writer = writer
                .as_mut()
                .ok_or_else(|| {
                    ProtocolViolationError::invalid_event_format(
                        None,
                        "INVALID_SEGMENT: active segment writer unavailable",
                    )
                })?;
            writeln!(active_writer, "{line}").map_err(ProtocolViolationError::from)?;

            let active_segment = catalog
                .active_segment_mut()
                .ok_or_else(|| {
                    ProtocolViolationError::invalid_event_format(
                        None,
                        "INVALID_SEGMENT: missing active segment after initialization",
                    )
                })?;
            active_segment.seq_end = event.seq;
            active_segment.last_digest = event.digest.clone();
            active_segment.event_count += 1;
            active_segment.size_bytes += line_len;

            if catalog.namespace.is_none() {
                catalog.namespace = Some(event.namespace.clone());
            }
            catalog.next_seq = event.seq + 1;
            catalog.head_digest = event.digest.clone();
            catalog_dirty = true;

            if event.event_type == EventType::TickSeal
                && let Some(root) = event.root_digest.as_deref()
            {
                sealed_roots.push(root.to_string());
            }
        }

        if let Some(writer) = writer.as_mut() {
            writer.flush().map_err(ProtocolViolationError::from)?;
        }
        if catalog_dirty {
            self.write_catalog(&catalog)
                .map_err(|err| ProtocolViolationError::invalid_event_format(None, err.to_string()))?;
        }
        Ok(sealed_roots)
    }

    fn persist_snapshot_for_root(&self, checkpoint_root: &str) -> Result<(), ProtocolViolationError> {
        let events = self
            .load()
            .map_err(|err| ProtocolViolationError::invalid_event_format(None, err.to_string()))?;
        let Some((checkpoint_index, checkpoint_event)) = events
            .iter()
            .enumerate()
            .find(|(_, event)| {
                event.event_type == EventType::TickSeal
                    && event.root_digest.as_deref() == Some(checkpoint_root)
            })
        else {
            return Ok(());
        };

        let checkpoint_state = ReplayEngine::replay(&events[..=checkpoint_index])?;
        let seal_timestamp_ms = checkpoint_event.timestamp_ms.ok_or_else(|| {
            ProtocolViolationError::invalid_event_format(
                Some(checkpoint_event.seq),
                "TICK_SEAL missing timestamp_ms",
            )
        })?;
        let root_bytes =
            decode_array_32(checkpoint_root).map_err(ProtocolViolationError::from_message)?;
        let seal_bytes =
            decode_array_32(&checkpoint_event.digest).map_err(ProtocolViolationError::from_message)?;
        let snapshot = StateSnapshot {
            namespace: checkpoint_event.namespace.clone(),
            tick: checkpoint_event.tick,
            seal_seq: checkpoint_event.seq,
            seal_timestamp_ms,
            root_digest: root_bytes,
            seal_digest: seal_bytes,
            state: checkpoint_state,
        };

        let encoded = snapshot.to_binary().map_err(ProtocolViolationError::from_message)?;
        std::fs::create_dir_all(self.snapshots_dir()).map_err(ProtocolViolationError::from)?;
        std::fs::write(self.snapshot_path_for_root(checkpoint_root), encoded)
            .map_err(ProtocolViolationError::from)?;
        Ok(())
    }

    fn roll_segment_if_needed(
        &self,
        catalog: &mut SegmentCatalog,
        next_line_len: u64,
        event: &Event,
    ) -> Result<(), Box<dyn Error>> {
        let needs_new_segment = match catalog.active_segment() {
            None => true,
            Some(segment) => {
                segment.event_count >= catalog.max_events_per_segment
                    || segment.size_bytes + next_line_len > catalog.max_bytes_per_segment
            }
        };

        if !needs_new_segment {
            return Ok(());
        }

        if let Some(active) = catalog.active_segment_mut() {
            active.sealed = true;
        }

        let prev_segment = catalog
            .segments
            .last()
            .map(|segment| segment.header().digest_hex())
            .transpose()?;
        let created_at = current_time_ms()?;
        let segment_id = self.next_segment_id(catalog, event.seq);
        let file_name = format!("{segment_id}.jsonl");
        let entry = SegmentCatalogEntry {
            segment_id: segment_id.clone(),
            file_name: file_name.clone(),
            namespace: event.namespace.clone(),
            seq_start: event.seq,
            seq_end: event.seq.saturating_sub(1),
            first_digest: event.digest.clone(),
            last_digest: catalog.head_digest.clone(),
            prev_segment,
            created_at,
            protocol_ver: PROTOCOL_VERSION.to_string(),
            sealed: false,
            event_count: 0,
            size_bytes: 0,
        };
        let header = entry.header();
        self.write_segment_header(&file_name, &header)?;
        catalog.active_segment_id = Some(segment_id);
        catalog.segments.push(entry);
        Ok(())
    }

    fn load_segment_events(
        &self,
        segment: &SegmentCatalogEntry,
    ) -> Result<Vec<Event>, Box<dyn Error>> {
        let path = self.segment_path(&segment.file_name);
        let file = File::open(&path)?;
        let mut events = Vec::new();
        for (line_number, line) in BufReader::new(file).lines().enumerate() {
            let line = line?;
            let line = line.trim();
            if line.is_empty() || line.starts_with(SEGMENT_PREFIX) {
                continue;
            }
            events.push(parse_event_line(line, &path, line_number + 1)?);
        }
        Ok(events)
    }

    fn load_legacy_events(&self) -> Result<Vec<Event>, Box<dyn Error>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&self.path)?;
        let mut events = Vec::new();
        for (line_number, line) in BufReader::new(file).lines().enumerate() {
            let line = line?;
            let line = line.trim();
            if line.is_empty() || line.starts_with(SEGMENT_PREFIX) {
                continue;
            }
            events.push(parse_event_line(line, &self.path, line_number + 1)?);
        }
        Ok(events)
    }

    fn load_last_legacy_event(&self) -> Result<Option<Event>, Box<dyn Error>> {
        if !self.path.exists() {
            return Ok(None);
        }

        let file = File::open(&self.path)?;
        let mut last_event = None;
        for (line_number, line) in BufReader::new(file).lines().enumerate() {
            let line = line?;
            let line = line.trim();
            if line.is_empty() || line.starts_with(SEGMENT_PREFIX) {
                continue;
            }
            last_event = Some(parse_event_line(line, &self.path, line_number + 1)?);
        }
        Ok(last_event)
    }

    fn migrate_legacy_log_under_lock(&self) -> Result<(), Box<dyn Error>> {
        if self.catalog_path().exists() {
            return Ok(());
        }

        let legacy_events = self.load_legacy_events()?;
        let mut catalog =
            SegmentCatalog::new(self.max_events_per_segment, self.max_bytes_per_segment);
        let tmp_segments_dir = self.derived_path(".segments.migrating");
        let tmp_catalog_path = self.derived_path(".catalog.migrating.json");

        if tmp_segments_dir.exists() {
            let _ = std::fs::remove_dir_all(&tmp_segments_dir);
        }
        if tmp_catalog_path.exists() {
            let _ = std::fs::remove_file(&tmp_catalog_path);
        }
        std::fs::create_dir_all(&tmp_segments_dir)?;

        for event in legacy_events {
            let line = to_canonical_string(&serde_json::to_value(&event)?)?;
            let line_len = line.len() as u64 + 1;
            self.roll_segment_if_needed_for_dir(&mut catalog, line_len, &event, &tmp_segments_dir)?;
            let active_segment = catalog
                .active_segment_mut()
                .ok_or("INVALID_SEGMENT: missing active segment during migration")?;
            let segment_path = tmp_segments_dir.join(&active_segment.file_name);
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(segment_path)?;
            writeln!(file, "{line}")?;
            drop(file);
            active_segment.seq_end = event.seq;
            active_segment.last_digest = event.digest.clone();
            active_segment.event_count += 1;
            active_segment.size_bytes += line_len;
            catalog.namespace = Some(event.namespace.clone());
            catalog.next_seq = event.seq + 1;
            catalog.head_digest = event.digest.clone();
        }

        let value = serde_json::to_value(&catalog)?;
        let canonical = to_canonical_string(&value)?;
        std::fs::write(&tmp_catalog_path, canonical)?;

        if self.segments_dir().exists() {
            let _ = std::fs::remove_dir_all(self.segments_dir());
        }
        std::fs::rename(&tmp_segments_dir, self.segments_dir())?;
        std::fs::rename(&tmp_catalog_path, self.catalog_path())?;

        if self.path.exists() {
            let _ = std::fs::rename(&self.path, self.derived_path(".legacy.jsonl"));
        }

        Ok(())
    }

    fn write_segment_header(
        &self,
        file_name: &str,
        header: &SegmentHeader,
    ) -> Result<(), Box<dyn Error>> {
        let value = serde_json::to_value(header)?;
        let canonical = to_canonical_string(&value)?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(self.segment_path(file_name))?;
        writeln!(file, "{SEGMENT_PREFIX}{canonical}")?;
        Ok(())
    }

    fn write_segment_header_to_dir(
        &self,
        segment_dir: &Path,
        file_name: &str,
        header: &SegmentHeader,
    ) -> Result<(), Box<dyn Error>> {
        let value = serde_json::to_value(header)?;
        let canonical = to_canonical_string(&value)?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(segment_dir.join(file_name))?;
        writeln!(file, "{SEGMENT_PREFIX}{canonical}")?;
        Ok(())
    }

    fn ensure_segment_storage(&self) -> Result<(), Box<dyn Error>> {
        std::fs::create_dir_all(self.segments_dir())
            .map_err(|err| -> Box<dyn Error> { Box::new(err) })
    }

    fn read_catalog_reconciled_optional(&self) -> Result<Option<SegmentCatalog>, Box<dyn Error>> {
        if !self.catalog_path().exists() {
            return Ok(None);
        }
        Ok(Some(self.read_catalog_reconciled()?))
    }

    fn read_catalog(&self) -> Result<SegmentCatalog, Box<dyn Error>> {
        let content = std::fs::read_to_string(self.catalog_path())?;
        Ok(serde_json::from_str(&content)?)
    }

    fn read_catalog_reconciled(&self) -> Result<SegmentCatalog, Box<dyn Error>> {
        let mut catalog = self.read_catalog()?;
        self.reconcile_catalog_tail(&mut catalog)?;
        Ok(catalog)
    }

    fn write_catalog(&self, catalog: &SegmentCatalog) -> Result<(), Box<dyn Error>> {
        let value = serde_json::to_value(catalog)?;
        let canonical = to_canonical_string(&value)?;
        let tmp_path = self.derived_path(".catalog.tmp");
        std::fs::write(&tmp_path, canonical)?;
        std::fs::rename(tmp_path, self.catalog_path())?;
        Ok(())
    }

    fn catalog_path(&self) -> PathBuf {
        self.derived_path(".catalog.json")
    }

    fn segments_dir(&self) -> PathBuf {
        self.derived_path(".segments")
    }

    fn lock_path(&self) -> PathBuf {
        self.derived_path(".lock")
    }

    fn segment_path(&self, file_name: &str) -> PathBuf {
        self.segments_dir().join(file_name)
    }

    fn snapshots_dir(&self) -> PathBuf {
        self.derived_path(".snapshots")
    }

    fn snapshot_path_for_root(&self, checkpoint_root: &str) -> PathBuf {
        self.snapshots_dir()
            .join(format!("{checkpoint_root}{SNAPSHOT_FILE_SUFFIX}"))
    }

    fn ordered_segments<'a>(
        &self,
        catalog: &'a SegmentCatalog,
    ) -> Result<Vec<&'a SegmentCatalogEntry>, Box<dyn Error>> {
        let mut segments: Vec<&SegmentCatalogEntry> = catalog.segments.iter().collect();
        segments.sort_by_key(|segment| segment.seq_start);

        for window in segments.windows(2) {
            let first = window[0];
            let second = window[1];
            if second.seq_start <= first.seq_end {
                return Err(format!(
                    "INVALID_SEGMENT: overlapping or unsorted segments {}..{} then {}..{}",
                    first.seq_start, first.seq_end, second.seq_start, second.seq_end
                )
                .into());
            }
        }

        Ok(segments)
    }

    fn next_segment_id(&self, catalog: &SegmentCatalog, seq_start: u64) -> String {
        format!("seg-{seq_start}-{}", catalog.segments.len())
    }

    fn reconcile_catalog_tail(&self, catalog: &mut SegmentCatalog) -> Result<(), Box<dyn Error>> {
        let Some(active_segment_id) = catalog.active_segment_id.as_deref() else {
            return Ok(());
        };
        let Some(active_index) = catalog
            .segments
            .iter()
            .position(|segment| segment.segment_id == active_segment_id)
        else {
            return Err("INVALID_SEGMENT: catalog active_segment_id does not exist".into());
        };

        let segment_path = self.segment_path(&catalog.segments[active_index].file_name);
        let tail = scan_segment_tail(&segment_path)?;

        if tail.event_count == 0 {
            catalog.next_seq = catalog.segments[active_index].seq_start;
            return Ok(());
        }

        let last_event = tail
            .last_event
            .ok_or("INVALID_SEGMENT: non-empty segment tail missing last event")?;
        let entry = &mut catalog.segments[active_index];
        entry.seq_end = last_event.seq;
        entry.last_digest = last_event.digest.clone();
        entry.event_count = tail.event_count;
        entry.size_bytes = tail.size_bytes;
        catalog.namespace = Some(last_event.namespace.clone());
        catalog.next_seq = last_event.seq + 1;
        catalog.head_digest = last_event.digest;
        Ok(())
    }

    fn roll_segment_if_needed_for_dir(
        &self,
        catalog: &mut SegmentCatalog,
        next_line_len: u64,
        event: &Event,
        segment_dir: &Path,
    ) -> Result<(), Box<dyn Error>> {
        let needs_new_segment = match catalog.active_segment() {
            None => true,
            Some(segment) => {
                segment.event_count >= catalog.max_events_per_segment
                    || segment.size_bytes + next_line_len > catalog.max_bytes_per_segment
            }
        };

        if !needs_new_segment {
            return Ok(());
        }

        if let Some(active) = catalog.active_segment_mut() {
            active.sealed = true;
        }

        let prev_segment = catalog
            .segments
            .last()
            .map(|segment| segment.header().digest_hex())
            .transpose()?;
        let created_at = current_time_ms()?;
        let segment_id = self.next_segment_id(catalog, event.seq);
        let file_name = format!("{segment_id}.jsonl");
        let entry = SegmentCatalogEntry {
            segment_id: segment_id.clone(),
            file_name: file_name.clone(),
            namespace: event.namespace.clone(),
            seq_start: event.seq,
            seq_end: event.seq.saturating_sub(1),
            first_digest: event.digest.clone(),
            last_digest: catalog.head_digest.clone(),
            prev_segment,
            created_at,
            protocol_ver: PROTOCOL_VERSION.to_string(),
            sealed: false,
            event_count: 0,
            size_bytes: 0,
        };
        let header = entry.header();
        self.write_segment_header_to_dir(segment_dir, &file_name, &header)?;
        catalog.active_segment_id = Some(segment_id);
        catalog.segments.push(entry);
        Ok(())
    }

    fn derived_path(&self, suffix: &str) -> PathBuf {
        let mut raw: OsString = self.path.as_os_str().to_os_string();
        raw.push(suffix);
        PathBuf::from(raw)
    }
}

fn current_time_ms() -> Result<u64, Box<dyn Error>> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| err.to_string())?
        .as_millis() as u64)
}

#[derive(Debug)]
struct SegmentTail {
    last_event: Option<Event>,
    event_count: u64,
    size_bytes: u64,
}

fn scan_segment_tail(path: &Path) -> Result<SegmentTail, Box<dyn Error>> {
    let file = File::open(path)?;
    let mut last_event = None;
    let mut event_count = 0u64;
    let mut size_bytes = 0u64;

    for (line_number, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(SEGMENT_PREFIX) {
            continue;
        }

        let event = parse_event_line(trimmed, path, line_number + 1)?;
        event_count += 1;
        size_bytes += line.len() as u64 + 1;
        last_event = Some(event);
    }

    Ok(SegmentTail {
        last_event,
        event_count,
        size_bytes,
    })
}

fn parse_event_line(line: &str, path: &Path, line_number: usize) -> Result<Event, Box<dyn Error>> {
    serde_json::from_str::<Event>(line).map_err(|err| {
        Box::new(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Failed to parse TRI-SYNC event in {} at line {}: {}",
                path.display(),
                line_number,
                err
            ),
        )) as Box<dyn Error>
    })
}

fn decode_array_32(value: &str) -> Result<[u8; 32], String> {
    let bytes = decode_hex(value)?;
    bytes
        .try_into()
        .map_err(|_| "expected 32-byte digest".to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::Value;

    use crate::event::{Event, ZERO_DIGEST_HEX};
    use crate::state_map::{BinaryStateMap, BsmValue};

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
        let _ = fs::remove_file(PathBuf::from(format!("{}.catalog.json", path.display())));
    }

    #[test]
    fn append_batch_appends_multiple_events_under_single_catalog_flush() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("current time should be later than epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tri-sync-batch-{unique}.jsonl"));
        let log = AppendOnlyEventLog::open(&path);

        let first = Event::state_write(
            0,
            0,
            "tenant-a",
            "tenant-a:key",
            BsmValue::Integer(1),
            false,
            ZERO_DIGEST_HEX,
            None,
        )
        .expect("first");
        let second = Event::state_write(
            1,
            1,
            "tenant-a",
            "tenant-a:key",
            BsmValue::Integer(2),
            false,
            first.digest.clone(),
            None,
        )
        .expect("second");
        let third = Event::state_delete(
            2,
            1,
            "tenant-a",
            "tenant-a:key",
            None,
            true,
            second.digest.clone(),
        )
        .expect("third");

        log.append_batch(&[first.clone(), second.clone(), third.clone()])
            .expect("append batch");

        let loaded = log.load().expect("load");
        assert_eq!(loaded, vec![first, second, third]);
        let tail = log.tail_metadata().expect("tail");
        assert_eq!(tail.next_seq, 3);
    }

    #[test]
    fn tick_seal_persists_snapshot_cache() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tri-sync-snapshot-{unique}.jsonl"));
        let log = AppendOnlyEventLog::open(&path);

        let first = Event::state_write(
            0,
            1,
            "tenant-a",
            "tenant-a:key",
            BsmValue::Integer(1),
            false,
            ZERO_DIGEST_HEX,
            None,
        )
        .expect("first");
        log.append(&first).expect("append first");

        let mut state = BinaryStateMap::new();
        state
            .set("tenant-a", "tenant-a:key", BsmValue::Integer(1))
            .expect("set");
        let root = state.root_digest_hex().expect("root");
        let seal = Event::tick_seal(1, 1, "tenant-a", 1, root.clone(), first.digest.clone(), 1_000)
            .expect("seal");
        log.append(&seal).expect("append seal");

        let snapshot = log
            .load_snapshot_for_root(&root)
            .expect("load snapshot")
            .expect("snapshot exists");
        assert_eq!(snapshot.seal_seq, 1);
        assert_eq!(snapshot.tick, 1);
        assert_eq!(snapshot.namespace, "tenant-a");
    }

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
        assert_eq!(header_after_second.seq_end, 1);
    }

    #[test]
    fn rolls_to_new_segment_without_rewriting_old_segments() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tri-sync-roll-{unique}.jsonl"));
        let log = AppendOnlyEventLog::open_with_limits(&path, 1, 1024 * 1024);

        let first = Event::state_write(
            0,
            0,
            "tenant-a",
            "tenant-a:a",
            BsmValue::Integer(1),
            false,
            ZERO_DIGEST_HEX,
            None,
        )
        .expect("first");
        log.append(&first).expect("append first");

        let second = Event::state_write(
            1,
            0,
            "tenant-a",
            "tenant-a:b",
            BsmValue::Integer(2),
            false,
            first.digest.clone(),
            None,
        )
        .expect("second");
        log.append(&second).expect("append second");

        let events = log.load().expect("load");
        assert_eq!(events.len(), 2);
        let header = log.load_header().expect("header").expect("active");
        assert_eq!(header.seq_start, 1);
        assert_eq!(header.seq_end, 1);
    }

    #[test]
    fn reconciles_stale_catalog_tail_before_next_append() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tri-sync-reconcile-{unique}.jsonl"));
        let log = AppendOnlyEventLog::open(&path);

        let first = Event::state_write(
            0,
            0,
            "tenant-a",
            "tenant-a:a",
            BsmValue::Integer(1),
            false,
            ZERO_DIGEST_HEX,
            None,
        )
        .expect("first");
        log.append(&first).expect("append first");

        let second = Event::state_write(
            1,
            0,
            "tenant-a",
            "tenant-a:b",
            BsmValue::Integer(2),
            false,
            first.digest.clone(),
            None,
        )
        .expect("second");
        log.append(&second).expect("append second");

        let catalog_path = PathBuf::from(format!("{}.catalog.json", path.display()));
        let mut catalog: Value =
            serde_json::from_str(&fs::read_to_string(&catalog_path).expect("catalog"))
                .expect("json");
        catalog["next_seq"] = Value::from(1u64);
        catalog["head_digest"] = Value::String(first.digest.clone());
        catalog["segments"][0]["seq_end"] = Value::from(0u64);
        catalog["segments"][0]["last_digest"] = Value::String(first.digest.clone());
        catalog["segments"][0]["event_count"] = Value::from(1u64);
        catalog["segments"][0]["size_bytes"] = Value::from(0u64);
        fs::write(
            &catalog_path,
            crate::canonical_json::to_canonical_string(&catalog).expect("canonical catalog"),
        )
        .expect("write stale catalog");

        let tail = log.tail_metadata().expect("tail metadata");
        assert_eq!(tail.next_seq, 2);
        assert_eq!(tail.prev_digest, second.digest);

        let third = Event::state_write(
            tail.next_seq,
            0,
            "tenant-a",
            "tenant-a:c",
            BsmValue::Integer(3),
            false,
            tail.prev_digest,
            None,
        )
        .expect("third");
        log.append(&third).expect("append third");
    }

    #[test]
    fn load_reports_segment_parse_context() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tri-sync-parse-{unique}.jsonl"));
        let log = AppendOnlyEventLog::open(&path);

        let first = Event::state_write(
            0,
            0,
            "tenant-a",
            "tenant-a:a",
            BsmValue::Integer(1),
            false,
            ZERO_DIGEST_HEX,
            None,
        )
        .expect("first");
        log.append(&first).expect("append first");

        let segments_dir = PathBuf::from(format!("{}.segments", path.display()));
        let mut entries = fs::read_dir(&segments_dir)
            .expect("segments dir")
            .map(|entry| entry.expect("dir entry").path())
            .collect::<Vec<_>>();
        entries.sort();
        let segment_path = entries.pop().expect("segment path");
        let mut contents = fs::read_to_string(&segment_path).expect("segment contents");
        contents.push_str("{bad json}\n");
        fs::write(&segment_path, contents).expect("write corrupt segment");

        let err = log.load().expect_err("corrupt segment must fail");
        let message = err.to_string();
        assert!(
            message.contains(&segment_path.display().to_string()),
            "got: {message}"
        );
        assert!(message.contains("line 3"), "got: {message}");
    }
}
