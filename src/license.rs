//! TRI-SYNC license validation.
//!
//! Every invocation of the `tri-sync` binary checks for a valid commercial
//! license key before running any command.  The key is read from the
//! `TRISYNC_LICENSE_KEY` environment variable.
//!
//! # Valid-key store
//!
//! Accepted keys are loaded from a newline-delimited text file.  The file
//! path is resolved in this order:
//!
//! 1. The `TRISYNC_LICENSE_KEYS_FILE` environment variable, if set and
//!    non-empty.
//! 2. `$HOME/.trisync/license_keys` (Unix) / `%USERPROFILE%\.trisync\license_keys` (Windows).
//! 3. `/etc/trisync/license_keys` (Linux/macOS only, for system-wide installs).
//!
//! Each non-empty, non-comment line (lines starting with `#` are ignored) in
//! the file is treated as one valid key.
//!
//! # Error behavior
//!
//! If no valid license key is found the binary prints a clear, actionable error
//! message to stderr and exits with code 1.  No protocol state is modified.

use std::env;
use std::fs;
use std::path::PathBuf;

/// The environment variable that must contain the license key.
pub const LICENSE_KEY_ENV: &str = "TRISYNC_LICENSE_KEY";

/// The environment variable that overrides the path to the valid-keys file.
pub const LICENSE_KEYS_FILE_ENV: &str = "TRISYNC_LICENSE_KEYS_FILE";

/// Check whether a valid license key is present.
///
/// Returns `Ok(())` when a valid key is found.  Returns `Err(String)` with a
/// human-readable message describing what is wrong and how to fix it.
///
/// Call this once at binary startup before executing any command.
pub fn check() -> Result<(), String> {
    let key = read_key()?;
    let valid_keys = load_valid_keys()?;

    if valid_keys.is_empty() {
        return Err(format!(
            "No license key store found.\n\
             Please ensure a valid-keys file exists at one of the default paths\n\
             or set {LICENSE_KEYS_FILE_ENV} to the correct path.\n\
             \n\
             To obtain a license key, visit: https://github.com/IknowwhoIamHAA/TRI-SYNC"
        ));
    }

    if valid_keys.contains(&key) {
        Ok(())
    } else {
        Err(format!(
            "Invalid or expired license key.\n\
             \n\
             The key set in ${LICENSE_KEY_ENV} was not found in the license key store.\n\
             \n\
             • To obtain or renew a license key, visit:\n  \
             https://github.com/IknowwhoIamHAA/TRI-SYNC\n\
             • Once you have a key, run:\n  \
             export {LICENSE_KEY_ENV}=<your-key>\n  \
             tri-sync <command>"
        ))
    }
}

/// Read the license key from the environment.
fn read_key() -> Result<String, String> {
    match env::var(LICENSE_KEY_ENV) {
        Ok(key) if !key.trim().is_empty() => Ok(key.trim().to_string()),
        Ok(_) => Err(format!(
            "The ${LICENSE_KEY_ENV} environment variable is set but empty.\n\
             Set it to your TRI-SYNC license key before running:\n  \
             export {LICENSE_KEY_ENV}=<your-key>"
        )),
        Err(_) => Err(format!(
            "TRI-SYNC requires a commercial license key.\n\
             \n\
             Set the ${LICENSE_KEY_ENV} environment variable to your license key:\n  \
             export {LICENSE_KEY_ENV}=<your-key>\n  \
             tri-sync <command>\n\
             \n\
             To obtain a license key, visit:\n  \
             https://github.com/IknowwhoIamHAA/TRI-SYNC"
        )),
    }
}

/// Load valid keys from the key-store file.
///
/// Returns an empty set if no key-store file is found (the caller is
/// responsible for treating an empty store as an error).
fn load_valid_keys() -> Result<std::collections::HashSet<String>, String> {
    let path = resolve_keys_file_path();

    let Some(path) = path else {
        return Ok(std::collections::HashSet::new());
    };

    let content = fs::read_to_string(&path).map_err(|err| {
        format!(
            "Failed to read license key store at {}: {err}\n\
             Check file permissions or set {LICENSE_KEYS_FILE_ENV} to the correct path.",
            path.display()
        )
    })?;

    Ok(parse_key_file_content(&content))
}

/// Parse a key-file content string and return the set of valid keys.
///
/// Empty lines and lines starting with `#` are ignored.
pub(crate) fn parse_key_file_content(content: &str) -> std::collections::HashSet<String> {
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(String::from)
        .collect()
}

/// Return the first existing candidate path for the valid-keys file, or `None`
/// if no candidate path exists on disk.
fn resolve_keys_file_path() -> Option<PathBuf> {
    // 1. Explicit override via environment variable.
    if let Ok(path) = env::var(LICENSE_KEYS_FILE_ENV) {
        let path = PathBuf::from(path.trim());
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }

    // 2. User home directory.
    let home_dir = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .map(PathBuf::from)
        .ok();

    if let Some(home) = home_dir {
        let candidate = home.join(".trisync").join("license_keys");
        if candidate.exists() {
            return Some(candidate);
        }
    }

    // 3. System-wide path (Linux/macOS).
    #[cfg(not(target_os = "windows"))]
    {
        let system = PathBuf::from("/etc/trisync/license_keys");
        if system.exists() {
            return Some(system);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::Mutex;

    use super::{LICENSE_KEY_ENV, LICENSE_KEYS_FILE_ENV, check, parse_key_file_content};

    // Serialize all env-mutating tests so they don't interfere with each other.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_env_locked<F: FnOnce()>(vars: &[(&str, Option<&str>)], f: F) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // Save and set
        let prev: Vec<Option<String>> = vars
            .iter()
            .map(|(k, v)| {
                let prev = std::env::var(k).ok();
                match v {
                    // SAFETY: guarded by ENV_LOCK; restored before returning.
                    Some(val) => unsafe { std::env::set_var(k, val) },
                    None => unsafe { std::env::remove_var(k) },
                }
                prev
            })
            .collect();

        f();

        // Restore
        for ((k, _), prev_val) in vars.iter().zip(prev.iter()) {
            match prev_val {
                // SAFETY: guarded by ENV_LOCK.
                Some(v) => unsafe { std::env::set_var(k, v) },
                None => unsafe { std::env::remove_var(k) },
            }
        }
    }

    // -------------------------------------------------------------------------
    // Pure-function tests — no env manipulation needed
    // -------------------------------------------------------------------------

    #[test]
    fn parse_key_file_ignores_blank_lines() {
        let content = "\n  \n\nVALID-KEY-1\n";
        let keys = parse_key_file_content(content);
        assert!(keys.contains("VALID-KEY-1"));
        assert_eq!(keys.len(), 1);
    }

    #[test]
    fn parse_key_file_ignores_comment_lines() {
        let content = "# comment\nREAL-KEY-999\n# another\n";
        let keys = parse_key_file_content(content);
        assert!(!keys.contains("# comment"));
        assert!(keys.contains("REAL-KEY-999"));
        assert_eq!(keys.len(), 1);
    }

    #[test]
    fn parse_key_file_trims_whitespace() {
        let content = "  PADDED-KEY  \n";
        let keys = parse_key_file_content(content);
        assert!(keys.contains("PADDED-KEY"));
    }

    #[test]
    fn parse_key_file_accepts_multiple_keys() {
        let content = "KEY-A\nKEY-B\nKEY-C\n";
        let keys = parse_key_file_content(content);
        assert_eq!(keys.len(), 3);
        assert!(keys.contains("KEY-A"));
        assert!(keys.contains("KEY-B"));
        assert!(keys.contains("KEY-C"));
    }

    // -------------------------------------------------------------------------
    // Integration tests — use ENV_LOCK to serialize env mutations
    // -------------------------------------------------------------------------

    #[test]
    fn rejects_missing_key_env() {
        with_env_locked(
            &[(LICENSE_KEY_ENV, None), (LICENSE_KEYS_FILE_ENV, None)],
            || {
                let err = check().expect_err("should fail without key");
                assert!(
                    err.contains(LICENSE_KEY_ENV),
                    "error should mention the env var: {err}"
                );
            },
        );
    }

    #[test]
    fn rejects_empty_key_env() {
        with_env_locked(
            &[(LICENSE_KEY_ENV, Some("")), (LICENSE_KEYS_FILE_ENV, None)],
            || {
                let err = check().expect_err("should fail with empty key");
                assert!(err.contains("empty"), "got: {err}");
            },
        );
    }

    #[test]
    fn accepts_valid_key_from_file() {
        let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
        writeln!(tmp, "# TRI-SYNC license keys").unwrap();
        writeln!(tmp, "TEST-KEY-VALID-12345").unwrap();
        writeln!(tmp, "ANOTHER-KEY-67890").unwrap();
        let path = tmp.path().to_str().expect("path").to_string();

        with_env_locked(
            &[
                (LICENSE_KEY_ENV, Some("TEST-KEY-VALID-12345")),
                (LICENSE_KEYS_FILE_ENV, Some(&path)),
            ],
            || {
                check().expect("valid key must be accepted");
            },
        );
    }

    #[test]
    fn rejects_invalid_key_from_file() {
        let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
        writeln!(tmp, "CORRECT-KEY-ABC").unwrap();
        let path = tmp.path().to_str().expect("path").to_string();

        with_env_locked(
            &[
                (LICENSE_KEY_ENV, Some("WRONG-KEY-XYZ")),
                (LICENSE_KEYS_FILE_ENV, Some(&path)),
            ],
            || {
                let err = check().expect_err("wrong key must be rejected");
                assert!(
                    err.contains("Invalid") || err.contains("expired"),
                    "got: {err}"
                );
            },
        );
    }
}
