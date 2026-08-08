# TRI-SYNC
A local‑first deterministic runtime for reproducible workflows, automation, and AI auditing. TRI‑SYNC provides canonical JSON encoding, a binary state map, multi‑tenant isolation, and deterministic replay — eliminating state drift, cloud lock‑in, and “it worked on my machine” failures.

## Components
- Binary state map (`BinaryStateMap`) keyed by `(tenant, key)` in deterministic order
- Canonical JSON encoder with stable object-key ordering
- SHA‑256 digest module for event payload integrity
- Multi-tenant key ordering via sorted tenant/key tuples
- Append-only event log serialized as canonical JSON lines
- Deterministic replay engine that rebuilds state from the log

## CLI
```bash
cargo run -- apply --log /tmp/tri-sync.log --tenant acme --key task --value queued
cargo run -- apply --log /tmp/tri-sync.log --tenant acme --key task --value running
cargo run -- delete --log /tmp/tri-sync.log --tenant acme --key task
cargo run -- replay --log /tmp/tri-sync.log
cargo run -- digest --input "hello"
cargo run -- example --log /tmp/tri-sync-example.log
```

## Basic workflow example
See `/home/runner/work/TRI-SYNC/TRI-SYNC/examples/basic_workflow.sh`.

## Protocol
See `/home/runner/work/TRI-SYNC/TRI-SYNC/docs/protocol.md` for the initial TRI‑SYNC protocol format and replay guarantees.
