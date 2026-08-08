use std::collections::BTreeMap;

use serde_json::Value;

use crate::hex::encode_hex;
use crate::key::TenantKey;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BinaryStateMap {
    inner: BTreeMap<TenantKey, Vec<u8>>,
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

    pub fn delete(&mut self, tenant: &str, key: &str) {
        self.inner.remove(&TenantKey::new(tenant, key));
    }

    pub fn delete_tenant_key(&mut self, key: &TenantKey) {
        self.inner.remove(key);
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
}

#[cfg(test)]
mod tests {
    use super::BinaryStateMap;

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
}
