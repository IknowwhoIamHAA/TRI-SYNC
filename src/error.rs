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
    InvalidEventFormat {
        seq: Option<u64>,
        detail: String,
    },
}

impl ProtocolViolationError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::SequenceGap { .. } => "SEQ_GAP",
            Self::DigestMismatch { .. } => "DIGEST_MISMATCH",
            Self::NamespaceBreach { .. } => "NAMESPACE_BREACH",
            Self::StateMismatch { .. } => "STATE_MISMATCH",
            Self::MissingTickSeal { .. } => "MISSING_TICK_SEAL",
            Self::InvalidEventFormat { .. } => "INVALID_EVENT_FORMAT",
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            Self::SequenceGap { .. }
            | Self::DigestMismatch { .. }
            | Self::InvalidEventFormat { .. } => 4,
            Self::NamespaceBreach { .. } => 5,
            Self::StateMismatch { .. } | Self::MissingTickSeal { .. } => 6,
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

        if message.contains("TICK_SEAL_FAIL")
            || message.contains("COMPACT_FAIL")
            || message.contains("TYPE_MISMATCH")
            || message.contains("KEY_NOT_FOUND")
            || message.contains("TIMESTAMP_REGRESSION")
            || message.contains("TICK_REGRESSION")
            || message.contains("PROTOCOL_ERROR")
        {
            return Self::state_mismatch(None, None, None, None, message);
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
            Self::DigestMismatch { detail, .. } => f.write_str(detail),
            Self::NamespaceBreach { detail, .. } => f.write_str(detail),
            Self::StateMismatch { detail, .. } => f.write_str(detail),
            Self::MissingTickSeal { detail, .. } => f.write_str(detail),
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
