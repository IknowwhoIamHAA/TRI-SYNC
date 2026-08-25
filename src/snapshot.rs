use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::canonical_json::to_canonical_string;
use crate::digest::sha256_hex;
use crate::hex::encode_hex;
use crate::state_map::BinaryStateMap;

/// A point-in-time snapshot of a single namespace's state.
///
/// `entries` maps each key to the hex-encoded bytes of its value.
/// `root_digest` is the SHA-256 of the canonical JSON encoding of
/// `{ "entries": <entries>, "namespace": <namespace> }`.
///
/// Use [`Snapshot::validate`] (or [`Snapshot::from_json`]) to verify
/// integrity before trusting the contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub namespace: String,
    pub entries: BTreeMap<String, String>,
    pub root_digest: String,
}

impl Snapshot {
    /// Build a snapshot for `namespace` from the current state of `state`.
    /// Only entries belonging to `namespace` are included.
    pub fn from_state_map(namespace: &str, state: &BinaryStateMap) -> Self {
        let entries: BTreeMap<String, String> = state
            .entries()
            .filter(|(tk, _)| tk.tenant == namespace)
            .map(|(tk, v)| (tk.key.clone(), encode_hex(v)))
            .collect();

        let root_digest = compute_root_digest(namespace, &entries);

        Snapshot {
            namespace: namespace.to_string(),
            entries,
            root_digest,
        }
    }

    /// Recompute the root digest and verify it matches the stored value.
    ///
    /// Returns `Err` if the digest does not match, indicating that the
    /// snapshot has been corrupted or tampered with.
    pub fn validate(&self) -> Result<(), String> {
        let expected = compute_root_digest(&self.namespace, &self.entries);
        if expected == self.root_digest {
            Ok(())
        } else {
            Err(format!(
                "snapshot digest mismatch for namespace \"{}\": \
                 expected {}, got {}",
                self.namespace, expected, self.root_digest
            ))
        }
    }

    /// Deserialise a snapshot from JSON and verify its root digest.
    ///
    /// Returns `Err` if parsing fails or if the digest does not match.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let snapshot: Snapshot =
            serde_json::from_str(json).map_err(|e| format!("snapshot parse error: {e}"))?;
        snapshot.validate()?;
        Ok(snapshot)
    }

    /// Serialise the snapshot to a JSON string.
    pub fn to_json(&self) -> String {
        to_canonical_string(&serde_json::to_value(self).expect("Snapshot serialisation failed"))
            .expect("canonical encoding should not fail")
    }
}

/// A snapshot of the entire multi-tenant state map, keyed by namespace.
///
/// `entries` maps each namespace to its key→hex-value pairs.
/// `root_digest` is the SHA-256 of the canonical JSON encoding of
/// `{ "entries": <entries> }`.
///
/// Namespace isolation invariant: callers that require a single-namespace
/// view should call [`MultiNamespaceSnapshot::validate_namespace_isolation`]
/// before trusting the data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultiNamespaceSnapshot {
    /// Namespace → (key → hex value)
    pub entries: BTreeMap<String, BTreeMap<String, String>>,
    pub root_digest: String,
}

impl MultiNamespaceSnapshot {
    /// Build a full snapshot of all namespaces present in `state`.
    pub fn from_state_map(state: &BinaryStateMap) -> Self {
        let mut entries: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        for (tk, v) in state.entries() {
            entries
                .entry(tk.tenant.clone())
                .or_default()
                .insert(tk.key.clone(), encode_hex(v));
        }

        let root_digest = compute_multi_root_digest(&entries);

        MultiNamespaceSnapshot {
            entries,
            root_digest,
        }
    }

    /// Recompute the root digest and verify it matches the stored value.
    pub fn validate(&self) -> Result<(), String> {
        let expected = compute_multi_root_digest(&self.entries);
        if expected == self.root_digest {
            Ok(())
        } else {
            Err(format!(
                "multi-namespace snapshot digest mismatch: \
                 expected {}, got {}",
                expected, self.root_digest
            ))
        }
    }

    /// Enforce namespace isolation: the snapshot must contain entries for
    /// exactly one namespace. Returns the namespace name on success.
    ///
    /// Returns `Err` if `entries` is empty or spans more than one namespace.
    pub fn validate_namespace_isolation(&self) -> Result<&str, String> {
        match self.entries.len() {
            0 => Err("snapshot contains no namespaces".to_string()),
            1 => Ok(self.entries.keys().next().unwrap()),
            n => {
                let namespaces: Vec<&str> = self.entries.keys().map(String::as_str).collect();
                Err(format!(
                    "namespace isolation violation: snapshot spans {n} namespaces: {}",
                    namespaces.join(", ")
                ))
            }
        }
    }

    /// Deserialise from JSON and verify the root digest.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let snap: MultiNamespaceSnapshot =
            serde_json::from_str(json).map_err(|e| format!("snapshot parse error: {e}"))?;
        snap.validate()?;
        Ok(snap)
    }

    /// Serialise the snapshot to a JSON string.
    pub fn to_json(&self) -> String {
        to_canonical_string(&serde_json::to_value(self).expect("Snapshot serialisation failed"))
            .expect("canonical encoding should not fail")
    }
}

fn compute_root_digest(namespace: &str, entries: &BTreeMap<String, String>) -> String {
    let payload = json!({ "entries": entries, "namespace": namespace });
    let canonical = to_canonical_string(&payload)
        .expect("canonical encoding of snapshot payload should not fail");
    sha256_hex(canonical.as_bytes())
}

fn compute_multi_root_digest(entries: &BTreeMap<String, BTreeMap<String, String>>) -> String {
    let payload = json!({ "entries": entries });
    let canonical = to_canonical_string(&payload)
        .expect("canonical encoding of snapshot payload should not fail");
    sha256_hex(canonical.as_bytes())
}

#[cfg(test)]
mod tests {
    use crate::state_map::BinaryStateMap;

    use super::{MultiNamespaceSnapshot, Snapshot};

    fn populated_state() -> BinaryStateMap {
        let mut state = BinaryStateMap::new();
        state.set("tenant-a", "counter", b"42".to_vec());
        state.set("tenant-a", "flag", b"on".to_vec());
        state
    }

    // --- Snapshot (single-namespace) ---

    #[test]
    fn snapshot_round_trips_through_json() {
        let state = populated_state();
        let snap = Snapshot::from_state_map("tenant-a", &state);
        let json = snap.to_json();
        let restored = Snapshot::from_json(&json).expect("round-trip should succeed");
        assert_eq!(snap, restored);
    }

    #[test]
    fn snapshot_validate_passes_for_fresh_snapshot() {
        let state = populated_state();
        let snap = Snapshot::from_state_map("tenant-a", &state);
        snap.validate().expect("fresh snapshot should be valid");
    }

    #[test]
    fn snapshot_validate_fails_on_tampered_entry() {
        let state = populated_state();
        let mut snap = Snapshot::from_state_map("tenant-a", &state);
        snap.entries
            .insert("counter".to_string(), "deadbeef".to_string());
        let err = snap.validate().unwrap_err();
        assert!(err.contains("digest mismatch"));
    }

    #[test]
    fn snapshot_validate_fails_on_tampered_digest() {
        let state = populated_state();
        let mut snap = Snapshot::from_state_map("tenant-a", &state);
        snap.root_digest = "0".repeat(64);
        let err = snap.validate().unwrap_err();
        assert!(err.contains("digest mismatch"));
    }

    #[test]
    fn snapshot_from_json_rejects_tampered_input() {
        let state = populated_state();
        let snap = Snapshot::from_state_map("tenant-a", &state);
        let mut json_val: serde_json::Value = serde_json::from_str(&snap.to_json()).unwrap();
        json_val["root_digest"] = serde_json::Value::String("0".repeat(64));
        let tampered = serde_json::to_string(&json_val).unwrap();
        assert!(Snapshot::from_json(&tampered).is_err());
    }

    #[test]
    fn snapshot_only_includes_requested_namespace() {
        let mut state = BinaryStateMap::new();
        state.set("tenant-a", "key-a", b"1".to_vec());
        state.set("tenant-b", "key-b", b"2".to_vec());

        let snap = Snapshot::from_state_map("tenant-a", &state);
        assert!(snap.entries.contains_key("key-a"));
        assert!(!snap.entries.contains_key("key-b"));
    }

    // --- MultiNamespaceSnapshot ---

    #[test]
    fn multi_snapshot_round_trips_through_json() {
        let state = populated_state();
        let snap = MultiNamespaceSnapshot::from_state_map(&state);
        let json = snap.to_json();
        let restored = MultiNamespaceSnapshot::from_json(&json).expect("round-trip should succeed");
        assert_eq!(snap, restored);
    }

    #[test]
    fn multi_snapshot_validate_passes_for_fresh_snapshot() {
        let state = populated_state();
        let snap = MultiNamespaceSnapshot::from_state_map(&state);
        snap.validate().expect("fresh snapshot should be valid");
    }

    #[test]
    fn multi_snapshot_validate_fails_on_tampered_entry() {
        let state = populated_state();
        let mut snap = MultiNamespaceSnapshot::from_state_map(&state);
        snap.entries
            .get_mut("tenant-a")
            .unwrap()
            .insert("counter".to_string(), "deadbeef".to_string());
        let err = snap.validate().unwrap_err();
        assert!(err.contains("digest mismatch"));
    }

    #[test]
    fn namespace_isolation_passes_for_single_namespace() {
        let state = populated_state();
        let snap = MultiNamespaceSnapshot::from_state_map(&state);
        let ns = snap
            .validate_namespace_isolation()
            .expect("single namespace should pass");
        assert_eq!(ns, "tenant-a");
    }

    #[test]
    fn namespace_isolation_rejects_mixed_namespaces() {
        let mut state = BinaryStateMap::new();
        state.set("tenant-a", "x", b"1".to_vec());
        state.set("tenant-b", "y", b"2".to_vec());

        let snap = MultiNamespaceSnapshot::from_state_map(&state);
        let err = snap.validate_namespace_isolation().unwrap_err();
        assert!(err.contains("namespace isolation violation"));
        assert!(err.contains("tenant-a"));
        assert!(err.contains("tenant-b"));
    }

    #[test]
    fn namespace_isolation_rejects_empty_snapshot() {
        let snap = MultiNamespaceSnapshot {
            entries: Default::default(),
            root_digest: String::new(),
        };
        let err = snap.validate_namespace_isolation().unwrap_err();
        assert!(err.contains("no namespaces"));
    }
}
