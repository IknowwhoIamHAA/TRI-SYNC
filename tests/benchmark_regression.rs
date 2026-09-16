use std::time::Instant;
use std::{fs, process::Command};

use tempfile::tempdir;
use tri_sync::backend::{EventLogBackend, FileSystemBackend};
use tri_sync::event::{Event, ZERO_DIGEST_HEX};
use tri_sync::state_map::BsmValue;

const EVENT_COUNT: usize = 10_000;
const MIN_BATCH_THROUGHPUT_EVENTS_PER_SEC: f64 = 200.0;
const CLI_EVENT_COUNT: usize = 300;
const MIN_BATCH_SPEEDUP_OVER_SHELL: f64 = 1.20;

fn tri_sync_bin() -> &'static str {
    env!("CARGO_BIN_EXE_tri-sync")
}

#[test]
fn batch_append_throughput_stays_above_floor() {
    let temp = tempdir().expect("tempdir");
    let log_path = temp.path().join("events.jsonl");
    let backend = FileSystemBackend::open(&log_path);

    let mut prev_digest = ZERO_DIGEST_HEX.to_string();
    let mut events = Vec::with_capacity(EVENT_COUNT);

    for seq in 0..EVENT_COUNT {
        let event = Event::state_write(
            seq as u64,
            seq as u64,
            "tenant-a",
            format!("tenant-a:key:{}", seq % 1000),
            BsmValue::Integer((seq % 50000) as i64),
            false,
            prev_digest.clone(),
            None,
        )
        .expect("event");
        prev_digest = event.digest.clone();
        events.push(event);
    }

    #[test]
    fn cli_batch_outperforms_shell_style_apply_loop() {
        let temp = tempdir().expect("tempdir");
        let shell_log_path = temp.path().join("shell-events.jsonl");
        let batch_log_path = temp.path().join("batch-events.jsonl");
        let ops_path = temp.path().join("ops.jsonl");

        let shell_started = Instant::now();
        for seq in 0..CLI_EVENT_COUNT {
            let output = Command::new(tri_sync_bin())
                .args([
                    "apply",
                    "--log",
                    shell_log_path.to_str().expect("shell log path"),
                    "--namespace",
                    "tenant-a",
                    "--key",
                    &format!("k{}", seq % 100),
                    "--value",
                    &format!("v{}", seq),
                    "--tick",
                    &seq.to_string(),
                ])
                .output()
                .expect("run apply");
            assert!(
                output.status.success(),
                "apply failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let shell_elapsed = shell_started.elapsed();

        let mut ops = String::new();
        for seq in 0..CLI_EVENT_COUNT {
            ops.push_str(&format!(
                r#"{{"op":"apply","key":"k{}","value":"v{}","tick":{}}}"#,
                seq % 100,
                seq,
                seq
            ));
            ops.push('\n');
        }
        fs::write(&ops_path, ops).expect("write ops file");

        let batch_started = Instant::now();
        let batch_output = Command::new(tri_sync_bin())
            .args([
                "apply-batch",
                "--log",
                batch_log_path.to_str().expect("batch log path"),
                "--namespace",
                "tenant-a",
                "--input",
                ops_path.to_str().expect("ops path"),
            ])
            .output()
            .expect("run apply-batch");
        assert!(
            batch_output.status.success(),
            "apply-batch failed: {}",
            String::from_utf8_lossy(&batch_output.stderr)
        );
        let batch_elapsed = batch_started.elapsed();

        let shell_backend = FileSystemBackend::open(&shell_log_path);
        let batch_backend = FileSystemBackend::open(&batch_log_path);
        assert_eq!(
            shell_backend.load().expect("load shell events").len(),
            CLI_EVENT_COUNT
        );
        assert_eq!(
            batch_backend.load().expect("load batch events").len(),
            CLI_EVENT_COUNT
        );

        let shell_throughput = CLI_EVENT_COUNT as f64 / shell_elapsed.as_secs_f64();
        let batch_throughput = CLI_EVENT_COUNT as f64 / batch_elapsed.as_secs_f64();
        assert!(
            batch_throughput >= shell_throughput * MIN_BATCH_SPEEDUP_OVER_SHELL,
            "expected batch throughput >= {:.2}x shell throughput; shell={:.2} ev/s batch={:.2} ev/s",
            MIN_BATCH_SPEEDUP_OVER_SHELL,
            shell_throughput,
            batch_throughput
        );
    }

    let started = Instant::now();
    backend.append_batch(&events).expect("append batch");
    let elapsed = started.elapsed();
    let throughput = EVENT_COUNT as f64 / elapsed.as_secs_f64();

    assert!(
        throughput >= MIN_BATCH_THROUGHPUT_EVENTS_PER_SEC,
        "batch throughput {:.2} events/sec fell below floor {:.2} events/sec",
        throughput,
        MIN_BATCH_THROUGHPUT_EVENTS_PER_SEC
    );
}
