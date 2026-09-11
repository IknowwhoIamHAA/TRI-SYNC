use std::fs;
use std::process::Command;

use tempfile::tempdir;
use tri_sync::backend::{EventLogBackend, FileSystemBackend};
use tri_sync::canonical_json::to_canonical_string;
use tri_sync::event::Event;
use tri_sync::state_map::BsmValue;

fn tri_sync_bin() -> &'static str {
    env!("CARGO_BIN_EXE_tri-sync")
}

#[test]
fn verify_with_checkpoint_root_replays_only_tail() {
    let temp = tempdir().expect("tempdir");
    let log_path = temp.path().join("events.jsonl");
    let backend = FileSystemBackend::open(&log_path);

    let first = Event::state_write(
        0,
        1,
        "tenant-a",
        "tenant-a:key",
        BsmValue::Integer(1),
        false,
        tri_sync::event::ZERO_DIGEST_HEX,
        None,
    )
    .expect("first");
    backend.append(&first).expect("append first");

    let mut state = tri_sync::state_map::BinaryStateMap::new();
    state
        .set("tenant-a", "tenant-a:key", BsmValue::Integer(1))
        .expect("set");
    let checkpoint_root = state.root_digest_hex().expect("checkpoint root");
    let seal = Event::tick_seal(
        1,
        1,
        "tenant-a",
        1,
        checkpoint_root.clone(),
        first.digest.clone(),
        10,
    )
    .expect("seal");
    backend.append(&seal).expect("append seal");

    let first_verify = Command::new(tri_sync_bin())
        .args(["verify", "--log"])
        .arg(&log_path)
        .output()
        .expect("run verify");
    assert!(
        first_verify.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&first_verify.stderr)
    );

    let second = Event::state_write(
        2,
        2,
        "tenant-a",
        "tenant-a:key",
        BsmValue::Integer(2),
        false,
        seal.digest.clone(),
        None,
    )
    .expect("second");
    backend.append(&second).expect("append second");

    let mut final_state = tri_sync::state_map::BinaryStateMap::new();
    final_state
        .set("tenant-a", "tenant-a:key", BsmValue::Integer(2))
        .expect("set");
    let final_root = final_state.root_digest_hex().expect("final root");
    let final_seal = Event::tick_seal(3, 2, "tenant-a", 3, final_root, second.digest.clone(), 20)
        .expect("final seal");
    backend.append(&final_seal).expect("append final seal");

    let resumed = Command::new(tri_sync_bin())
        .args(["verify", "--log"])
        .arg(&log_path)
        .args(["--checkpoint-root", &checkpoint_root])
        .output()
        .expect("run checkpoint verify");
    assert!(
        resumed.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&resumed.stderr)
    );

    let stdout = String::from_utf8(resumed.stdout).expect("utf8 stdout");
    assert!(stdout.contains("checkpoint_root="));
    assert!(stdout.contains(&checkpoint_root));
    assert!(stdout.contains("verified_events=2"), "stdout: {stdout}");
}

#[test]
fn verify_missing_checkpoint_root_emits_json_error() {
    let temp = tempdir().expect("tempdir");
    let log_path = temp.path().join("events.jsonl");
    let backend = FileSystemBackend::open(&log_path);

    let first = Event::state_write(
        0,
        0,
        "tenant-a",
        "tenant-a:key",
        BsmValue::Integer(1),
        false,
        tri_sync::event::ZERO_DIGEST_HEX,
        None,
    )
    .expect("first");
    backend.append(&first).expect("append first");

    let output = Command::new(tri_sync_bin())
        .args([
            "verify",
            "--log",
            log_path.to_str().expect("log path"),
            "--checkpoint-root",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        ])
        .output()
        .expect("run verify");

    assert_eq!(output.status.code(), Some(6));
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    let json: serde_json::Value = serde_json::from_str(stderr.trim()).expect("json stderr");
    assert_eq!(json["error_type"], "MissingTickSeal");
    assert_eq!(json["code"], "MISSING_TICK_SEAL");
    assert_eq!(json["exit_code"], 6);
}

#[test]
fn replay_mixed_namespace_emits_json_error() {
    let temp = tempdir().expect("tempdir");
    let log_path = temp.path().join("mixed.jsonl");

    let first = Event::state_write(
        0,
        0,
        "tenant-a",
        "tenant-a:key",
        BsmValue::Integer(1),
        false,
        tri_sync::event::ZERO_DIGEST_HEX,
        None,
    )
    .expect("first");
    let second = Event::state_write(
        1,
        0,
        "tenant-b",
        "tenant-b:key",
        BsmValue::Integer(2),
        false,
        first.digest.clone(),
        None,
    )
    .expect("second");

    let payload = [first, second]
        .into_iter()
        .map(|event| {
            let value = serde_json::to_value(event).expect("event json");
            to_canonical_string(&value).expect("canonical event")
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&log_path, format!("{payload}\n")).expect("write log");

    let output = Command::new(tri_sync_bin())
        .args(["replay", "--log"])
        .arg(&log_path)
        .output()
        .expect("run replay");

    assert_eq!(output.status.code(), Some(5));
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    let json: serde_json::Value = serde_json::from_str(stderr.trim()).expect("json stderr");
    assert_eq!(json["error_type"], "NamespaceBreach");
    assert_eq!(json["code"], "NAMESPACE_BREACH");
    assert_eq!(json["exit_code"], 5);
}

#[test]
fn enterprise_gated_command_without_license_emits_json_error() {
    let temp = tempdir().expect("tempdir");
    let log_path = temp.path().join("events.jsonl");

    let output = Command::new(tri_sync_bin())
        .env_remove("TRISYNC_LICENSE_KEY")
        .env_remove("TRISYNC_LICENSE_KEYS_FILE")
        .args(["report", "--log", log_path.to_str().expect("log path")])
        .output()
        .expect("run report");

    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    let json: serde_json::Value = serde_json::from_str(stderr.trim()).expect("json stderr");
    assert_eq!(json["error_type"], "LicenseRequired");
    assert_eq!(json["code"], "LICENSE_REQUIRED");
    assert_eq!(json["exit_code"], 3);
}
