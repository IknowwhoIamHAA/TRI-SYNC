use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::canonical_json::to_canonical_string;
use crate::digest::sha256_hex;
use crate::hex::{decode_hex, encode_hex};
use crate::key::TenantKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Set,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub sequence: u64,
    pub tenant: String,
    pub key: String,
    pub kind: EventKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_hex: Option<String>,
    pub payload_sha256: String,
}

impl Event {
    pub fn new_set(
        sequence: u64,
        tenant: impl Into<String>,
        key: impl Into<String>,
        value: &[u8],
    ) -> Self {
        let event = Self {
            sequence,
            tenant: tenant.into(),
            key: key.into(),
            kind: EventKind::Set,
            value_hex: Some(encode_hex(value)),
            payload_sha256: String::new(),
        };
        event.with_digest()
    }

    pub fn new_delete(sequence: u64, tenant: impl Into<String>, key: impl Into<String>) -> Self {
        let event = Self {
            sequence,
            tenant: tenant.into(),
            key: key.into(),
            kind: EventKind::Delete,
            value_hex: None,
            payload_sha256: String::new(),
        };
        event.with_digest()
    }

    pub fn tenant_key(&self) -> TenantKey {
        TenantKey::new(self.tenant.clone(), self.key.clone())
    }

    pub fn value_bytes(&self) -> Result<Option<Vec<u8>>, String> {
        self.value_hex
            .as_ref()
            .map(|value| decode_hex(value))
            .transpose()
    }

    pub fn validate_digest(&self) -> Result<(), String> {
        let expected = self.expected_payload_digest()?;
        if expected == self.payload_sha256 {
            Ok(())
        } else {
            Err(format!(
                "event payload digest mismatch at sequence {}: expected {}, got {}",
                self.sequence, expected, self.payload_sha256
            ))
        }
    }

    fn with_digest(mut self) -> Self {
        self.payload_sha256 = self
            .expected_payload_digest()
            .expect("serializing event payload for digest should not fail");
        self
    }

    fn expected_payload_digest(&self) -> Result<String, String> {
        let payload = json!({
            "sequence": self.sequence,
            "tenant": self.tenant,
            "key": self.key,
            "kind": self.kind,
            "value_hex": self.value_hex,
        });
        let canonical = to_canonical_string(&payload).map_err(|err| err.to_string())?;
        Ok(sha256_hex(canonical.as_bytes()))
    }
}
