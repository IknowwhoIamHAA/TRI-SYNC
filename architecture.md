# TRI-SYNC Architecture

## Runtime Layers
1. **Protocol Layer** — Encoding, invariants, digest rules.
2. **State Layer** — Binary map, canonical JSON, tenant isolation.
   - `BinaryStateMap` — core key-value store with BSM binary encoding and root-digest computation.
   - `TransactionalStateMap` — wraps `BinaryStateMap` in a `Mutex`; provides atomic batch
     mutations via a clone-stage-commit pattern.
3. **Event Layer** — Append-only log, replay engine.
   - `AppendOnlyEventLog` — acquires an exclusive OS-level lock on a `.lock` sidecar before
     each append; updates `SegmentHeader.seq_end` atomically via `.tmp` + `rename` after each write.
   - `ReplayEngine` — pure function of the event slice; enforces all replay guards.
4. **Execution Layer** — Deterministic workflow runner.
5. **CLI Layer** — Developer interface for running, inspecting, replaying.
   - `apply` and `delete` subcommands accept a `--tick` flag (default `0`).

## Data Flow
Event → Canonical Encoding → State Update → Digest → Log → Replay
