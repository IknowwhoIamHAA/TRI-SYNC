use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::canonical_json::to_canonical_string;
use crate::decimal::validate_decimal;
use crate::digest::sha256_hex;
use crate::key::{validate_key, validate_namespace};
use crate::state_map::BsmValue;

pub const ZERO_DIGEST_HEX: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventType {
    #[serde(rename = "STATE_WRITE")]
    StateWrite,
    #[serde(rename = "STATE_DELETE")]
    StateDelete,
    #[serde(rename = "STATE_BATCH")]
    StateBatch,
    #[serde(rename = "TICK_SEAL")]
    TickSeal,
    #[serde(rename = "COMPACT")]
    Compact,
    #[serde(rename = "PROTOCOL_ERROR")]
    ProtocolError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BatchOpType {
    #[serde(rename = "STATE_WRITE")]
    StateWrite,
    #[serde(rename = "STATE_DELETE")]
    StateDelete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchOp {
    #[serde(rename = "type")]
    pub op_type: BatchOpType,
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_type: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_value_digest: Option<String>,
    pub idempotent: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub seq: u64,
    pub tick: u64,
    pub namespace: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_type: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_value_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotent: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub ops: Option<Vec<BatchOp>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp_ms: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_seq_start: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_seq_end: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_uri: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offending_seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,

    pub prev_digest: String,
    pub digest: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
}

impl Event {
    pub fn state_write(
        seq: u64,
        tick: u64,
        namespace: impl Into<String>,
        key: impl Into<String>,
        value: BsmValue,
        idempotent: bool,
        prev_digest: impl Into<String>,
        metadata: Option<Map<String, Value>>,
    ) -> Result<Self, String> {
        let namespace = namespace.into();
        let key = key.into();

        validate_namespace(&namespace)?;
        validate_key(&namespace, &key)?;

        let (value_type, value_json) = bsm_value_to_event_value(&value)?;

        let mut event = Self {
            event_type: EventType::StateWrite,
            seq,
            tick,
            namespace,
            key: Some(key),
            value_type: Some(value_type),
            value: Some(value_json),
            prev_value_digest: None,
            idempotent: Some(idempotent),
            ops: None,
            event_count: None,
            root_digest: None,
            timestamp_ms: None,
            snapshot_digest: None,
            archived_seq_start: None,
            archived_seq_end: None,
            archive_uri: None,
            error_code: None,
            offending_seq: None,
            detail: None,
            prev_digest: prev_digest.into(),
            digest: ZERO_DIGEST_HEX.to_string(),
            metadata,
        };
        event.refresh_digest()?;
        Ok(event)
    }

    pub fn state_delete(
        seq: u64,
        tick: u64,
        namespace: impl Into<String>,
        key: impl Into<String>,
        prev_value_digest: Option<String>,
        idempotent: bool,
        prev_digest: impl Into<String>,
    ) -> Result<Self, String> {
        let namespace = namespace.into();
        let key = key.into();

        validate_namespace(&namespace)?;
        validate_key(&namespace, &key)?;

        let mut event = Self {
            event_type: EventType::StateDelete,
            seq,
            tick,
            namespace,
            key: Some(key),
            value_type: None,
            value: None,
            prev_value_digest,
            idempotent: Some(idempotent),
            ops: None,
            event_count: None,
            root_digest: None,
            timestamp_ms: None,
            snapshot_digest: None,
            archived_seq_start: None,
            archived_seq_end: None,
            archive_uri: None,
            error_code: None,
            offending_seq: None,
            detail: None,
            prev_digest: prev_digest.into(),
            digest: ZERO_DIGEST_HEX.to_string(),
            metadata: None,
        };
        event.refresh_digest()?;
        Ok(event)
    }

    pub fn state_batch(
        seq: u64,
        tick: u64,
        namespace: impl Into<String>,
        ops: Vec<BatchOp>,
        prev_digest: impl Into<String>,
    ) -> Result<Self, String> {
        let namespace = namespace.into();
        validate_namespace(&namespace)?;
        for op in &ops {
            validate_key(&namespace, &op.key)?;
        }

        let mut event = Self {
            event_type: EventType::StateBatch,
            seq,
            tick,
            namespace,
            key: None,
            value_type: None,
            value: None,
            prev_value_digest: None,
            idempotent: None,
            ops: Some(ops),
            event_count: None,
            root_digest: None,
            timestamp_ms: None,
            snapshot_digest: None,
            archived_seq_start: None,
            archived_seq_end: None,
            archive_uri: None,
            error_code: None,
            offending_seq: None,
            detail: None,
            prev_digest: prev_digest.into(),
            digest: ZERO_DIGEST_HEX.to_string(),
            metadata: None,
        };
        event.refresh_digest()?;
        Ok(event)
    }

    pub fn tick_seal(
        seq: u64,
        tick: u64,
        namespace: impl Into<String>,
        event_count: u32,
        root_digest: impl Into<String>,
        prev_digest: impl Into<String>,
        timestamp_ms: u64,
    ) -> Result<Self, String> {
        let namespace = namespace.into();
        validate_namespace(&namespace)?;

        let mut event = Self {
            event_type: EventType::TickSeal,
            seq,
            tick,
            namespace,
            key: None,
            value_type: None,
            value: None,
            prev_value_digest: None,
            idempotent: None,
            ops: None,
            event_count: Some(event_count),
            root_digest: Some(root_digest.into()),
            timestamp_ms: Some(timestamp_ms),
            snapshot_digest: None,
            archived_seq_start: None,
            archived_seq_end: None,
            archive_uri: None,
            error_code: None,
            offending_seq: None,
            detail: None,
            prev_digest: prev_digest.into(),
            digest: ZERO_DIGEST_HEX.to_string(),
            metadata: None,
        };
        event.refresh_digest()?;
        Ok(event)
    }

    pub fn compact(
        seq: u64,
        tick: u64,
        namespace: impl Into<String>,
        snapshot_digest: impl Into<String>,
        archived_seq_start: u64,
        archived_seq_end: u64,
        archive_uri: impl Into<String>,
        prev_digest: impl Into<String>,
    ) -> Result<Self, String> {
        let namespace = namespace.into();
        validate_namespace(&namespace)?;

        let mut event = Self {
            event_type: EventType::Compact,
            seq,
            tick,
            namespace,
            key: None,
            value_type: None,
            value: None,
            prev_value_digest: None,
            idempotent: None,
            ops: None,
            event_count: None,
            root_digest: None,
            timestamp_ms: None,
            snapshot_digest: Some(snapshot_digest.into()),
            archived_seq_start: Some(archived_seq_start),
            archived_seq_end: Some(archived_seq_end),
            archive_uri: Some(archive_uri.into()),
            error_code: None,
            offending_seq: None,
            detail: None,
            prev_digest: prev_digest.into(),
            digest: ZERO_DIGEST_HEX.to_string(),
            metadata: None,
        };
        event.refresh_digest()?;
        Ok(event)
    }

    pub fn protocol_error(
        seq: u64,
        tick: u64,
        namespace: impl Into<String>,
        error_code: impl Into<String>,
        offending_seq: Option<u64>,
        detail: Option<String>,
        prev_digest: impl Into<String>,
    ) -> Result<Self, String> {
        let namespace = namespace.into();
        validate_namespace(&namespace)?;

        let mut event = Self {
            event_type: EventType::ProtocolError,
            seq,
            tick,
            namespace,
            key: None,
            value_type: None,
            value: None,
            prev_value_digest: None,
            idempotent: None,
            ops: None,
            event_count: None,
            root_digest: None,
            timestamp_ms: None,
            snapshot_digest: None,
            archived_seq_start: None,
            archived_seq_end: None,
            archive_uri: None,
            error_code: Some(error_code.into()),
            offending_seq,
            detail,
            prev_digest: prev_digest.into(),
            digest: ZERO_DIGEST_HEX.to_string(),
            metadata: None,
        };
        event.refresh_digest()?;
        Ok(event)
    }

    pub fn digest_input(&self) -> Result<Value, String> {
        self.validate_payload_invariants()?;
        let mut event = serde_json::to_value(self).map_err(|err| err.to_string())?;
        let object = event
            .as_object_mut()
            .ok_or_else(|| "event serialization must produce JSON object".to_string())?;
        object.insert(
            "digest".to_string(),
            Value::String(ZERO_DIGEST_HEX.to_string()),
        );
        object.remove("metadata");
        Ok(Value::Object(object.clone()))
    }

    pub fn expected_digest(&self) -> Result<String, String> {
        let canonical = to_canonical_string(&self.digest_input()?)?;
        Ok(sha256_hex(canonical.as_bytes()))
    }

    pub fn refresh_digest(&mut self) -> Result<(), String> {
        self.digest = self.expected_digest()?;
        Ok(())
    }

    pub fn validate_digest(&self) -> Result<(), String> {
        let expected = self.expected_digest()?;
        if expected == self.digest {
            Ok(())
        } else {
            Err(format!(
                "DIGEST_MISMATCH: expected {}, got {} at seq {}",
                expected, self.digest, self.seq
            ))
        }
    }

    pub fn validate_prev_digest(&self, expected_prev_digest: &str) -> Result<(), String> {
        if self.prev_digest == expected_prev_digest {
            Ok(())
        } else {
            Err(format!(
                "DIGEST_MISMATCH: prev_digest mismatch at seq {} (expected {}, got {})",
                self.seq, expected_prev_digest, self.prev_digest
            ))
        }
    }

    pub fn is_idempotent(&self) -> bool {
        match self.event_type {
            EventType::StateWrite | EventType::StateDelete => self.idempotent.unwrap_or(false),
            EventType::StateBatch => false,
            EventType::TickSeal | EventType::Compact | EventType::ProtocolError => false,
        }
    }

    pub fn state_write_value(&self) -> Result<Option<BsmValue>, String> {
        if self.event_type != EventType::StateWrite {
            return Ok(None);
        }

        let type_tag = self
            .value_type
            .ok_or_else(|| "STATE_WRITE missing value_type".to_string())?;
        let value = self
            .value
            .as_ref()
            .ok_or_else(|| "STATE_WRITE missing value".to_string())?;

        Ok(Some(event_value_to_bsm(type_tag, value)?))
    }

    fn validate_payload_invariants(&self) -> Result<(), String> {
        validate_namespace(&self.namespace)?;
        match self.event_type {
            EventType::StateWrite => {
                let key = self
                    .key
                    .as_deref()
                    .ok_or_else(|| "STATE_WRITE missing key".to_string())?;
                validate_key(&self.namespace, key)?;
                let type_tag = self
                    .value_type
                    .ok_or_else(|| "STATE_WRITE missing value_type".to_string())?;
                let value = self
                    .value
                    .as_ref()
                    .ok_or_else(|| "STATE_WRITE missing value".to_string())?;
                let _ = event_value_to_bsm(type_tag, value)?;
            }
            EventType::StateDelete => {
                let key = self
                    .key
                    .as_deref()
                    .ok_or_else(|| "STATE_DELETE missing key".to_string())?;
                validate_key(&self.namespace, key)?;
            }
            EventType::StateBatch => {
                let ops = self
                    .ops
                    .as_ref()
                    .ok_or_else(|| "STATE_BATCH missing ops".to_string())?;
                for op in ops {
                    validate_key(&self.namespace, &op.key)?;
                    if op.op_type == BatchOpType::StateWrite {
                        let type_tag = op
                            .value_type
                            .ok_or_else(|| "STATE_WRITE op missing value_type".to_string())?;
                        let value = op
                            .value
                            .as_ref()
                            .ok_or_else(|| "STATE_WRITE op missing value".to_string())?;
                        let _ = event_value_to_bsm(type_tag, value)?;
                    }
                }
            }
            EventType::TickSeal | EventType::Compact | EventType::ProtocolError => {}
        }
        Ok(())
    }
}

fn bsm_value_to_event_value(value: &BsmValue) -> Result<(u8, Value), String> {
    match value {
        BsmValue::Boolean(v) => Ok((0x01, Value::Bool(*v))),
        BsmValue::Integer(v) => Ok((0x02, Value::Number((*v).into()))),
        BsmValue::Decimal(v) => {
            validate_decimal(v)?;
            Ok((0x03, Value::String(v.clone())))
        }
        BsmValue::String(v) => Ok((0x04, Value::String(v.clone()))),
        BsmValue::Bytes(v) => Ok((0x05, Value::String(crate::hex::encode_hex(v)))),
        BsmValue::Null => Ok((0x06, Value::Null)),
    }
}

pub fn event_value_to_bsm(type_tag: u8, value: &Value) -> Result<BsmValue, String> {
    match type_tag {
        0x01 => value
            .as_bool()
            .map(BsmValue::Boolean)
            .ok_or_else(|| "TYPE_MISMATCH: expected boolean".to_string()),
        0x02 => value
            .as_i64()
            .map(BsmValue::Integer)
            .ok_or_else(|| "TYPE_MISMATCH: expected int64".to_string()),
        0x03 => {
            let v = value
                .as_str()
                .ok_or_else(|| "TYPE_MISMATCH: expected decimal string".to_string())?;
            validate_decimal(v)?;
            Ok(BsmValue::Decimal(v.to_string()))
        }
        0x04 => value
            .as_str()
            .map(|v| BsmValue::String(v.to_string()))
            .ok_or_else(|| "TYPE_MISMATCH: expected UTF-8 string".to_string()),
        0x05 => value
            .as_str()
            .ok_or_else(|| "TYPE_MISMATCH: expected hex string for bytes".to_string())
            .and_then(|v| crate::hex::decode_hex(v).map(BsmValue::Bytes)),
        0x06 => {
            if value.is_null() {
                Ok(BsmValue::Null)
            } else {
                Err("TYPE_MISMATCH: null tag requires null value".to_string())
            }
        }
        _ => Err(format!("TYPE_MISMATCH: unknown type tag 0x{type_tag:02x}")),
    }
}

pub fn write_op(
    key: impl Into<String>,
    value: BsmValue,
    idempotent: bool,
) -> Result<BatchOp, String> {
    let key = key.into();
    let (value_type, value) = bsm_value_to_event_value(&value)?;
    Ok(BatchOp {
        op_type: BatchOpType::StateWrite,
        key,
        value_type: Some(value_type),
        value: Some(value),
        prev_value_digest: None,
        idempotent,
    })
}

pub fn delete_op(
    key: impl Into<String>,
    prev_value_digest: Option<String>,
    idempotent: bool,
) -> BatchOp {
    BatchOp {
        op_type: BatchOpType::StateDelete,
        key: key.into(),
        value_type: None,
        value: None,
        prev_value_digest,
        idempotent,
    }
}

pub fn protocol_error_event(
    seq: u64,
    tick: u64,
    namespace: &str,
    error_code: &str,
    offending_seq: Option<u64>,
    detail: Option<String>,
    prev_digest: &str,
) -> Result<Event, String> {
    Event::protocol_error(
        seq,
        tick,
        namespace.to_string(),
        error_code.to_string(),
        offending_seq,
        detail,
        prev_digest.to_string(),
    )
}

pub fn value_digest(value: &BsmValue) -> Result<String, String> {
    crate::state_map::BinaryStateMap::value_digest_hex(value)
}

pub fn minimal_state_write_json(
    seq: u64,
    tick: u64,
    namespace: &str,
    key: &str,
    value_type: u8,
    value: Value,
    prev_digest: &str,
) -> Value {
    json!({
        "type": "STATE_WRITE",
        "seq": seq,
        "tick": tick,
        "namespace": namespace,
        "key": key,
        "value_type": value_type,
        "value": value,
        "idempotent": false,
        "prev_digest": prev_digest,
        "digest": ZERO_DIGEST_HEX
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Event, EventType, ZERO_DIGEST_HEX};
    use crate::state_map::BsmValue;

    #[test]
    fn computes_digest_with_zeroed_digest_field() {
        let event = Event::state_write(
            1,
            0,
            "tenant-a",
            "tenant-a:key",
            BsmValue::String("value".to_string()),
            false,
            ZERO_DIGEST_HEX,
            None,
        )
        .expect("event creation should succeed");

        assert!(event.validate_digest().is_ok());
    }

    #[test]
    fn excludes_metadata_from_digest() {
        let mut event_a = Event::state_write(
            1,
            0,
            "tenant-a",
            "tenant-a:key",
            BsmValue::String("value".to_string()),
            false,
            ZERO_DIGEST_HEX,
            None,
        )
        .expect("event creation should succeed");

        let mut event_b = event_a.clone();
        event_b.metadata = Some(serde_json::Map::from_iter([(
            "precision_loss".to_string(),
            serde_json::Value::Bool(true),
        )]));
        event_b.refresh_digest().expect("refresh digest");

        assert_eq!(event_a.digest, event_b.digest);

        event_a.event_type = EventType::StateDelete;
        assert_ne!(event_a.expected_digest().expect("digest"), event_b.digest);
    }

    #[test]
    fn rejects_non_canonical_decimal_in_digest_computation() {
        let mut event = Event::state_write(
            1,
            0,
            "tenant-a",
            "tenant-a:ratio",
            BsmValue::Decimal("1.23".to_string()),
            false,
            ZERO_DIGEST_HEX,
            None,
        )
        .expect("event creation should succeed");
        event.value = Some(json!("01.230"));
        let err = event
            .expected_digest()
            .expect_err("expected invalid numeric");
        assert!(err.contains("INVALID_NUMERIC"));
    }
}
