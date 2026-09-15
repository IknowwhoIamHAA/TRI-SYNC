//! TRI-SYNC offline license validation.
//!
//! Core local verification and community workflows are available without a
//! license. Enterprise features validate a signed license document from
//! `TRISYNC_LICENSE` or a local license file.

use std::env;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::canonical_json::to_canonical_string;
use crate::hex::decode_hex;

/// The environment variable that may contain a signed license document.
pub const LICENSE_ENV: &str = "TRISYNC_LICENSE";

/// The environment variable that overrides the path to the local license file.
pub const LICENSE_FILE_ENV: &str = "TRISYNC_LICENSE_FILE";

/// Embedded Ed25519 public key used to verify signed TRI-SYNC licenses.
pub const PUBLIC_KEY: &str = "acaa9e80d4fd6ffcce6a633f45e28fc91e0d9b851e44e6c72ca47b713c1ba408";

const DEFAULT_LICENSE_FILE_NAME: &str = "license.json";
const PROJECT_LICENSE_FILE_NAME: &str = "trisync-license.json";
const LICENSE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LicenseMode {
    Community,
    Licensed(LicenseDocument),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicenseDocument {
    pub license_version: u32,
    pub license_id: String,
    pub holder: String,
    pub tier: String,
    #[serde(default)]
    pub features: Vec<String>,
    pub issued_at: u64,
    #[serde(default)]
    pub expires_at: Option<u64>,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct LicensePayload<'a> {
    license_version: u32,
    license_id: &'a str,
    holder: &'a str,
    tier: &'a str,
    features: &'a [String],
    issued_at: u64,
    expires_at: Option<u64>,
}

#[derive(Debug, Clone)]
struct LoadedLicense {
    contents: String,
    source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "error_type")]
pub enum LicenseError {
    LicenseRequired {
        feature: String,
        detail: String,
    },
    EmptyLicenseKey {
        feature: Option<String>,
        detail: String,
    },
    LicenseStoreMissing {
        feature: Option<String>,
        detail: String,
    },
    InvalidLicenseKey {
        feature: Option<String>,
        detail: String,
    },
}

impl LicenseDocument {
    fn payload(&self) -> LicensePayload<'_> {
        LicensePayload {
            license_version: self.license_version,
            license_id: &self.license_id,
            holder: &self.holder,
            tier: &self.tier,
            features: &self.features,
            issued_at: self.issued_at,
            expires_at: self.expires_at,
        }
    }
}

impl LicenseError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::LicenseRequired { .. } => "LICENSE_REQUIRED",
            Self::EmptyLicenseKey { .. } => "LICENSE_KEY_EMPTY",
            Self::LicenseStoreMissing { .. } => "LICENSE_STORE_MISSING",
            Self::InvalidLicenseKey { .. } => "LICENSE_INVALID",
        }
    }

    pub fn exit_code(&self) -> i32 {
        3
    }

    pub fn to_json_value(&self) -> Value {
        let mut value = serde_json::to_value(self).unwrap_or_else(|_| {
            json!({
                "error_type": "LicenseRequired",
                "feature": "unknown",
                "detail": "failed to serialize license error"
            })
        });
        if let Some(object) = value.as_object_mut() {
            object.insert("code".to_string(), Value::String(self.code().to_string()));
            object.insert("exit_code".to_string(), Value::from(self.exit_code()));
            object.insert("message".to_string(), Value::String(self.to_string()));
        }
        value
    }

    pub fn to_stderr_json(&self) -> String {
        to_canonical_string(&self.to_json_value())
            .unwrap_or_else(|_| format!(r#"{{"code":"{}","message":"{}"}}"#, self.code(), self))
    }
}

impl Display for LicenseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::LicenseRequired { detail, .. }
            | Self::EmptyLicenseKey { detail, .. }
            | Self::LicenseStoreMissing { detail, .. }
            | Self::InvalidLicenseKey { detail, .. } => f.write_str(detail),
        }
    }
}

/// Return the current runtime mode.
pub fn current_mode() -> LicenseMode {
    match load_license_source(None) {
        Ok(Some(loaded)) => verify_loaded_license(&loaded, None)
            .map(LicenseMode::Licensed)
            .unwrap_or(LicenseMode::Community),
        Ok(None) | Err(_) => LicenseMode::Community,
    }
}

/// Require a valid commercial license for an enterprise feature.
pub fn require_enterprise(feature: &str) -> Result<(), String> {
    require_enterprise_detailed(feature).map_err(|err| err.to_string())
}

/// Require a valid commercial license for an enterprise feature with a structured error.
pub fn require_enterprise_detailed(feature: &str) -> Result<(), LicenseError> {
    let feature_name = feature.to_string();
    let loaded = load_license_source(Some(feature_name.clone()))?;

    let Some(loaded) = loaded else {
        return Err(LicenseError::LicenseRequired {
            feature: feature_name.clone(),
            detail: enterprise_feature_detail(
                feature,
                &format!(
                    "No signed TRI-SYNC license was provided. Supply one with ${LICENSE_ENV} or by placing a JSON license at ~/.trisync/{DEFAULT_LICENSE_FILE_NAME} or ./{PROJECT_LICENSE_FILE_NAME}."
                ),
            ),
        });
    };

    verify_loaded_license(&loaded, Some(feature_name.clone())).map(|_| ())
}

/// Validate the configured license if one is present.
///
/// Missing license material is treated as community mode and returns `Ok(())`.
pub fn check() -> Result<(), String> {
    check_detailed(None).map_err(|err| err.to_string())
}

pub fn check_detailed(feature: Option<String>) -> Result<(), LicenseError> {
    let Some(loaded) = load_license_source(feature.clone())? else {
        return Ok(());
    };

    verify_loaded_license(&loaded, feature).map(|_| ())
}

fn load_license_source(feature: Option<String>) -> Result<Option<LoadedLicense>, LicenseError> {
    match env::var(LICENSE_ENV) {
        Ok(value) if !value.trim().is_empty() => {
            return Ok(Some(LoadedLicense {
                contents: value,
                source: format!("${LICENSE_ENV}"),
            }));
        }
        Ok(_) => {
            return Err(LicenseError::EmptyLicenseKey {
                feature,
                detail: format!(
                    "The ${LICENSE_ENV} environment variable is set but empty. Set it to a signed TRI-SYNC license document or unset it to continue in community mode."
                ),
            });
        }
        Err(_) => {}
    }

    let Some(path) = resolve_license_file_path() else {
        return Ok(None);
    };

    let content = fs::read_to_string(&path).map_err(|err| LicenseError::LicenseStoreMissing {
        feature: feature.clone(),
        detail: format!(
            "Failed to read TRI-SYNC license file at {}: {err}\nCheck file permissions or set {LICENSE_FILE_ENV} to the correct path.",
            path.display()
        ),
    })?;

    if content.trim().is_empty() {
        return Err(LicenseError::EmptyLicenseKey {
            feature,
            detail: format!(
                "The TRI-SYNC license file at {} is empty. Provide a signed license document or remove the file to continue in community mode.",
                path.display()
            ),
        });
    }

    Ok(Some(LoadedLicense {
        contents: content,
        source: path.display().to_string(),
    }))
}

fn verify_loaded_license(
    loaded: &LoadedLicense,
    feature: Option<String>,
) -> Result<LicenseDocument, LicenseError> {
    let document = parse_license_document(&loaded.contents, &loaded.source, feature.clone())?;
    verify_license_document(&document, &loaded.source, feature)?;
    Ok(document)
}

fn parse_license_document(
    raw: &str,
    source: &str,
    feature: Option<String>,
) -> Result<LicenseDocument, LicenseError> {
    let parsed = serde_json::from_str::<LicenseDocument>(raw).map_err(|err| {
        LicenseError::InvalidLicenseKey {
            feature: feature.clone(),
            detail: format!(
                "Failed to parse TRI-SYNC license from {source}: {err}. The license must be a JSON document signed for offline verification."
            ),
        }
    })?;

    if parsed.license_version != LICENSE_SCHEMA_VERSION {
        return Err(LicenseError::InvalidLicenseKey {
            feature,
            detail: format!(
                "Unsupported TRI-SYNC license schema version {} in {source}. Expected version {LICENSE_SCHEMA_VERSION}.",
                parsed.license_version
            ),
        });
    }

    if parsed.license_id.trim().is_empty()
        || parsed.holder.trim().is_empty()
        || parsed.tier.trim().is_empty()
    {
        return Err(LicenseError::InvalidLicenseKey {
            feature,
            detail: format!(
                "The TRI-SYNC license from {source} is missing required fields (`license_id`, `holder`, or `tier`)."
            ),
        });
    }

    Ok(parsed)
}

fn verify_license_document(
    document: &LicenseDocument,
    source: &str,
    feature: Option<String>,
) -> Result<(), LicenseError> {
    let payload = serde_json::to_value(document.payload()).map_err(|err| {
        LicenseError::InvalidLicenseKey {
            feature: feature.clone(),
            detail: format!("Failed to encode TRI-SYNC license payload from {source}: {err}"),
        }
    })?;
    let canonical_payload =
        to_canonical_string(&payload).map_err(|err| LicenseError::InvalidLicenseKey {
            feature: feature.clone(),
            detail: format!("Failed to canonicalize TRI-SYNC license payload from {source}: {err}"),
        })?;

    let public_key_bytes =
        decode_hex(PUBLIC_KEY).map_err(|err| LicenseError::InvalidLicenseKey {
            feature: feature.clone(),
            detail: format!("Embedded TRI-SYNC public key is invalid: {err}"),
        })?;
    let public_key_bytes: [u8; 32] =
        public_key_bytes
            .try_into()
            .map_err(|_| LicenseError::InvalidLicenseKey {
                feature: feature.clone(),
                detail: "Embedded TRI-SYNC public key must be exactly 32 bytes.".to_string(),
            })?;
    let verifying_key = VerifyingKey::from_bytes(&public_key_bytes).map_err(|err| {
        LicenseError::InvalidLicenseKey {
            feature: feature.clone(),
            detail: format!("Embedded TRI-SYNC public key is invalid: {err}"),
        }
    })?;

    let signature_bytes =
        decode_hex(document.signature.trim()).map_err(|err| LicenseError::InvalidLicenseKey {
            feature: feature.clone(),
            detail: format!("TRI-SYNC license signature in {source} is not valid hex: {err}"),
        })?;
    let signature = Signature::try_from(signature_bytes.as_slice()).map_err(|err| {
        LicenseError::InvalidLicenseKey {
            feature: feature.clone(),
            detail: format!("TRI-SYNC license signature in {source} is malformed: {err}"),
        }
    })?;

    verifying_key
        .verify(canonical_payload.as_bytes(), &signature)
        .map_err(|_| LicenseError::InvalidLicenseKey {
            feature: feature.clone(),
            detail: format!(
                "TRI-SYNC license signature verification failed for {source}. Provide a valid offline license document signed by TRI-SYNC."
            ),
        })?;

    if let Some(expires_at) = document.expires_at {
        if expires_at < unix_timestamp_now() {
            return Err(LicenseError::InvalidLicenseKey {
                feature,
                detail: format!(
                    "The TRI-SYNC license in {source} expired at unix timestamp {expires_at}. Provide a renewed signed license document."
                ),
            });
        }
    }

    Ok(())
}

fn enterprise_feature_detail(feature: &str, detail: &str) -> String {
    format!(
        "{feature} is an enterprise feature and requires a valid offline TRI-SYNC license.\n\n{detail}"
    )
}

fn unix_timestamp_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn resolve_license_file_path() -> Option<PathBuf> {
    if let Ok(path) = env::var(LICENSE_FILE_ENV) {
        let path = PathBuf::from(path.trim());
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }

    let home_dir = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .map(PathBuf::from)
        .ok();

    if let Some(home) = home_dir {
        let candidate = home.join(".trisync").join(DEFAULT_LICENSE_FILE_NAME);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    if let Ok(current_dir) = env::current_dir() {
        let candidate = current_dir.join(PROJECT_LICENSE_FILE_NAME);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::Mutex;

    use ed25519_dalek::{Signer, SigningKey};

    use super::{
        LICENSE_ENV, LICENSE_FILE_ENV, LICENSE_SCHEMA_VERSION, LicenseDocument, LicenseError,
        LicenseMode, check, check_detailed, current_mode, require_enterprise,
        require_enterprise_detailed,
    };

    static ENV_LOCK: Mutex<()> = Mutex::new(());
    const TEST_PRIVATE_KEY: [u8; 32] = [
        0x3c, 0x56, 0x5b, 0xe7, 0xd1, 0xf9, 0xa6, 0x5f, 0xa8, 0x41, 0xa8, 0x12, 0x3d, 0x3d, 0x05,
        0x49, 0xfc, 0xf6, 0x25, 0x25, 0xfe, 0x28, 0x56, 0xcf, 0xdd, 0x51, 0x93, 0xd8, 0xc3, 0x32,
        0x8f, 0x9d,
    ];

    fn with_env_locked<F: FnOnce()>(vars: &[(&str, Option<&str>)], f: F) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let prev: Vec<Option<String>> = vars
            .iter()
            .map(|(k, v)| {
                let prev = std::env::var(k).ok();
                match v {
                    Some(val) => unsafe { std::env::set_var(k, val) },
                    None => unsafe { std::env::remove_var(k) },
                }
                prev
            })
            .collect();

        f();

        for ((k, _), prev_val) in vars.iter().zip(prev.iter()) {
            match prev_val {
                Some(v) => unsafe { std::env::set_var(k, v) },
                None => unsafe { std::env::remove_var(k) },
            }
        }
    }

    fn signed_license_json(expires_at: Option<u64>) -> String {
        let mut document = LicenseDocument {
            license_version: LICENSE_SCHEMA_VERSION,
            license_id: "lic_test_enterprise".to_string(),
            holder: "Example Corp".to_string(),
            tier: "enterprise".to_string(),
            features: vec![
                "commercial-production".to_string(),
                "compliance-reporting".to_string(),
            ],
            issued_at: 1_757_913_600,
            expires_at,
            signature: String::new(),
        };

        let payload = serde_json::to_value(document.payload()).expect("payload value");
        let canonical_payload =
            crate::canonical_json::to_canonical_string(&payload).expect("payload json");
        let signing_key = SigningKey::from_bytes(&TEST_PRIVATE_KEY);
        let signature = signing_key.sign(canonical_payload.as_bytes());
        document.signature = crate::hex::encode_hex(&signature.to_bytes());
        serde_json::to_string(&document).expect("license json")
    }

    #[test]
    fn current_mode_defaults_to_community_without_license() {
        with_env_locked(&[(LICENSE_ENV, None), (LICENSE_FILE_ENV, None)], || {
            assert_eq!(current_mode(), LicenseMode::Community);
        });
    }

    #[test]
    fn check_allows_community_mode_without_license() {
        with_env_locked(&[(LICENSE_ENV, None), (LICENSE_FILE_ENV, None)], || {
            check().expect("missing license should keep community mode");
        });
    }

    #[test]
    fn accepts_valid_license_from_environment() {
        let license = signed_license_json(Some(u64::MAX));
        with_env_locked(
            &[(LICENSE_ENV, Some(&license)), (LICENSE_FILE_ENV, None)],
            || {
                check_detailed(None).expect("valid license should verify");
                match current_mode() {
                    LicenseMode::Licensed(license) => assert_eq!(license.tier, "enterprise"),
                    LicenseMode::Community => panic!("expected licensed mode"),
                }
            },
        );
    }

    #[test]
    fn accepts_valid_license_from_file() {
        let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
        write!(tmp, "{}", signed_license_json(Some(u64::MAX))).expect("write license");
        let path = tmp.path().to_str().expect("path").to_string();

        with_env_locked(
            &[(LICENSE_ENV, None), (LICENSE_FILE_ENV, Some(&path))],
            || {
                check().expect("valid license file must be accepted");
            },
        );
    }

    #[test]
    fn rejects_invalid_license_signature() {
        let mut document: serde_json::Value =
            serde_json::from_str(&signed_license_json(Some(u64::MAX))).expect("license json");
        document["holder"] = serde_json::Value::String("Tampered Holder".to_string());
        let license = serde_json::to_string(&document).expect("tampered license");

        with_env_locked(
            &[(LICENSE_ENV, Some(&license)), (LICENSE_FILE_ENV, None)],
            || {
                let err = require_enterprise("Automated compliance reporting")
                    .expect_err("tampered license must be rejected");
                assert!(err.contains("signature verification failed"), "got: {err}");
            },
        );
    }

    #[test]
    fn rejects_expired_license() {
        let license = signed_license_json(Some(1));
        with_env_locked(
            &[(LICENSE_ENV, Some(&license)), (LICENSE_FILE_ENV, None)],
            || {
                let err = require_enterprise("Commercial production execution")
                    .expect_err("expired license must be rejected");
                assert!(err.contains("expired"), "got: {err}");
            },
        );
    }

    #[test]
    fn enterprise_features_explain_the_license_requirement() {
        with_env_locked(&[(LICENSE_ENV, None), (LICENSE_FILE_ENV, None)], || {
            let err = require_enterprise("Automated compliance reporting")
                .expect_err("enterprise feature should require a license");
            assert!(err.contains("Automated compliance reporting"), "got: {err}");
            assert!(err.contains(LICENSE_ENV), "got: {err}");
        });
    }

    #[test]
    fn enterprise_feature_returns_structured_license_error() {
        with_env_locked(&[(LICENSE_ENV, None), (LICENSE_FILE_ENV, None)], || {
            let err = require_enterprise_detailed("Automated compliance reporting")
                .expect_err("enterprise feature should require a license");
            assert!(matches!(err, LicenseError::LicenseRequired { .. }));
            assert_eq!(err.code(), "LICENSE_REQUIRED");
            assert_eq!(err.exit_code(), 3);
        });
    }

    #[test]
    fn rejects_empty_license_environment() {
        with_env_locked(&[(LICENSE_ENV, Some("")), (LICENSE_FILE_ENV, None)], || {
            let err = check().expect_err("empty environment license should fail");
            assert!(err.contains("empty"), "got: {err}");
        });
    }
}
