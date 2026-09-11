//! Pluggable event-log storage backends.
//!
//! # Example
//!
//! ```rust
//! use tri_sync::backend::EventLogBackend;
//! use tri_sync::error::ProtocolViolationError;
//! use tri_sync::event::Event;
//!
//! struct NoopBackend;
//!
//! impl EventLogBackend for NoopBackend {
//!     fn append(&self, _event: &Event) -> Result<(), ProtocolViolationError> {
//!         Ok(())
//!     }
//!
//!     fn load(&self) -> Result<Vec<Event>, ProtocolViolationError> {
//!         Ok(Vec::new())
//!     }
//!
//!     fn next_sequence(&self) -> Result<u64, ProtocolViolationError> {
//!         Ok(0)
//!     }
//!
//!     fn lock_for_write(&self) -> Result<(), ProtocolViolationError> {
//!         Ok(())
//!     }
//! }
//! ```
//!
//! Protocol logic can depend on [`EventLogBackend`] while backend-specific code
//! handles persistence, locking, and transport concerns.

use std::path::Path;
use std::sync::Mutex;

use crate::error::ProtocolViolationError;
use crate::event::{Event, ZERO_DIGEST_HEX};
use crate::event_log::{AppendOnlyEventLog, SegmentHeader};

/// Storage abstraction for event-log persistence.
///
/// Protocol code can depend on this trait instead of concrete filesystem I/O,
/// allowing alternate backends such as object stores or databases to plug in
/// without changing replay or verification logic.
pub trait EventLogBackend: Send + Sync {
    fn append(&self, event: &Event) -> Result<(), ProtocolViolationError>;
    fn load(&self) -> Result<Vec<Event>, ProtocolViolationError>;
    fn next_sequence(&self) -> Result<u64, ProtocolViolationError>;
    fn lock_for_write(&self) -> Result<(), ProtocolViolationError>;
}

#[derive(Debug, Clone)]
pub struct FileSystemBackend {
    inner: AppendOnlyEventLog,
}

impl FileSystemBackend {
    pub fn open(path: impl AsRef<Path>) -> Self {
        Self {
            inner: AppendOnlyEventLog::open(path),
        }
    }

    pub fn path(&self) -> &Path {
        self.inner.path()
    }

    pub fn load_header(&self) -> Result<Option<SegmentHeader>, ProtocolViolationError> {
        self.inner.load_header()
    }
}

impl EventLogBackend for FileSystemBackend {
    fn append(&self, event: &Event) -> Result<(), ProtocolViolationError> {
        self.inner.append(event)
    }

    fn load(&self) -> Result<Vec<Event>, ProtocolViolationError> {
        self.inner.load()
    }

    fn next_sequence(&self) -> Result<u64, ProtocolViolationError> {
        self.inner.next_sequence()
    }

    fn lock_for_write(&self) -> Result<(), ProtocolViolationError> {
        self.inner.lock_for_write()
    }
}

#[derive(Debug, Default)]
pub struct InMemoryBackend {
    events: Mutex<Vec<Event>>,
}

impl InMemoryBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_events(events: Vec<Event>) -> Self {
        Self {
            events: Mutex::new(events),
        }
    }
}

impl EventLogBackend for InMemoryBackend {
    fn append(&self, event: &Event) -> Result<(), ProtocolViolationError> {
        let mut events = self.events.lock().map_err(|_| {
            ProtocolViolationError::invalid_event_format(
                Some(event.seq),
                "in-memory backend mutex was poisoned",
            )
        })?;

        let expected_seq = events.last().map_or(0, |last| last.seq + 1);
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

        let expected_prev = events
            .last()
            .map_or(ZERO_DIGEST_HEX.to_string(), |last| last.digest.clone());
        event
            .validate_prev_digest(&expected_prev)
            .map_err(ProtocolViolationError::from_message)?;
        event
            .validate_digest()
            .map_err(ProtocolViolationError::from_message)?;

        if let Some(last) = events.last() {
            if last.namespace != event.namespace {
                return Err(ProtocolViolationError::namespace_breach(
                    Some(last.namespace.clone()),
                    Some(event.namespace.clone()),
                    event.key.clone(),
                    format!(
                        "NAMESPACE_LEAK: mixed namespaces in one log backend (expected {}, got {})",
                        last.namespace, event.namespace
                    ),
                ));
            }
        }

        events.push(event.clone());
        Ok(())
    }

    fn load(&self) -> Result<Vec<Event>, ProtocolViolationError> {
        self.events
            .lock()
            .map_err(|_| {
                ProtocolViolationError::invalid_event_format(
                    None,
                    "in-memory backend mutex was poisoned",
                )
            })
            .map(|events| events.clone())
    }

    fn next_sequence(&self) -> Result<u64, ProtocolViolationError> {
        self.events
            .lock()
            .map_err(|_| {
                ProtocolViolationError::invalid_event_format(
                    None,
                    "in-memory backend mutex was poisoned",
                )
            })
            .map(|events| events.last().map_or(0, |event| event.seq + 1))
    }

    fn lock_for_write(&self) -> Result<(), ProtocolViolationError> {
        self.events
            .lock()
            .map_err(|_| {
                ProtocolViolationError::invalid_event_format(
                    None,
                    "in-memory backend mutex was poisoned",
                )
            })
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::{EventLogBackend, InMemoryBackend};
    use crate::event::{Event, ZERO_DIGEST_HEX};
    use crate::state_map::BsmValue;

    #[test]
    fn in_memory_backend_appends_and_loads_events() {
        let backend = InMemoryBackend::new();
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
        .expect("first event");

        backend.append(&first).expect("append");
        backend.lock_for_write().expect("lock");

        let events = backend.load().expect("load");
        assert_eq!(events, vec![first]);
        assert_eq!(backend.next_sequence().expect("next seq"), 1);
    }
}
