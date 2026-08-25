use std::collections::BTreeMap;

use crate::canonical_json::to_canonical_string;
use crate::digest::sha256_hex;
use crate::key::validate_key;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BsmValue {
    Boolean(bool),
    Integer(i64),
    Decimal(String),
    String(String),
    Bytes(Vec<u8>),
    Null,
}

impl BsmValue {
    pub fn type_tag(&self) -> u8 {
        match self {
            Self::Boolean(_) => 0x01,
            Self::Integer(_) => 0x02,
            Self::Decimal(_) => 0x03,
            Self::String(_) => 0x04,
            Self::Bytes(_) => 0x05,
            Self::Null => 0x06,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BinaryStateMap {
    inner: BTreeMap<String, BsmValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateSnapshot {
    pub namespace: String,
    pub tick: u64,
    pub root_digest: [u8; 32],
    pub state: BinaryStateMap,
}

impl BinaryStateMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(
        &mut self,
        namespace: &str,
        key: impl Into<String>,
        value: BsmValue,
    ) -> Result<(), String> {
        let key = key.into();
        validate_key(namespace, &key)?;
        self.inner.insert(key, value);
        Ok(())
    }

    pub fn set_validated(&mut self, key: impl Into<String>, value: BsmValue) {
        self.inner.insert(key.into(), value);
    }

    pub fn delete(&mut self, namespace: &str, key: &str) -> Result<(), String> {
        validate_key(namespace, key)?;
        self.inner.remove(key);
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<&BsmValue> {
        self.inner.get(key)
    }

    pub fn entries(&self) -> impl Iterator<Item = (&String, &BsmValue)> {
        self.inner.iter()
    }

    pub fn to_binary(&self) -> Result<Vec<u8>, String> {
        let mut entries: Vec<(&String, &BsmValue)> = self.inner.iter().collect();
        entries.sort_by(|(ka, _), (kb, _)| ka.as_bytes().cmp(kb.as_bytes()));

        let mut out = Vec::new();
        out.extend_from_slice(&(entries.len() as u32).to_be_bytes());

        for (key, value) in entries {
            if key.as_bytes().len() > u16::MAX as usize {
                return Err("key too long for wire format".to_string());
            }

            out.extend_from_slice(&(key.as_bytes().len() as u16).to_be_bytes());
            out.extend_from_slice(key.as_bytes());
            out.push(value.type_tag());
            encode_value_payload(value, &mut out)?;
        }

        Ok(out)
    }

    pub fn from_binary(bytes: &[u8]) -> Result<Self, String> {
        let mut cursor = 0usize;
        let entry_count = read_u32(bytes, &mut cursor)? as usize;

        let mut map = BTreeMap::new();
        let mut previous_key: Option<String> = None;

        for _ in 0..entry_count {
            let key_len = read_u16(bytes, &mut cursor)? as usize;
            let key_bytes = read_exact(bytes, &mut cursor, key_len)?;
            let key = String::from_utf8(key_bytes.to_vec())
                .map_err(|_| "key must be valid UTF-8".to_string())?;

            if let Some(prev) = &previous_key {
                if prev.as_bytes() >= key.as_bytes() {
                    return Err("ORDER_VIOLATION: keys must be strictly increasing".to_string());
                }
            }
            previous_key = Some(key.clone());

            let type_tag = read_u8(bytes, &mut cursor)?;
            let value = decode_value_payload(type_tag, bytes, &mut cursor)?;
            map.insert(key, value);
        }

        if cursor != bytes.len() {
            return Err("unexpected trailing bytes in BSM".to_string());
        }

        Ok(Self { inner: map })
    }

    pub fn root_digest_hex(&self) -> Result<String, String> {
        Ok(sha256_hex(&self.to_binary()?))
    }

    pub fn to_canonical_json(&self) -> Result<String, String> {
        let value = serde_json::to_value(self.to_json_value()).map_err(|err| err.to_string())?;
        to_canonical_string(&value)
    }

    pub fn to_json_value(&self) -> BTreeMap<String, serde_json::Value> {
        let mut out = BTreeMap::new();
        for (key, value) in &self.inner {
            out.insert(key.clone(), value_to_json(value));
        }
        out
    }

    pub fn value_digest_hex(value: &BsmValue) -> Result<String, String> {
        let mut encoded = vec![value.type_tag()];
        encode_value_payload(value, &mut encoded)?;
        Ok(sha256_hex(&encoded))
    }
}

impl StateSnapshot {
    pub fn to_binary(&self) -> Result<Vec<u8>, String> {
        if self.namespace.as_bytes().len() > u16::MAX as usize {
            return Err("namespace too large".to_string());
        }

        let mut out = Vec::new();
        out.extend_from_slice(&(self.namespace.as_bytes().len() as u16).to_be_bytes());
        out.extend_from_slice(self.namespace.as_bytes());
        out.extend_from_slice(&self.tick.to_be_bytes());
        out.extend_from_slice(&self.root_digest);
        out.extend_from_slice(&self.state.to_binary()?);
        Ok(out)
    }

    pub fn from_binary(bytes: &[u8]) -> Result<Self, String> {
        let mut cursor = 0usize;
        let namespace_len = read_u16(bytes, &mut cursor)? as usize;
        let namespace = String::from_utf8(read_exact(bytes, &mut cursor, namespace_len)?.to_vec())
            .map_err(|_| "snapshot namespace must be UTF-8".to_string())?;
        let tick = read_u64(bytes, &mut cursor)?;
        let mut root_digest = [0u8; 32];
        root_digest.copy_from_slice(read_exact(bytes, &mut cursor, 32)?);

        let state = BinaryStateMap::from_binary(&bytes[cursor..])?;

        Ok(Self {
            namespace,
            tick,
            root_digest,
            state,
        })
    }
}

fn value_to_json(value: &BsmValue) -> serde_json::Value {
    match value {
        BsmValue::Boolean(v) => serde_json::Value::Bool(*v),
        BsmValue::Integer(v) => serde_json::Value::Number((*v).into()),
        BsmValue::Decimal(v) => serde_json::Value::String(v.clone()),
        BsmValue::String(v) => serde_json::Value::String(v.clone()),
        BsmValue::Bytes(v) => serde_json::Value::String(crate::hex::encode_hex(v)),
        BsmValue::Null => serde_json::Value::Null,
    }
}

fn encode_value_payload(value: &BsmValue, out: &mut Vec<u8>) -> Result<(), String> {
    match value {
        BsmValue::Boolean(v) => out.push(u8::from(*v)),
        BsmValue::Integer(v) => out.extend_from_slice(&v.to_be_bytes()),
        BsmValue::Decimal(v) => {
            if v.is_empty() {
                return Err("decimal payload must not be empty".to_string());
            }
            out.extend_from_slice(&(v.len() as u32).to_be_bytes());
            out.extend_from_slice(v.as_bytes());
        }
        BsmValue::String(v) => {
            out.extend_from_slice(&(v.len() as u32).to_be_bytes());
            out.extend_from_slice(v.as_bytes());
        }
        BsmValue::Bytes(v) => {
            out.extend_from_slice(&(v.len() as u32).to_be_bytes());
            out.extend_from_slice(v);
        }
        BsmValue::Null => {}
    }
    Ok(())
}

fn decode_value_payload(
    type_tag: u8,
    bytes: &[u8],
    cursor: &mut usize,
) -> Result<BsmValue, String> {
    match type_tag {
        0x01 => {
            let flag = read_u8(bytes, cursor)?;
            match flag {
                0 => Ok(BsmValue::Boolean(false)),
                1 => Ok(BsmValue::Boolean(true)),
                _ => Err("boolean payload must be 0 or 1".to_string()),
            }
        }
        0x02 => Ok(BsmValue::Integer(read_i64(bytes, cursor)?)),
        0x03 => {
            let len = read_u32(bytes, cursor)? as usize;
            let payload = read_exact(bytes, cursor, len)?;
            let decimal = String::from_utf8(payload.to_vec())
                .map_err(|_| "decimal payload must be UTF-8".to_string())?;
            Ok(BsmValue::Decimal(decimal))
        }
        0x04 => {
            let len = read_u32(bytes, cursor)? as usize;
            let payload = read_exact(bytes, cursor, len)?;
            let string = String::from_utf8(payload.to_vec())
                .map_err(|_| "string payload must be UTF-8".to_string())?;
            Ok(BsmValue::String(string))
        }
        0x05 => {
            let len = read_u32(bytes, cursor)? as usize;
            Ok(BsmValue::Bytes(read_exact(bytes, cursor, len)?.to_vec()))
        }
        0x06 => Ok(BsmValue::Null),
        _ => Err(format!("unknown value type tag: 0x{type_tag:02x}")),
    }
}

fn read_exact<'a>(bytes: &'a [u8], cursor: &mut usize, len: usize) -> Result<&'a [u8], String> {
    let end = cursor
        .checked_add(len)
        .ok_or_else(|| "cursor overflow while decoding".to_string())?;
    if end > bytes.len() {
        return Err("unexpected EOF while decoding".to_string());
    }
    let slice = &bytes[*cursor..end];
    *cursor = end;
    Ok(slice)
}

fn read_u8(bytes: &[u8], cursor: &mut usize) -> Result<u8, String> {
    Ok(read_exact(bytes, cursor, 1)?[0])
}

fn read_u16(bytes: &[u8], cursor: &mut usize) -> Result<u16, String> {
    let mut data = [0u8; 2];
    data.copy_from_slice(read_exact(bytes, cursor, 2)?);
    Ok(u16::from_be_bytes(data))
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, String> {
    let mut data = [0u8; 4];
    data.copy_from_slice(read_exact(bytes, cursor, 4)?);
    Ok(u32::from_be_bytes(data))
}

fn read_u64(bytes: &[u8], cursor: &mut usize) -> Result<u64, String> {
    let mut data = [0u8; 8];
    data.copy_from_slice(read_exact(bytes, cursor, 8)?);
    Ok(u64::from_be_bytes(data))
}

fn read_i64(bytes: &[u8], cursor: &mut usize) -> Result<i64, String> {
    let mut data = [0u8; 8];
    data.copy_from_slice(read_exact(bytes, cursor, 8)?);
    Ok(i64::from_be_bytes(data))
}

#[cfg(test)]
mod tests {
    use super::{BinaryStateMap, BsmValue, StateSnapshot};

    #[test]
    fn maintains_multi_tenant_ordering() {
        let mut state = BinaryStateMap::new();
        state
            .set("tenant-a", "tenant-a:zeta", BsmValue::Boolean(true))
            .expect("set should succeed");
        state
            .set("tenant-a", "tenant-a:beta", BsmValue::Integer(2))
            .expect("set should succeed");
        state
            .set("tenant-b", "tenant-b:alpha", BsmValue::Null)
            .expect("set should succeed");

        let ordered: Vec<String> = state.entries().map(|(k, _)| k.clone()).collect();
        assert_eq!(
            ordered,
            vec![
                "tenant-a:beta".to_string(),
                "tenant-a:zeta".to_string(),
                "tenant-b:alpha".to_string(),
            ]
        );
    }

    #[test]
    fn round_trips_binary_format() {
        let mut state = BinaryStateMap::new();
        state
            .set("tenant-a", "tenant-a:b", BsmValue::Bytes(vec![1, 2]))
            .expect("set should succeed");
        state
            .set(
                "tenant-a",
                "tenant-a:a",
                BsmValue::Decimal("1.25".to_string()),
            )
            .expect("set should succeed");

        let encoded = state.to_binary().expect("encode should succeed");
        let decoded = BinaryStateMap::from_binary(&encoded).expect("decode should succeed");
        assert_eq!(decoded, state);
    }

    #[test]
    fn round_trips_snapshot_format() {
        let mut state = BinaryStateMap::new();
        state
            .set(
                "tenant-a",
                "tenant-a:key",
                BsmValue::String("v".to_string()),
            )
            .expect("set should succeed");

        let snapshot = StateSnapshot {
            namespace: "tenant-a".to_string(),
            tick: 7,
            root_digest: [42u8; 32],
            state,
        };

        let encoded = snapshot
            .to_binary()
            .expect("snapshot encode should succeed");
        let decoded = StateSnapshot::from_binary(&encoded).expect("snapshot decode should succeed");
        assert_eq!(decoded, snapshot);
    }
}
