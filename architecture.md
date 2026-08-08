# TRI-SYNC Architecture

## Runtime Layers
1. **Protocol Layer** — Encoding, invariants, digest rules.
2. **State Layer** — Binary map, canonical JSON, tenant isolation.
3. **Event Layer** — Append-only log, replay engine.
4. **Execution Layer** — Deterministic workflow runner.
5. **CLI Layer** — Developer interface for running, inspecting, replaying.

## Data Flow
Event → Canonical Encoding → State Update → Digest → Log → Replay
