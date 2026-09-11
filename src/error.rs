use std::fmt;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ProtocolViolationError {
    pub error_type: &'static str,
    pub code: &'static str,
    pub exit_code: i32,
    pub namespace: Option<String>,
    pub seq: Option<u64>,
    pub message: String,
}

impl ProtocolViolationError {
    pub fn sequence_collision(namespace: String, seq: u64) -> Self {
        Self {
            error_type: "SequenceCollision",
            code: "SEQUENCE_COLLISION",
            exit_code: 7,
            namespace: Some(namespace.clone()),
            seq: Some(seq),
            message: format!("namespace {namespace} already contains seq {seq}"),
        }
    }

    pub fn sequence_gap(expected_seq: u64, actual_seq: u64) -> Self {
        Self {
            error_type: "SequenceGap",
            code: "SEQUENCE_GAP",
            exit_code: 4,
            namespace: None,
            seq: Some(actual_seq),
            message: format!("expected seq {expected_seq}, got {actual_seq}"),
        }
    }

    pub fn namespace_breach(expected: String, actual: String) -> Self {
        Self {
            error_type: "NamespaceBreach",
            code: "NAMESPACE_BREACH",
            exit_code: 5,
            namespace: Some(actual.clone()),
            seq: None,
            message: format!(
                "mixed namespaces in one log file (expected {expected}, got {actual})"
            ),
        }
    }

    pub fn to_json_line(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            format!(
                r#"{{"error_type":"{}","code":"{}","exit_code":{}}}"#,
                self.error_type, self.code, self.exit_code
            )
        })
    }
}

impl fmt::Display for ProtocolViolationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ProtocolViolationError {}
