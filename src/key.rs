#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TenantKey {
    pub tenant: String,
    pub key: String,
}

impl TenantKey {
    pub fn new(tenant: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            tenant: tenant.into(),
            key: key.into(),
        }
    }
}
