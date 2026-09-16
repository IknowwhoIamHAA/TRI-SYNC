use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::tempdir;
use tri_sync::backend::{EventLogBackend, FileSystemBackend};
use tri_sync::canonical_json::to_canonical_string;
use tri_sync::event::Event;
use tri_sync::state_map::BsmValue;

fn tri_sync_bin() -> &'static str {
    env!("CARGO_BIN_EXE_tri-sync")
}

fn snapshot_cache_path(log_path: &Path, checkpoint_root: &str) -> PathBuf {
    PathBuf::from(format!("{}.snapshots", log_path.display()))
        .join(format!("{checkpoint_root}.snapshot.bin"))
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
fn replay_sequence_collision_emits_json_error() {
    let temp = tempdir().expect("tempdir");
    let log_path = temp.path().join("collision.jsonl");

    let first = Event::state_write(
        0,
        0,
        "tenant-a",
        "tenant-a:key-a",
        BsmValue::Integer(1),
        false,
        tri_sync::event::ZERO_DIGEST_HEX,
        None,
    )
    .expect("first");
    let second = Event::state_write(
        0,
        0,
        "tenant-a",
        "tenant-a:key-b",
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
    assert_eq!(json["error_type"], "SequenceCollision");
    assert_eq!(json["code"], "SEQUENCE_COLLISION");
    assert_eq!(json["exit_code"], 5);
    assert_eq!(json["seq"], 0);
}

#[test]
fn verify_checkpoint_root_rejects_ambiguous_lineage() {
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

    let mut state_one = tri_sync::state_map::BinaryStateMap::new();
    state_one
        .set("tenant-a", "tenant-a:key", BsmValue::Integer(1))
        .expect("set");
    let repeated_root = state_one.root_digest_hex().expect("root one");
    let first_seal = Event::tick_seal(
        1,
        1,
        "tenant-a",
        1,
        repeated_root.clone(),
        first.digest.clone(),
        10,
    )
    .expect("first seal");
    backend.append(&first_seal).expect("append first seal");

    let second = Event::state_write(
        2,
        2,
        "tenant-a",
        "tenant-a:key",
        BsmValue::Integer(2),
        false,
        first_seal.digest.clone(),
        None,
    )
    .expect("second");
    backend.append(&second).expect("append second");

    let third = Event::state_write(
        3,
        3,
        "tenant-a",
        "tenant-a:key",
        BsmValue::Integer(1),
        false,
        second.digest.clone(),
        None,
    )
    .expect("third");
    backend.append(&third).expect("append third");

    let repeated_seal = Event::tick_seal(
        4,
        3,
        "tenant-a",
        4,
        repeated_root.clone(),
        third.digest.clone(),
        20,
    )
    .expect("repeated seal");
    backend
        .append(&repeated_seal)
        .expect("append repeated seal");

    let initial_verify = Command::new(tri_sync_bin())
        .args(["verify", "--log"])
        .arg(&log_path)
        .output()
        .expect("run verify");
    assert!(
        initial_verify.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&initial_verify.stderr)
    );

    let output = Command::new(tri_sync_bin())
        .args(["verify", "--log"])
        .arg(&log_path)
        .args(["--checkpoint-root", &repeated_root])
        .output()
        .expect("run checkpoint verify");

    assert_eq!(output.status.code(), Some(6));
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    let json: serde_json::Value = serde_json::from_str(stderr.trim()).expect("json stderr");
    assert_eq!(json["error_type"], "StateMismatch");
    assert_eq!(json["code"], "STATE_MISMATCH");
    assert_eq!(json["exit_code"], 6);
}

#[test]
fn verify_with_checkpoint_root_uses_persisted_snapshot_cache() {
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

    let mut checkpoint_state = tri_sync::state_map::BinaryStateMap::new();
    checkpoint_state
        .set("tenant-a", "tenant-a:key", BsmValue::Integer(1))
        .expect("set");
    let checkpoint_root = checkpoint_state.root_digest_hex().expect("checkpoint root");
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

    let segments_dir = std::path::PathBuf::from(format!("{}.segments", log_path.display()));
    let segment_path = fs::read_dir(&segments_dir)
        .expect("segments dir")
        .next()
        .expect("segment entry")
        .expect("dir entry")
        .path();
    let lines: Vec<String> = fs::read_to_string(&segment_path)
        .expect("segment contents")
        .lines()
        .map(str::to_string)
        .collect();
    assert!(lines.len() >= 4, "unexpected segment format");
    let mut mutated = Vec::new();
    mutated.push(lines[0].clone());
    mutated.extend(lines.iter().skip(2).cloned());
    fs::write(&segment_path, format!("{}\n", mutated.join("\n"))).expect("rewrite segment");

    let output = Command::new(tri_sync_bin())
        .args(["verify", "--log"])
        .arg(&log_path)
        .args(["--checkpoint-root", &checkpoint_root])
        .output()
        .expect("run checkpoint verify");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(stdout.contains("checkpoint_root="));
    assert!(stdout.contains("verified_events=1"), "stdout: {stdout}");
}

#[test]
fn verify_with_checkpoint_root_falls_back_when_snapshot_cache_missing() {
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

    let mut checkpoint_state = tri_sync::state_map::BinaryStateMap::new();
    checkpoint_state
        .set("tenant-a", "tenant-a:key", BsmValue::Integer(1))
        .expect("set");
    let checkpoint_root = checkpoint_state.root_digest_hex().expect("checkpoint root");
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

    let cache_path = snapshot_cache_path(&log_path, &checkpoint_root);
    fs::remove_file(&cache_path).expect("remove snapshot cache");

    let output = Command::new(tri_sync_bin())
        .args(["verify", "--log"])
        .arg(&log_path)
        .args(["--checkpoint-root", &checkpoint_root])
        .output()
        .expect("run checkpoint verify");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(stdout.contains("verified_events=1"), "stdout: {stdout}");
}

#[test]
fn verify_with_checkpoint_root_falls_back_when_snapshot_cache_corrupt() {
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

    let mut checkpoint_state = tri_sync::state_map::BinaryStateMap::new();
    checkpoint_state
        .set("tenant-a", "tenant-a:key", BsmValue::Integer(1))
        .expect("set");
    let checkpoint_root = checkpoint_state.root_digest_hex().expect("checkpoint root");
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

    let cache_path = snapshot_cache_path(&log_path, &checkpoint_root);
    fs::write(&cache_path, [0x00, 0xFF, 0x12, 0x34]).expect("corrupt snapshot cache");

    let output = Command::new(tri_sync_bin())
        .args(["verify", "--log"])
        .arg(&log_path)
        .args(["--checkpoint-root", &checkpoint_root])
        .output()
        .expect("run checkpoint verify");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(stdout.contains("verified_events=1"), "stdout: {stdout}");
}

#[test]
fn verify_with_checkpoint_root_rebuilds_when_snapshot_cache_stale() {
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

    let mut state1 = tri_sync::state_map::BinaryStateMap::new();
    state1
        .set("tenant-a", "tenant-a:key", BsmValue::Integer(1))
        .expect("set");
    let checkpoint_root_one = state1.root_digest_hex().expect("checkpoint root one");
    let seal_one = Event::tick_seal(
        1,
        1,
        "tenant-a",
        1,
        checkpoint_root_one.clone(),
        first.digest.clone(),
        10,
    )
    .expect("seal one");
    backend.append(&seal_one).expect("append seal one");

    let second = Event::state_write(
        2,
        2,
        "tenant-a",
        "tenant-a:key",
        BsmValue::Integer(2),
        false,
        seal_one.digest.clone(),
        None,
    )
    .expect("second");
    backend.append(&second).expect("append second");

    let mut state2 = tri_sync::state_map::BinaryStateMap::new();
    state2
        .set("tenant-a", "tenant-a:key", BsmValue::Integer(2))
        .expect("set");
    let checkpoint_root_two = state2.root_digest_hex().expect("checkpoint root two");
    let seal_two = Event::tick_seal(
        3,
        2,
        "tenant-a",
        3,
        checkpoint_root_two.clone(),
        second.digest.clone(),
        20,
    )
    .expect("seal two");
    backend.append(&seal_two).expect("append seal two");

    let snapshot_one = snapshot_cache_path(&log_path, &checkpoint_root_one);
    let snapshot_two = snapshot_cache_path(&log_path, &checkpoint_root_two);
    let snapshot_two_bytes = fs::read(&snapshot_two).expect("read second snapshot");
    fs::write(&snapshot_one, snapshot_two_bytes).expect("overwrite stale snapshot");

    let output = Command::new(tri_sync_bin())
        .args(["verify", "--log"])
        .arg(&log_path)
        .args(["--checkpoint-root", &checkpoint_root_one])
        .output()
        .expect("run checkpoint verify");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(stdout.contains("verified_events=2"), "stdout: {stdout}");
}

#[test]
fn enterprise_gated_command_without_license_emits_json_error() {
    let temp = tempdir().expect("tempdir");
    let log_path = temp.path().join("events.jsonl");

    let output = Command::new(tri_sync_bin())
        .env_remove("TRISYNC_LICENSE")
        .env_remove("TRISYNC_LICENSE_FILE")
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

#[test]
fn apply_rejects_reserved_system_namespace() {
    let temp = tempdir().expect("tempdir");
    let log_path = temp.path().join("events.jsonl");

    let output = Command::new(tri_sync_bin())
        .args([
            "apply",
            "--log",
            log_path.to_str().expect("log path"),
            "--namespace",
            "trisync-system",
            "--key",
            "job-status",
            "--value",
            "running",
            "--tick",
            "1",
        ])
        .output()
        .expect("run apply");

    assert!(!output.status.success(), "apply unexpectedly succeeded");
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    let json: serde_json::Value = serde_json::from_str(stderr.trim()).expect("json stderr");
    assert_eq!(json["code"], "INVALID_NAMESPACE");
    let message = json["message"].as_str().expect("message string");
    assert!(
        message.contains("INVALID_NAMESPACE"),
        "unexpected message: {message}"
    );
}

#[test]
fn apply_batch_matches_sequential_apply_delete_parity() {
    let temp = tempdir().expect("tempdir");
    let batch_log = temp.path().join("batch-events.jsonl");
    let sequential_log = temp.path().join("sequential-events.jsonl");
    let ops_path = temp.path().join("ops.jsonl");

    fs::write(
        &ops_path,
        [
            r#"{"op":"apply","key":"job-status","value":"running","tick":1}"#,
            r#"{"op":"apply","key":"job-status","value":"done","tick":2}"#,
            r#"{"op":"delete","key":"job-status","tick":3}"#,
        ]
        .join("\n")
            + "\n",
    )
    .expect("write batch ops");

    let batch_output = Command::new(tri_sync_bin())
        .args([
            "apply-batch",
            "--log",
            batch_log.to_str().expect("batch log path"),
            "--namespace",
            "tenant-a",
            "--input",
            ops_path.to_str().expect("ops path"),
        ])
        .output()
        .expect("run apply-batch");
    assert!(
        batch_output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&batch_output.stderr)
    );

    for (tick, value) in [("1", "running"), ("2", "done")] {
        let output = Command::new(tri_sync_bin())
            .args([
                "apply",
                "--log",
                sequential_log.to_str().expect("sequential log path"),
                "--namespace",
                "tenant-a",
                "--key",
                "job-status",
                "--value",
                value,
                "--tick",
                tick,
            ])
            .output()
            .expect("run apply");
        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let delete_output = Command::new(tri_sync_bin())
        .args([
            "delete",
            "--log",
            sequential_log.to_str().expect("sequential log path"),
            "--namespace",
            "tenant-a",
            "--key",
            "job-status",
            "--tick",
            "3",
        ])
        .output()
        .expect("run delete");
    assert!(
        delete_output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&delete_output.stderr)
    );

    let verify_batch = Command::new(tri_sync_bin())
        .args(["verify", "--log"])
        .arg(&batch_log)
        .output()
        .expect("verify batch log");
    assert!(
        verify_batch.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&verify_batch.stderr)
    );

    let verify_sequential = Command::new(tri_sync_bin())
        .args(["verify", "--log"])
        .arg(&sequential_log)
        .output()
        .expect("verify sequential log");
    assert!(
        verify_sequential.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&verify_sequential.stderr)
    );

    let batch_stdout = String::from_utf8(verify_batch.stdout).expect("batch stdout");
    let sequential_stdout = String::from_utf8(verify_sequential.stdout).expect("sequential stdout");
    assert_eq!(
        extract_field(&batch_stdout, "root_digest"),
        extract_field(&sequential_stdout, "root_digest")
    );
}

fn extract_field<'a>(output: &'a str, key: &str) -> &'a str {
    output
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key}=")))
        .expect("field missing")
}
