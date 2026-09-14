# TRI-SYNC Architecture

## Runtime Layers
1. **Protocol Layer** — Encoding, invariants, digest rules.
2. **State Layer** — Binary map, canonical JSON, tenant isolation.
   - `BinaryStateMap` — core key-value store with BSM binary encoding and root-digest computation.
   - `TransactionalStateMap` — wraps `BinaryStateMap` in a `Mutex`; provides atomic batch
     mutations via a clone-stage-commit pattern.
3. **Event Layer** — Append-only log, replay engine.
   - `EventLogBackend` — pluggable storage trait with `append`, `load`, `next_sequence`,
     and `lock_for_write`.
   - `FileSystemBackend` — wraps `AppendOnlyEventLog`, acquires an exclusive OS-level
     lock on a `.lock` sidecar before each append, and updates `SegmentHeader.seq_end`
     atomically via `.tmp` + `rename`.
   - `InMemoryBackend` — test/iteration backend with no filesystem dependency.
   - `ReplayEngine` — pure protocol logic over event slices; can replay from genesis or
     resume from a trusted `TICK_SEAL` snapshot checkpoint.
4. **Execution Layer** — Deterministic workflow runner.
5. **CLI Layer** — Developer interface for running, inspecting, replaying.
   - `apply` and `delete` subcommands accept a `--tick` flag (default `0`).
   - `verify --checkpoint-root <digest>` reuses a verified snapshot cache to validate
     only the suffix after the trusted checkpoint.
   - Protocol violations are emitted as structured JSON for compliance monitoring.

## Data Flow
Event → Canonical Encoding → State Update → Digest → Log → Replay
