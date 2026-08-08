#!/usr/bin/env bash
set -euo pipefail

LOG_FILE="${1:-/tmp/tri-sync.log}"

cargo run -- apply --log "$LOG_FILE" --tenant tenant-a --key workflow --value queued
cargo run -- apply --log "$LOG_FILE" --tenant tenant-a --key workflow --value running
cargo run -- apply --log "$LOG_FILE" --tenant tenant-b --key workflow --value queued
cargo run -- delete --log "$LOG_FILE" --tenant tenant-b --key workflow
cargo run -- replay --log "$LOG_FILE"
