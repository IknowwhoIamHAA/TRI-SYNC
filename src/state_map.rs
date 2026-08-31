use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::canonical_json::to_canonical_string;
use crate::decimal::validate_decimal;
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

/// A single per-key difference produced by [`BinaryStateMap::diff`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateDiff {
    /// Key is present in the second map but not the first.
    Added { key: String, value: BsmValue },
    /// Key is present in the first map but not the second.
    Removed { key: String, value: BsmValue },
    /// Key is present in both maps but with different values.
    Changed {
        key: String,
        from: BsmValue,
        to: BsmValue,
    },
}

impl StateDiff {
    pub fn key(&self) -> &str {
        match self {
            Self::Added { key, .. } | Self::Removed { key, .. } | Self::Changed { key, .. } => key,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BinaryStateMap {
    inner: BTreeMap<String, BsmValue>,
}

/// A thread-safe, transactional wrapper around [`BinaryStateMap`].
///
/// All mutations are guarded by an internal mutex, and batch operations
/// are applied to a staged copy before being committed atomically.
#[derive(Debug, Default)]
pub struct TransactionalStateMap {
    inner: Mutex<BinaryStateMap>,
}

impl TransactionalStateMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_state(state: BinaryStateMap) -> Self {
        Self {
            inner: Mutex::new(state),
        }
    }

    /// Acquire a lock and return a guard to the inner [`BinaryStateMap`].
    pub fn lock(&self) -> Result<MutexGuard<'_, BinaryStateMap>, String> {
        self.inner
            .lock()
            .map_err(|_: PoisonError<_>| "STATE_LOCK_POISON: mutex was poisoned".to_string())
    }

    /// Apply a batch of mutations atomically.
    ///
    /// `f` receives a mutable reference to a **staged copy** of the current
    /// state.  Only if `f` returns `Ok(())` is the staged copy committed.
    /// Any error from `f` leaves the live state unchanged.
    pub fn apply_batch<F>(&self, f: F) -> Result<(), String>
    where
        F: FnOnce(&mut BinaryStateMap) -> Result<(), String>,
    {
        let mut guard = self.lock()?;
        let mut staged = guard.clone();
        f(&mut staged)?;
        *guard = staged;
        Ok(())
    }

    /// Read a snapshot of the current state without holding the lock.
    pub fn snapshot(&self) -> Result<BinaryStateMap, String> {
        Ok(self.lock()?.clone())
    }
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
        self.set_validated(key, value)
    }

    pub fn set_validated(&mut self, key: impl Into<String>, value: BsmValue) -> Result<(), String> {
        self.insert_checked(key.into(), value)
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
            if key.len() > u16::MAX as usize {
                return Err("key too long for wire format".to_string());
            }

            out.extend_from_slice(&(key.len() as u16).to_be_bytes());
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
            if map.insert(key, value).is_some() {
                return Err("ORDER_VIOLATION: duplicate key detected".to_string());
            }
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
        self.validate_invariants()?;
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
        normalize_value(value.clone())?;
        let mut encoded = vec![value.type_tag()];
        encode_value_payload(value, &mut encoded)?;
        Ok(sha256_hex(&encoded))
    }

    /// Compare two snapshots and return per-key differences.
    ///
    /// Returns one [`StateDiff`] for every key that exists in `a` or `b` but
    /// not both, or whose value differs between the two maps.  Keys that are
    /// identical in both snapshots are omitted.
    pub fn diff(a: &BinaryStateMap, b: &BinaryStateMap) -> Vec<StateDiff> {
        let mut result = Vec::new();

        // Keys only in `a` (removed) and keys in both (possibly changed).
        for (key, val_a) in &a.inner {
            match b.inner.get(key) {
                None => result.push(StateDiff::Removed {
                    key: key.clone(),
                    value: val_a.clone(),
                }),
                Some(val_b) if val_b != val_a => result.push(StateDiff::Changed {
                    key: key.clone(),
                    from: val_a.clone(),
                    to: val_b.clone(),
                }),
                _ => {}
            }
        }

        // Keys only in `b` (added).
        for (key, val_b) in &b.inner {
            if !a.inner.contains_key(key) {
                result.push(StateDiff::Added {
                    key: key.clone(),
                    value: val_b.clone(),
                });
            }
        }

        // Sort by key for deterministic output.
        result.sort_by(|x, y| x.key().cmp(y.key()));
        result
    }

    fn insert_checked(&mut self, key: String, value: BsmValue) -> Result<(), String> {
        let value = normalize_value(value)?;
        if let Some(existing) = self.inner.get(&key) {
            if existing.type_tag() != value.type_tag() {
                return Err(format!(
                    "TYPE_MISMATCH: key {} expected type 0x{:02x}, got 0x{:02x}",
                    key,
                    existing.type_tag(),
                    value.type_tag()
                ));
            }
        }
        self.inner.insert(key, value);
        Ok(())
    }

    fn validate_invariants(&self) -> Result<(), String> {
        for value in self.inner.values() {
            normalize_value(value.clone())?;
        }
        Ok(())
    }
}

impl StateSnapshot {
    pub fn to_binary(&self) -> Result<Vec<u8>, String> {
        if self.namespace.len() > u16::MAX as usize {
            return Err("namespace too large".to_string());
        }
        for key in self.state.inner.keys() {
            validate_key(&self.namespace, key)?;
        }
        let expected_root = self.state.root_digest_hex()?;
        let expected_root_bytes = crate::hex::decode_hex(&expected_root)?;
        if expected_root_bytes.as_slice() != self.root_digest {
            return Err("TICK_SEAL_FAIL: snapshot root digest mismatch".to_string());
        }

        let mut out = Vec::new();
        out.extend_from_slice(&(self.namespace.len() as u16).to_be_bytes());
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
        for key in state.inner.keys() {
            validate_key(&namespace, key)?;
        }
        let computed_root = state.root_digest_hex()?;
        let computed_root_bytes = crate::hex::decode_hex(&computed_root)?;
        if computed_root_bytes.as_slice() != root_digest {
            return Err("TICK_SEAL_FAIL: snapshot root digest mismatch".to_string());
        }

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
            validate_decimal(v)?;
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
            validate_decimal(&decimal)?;
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

fn normalize_value(value: BsmValue) -> Result<BsmValue, String> {
    match value {
        BsmValue::Decimal(decimal) => {
            validate_decimal(&decimal)?;
            Ok(BsmValue::Decimal(decimal))
        }
        other => Ok(other),
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
    use super::{BinaryStateMap, BsmValue, StateDiff, StateSnapshot};

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
        let root_digest = state.root_digest_hex().expect("root digest");
        let root_digest_bytes = crate::hex::decode_hex(&root_digest).expect("decode root digest");
        let mut root_digest_array = [0u8; 32];
        root_digest_array.copy_from_slice(&root_digest_bytes);

        let snapshot = StateSnapshot {
            namespace: "tenant-a".to_string(),
            tick: 7,
            root_digest: root_digest_array,
            state,
        };

        let encoded = snapshot
            .to_binary()
            .expect("snapshot encode should succeed");
        let decoded = StateSnapshot::from_binary(&encoded).expect("snapshot decode should succeed");
        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn rejects_non_canonical_decimal_on_set() {
        let mut state = BinaryStateMap::new();
        let err = state
            .set(
                "tenant-a",
                "tenant-a:ratio",
                BsmValue::Decimal("001.2300".to_string()),
            )
            .expect_err("non-canonical decimal should fail");
        assert!(err.contains("INVALID_NUMERIC"));
    }

    #[test]
    fn rejects_type_drift_on_update() {
        let mut state = BinaryStateMap::new();
        state
            .set("tenant-a", "tenant-a:stable", BsmValue::Integer(1))
            .expect("set should succeed");
        let err = state
            .set(
                "tenant-a",
                "tenant-a:stable",
                BsmValue::String("one".to_string()),
            )
            .expect_err("type drift should fail");
        assert!(err.contains("TYPE_MISMATCH"));
    }

    #[test]
    fn rejects_snapshot_root_digest_mismatch() {
        let mut state = BinaryStateMap::new();
        state
            .set("tenant-a", "tenant-a:key", BsmValue::Integer(1))
            .expect("set should succeed");
        let mut root_digest = [0u8; 32];
        root_digest[0] = 1;
        let snapshot = StateSnapshot {
            namespace: "tenant-a".to_string(),
            tick: 1,
            root_digest,
            state,
        };
        let err = snapshot
            .to_binary()
            .expect_err("snapshot root mismatch should fail");
        assert!(err.contains("TICK_SEAL_FAIL"));
    }

    #[test]
    fn rejects_mixed_namespace_snapshot() {
        let mut state = BinaryStateMap::new();
        state
            .set_validated(
                "tenant-b:key",
                BsmValue::String("invalid namespace placement".to_string()),
            )
            .expect("value normalization should still succeed");
        let root_digest = state.root_digest_hex().expect("root digest");
        let root_digest_bytes = crate::hex::decode_hex(&root_digest).expect("decode root digest");
        let mut root_digest_array = [0u8; 32];
        root_digest_array.copy_from_slice(&root_digest_bytes);
        let snapshot = StateSnapshot {
            namespace: "tenant-a".to_string(),
            tick: 1,
            root_digest: root_digest_array,
            state,
        };
        let err = snapshot
            .to_binary()
            .expect_err("mixed namespace snapshot should fail");
        assert!(err.contains("NAMESPACE_LEAK"));
    }

    #[test]
    fn cross_language_determinism_vector_root_digest_stable() {
        let mut state = BinaryStateMap::new();
        state
            .set("tenant-a", "tenant-a:counter", BsmValue::Integer(42))
            .expect("set should succeed");
        state
            .set(
                "tenant-a",
                "tenant-a:ratio",
                BsmValue::Decimal("3.14".to_string()),
            )
            .expect("set should succeed");
        state
            .set("tenant-a", "tenant-a:flag", BsmValue::Boolean(true))
            .expect("set should succeed");

        let digest = state.root_digest_hex().expect("root digest");
        assert_eq!(
            digest,
            "768e154f65fb12f4419452ac76223006bf9097187294b0d9cec1260e22c664d3"
        );
    }

    // ---------------------------------------------------------------------------
    // Fix 5: TransactionalStateMap tests
    // ---------------------------------------------------------------------------

    #[test]
    fn transactional_batch_commits_on_success() {
        use super::TransactionalStateMap;

        let tsm = TransactionalStateMap::new();
        tsm.apply_batch(|state| {
            state.set("tenant-a", "tenant-a:x", BsmValue::Integer(1))?;
            state.set("tenant-a", "tenant-a:y", BsmValue::Integer(2))
        })
        .expect("batch should commit");

        let snap = tsm.snapshot().expect("snapshot");
        assert_eq!(snap.get("tenant-a:x"), Some(&BsmValue::Integer(1)));
        assert_eq!(snap.get("tenant-a:y"), Some(&BsmValue::Integer(2)));
    }

    #[test]
    fn transactional_batch_rolls_back_on_failure() {
        use super::TransactionalStateMap;

        let tsm = TransactionalStateMap::new();
        let err = tsm
            .apply_batch(|state| {
                state.set("tenant-a", "tenant-a:x", BsmValue::Integer(99))?;
                Err("deliberate failure".to_string())
            })
            .expect_err("batch should fail");
        assert!(err.contains("deliberate failure"));

        let snap = tsm.snapshot().expect("snapshot");
        assert!(
            snap.get("tenant-a:x").is_none(),
            "state must be unchanged after rollback"
        );
    }

    // ---------------------------------------------------------------------------
    // Pinned BSM wire-format vector tests
    //
    // These vectors pin the exact wire encoding for each supported value type.
    // Any change to the BSM serialisation format will break these tests — that is
    // intentional: the wire format is frozen at v1.0.0.
    // ---------------------------------------------------------------------------

    /// Empty BSM: entry_count(u32 BE) = 0x00000000.
    #[test]
    fn wire_vector_empty_bsm_is_four_zero_bytes() {
        let state = BinaryStateMap::new();
        let bytes = state.to_binary().expect("to_binary");
        assert_eq!(bytes, [0x00, 0x00, 0x00, 0x00]);
        assert_eq!(
            crate::digest::sha256_hex(&bytes),
            "df3f619804a92fdb4057192dc43dd748ea778adc52bc498ce80524c014b81119"
        );
    }

    /// Boolean(true) entry.
    /// Wire: entry_count=1 | key_len=6 | "a:flag" | type=0x01 | 0x01
    #[test]
    fn wire_vector_single_boolean_true() {
        let mut state = BinaryStateMap::new();
        state
            .set("a", "a:flag", BsmValue::Boolean(true))
            .expect("set");
        let bytes = state.to_binary().expect("to_binary");
        // entry_count (4B) + key_len (2B) + key (6B) + type (1B) + bool (1B) = 14 bytes
        assert_eq!(bytes.len(), 14);
        #[rustfmt::skip]
        let expected: &[u8] = &[
            0x00, 0x00, 0x00, 0x01,                   // entry_count = 1
            0x00, 0x06,                               // key_len = 6
            b'a', b':', b'f', b'l', b'a', b'g',      // key = "a:flag"
            0x01,                                     // type = BOOLEAN
            0x01,                                     // value = true
        ];
        assert_eq!(bytes, expected);
        assert_eq!(
            crate::digest::sha256_hex(&bytes),
            "897249c82a218bc108e20736341dafa170deff8045951ac76deb26d9e3a489b9"
        );
    }

    /// Integer(42) entry.
    /// Wire: entry_count=1 | key_len=9 | "a:counter" | type=0x02 | i64 BE
    #[test]
    fn wire_vector_single_integer_42() {
        let mut state = BinaryStateMap::new();
        state
            .set("a", "a:counter", BsmValue::Integer(42))
            .expect("set");
        let bytes = state.to_binary().expect("to_binary");
        // entry_count (4B) + key_len (2B) + key (9B) + type (1B) + i64 (8B) = 24 bytes
        assert_eq!(bytes.len(), 24);
        #[rustfmt::skip]
        let expected: &[u8] = &[
            0x00, 0x00, 0x00, 0x01,                                   // entry_count = 1
            0x00, 0x09,                                               // key_len = 9
            b'a', b':', b'c', b'o', b'u', b'n', b't', b'e', b'r',   // "a:counter"
            0x02,                                                     // type = INTEGER
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2a,          // 42 as i64 BE
        ];
        assert_eq!(bytes, expected);
        assert_eq!(
            crate::digest::sha256_hex(&bytes),
            "6ed61f752883704fde3678364580d328da813532682dd589ab8e1c65b997c2d6"
        );
    }

    /// §3.5 three-entry conformance vector.
    /// Keys in wire order (UTF-8 lex): counter < flag < ratio.
    /// Root digest MUST equal the pinned cross-language vector.
    #[test]
    fn wire_vector_section_3_5_three_entries_exact_bytes() {
        let mut state = BinaryStateMap::new();
        state
            .set("tenant-a", "tenant-a:counter", BsmValue::Integer(42))
            .expect("set counter");
        state
            .set(
                "tenant-a",
                "tenant-a:ratio",
                BsmValue::Decimal("3.14".to_string()),
            )
            .expect("set ratio");
        state
            .set("tenant-a", "tenant-a:flag", BsmValue::Boolean(true))
            .expect("set flag");

        let bytes = state.to_binary().expect("to_binary");
        assert_eq!(bytes.len(), 73, "wire length must be exactly 73 bytes");

        // Pin the exact wire hex so any encoding change is immediately visible.
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            concat!(
                "00000003", // entry_count = 3
                "0010",
                "74656e616e742d613a636f756e746572", // "tenant-a:counter" (16B)
                "02",
                "000000000000002a", // INTEGER 42
                "000d",
                "74656e616e742d613a666c6167", // "tenant-a:flag" (13B)
                "01",
                "01", // BOOLEAN true
                "000e",
                "74656e616e742d613a726174696f", // "tenant-a:ratio" (14B)
                "03",
                "00000004",
                "332e3134", // DECIMAL "3.14"
            )
        );

        assert_eq!(
            state.root_digest_hex().expect("root digest"),
            "768e154f65fb12f4419452ac76223006bf9097187294b0d9cec1260e22c664d3"
        );
    }

    /// Round-trip: encode → decode → encode produces identical bytes.
    #[test]
    fn wire_vector_round_trip_encode_decode_encode() {
        let mut state = BinaryStateMap::new();
        state.set("t", "t:a", BsmValue::Integer(1)).expect("set a");
        state
            .set("t", "t:b", BsmValue::Boolean(false))
            .expect("set b");
        state
            .set("t", "t:c", BsmValue::Decimal("9.99".to_string()))
            .expect("set c");
        state
            .set("t", "t:d", BsmValue::String("hello".to_string()))
            .expect("set d");
        state
            .set("t", "t:e", BsmValue::Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF]))
            .expect("set e");
        state.set("t", "t:f", BsmValue::Null).expect("set f");

        let bytes1 = state.to_binary().expect("encode");
        let decoded = BinaryStateMap::from_binary(&bytes1).expect("decode");
        let bytes2 = decoded.to_binary().expect("re-encode");
        assert_eq!(bytes1, bytes2, "round-trip must be byte-identical");
        assert_eq!(
            state.root_digest_hex().expect("digest1"),
            decoded.root_digest_hex().expect("digest2"),
            "root digest must be identical after round-trip"
        );
    }

    // ---------------------------------------------------------------------------
    // to_binary() round-trip — every BsmValue variant individually
    // ---------------------------------------------------------------------------

    #[test]
    fn to_binary_round_trip_boolean_false() {
        let mut state = BinaryStateMap::new();
        state
            .set("t", "t:v", BsmValue::Boolean(false))
            .expect("set");
        let encoded = state.to_binary().expect("encode");
        let decoded = BinaryStateMap::from_binary(&encoded).expect("decode");
        assert_eq!(decoded.get("t:v"), Some(&BsmValue::Boolean(false)));
    }

    #[test]
    fn to_binary_round_trip_integer_negative() {
        let mut state = BinaryStateMap::new();
        state.set("t", "t:v", BsmValue::Integer(-1)).expect("set");
        let encoded = state.to_binary().expect("encode");
        let decoded = BinaryStateMap::from_binary(&encoded).expect("decode");
        assert_eq!(decoded.get("t:v"), Some(&BsmValue::Integer(-1)));
    }

    #[test]
    fn to_binary_round_trip_integer_min_max() {
        let mut state = BinaryStateMap::new();
        state
            .set("t", "t:min", BsmValue::Integer(i64::MIN))
            .expect("set min");
        state
            .set("t", "t:max", BsmValue::Integer(i64::MAX))
            .expect("set max");
        let encoded = state.to_binary().expect("encode");
        let decoded = BinaryStateMap::from_binary(&encoded).expect("decode");
        assert_eq!(decoded.get("t:min"), Some(&BsmValue::Integer(i64::MIN)));
        assert_eq!(decoded.get("t:max"), Some(&BsmValue::Integer(i64::MAX)));
    }

    #[test]
    fn to_binary_round_trip_decimal() {
        let mut state = BinaryStateMap::new();
        state
            .set("t", "t:v", BsmValue::Decimal("0.0000001".to_string()))
            .expect("set");
        let encoded = state.to_binary().expect("encode");
        let decoded = BinaryStateMap::from_binary(&encoded).expect("decode");
        assert_eq!(
            decoded.get("t:v"),
            Some(&BsmValue::Decimal("0.0000001".to_string()))
        );
    }

    #[test]
    fn to_binary_round_trip_string() {
        let mut state = BinaryStateMap::new();
        state
            .set(
                "t",
                "t:v",
                BsmValue::String("hello, 世界 \u{0000}".to_string()),
            )
            .expect("set");
        let encoded = state.to_binary().expect("encode");
        let decoded = BinaryStateMap::from_binary(&encoded).expect("decode");
        assert_eq!(
            decoded.get("t:v"),
            Some(&BsmValue::String("hello, 世界 \u{0000}".to_string()))
        );
    }

    #[test]
    fn to_binary_round_trip_bytes() {
        let payload: Vec<u8> = (0u8..=255).collect();
        let mut state = BinaryStateMap::new();
        state
            .set("t", "t:v", BsmValue::Bytes(payload.clone()))
            .expect("set");
        let encoded = state.to_binary().expect("encode");
        let decoded = BinaryStateMap::from_binary(&encoded).expect("decode");
        assert_eq!(decoded.get("t:v"), Some(&BsmValue::Bytes(payload)));
    }

    #[test]
    fn to_binary_round_trip_null() {
        let mut state = BinaryStateMap::new();
        state.set("t", "t:v", BsmValue::Null).expect("set");
        let encoded = state.to_binary().expect("encode");
        let decoded = BinaryStateMap::from_binary(&encoded).expect("decode");
        assert_eq!(decoded.get("t:v"), Some(&BsmValue::Null));
    }

    // ---------------------------------------------------------------------------
    // from_binary() ordering violation rejection
    // ---------------------------------------------------------------------------

    #[test]
    fn from_binary_rejects_out_of_order_keys() {
        // Manually craft bytes with keys in reverse order: "t:b" before "t:a"
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2u32.to_be_bytes()); // entry_count = 2
        // First entry: "t:b" = Null
        let k1 = b"t:b";
        bytes.extend_from_slice(&(k1.len() as u16).to_be_bytes());
        bytes.extend_from_slice(k1);
        bytes.push(0x06); // Null
        // Second entry: "t:a" = Null (out of order)
        let k2 = b"t:a";
        bytes.extend_from_slice(&(k2.len() as u16).to_be_bytes());
        bytes.extend_from_slice(k2);
        bytes.push(0x06); // Null

        let err = BinaryStateMap::from_binary(&bytes).expect_err("out-of-order should fail");
        assert!(
            err.contains("ORDER_VIOLATION"),
            "expected ORDER_VIOLATION, got: {err}"
        );
    }

    #[test]
    fn from_binary_rejects_duplicate_keys() {
        // Craft bytes with two identical keys (same key twice — still "ordered" but duplicate)
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2u32.to_be_bytes());
        let k = b"t:a";
        for _ in 0..2 {
            bytes.extend_from_slice(&(k.len() as u16).to_be_bytes());
            bytes.extend_from_slice(k);
            bytes.push(0x06); // Null
        }
        let err = BinaryStateMap::from_binary(&bytes).expect_err("duplicate key should fail");
        assert!(
            err.contains("ORDER_VIOLATION"),
            "expected ORDER_VIOLATION, got: {err}"
        );
    }

    #[test]
    fn from_binary_rejects_truncated_input() {
        let mut state = BinaryStateMap::new();
        state.set("t", "t:v", BsmValue::Integer(42)).expect("set");
        let mut bytes = state.to_binary().expect("encode");
        bytes.pop(); // remove last byte
        BinaryStateMap::from_binary(&bytes).expect_err("truncated input should fail");
    }

    #[test]
    fn from_binary_rejects_trailing_bytes() {
        let mut state = BinaryStateMap::new();
        state.set("t", "t:v", BsmValue::Integer(1)).expect("set");
        let mut bytes = state.to_binary().expect("encode");
        bytes.push(0xff); // extra byte
        let err = BinaryStateMap::from_binary(&bytes).expect_err("trailing bytes should fail");
        assert!(
            err.contains("trailing"),
            "expected trailing bytes error, got: {err}"
        );
    }

    // ---------------------------------------------------------------------------
    // root_digest_hex() stability
    // ---------------------------------------------------------------------------

    #[test]
    fn root_digest_hex_is_stable_regardless_of_insertion_order() {
        // Insert in one order
        let mut s1 = BinaryStateMap::new();
        s1.set("t", "t:a", BsmValue::Integer(1)).expect("set a");
        s1.set("t", "t:b", BsmValue::Boolean(true)).expect("set b");
        s1.set("t", "t:c", BsmValue::Null).expect("set c");

        // Insert in reverse order
        let mut s2 = BinaryStateMap::new();
        s2.set("t", "t:c", BsmValue::Null).expect("set c");
        s2.set("t", "t:b", BsmValue::Boolean(true)).expect("set b");
        s2.set("t", "t:a", BsmValue::Integer(1)).expect("set a");

        assert_eq!(
            s1.root_digest_hex().expect("s1 digest"),
            s2.root_digest_hex().expect("s2 digest"),
            "root digest must be independent of insertion order"
        );
    }

    #[test]
    fn root_digest_hex_changes_when_value_changes() {
        let mut s = BinaryStateMap::new();
        s.set("t", "t:v", BsmValue::Integer(1)).expect("set");
        let d1 = s.root_digest_hex().expect("digest1");

        s.set("t", "t:v", BsmValue::Integer(2)).expect("update");
        let d2 = s.root_digest_hex().expect("digest2");

        assert_ne!(d1, d2, "digest must change when a value changes");
    }

    #[test]
    fn root_digest_hex_changes_when_key_added_or_removed() {
        let mut s = BinaryStateMap::new();
        s.set("t", "t:a", BsmValue::Integer(1)).expect("set a");
        let d1 = s.root_digest_hex().expect("d1");

        s.set("t", "t:b", BsmValue::Integer(2)).expect("set b");
        let d2 = s.root_digest_hex().expect("d2");
        assert_ne!(d1, d2, "digest must change when a key is added");

        s.delete("t", "t:b").expect("delete b");
        let d3 = s.root_digest_hex().expect("d3");
        assert_eq!(d1, d3, "digest must return to original after key removed");
    }

    // ---------------------------------------------------------------------------
    // to_canonical_json() canonicalization — all types
    // ---------------------------------------------------------------------------

    #[test]
    fn to_canonical_json_all_value_types() {
        let mut state = BinaryStateMap::new();
        state
            .set("t", "t:bool", BsmValue::Boolean(true))
            .expect("set bool");
        state
            .set("t", "t:dec", BsmValue::Decimal("1.5".to_string()))
            .expect("set dec");
        state
            .set("t", "t:int", BsmValue::Integer(99))
            .expect("set int");
        state.set("t", "t:null", BsmValue::Null).expect("set null");
        state
            .set("t", "t:str", BsmValue::String("hi".to_string()))
            .expect("set str");

        let json = state.to_canonical_json().expect("canonical json");
        // Keys must be sorted: bool < dec < int < null < str
        assert!(
            json.contains(r#""t:bool":true"#),
            "bool should be true: {json}"
        );
        assert!(
            json.contains(r#""t:dec":"1.5""#),
            "decimal should be string: {json}"
        );
        assert!(
            json.contains(r#""t:int":99"#),
            "int should be number: {json}"
        );
        assert!(
            json.contains(r#""t:null":null"#),
            "null should be null: {json}"
        );
        assert!(json.contains(r#""t:str":"hi""#), "string: {json}");
        // Verify full sorted key order
        let bool_pos = json.find("t:bool").expect("bool pos");
        let dec_pos = json.find("t:dec").expect("dec pos");
        let int_pos = json.find("t:int").expect("int pos");
        let null_pos = json.find("t:null").expect("null pos");
        let str_pos = json.find("t:str").expect("str pos");
        assert!(
            bool_pos < dec_pos && dec_pos < int_pos && int_pos < null_pos && null_pos < str_pos,
            "keys must be in UTF-8 lexicographic order: {json}"
        );
    }

    #[test]
    fn to_canonical_json_bytes_encoded_as_hex() {
        let mut state = BinaryStateMap::new();
        state
            .set("t", "t:v", BsmValue::Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF]))
            .expect("set");
        let json = state.to_canonical_json().expect("json");
        assert!(
            json.contains("deadbeef"),
            "bytes should be hex-encoded in canonical JSON: {json}"
        );
    }

    #[test]
    fn to_canonical_json_no_whitespace() {
        let mut state = BinaryStateMap::new();
        state.set("t", "t:a", BsmValue::Integer(1)).expect("set a");
        state.set("t", "t:b", BsmValue::Integer(2)).expect("set b");
        let json = state.to_canonical_json().expect("json");
        assert!(
            !json.contains(' ') && !json.contains('\n') && !json.contains('\t'),
            "canonical JSON must contain no whitespace: {json}"
        );
    }

    // ---------------------------------------------------------------------------
    // value_digest_hex() correctness — all six variants
    // ---------------------------------------------------------------------------

    #[test]
    fn value_digest_hex_boolean_true_is_deterministic() {
        let d1 = BinaryStateMap::value_digest_hex(&BsmValue::Boolean(true)).expect("d1");
        let d2 = BinaryStateMap::value_digest_hex(&BsmValue::Boolean(true)).expect("d2");
        assert_eq!(d1, d2);
        assert_eq!(d1.len(), 64, "digest must be 64 hex chars");
    }

    #[test]
    fn value_digest_hex_boolean_true_differs_from_false() {
        let dt = BinaryStateMap::value_digest_hex(&BsmValue::Boolean(true)).expect("true");
        let df = BinaryStateMap::value_digest_hex(&BsmValue::Boolean(false)).expect("false");
        assert_ne!(dt, df);
    }

    #[test]
    fn value_digest_hex_integer_is_deterministic() {
        let d = BinaryStateMap::value_digest_hex(&BsmValue::Integer(42)).expect("d");
        assert_eq!(d.len(), 64);
        // SHA-256(type_tag=0x02 || i64_BE(42))
        // bytes: [0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2a]
        let expected =
            crate::digest::sha256_hex(&[0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2a]);
        assert_eq!(d, expected);
    }

    #[test]
    fn value_digest_hex_decimal_is_deterministic() {
        let d =
            BinaryStateMap::value_digest_hex(&BsmValue::Decimal("3.14".to_string())).expect("d");
        assert_eq!(d.len(), 64);
        assert_eq!(
            d,
            BinaryStateMap::value_digest_hex(&BsmValue::Decimal("3.14".to_string())).expect("d2")
        );
    }

    #[test]
    fn value_digest_hex_decimal_rejects_non_canonical() {
        BinaryStateMap::value_digest_hex(&BsmValue::Decimal("001.0".to_string()))
            .expect_err("non-canonical decimal must fail");
    }

    #[test]
    fn value_digest_hex_string_is_deterministic() {
        let d =
            BinaryStateMap::value_digest_hex(&BsmValue::String("hello".to_string())).expect("d");
        assert_eq!(d.len(), 64);
    }

    #[test]
    fn value_digest_hex_bytes_is_deterministic() {
        let d = BinaryStateMap::value_digest_hex(&BsmValue::Bytes(vec![1, 2, 3])).expect("d");
        assert_eq!(d.len(), 64);
    }

    #[test]
    fn value_digest_hex_null_is_deterministic() {
        let d1 = BinaryStateMap::value_digest_hex(&BsmValue::Null).expect("d1");
        let d2 = BinaryStateMap::value_digest_hex(&BsmValue::Null).expect("d2");
        assert_eq!(d1, d2);
        assert_eq!(d1.len(), 64);
    }

    #[test]
    fn value_digest_hex_all_types_are_distinct() {
        let digests = [
            BinaryStateMap::value_digest_hex(&BsmValue::Boolean(true)).unwrap(),
            BinaryStateMap::value_digest_hex(&BsmValue::Boolean(false)).unwrap(),
            BinaryStateMap::value_digest_hex(&BsmValue::Integer(0)).unwrap(),
            BinaryStateMap::value_digest_hex(&BsmValue::Decimal("0".to_string())).unwrap(),
            BinaryStateMap::value_digest_hex(&BsmValue::String(String::new())).unwrap(),
            BinaryStateMap::value_digest_hex(&BsmValue::Bytes(vec![])).unwrap(),
            BinaryStateMap::value_digest_hex(&BsmValue::Null).unwrap(),
        ];
        // Each variant has a different type tag byte, so all digests must be distinct.
        for i in 0..digests.len() {
            for j in (i + 1)..digests.len() {
                assert_ne!(
                    digests[i], digests[j],
                    "variants {i} and {j} must produce different digests"
                );
            }
        }
    }

    // ---------------------------------------------------------------------------
    // BinaryStateMap::diff() conformance tests
    // ---------------------------------------------------------------------------

    #[test]
    fn diff_empty_maps_is_empty() {
        let a = BinaryStateMap::new();
        let b = BinaryStateMap::new();
        assert!(BinaryStateMap::diff(&a, &b).is_empty());
    }

    #[test]
    fn diff_identical_maps_is_empty() {
        let mut a = BinaryStateMap::new();
        a.set("t", "t:x", BsmValue::Integer(1)).unwrap();
        a.set("t", "t:y", BsmValue::Boolean(true)).unwrap();
        let b = a.clone();
        assert!(BinaryStateMap::diff(&a, &b).is_empty());
    }

    #[test]
    fn diff_detects_added_key() {
        let a = BinaryStateMap::new();
        let mut b = BinaryStateMap::new();
        b.set("t", "t:new", BsmValue::Integer(42)).unwrap();

        let diffs = BinaryStateMap::diff(&a, &b);
        assert_eq!(diffs.len(), 1);
        assert!(matches!(
            &diffs[0],
            StateDiff::Added { key, value } if key == "t:new" && *value == BsmValue::Integer(42)
        ));
    }

    #[test]
    fn diff_detects_removed_key() {
        let mut a = BinaryStateMap::new();
        a.set("t", "t:gone", BsmValue::Null).unwrap();
        let b = BinaryStateMap::new();

        let diffs = BinaryStateMap::diff(&a, &b);
        assert_eq!(diffs.len(), 1);
        assert!(matches!(
            &diffs[0],
            StateDiff::Removed { key, value } if key == "t:gone" && *value == BsmValue::Null
        ));
    }

    #[test]
    fn diff_detects_changed_value() {
        let mut a = BinaryStateMap::new();
        a.set("t", "t:v", BsmValue::Integer(1)).unwrap();
        let mut b = BinaryStateMap::new();
        b.set("t", "t:v", BsmValue::Integer(2)).unwrap();

        let diffs = BinaryStateMap::diff(&a, &b);
        assert_eq!(diffs.len(), 1);
        assert!(matches!(
            &diffs[0],
            StateDiff::Changed { key, from, to }
                if key == "t:v"
                && *from == BsmValue::Integer(1)
                && *to == BsmValue::Integer(2)
        ));
    }

    #[test]
    fn diff_result_is_sorted_by_key() {
        let mut a = BinaryStateMap::new();
        a.set("t", "t:a", BsmValue::Integer(1)).unwrap();
        a.set("t", "t:b", BsmValue::Integer(2)).unwrap();
        a.set("t", "t:c", BsmValue::Integer(3)).unwrap();

        let mut b = BinaryStateMap::new();
        b.set("t", "t:a", BsmValue::Integer(9)).unwrap(); // changed
        // t:b removed
        b.set("t", "t:d", BsmValue::Integer(4)).unwrap(); // added

        let diffs = BinaryStateMap::diff(&a, &b);
        let keys: Vec<&str> = diffs.iter().map(|d| d.key()).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "diff result must be sorted by key");
    }

    #[test]
    fn diff_with_all_diff_types_simultaneously() {
        let mut a = BinaryStateMap::new();
        a.set("t", "t:keep", BsmValue::Boolean(true)).unwrap();
        a.set("t", "t:change", BsmValue::Integer(1)).unwrap();
        a.set("t", "t:remove", BsmValue::Null).unwrap();

        let mut b = BinaryStateMap::new();
        b.set("t", "t:keep", BsmValue::Boolean(true)).unwrap(); // unchanged
        b.set("t", "t:change", BsmValue::Integer(99)).unwrap(); // changed
        // t:remove → absent
        b.set("t", "t:add", BsmValue::String("new".to_string()))
            .unwrap(); // added

        let diffs = BinaryStateMap::diff(&a, &b);
        assert_eq!(diffs.len(), 3, "keep must be excluded, 3 diffs expected");
        assert!(diffs.iter().all(|d| d.key() != "t:keep"));
        assert!(
            diffs
                .iter()
                .any(|d| matches!(d, StateDiff::Added { key, .. } if key == "t:add"))
        );
        assert!(
            diffs
                .iter()
                .any(|d| matches!(d, StateDiff::Removed { key, .. } if key == "t:remove"))
        );
        assert!(
            diffs
                .iter()
                .any(|d| matches!(d, StateDiff::Changed { key, .. } if key == "t:change"))
        );
    }
}
