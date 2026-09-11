use std::error::Error;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::canonical_json::to_canonical_string;
use crate::digest::sha256_hex;
use crate::event::{Event, ZERO_DIGEST_HEX};

const SEGMENT_PREFIX: &str = "#SEGMENT ";
const PROTOCOL_VERSION: &str = "1.0.0";
const CATALOG_VERSION: &str = "1.0.0";
const DEFAULT_MAX_EVENTS_PER_SEGMENT: u64 = 10_000;
const DEFAULT_MAX_BYTES_PER_SEGMENT: u64 = 4 * 1024 * 1024;

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

    pub fn append(&self, event: &Event) -> Result<(), Box<dyn Error>> {
        let lock_path = self.lock_path();
        let lock_file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)?;
        lock_file.lock_exclusive()?;
        let result = self.append_under_lock(event);
        drop(lock_file);
        result
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
            let catalog = self.read_catalog()?;
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
            let catalog = self.read_catalog()?;
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
            let catalog = self.read_catalog()?;
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

    fn append_under_lock(&self, event: &Event) -> Result<(), Box<dyn Error>> {
        if self.catalog_path().exists() {
            return self.append_segmented_under_lock(event);
        }

        if self.path.exists() && self.path.metadata()?.len() > 0 {
            self.migrate_legacy_log_under_lock()?;
            return self.append_segmented_under_lock(event);
        }

        self.append_segmented_under_lock(event)
    }

    fn append_segmented_under_lock(&self, event: &Event) -> Result<(), Box<dyn Error>> {
        let mut catalog = self.read_catalog_optional()?.unwrap_or_else(|| {
            SegmentCatalog::new(self.max_events_per_segment, self.max_bytes_per_segment)
        });

        let expected_seq = catalog.next_seq;
        if event.seq != expected_seq {
            return Err(if event.seq < expected_seq {
                format!(
                    "SEQUENCE_COLLISION: namespace {} already contains seq {}",
                    event.namespace, event.seq
                )
                .into()
            } else {
                format!("SEQ_GAP: expected seq {}, got {}", expected_seq, event.seq).into()
            });
        }

        event.validate_prev_digest(&catalog.head_digest)?;
        event.validate_digest()?;

        if let Some(namespace) = &catalog.namespace {
            if namespace != &event.namespace {
                return Err("NAMESPACE_LEAK: mixed namespaces in one log file".into());
            }
        }

        let line = to_canonical_string(&serde_json::to_value(event)?)?;
        let line_len = line.len() as u64 + 1;
        self.ensure_segment_storage()?;
        self.roll_segment_if_needed(&mut catalog, line_len, event)?;

        let active_segment = catalog
            .active_segment_mut()
            .ok_or("INVALID_SEGMENT: missing active segment after initialization")?;
        let segment_path = self.segment_path(&active_segment.file_name);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&segment_path)?;
        writeln!(file, "{line}")?;
        drop(file);

        active_segment.seq_end = event.seq;
        active_segment.last_digest = event.digest.clone();
        active_segment.event_count += 1;
        active_segment.size_bytes += line_len;

        if catalog.namespace.is_none() {
            catalog.namespace = Some(event.namespace.clone());
        }
        catalog.next_seq = event.seq + 1;
        catalog.head_digest = event.digest.clone();

        self.write_catalog(&catalog)?;
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
        let file = File::open(path)?;
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

    fn load_legacy_events(&self) -> Result<Vec<Event>, Box<dyn Error>> {
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

    fn load_last_legacy_event(&self) -> Result<Option<Event>, Box<dyn Error>> {
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

    fn read_catalog_optional(&self) -> Result<Option<SegmentCatalog>, Box<dyn Error>> {
        if !self.catalog_path().exists() {
            return Ok(None);
        }
        Ok(Some(self.read_catalog()?))
    }

    fn read_catalog(&self) -> Result<SegmentCatalog, Box<dyn Error>> {
        let content = std::fs::read_to_string(self.catalog_path())?;
        Ok(serde_json::from_str(&content)?)
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
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
        let _ = fs::remove_file(PathBuf::from(format!("{}.catalog.json", path.display())));
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
}
