use std::error::Error;
use std::fmt::{self, Display, Formatter};

use serde::Serialize;
use serde_json::{Value, json};

use crate::canonical_json::to_canonical_string;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "error_type")]
pub enum ProtocolViolationError {
    SequenceGap {
        expected_seq: u64,
        actual_seq: u64,
    },
    SequenceCollision {
        namespace: Option<String>,
        seq: u64,
        detail: String,
    },
    DuplicateEvent {
        digest: String,
        seq: u64,
        namespace: String,
        tick: u64,
    },
    DigestMismatch {
        seq: Option<u64>,
        expected: Option<String>,
        actual: Option<String>,
        detail: String,
    },
    NamespaceBreach {
        expected_namespace: Option<String>,
        actual_namespace: Option<String>,
        key: Option<String>,
        detail: String,
    },
    StateMismatch {
        seq: Option<u64>,
        expected_root: Option<String>,
        actual_root: Option<String>,
        checkpoint_root: Option<String>,
        detail: String,
    },
    MissingTickSeal {
        checkpoint_root: Option<String>,
        detail: String,
    },
    TimestampRegression {
        previous: u64,
        current: u64,
        seq: u64,
        namespace: String,
        tick: u64,
    },
    TickRegression {
        previous: u64,
        current: u64,
        seq: u64,
        namespace: String,
        tick: u64,
    },
    CompactMismatch {
        expected: String,
        found: String,
        seq: u64,
        namespace: String,
        tick: u64,
    },
    ProtocolErrorEvent {
        code: String,
        detail: Option<String>,
        seq: u64,
        namespace: String,
        tick: u64,
    },
    InvalidEventFormat {
        seq: Option<u64>,
        detail: String,
    },
}

impl ProtocolViolationError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::SequenceGap { .. } => "SEQ_GAP",
            Self::SequenceCollision { .. } => "SEQUENCE_COLLISION",
            Self::DuplicateEvent { .. } => "DUPLICATE_EVENT",
            Self::DigestMismatch { .. } => "DIGEST_MISMATCH",
            Self::NamespaceBreach { .. } => "NAMESPACE_BREACH",
            Self::StateMismatch { .. } => "STATE_MISMATCH",
            Self::MissingTickSeal { .. } => "MISSING_TICK_SEAL",
            Self::TimestampRegression { .. } => "TIMESTAMP_REGRESSION",
            Self::TickRegression { .. } => "TICK_REGRESSION",
            Self::CompactMismatch { .. } => "COMPACT_MISMATCH",
            Self::ProtocolErrorEvent { .. } => "PROTOCOL_ERROR_EVENT",
            Self::InvalidEventFormat { detail, .. } => {
                if detail.starts_with("INVALID_NAMESPACE:") {
                    "INVALID_NAMESPACE"
                } else {
                    "INVALID_EVENT_FORMAT"
                }
            }
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            Self::SequenceGap { .. }
            | Self::DigestMismatch { .. }
            | Self::InvalidEventFormat { .. }
            | Self::DuplicateEvent { .. }
            | Self::ProtocolErrorEvent { .. } => 4,
            Self::SequenceCollision { .. } => 5,
            Self::NamespaceBreach { .. } => 5,
            Self::StateMismatch { .. }
            | Self::MissingTickSeal { .. }
            | Self::TimestampRegression { .. }
            | Self::TickRegression { .. }
            | Self::CompactMismatch { .. } => 6,
        }
    }

    pub fn invalid_event_format(seq: Option<u64>, detail: impl Into<String>) -> Self {
        Self::InvalidEventFormat {
            seq,
            detail: detail.into(),
        }
    }

    pub fn missing_tick_seal(checkpoint_root: Option<String>, detail: impl Into<String>) -> Self {
        Self::MissingTickSeal {
            checkpoint_root,
            detail: detail.into(),
        }
    }

    pub fn invalid_namespace(
        seq: Option<u64>,
        _namespace: Option<String>,
        detail: impl Into<String>,
    ) -> Self {
        let detail = detail.into();
        Self::InvalidEventFormat {
            seq,
            detail: if detail.starts_with("INVALID_NAMESPACE:") {
                detail
            } else {
                format!("INVALID_NAMESPACE: {detail}")
            },
        }
    }

    pub fn state_mismatch(
        seq: Option<u64>,
        expected_root: Option<String>,
        actual_root: Option<String>,
        checkpoint_root: Option<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self::StateMismatch {
            seq,
            expected_root,
            actual_root,
            checkpoint_root,
            detail: detail.into(),
        }
    }

    pub fn namespace_breach(
        expected_namespace: Option<String>,
        actual_namespace: Option<String>,
        key: Option<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self::NamespaceBreach {
            expected_namespace,
            actual_namespace,
            key,
            detail: detail.into(),
        }
    }

    pub fn sequence_collision(
        namespace: Option<String>,
        seq: u64,
        detail: impl Into<String>,
    ) -> Self {
        Self::SequenceCollision {
            namespace,
            seq,
            detail: detail.into(),
        }
    }

    pub fn to_json_value(&self) -> Value {
        let mut value = serde_json::to_value(self).unwrap_or_else(|_| {
            json!({
                "error_type": "InvalidEventFormat",
                "detail": "failed to serialize protocol violation error"
            })
        });

        if let Some(object) = value.as_object_mut() {
            object.insert("code".to_string(), Value::String(self.code().to_string()));
            object.insert("exit_code".to_string(), Value::from(self.exit_code()));
            object.insert("message".to_string(), Value::String(self.to_string()));
        }

        value
    }

    pub fn to_stderr_json(&self) -> String {
        to_canonical_string(&self.to_json_value())
            .unwrap_or_else(|_| format!(r#"{{"code":"{}","message":"{}"}}"#, self.code(), self))
    }

    pub fn from_message(message: impl Into<String>) -> Self {
        let message = message.into();

        if let Some((expected_seq, actual_seq)) = parse_expected_actual_numbers(&message, "SEQ_GAP")
        {
            return Self::SequenceGap {
                expected_seq,
                actual_seq,
            };
        }

        if message.contains("SEQUENCE_COLLISION") {
            return Self::SequenceCollision {
                namespace: None,
                seq: parse_seq(&message).unwrap_or(0),
                detail: message,
            };
        }

        if message.contains("DUPLICATE_EVENT") {
            return Self::DuplicateEvent {
                digest: String::new(),
                seq: parse_seq(&message).unwrap_or(0),
                namespace: String::new(),
                tick: 0,
            };
        }

        if message.contains("DIGEST_MISMATCH") {
            return Self::DigestMismatch {
                seq: parse_seq(&message),
                expected: parse_expected_value(&message),
                actual: parse_actual_value(&message),
                detail: message,
            };
        }

        if message.contains("NAMESPACE_LEAK")
            || message.contains("namespace boundary")
            || message.contains("outside namespace")
        {
            return Self::namespace_breach(None, None, None, message);
        }

        if message.starts_with("INVALID_NAMESPACE:") {
            return Self::invalid_namespace(parse_seq(&message), None, message);
        }

        if message.contains("TICK_SEAL_FAIL")
            || message.contains("COMPACT_FAIL")
            || message.contains("TYPE_MISMATCH")
            || message.contains("KEY_NOT_FOUND")
        {
            return Self::state_mismatch(None, None, None, None, message);
        }

        if message.contains("TIMESTAMP_REGRESSION") {
            return Self::TimestampRegression {
                previous: 0,
                current: 0,
                seq: parse_seq(&message).unwrap_or(0),
                namespace: String::new(),
                tick: 0,
            };
        }

        if message.contains("TICK_REGRESSION") {
            return Self::TickRegression {
                previous: 0,
                current: 0,
                seq: parse_seq(&message).unwrap_or(0),
                namespace: String::new(),
                tick: 0,
            };
        }

        if message.contains("PROTOCOL_ERROR") {
            return Self::ProtocolErrorEvent {
                code: "PROTOCOL_ERROR".to_string(),
                detail: Some(message),
                seq: 0,
                namespace: String::new(),
                tick: 0,
            };
        }

        Self::invalid_event_format(parse_seq(&message), message)
    }
}

impl Display for ProtocolViolationError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::SequenceGap {
                expected_seq,
                actual_seq,
            } => write!(f, "SEQ_GAP: expected seq {expected_seq}, got {actual_seq}"),
            Self::SequenceCollision { detail, .. } => f.write_str(detail),
            Self::DuplicateEvent {
                digest,
                seq,
                namespace,
                tick,
            } => write!(
                f,
                "DUPLICATE_EVENT: non-idempotent duplicate detected at seq {seq} (digest={digest}, namespace={namespace}, tick={tick})"
            ),
            Self::DigestMismatch { detail, .. } => f.write_str(detail),
            Self::NamespaceBreach { detail, .. } => f.write_str(detail),
            Self::StateMismatch { detail, .. } => f.write_str(detail),
            Self::MissingTickSeal { detail, .. } => f.write_str(detail),
            Self::TimestampRegression {
                previous,
                current,
                seq,
                namespace,
                tick,
            } => write!(
                f,
                "TIMESTAMP_REGRESSION: TICK_SEAL at seq {seq} has timestamp {current} < previous {previous} (namespace={namespace}, tick={tick})"
            ),
            Self::TickRegression {
                previous,
                current,
                seq,
                namespace,
                tick,
            } => write!(
                f,
                "TICK_REGRESSION: event at seq {seq} has tick {current} < previous max tick {previous} (namespace={namespace}, tick={tick})"
            ),
            Self::CompactMismatch {
                expected,
                found,
                seq,
                namespace,
                tick,
            } => write!(
                f,
                "COMPACT_MISMATCH: current state root {found} does not match snapshot_digest {expected} at seq {seq} (namespace={namespace}, tick={tick})"
            ),
            Self::ProtocolErrorEvent {
                code,
                detail,
                seq,
                namespace,
                tick,
            } => {
                let suffix = detail
                    .as_ref()
                    .map(|d| format!(": {d}"))
                    .unwrap_or_default();
                write!(
                    f,
                    "PROTOCOL_ERROR_EVENT: at seq {seq}: {code}{suffix} (namespace={namespace}, tick={tick})"
                )
            }
            Self::InvalidEventFormat { detail, .. } => f.write_str(detail),
        }
    }
}

impl Error for ProtocolViolationError {}

impl From<String> for ProtocolViolationError {
    fn from(value: String) -> Self {
        Self::from_message(value)
    }
}

impl From<std::io::Error> for ProtocolViolationError {
    fn from(value: std::io::Error) -> Self {
        Self::invalid_event_format(None, value.to_string())
    }
}

impl From<serde_json::Error> for ProtocolViolationError {
    fn from(value: serde_json::Error) -> Self {
        Self::invalid_event_format(None, value.to_string())
    }
}

fn parse_seq(message: &str) -> Option<u64> {
    message
        .split("seq ")
        .nth(1)
        .and_then(|rest| rest.split(|ch: char| !ch.is_ascii_digit()).next())
        .and_then(|digits| digits.parse().ok())
}

fn parse_expected_actual_numbers(message: &str, code: &str) -> Option<(u64, u64)> {
    if !message.starts_with(code) {
        return None;
    }

    let expected = parse_u64_after(message, "expected seq ")?;
    let actual = parse_u64_after(message, "got ")?;
    Some((expected, actual))
}

fn parse_expected_value(message: &str) -> Option<String> {
    if let Some(start) = message.find("expected ") {
        let start = start + "expected ".len();
        let value = message[start..]
            .split(", got ")
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        return Some(value.to_string());
    }
    None
}

fn parse_actual_value(message: &str) -> Option<String> {
    if let Some(start) = message.find("got ") {
        let start = start + "got ".len();
        let value = message[start..]
            .split(" at seq ")
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        return Some(value.to_string());
    }
    None
}

fn parse_u64_after(message: &str, needle: &str) -> Option<u64> {
    message
        .find(needle)
        .map(|index| &message[index + needle.len()..])
        .and_then(|rest| {
            let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
            (!digits.is_empty()).then_some(digits)
        })
        .and_then(|digits| digits.parse().ok())
}
