use std::collections::BTreeMap;

use serde_json::Value;

use crate::hex::encode_hex;
use crate::key::TenantKey;

/// The semantic type of a value stored under a key.
///
/// Once a key is written with a particular `ValueType`, subsequent writes
/// must use the same type or they will be rejected by the type-stable
/// setter methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    Bytes,
    Decimal,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BinaryStateMap {
    inner: BTreeMap<TenantKey, Vec<u8>>,
    types: BTreeMap<TenantKey, ValueType>,
}

impl BinaryStateMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, tenant: impl Into<String>, key: impl Into<String>, value: Vec<u8>) {
        self.inner.insert(TenantKey::new(tenant, key), value);
    }

    pub fn set_tenant_key(&mut self, key: TenantKey, value: Vec<u8>) {
        self.inner.insert(key, value);
    }

    /// Type-stable bytes setter. Rejects the write if the key was previously
    /// written with `ValueType::Decimal`.
    pub fn set_bytes(
        &mut self,
        tenant: impl Into<String>,
        key: impl Into<String>,
        value: Vec<u8>,
    ) -> Result<(), String> {
        let tk = TenantKey::new(tenant, key);
        self.enforce_type(&tk, ValueType::Bytes)?;
        self.types.insert(tk.clone(), ValueType::Bytes);
        self.inner.insert(tk, value);
        Ok(())
    }

    /// Type-stable decimal setter. The caller-provided `decimal` string is
    /// stored as raw bytes. Rejects the write if the key was previously
    /// written with `ValueType::Bytes`.
    ///
    /// Note: canonical-form validation of the decimal string is the
    /// responsibility of the caller.
    pub fn set_decimal(
        &mut self,
        tenant: impl Into<String>,
        key: impl Into<String>,
        decimal: impl Into<String>,
    ) -> Result<(), String> {
        let tk = TenantKey::new(tenant, key);
        self.enforce_type(&tk, ValueType::Decimal)?;
        let bytes = decimal.into().into_bytes();
        self.types.insert(tk.clone(), ValueType::Decimal);
        self.inner.insert(tk, bytes);
        Ok(())
    }

    /// Returns the recorded `ValueType` for the given key, if any.
    pub fn value_type(&self, tenant: &str, key: &str) -> Option<ValueType> {
        self.types.get(&TenantKey::new(tenant, key)).copied()
    }

    pub fn delete(&mut self, tenant: &str, key: &str) {
        let tk = TenantKey::new(tenant, key);
        self.inner.remove(&tk);
        self.types.remove(&tk);
    }

    pub fn delete_tenant_key(&mut self, key: &TenantKey) {
        self.inner.remove(key);
        self.types.remove(key);
    }

    pub fn get(&self, tenant: &str, key: &str) -> Option<&[u8]> {
        self.inner
            .get(&TenantKey::new(tenant, key))
            .map(std::ops::Deref::deref)
    }

    pub fn entries(&self) -> impl Iterator<Item = (&TenantKey, &Vec<u8>)> {
        self.inner.iter()
    }

    pub fn to_nested_hex_json(&self) -> Value {
        let mut tenants: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        for (tenant_key, value) in self.entries() {
            tenants
                .entry(tenant_key.tenant.clone())
                .or_default()
                .insert(tenant_key.key.clone(), encode_hex(value));
        }

        serde_json::to_value(tenants).expect("BTreeMap serialization should never fail")
    }

    fn enforce_type(&self, key: &TenantKey, new_type: ValueType) -> Result<(), String> {
        if let Some(existing) = self.types.get(key) {
            if *existing != new_type {
                return Err(format!(
                    "type stability violation: key \"{}/{}\" has type {:?}, \
                     cannot overwrite with {:?}",
                    key.tenant, key.key, existing, new_type
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{BinaryStateMap, ValueType};

    #[test]
    fn maintains_multi_tenant_ordering() {
        let mut state = BinaryStateMap::new();
        state.set("tenant-b", "alpha", vec![1]);
        state.set("tenant-a", "zeta", vec![2]);
        state.set("tenant-a", "beta", vec![3]);

        let ordered: Vec<(String, String)> = state
            .entries()
            .map(|(k, _)| (k.tenant.clone(), k.key.clone()))
            .collect();
        assert_eq!(
            ordered,
            vec![
                ("tenant-a".to_string(), "beta".to_string()),
                ("tenant-a".to_string(), "zeta".to_string()),
                ("tenant-b".to_string(), "alpha".to_string()),
            ]
        );
    }

    #[test]
    fn set_bytes_records_type_and_allows_overwrite_with_same_type() {
        let mut state = BinaryStateMap::new();
        state.set_bytes("t", "k", b"hello".to_vec()).unwrap();
        assert_eq!(state.value_type("t", "k"), Some(ValueType::Bytes));
        // Overwriting with the same type is allowed.
        state.set_bytes("t", "k", b"world".to_vec()).unwrap();
        assert_eq!(state.get("t", "k"), Some(b"world".as_ref()));
    }

    #[test]
    fn set_decimal_records_type_and_allows_overwrite_with_same_type() {
        let mut state = BinaryStateMap::new();
        state.set_decimal("t", "counter", "1").unwrap();
        assert_eq!(state.value_type("t", "counter"), Some(ValueType::Decimal));
        state.set_decimal("t", "counter", "2").unwrap();
        assert_eq!(state.get("t", "counter"), Some(b"2".as_ref()));
    }

    #[test]
    fn type_stability_rejects_bytes_to_decimal_transition() {
        let mut state = BinaryStateMap::new();
        state.set_bytes("t", "k", b"raw".to_vec()).unwrap();
        let err = state.set_decimal("t", "k", "1").unwrap_err();
        assert!(err.contains("type stability violation"));
    }

    #[test]
    fn type_stability_rejects_decimal_to_bytes_transition() {
        let mut state = BinaryStateMap::new();
        state.set_decimal("t", "k", "1").unwrap();
        let err = state.set_bytes("t", "k", b"raw".to_vec()).unwrap_err();
        assert!(err.contains("type stability violation"));
    }

    #[test]
    fn delete_clears_type_registry() {
        let mut state = BinaryStateMap::new();
        state.set_bytes("t", "k", b"v".to_vec()).unwrap();
        state.delete("t", "k");
        assert_eq!(state.value_type("t", "k"), None);
        // After deletion the key may be reused with a different type.
        state.set_decimal("t", "k", "42").unwrap();
        assert_eq!(state.value_type("t", "k"), Some(ValueType::Decimal));
    }
}
