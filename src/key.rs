pub const RESERVED_SYSTEM_NAMESPACE: &str = "trisync-system";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TenantKey {
    pub namespace: String,
    pub key: String,
}

impl TenantKey {
    pub fn new(namespace: impl Into<String>, key: impl Into<String>) -> Result<Self, String> {
        let namespace = namespace.into();
        let key = key.into();
        validate_namespace(&namespace)?;
        validate_key(&namespace, &key)?;
        Ok(Self { namespace, key })
    }
}

pub fn validate_namespace(namespace: &str) -> Result<(), String> {
    let bytes = namespace.as_bytes();
    if !(3..=63).contains(&bytes.len()) {
        return Err("namespace must be 3-63 bytes".to_string());
    }

    if namespace.starts_with('-') || namespace.ends_with('-') {
        return Err("namespace may not start or end with '-'".to_string());
    }

    for b in bytes {
        let valid = b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-';
        if !valid {
            return Err("namespace must match [a-z0-9-]".to_string());
        }
    }

    if namespace == RESERVED_SYSTEM_NAMESPACE {
        return Err(format!(
            "namespace '{RESERVED_SYSTEM_NAMESPACE}' is reserved and may not be used by tenants"
        ));
    }

    Ok(())
}

pub fn validate_key(namespace: &str, key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("key must not be empty".to_string());
    }
    if key.as_bytes().len() > 512 {
        return Err("key must be <=512 bytes".to_string());
    }
    if key.as_bytes().contains(&0) {
        return Err("key must not contain null byte".to_string());
    }

    let expected = format!("{namespace}:");
    if !key.starts_with(&expected) {
        return Err("NAMESPACE_LEAK: key does not match namespace prefix".to_string());
    }

    let suffix = &key[expected.len()..];
    if suffix.is_empty() || suffix.starts_with(':') {
        return Err("key suffix must be non-empty and may not start with ':'".to_string());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate_key, validate_namespace};

    #[test]
    fn validates_namespace_pattern() {
        assert!(validate_namespace("tenant-a").is_ok());
        assert!(validate_namespace("A").is_err());
        assert!(validate_namespace("ab").is_err());
    }

    #[test]
    fn validates_namespace_key_prefix() {
        assert!(validate_key("tenant-a", "tenant-a:counter").is_ok());
        assert!(validate_key("tenant-a", "tenant-b:counter").is_err());
    }

    #[test]
    fn rejects_reserved_system_namespace() {
        assert!(validate_namespace("trisync-system").is_err());
    }
}
