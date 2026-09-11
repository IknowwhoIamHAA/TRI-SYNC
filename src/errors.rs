use serde::Serialize;
use serde_json::{Value, json};
use std::fmt::{self, Display, Formatter};

pub type ProtocolResult<T> = Result<T, ProtocolError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolErrorSeverity {
    Warning,
    Error,
    Fatal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolPhase {
    Append,
    Replay,
    Verify,
    Snapshot,
    License,
    Report,
    Storage,
    Input,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolAction {
    Reject,
    Halt,
    Warn,
    Quarantine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProtocolErrorReason {
    DigestMismatch,
    SeqGap,
    TickSealFail,
    NamespaceLeak,
    TypeMismatch,
    OrderViolation,
    DuplicateEvent,
    TimestampRegression,
    CompactFail,
    TickRegression,
    KeyNotFound,
    InvalidKey,
    InvalidNamespace,
    InvalidNumeric,
    BatchRollback,
    WarnDuplicate,
    InvalidSegment,
    StateLockPoison,
    ProtocolError,
    IoError,
    LicenseRequired,
    UnsupportedFormat,
}

impl ProtocolErrorReason {
    pub fn code(self) -> &'static str {
        match self {
            Self::DigestMismatch => "DIGEST_MISMATCH",
            Self::SeqGap => "SEQ_GAP",
            Self::TickSealFail => "TICK_SEAL_FAIL",
            Self::NamespaceLeak => "NAMESPACE_LEAK",
            Self::TypeMismatch => "TYPE_MISMATCH",
            Self::OrderViolation => "ORDER_VIOLATION",
            Self::DuplicateEvent => "DUPLICATE_EVENT",
            Self::TimestampRegression => "TIMESTAMP_REGRESSION",
            Self::CompactFail => "COMPACT_FAIL",
            Self::TickRegression => "TICK_REGRESSION",
            Self::KeyNotFound => "KEY_NOT_FOUND",
            Self::InvalidKey => "INVALID_KEY",
            Self::InvalidNamespace => "INVALID_NAMESPACE",
            Self::InvalidNumeric => "INVALID_NUMERIC",
            Self::BatchRollback => "BATCH_ROLLBACK",
            Self::WarnDuplicate => "WARN_DUPLICATE",
            Self::InvalidSegment => "INVALID_SEGMENT",
            Self::StateLockPoison => "STATE_LOCK_POISON",
            Self::ProtocolError => "PROTOCOL_ERROR",
            Self::IoError => "IO_ERROR",
            Self::LicenseRequired => "LICENSE_REQUIRED",
            Self::UnsupportedFormat => "UNSUPPORTED_FORMAT",
        }
    }

    pub fn severity(self) -> ProtocolErrorSeverity {
        match self {
            Self::KeyNotFound
            | Self::InvalidKey
            | Self::InvalidNamespace
            | Self::InvalidNumeric
            | Self::BatchRollback
            | Self::UnsupportedFormat
            | Self::LicenseRequired => ProtocolErrorSeverity::Error,
            Self::WarnDuplicate => ProtocolErrorSeverity::Warning,
            _ => ProtocolErrorSeverity::Fatal,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProtocolError {
    pub code: String,
    pub severity: ProtocolErrorSeverity,
    pub phase: ProtocolPhase,
    pub action: ProtocolAction,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tick: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offending_seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
}

impl ProtocolError {
    pub fn new(
        reason: ProtocolErrorReason,
        phase: ProtocolPhase,
        action: ProtocolAction,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: reason.code().to_string(),
            severity: reason.severity(),
            phase,
            action,
            message: message.into(),
            namespace: None,
            seq: None,
            tick: None,
            offending_seq: None,
            expected: None,
            actual: None,
        }
    }

    pub fn from_message(
        phase: ProtocolPhase,
        action: ProtocolAction,
        message: impl Into<String>,
    ) -> Self {
        let message = message.into();
        let code = message
            .split_once(':')
            .map(|(prefix, _)| prefix)
            .unwrap_or(message.as_str())
            .trim();
        let detail = message
            .split_once(':')
            .map(|(_, detail)| detail.trim().to_string())
            .unwrap_or_else(|| message.clone());

        let reason = match code {
            "DIGEST_MISMATCH" => ProtocolErrorReason::DigestMismatch,
            "SEQ_GAP" => ProtocolErrorReason::SeqGap,
            "TICK_SEAL_FAIL" => ProtocolErrorReason::TickSealFail,
            "NAMESPACE_LEAK" => ProtocolErrorReason::NamespaceLeak,
            "TYPE_MISMATCH" => ProtocolErrorReason::TypeMismatch,
            "ORDER_VIOLATION" => ProtocolErrorReason::OrderViolation,
            "DUPLICATE_EVENT" => ProtocolErrorReason::DuplicateEvent,
            "TIMESTAMP_REGRESSION" => ProtocolErrorReason::TimestampRegression,
            "COMPACT_FAIL" => ProtocolErrorReason::CompactFail,
            "TICK_REGRESSION" => ProtocolErrorReason::TickRegression,
            "KEY_NOT_FOUND" => ProtocolErrorReason::KeyNotFound,
            "INVALID_KEY" => ProtocolErrorReason::InvalidKey,
            "INVALID_NAMESPACE" => ProtocolErrorReason::InvalidNamespace,
            "INVALID_NUMERIC" => ProtocolErrorReason::InvalidNumeric,
            "BATCH_ROLLBACK" => ProtocolErrorReason::BatchRollback,
            "WARN_DUPLICATE" => ProtocolErrorReason::WarnDuplicate,
            "INVALID_SEGMENT" => ProtocolErrorReason::InvalidSegment,
            "STATE_LOCK_POISON" => ProtocolErrorReason::StateLockPoison,
            "PROTOCOL_ERROR" => ProtocolErrorReason::ProtocolError,
            "IO_ERROR" => ProtocolErrorReason::IoError,
            "LICENSE_REQUIRED" => ProtocolErrorReason::LicenseRequired,
            "UNSUPPORTED_FORMAT" => ProtocolErrorReason::UnsupportedFormat,
            _ => ProtocolErrorReason::IoError,
        };

        Self::new(reason, phase, action, detail)
    }

    pub fn with_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = Some(namespace.into());
        self
    }

    pub fn with_seq(mut self, seq: u64) -> Self {
        self.seq = Some(seq);
        self
    }

    pub fn with_tick(mut self, tick: u64) -> Self {
        self.tick = Some(tick);
        self
    }

    pub fn with_offending_seq(mut self, offending_seq: u64) -> Self {
        self.offending_seq = Some(offending_seq);
        self
    }

    pub fn with_expected(mut self, expected: impl Into<String>) -> Self {
        self.expected = Some(expected.into());
        self
    }

    pub fn with_actual(mut self, actual: impl Into<String>) -> Self {
        self.actual = Some(actual.into());
        self
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn to_json_value(&self) -> Value {
        json!(self)
    }
}

impl Display for ProtocolError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ProtocolError {}
