# TRI-SYNC Protocol Specification

## 1. Overview
TRI-SYNC is a deterministic runtime protocol designed to guarantee reproducible state, auditable workflows, and cloud-neutral execution. The protocol defines strict invariants for encoding, state management, event logging, and replay.

## 2. Core Invariants
- **Binary State Map** — All numeric values encoded as F64 big-endian with no padding.
- **Canonical JSON Encoding** — Deterministic numeric formatting; no scientific notation; locale-neutral.
- **UTF-8 Lexicographic Key Ordering** — Ensures stable multi-tenant isolation and reproducible state boundaries.
- **SHA-256 Digests** — Every state transition produces a canonical digest.
- **Append-Only Event Log** — Immutable, replayable, portable.
- **Deterministic Replay** — Identical input produces identical output across machines and environments.
- **No Outbound Mutation** — Lens-only, observe-only runtime mode.

## 3. Event Schema

Each event written to the TRI-SYNC append-only log follows this canonical structure:

```json
{
  "type":               "<STATE_WRITE | STATE_DELETE | STATE_BATCH | TICK_SEAL | COMPACT | PROTOCOL_ERROR>",
  "seq":                <u64>,
  "tick":               <u64>,
  "namespace":          "<tenant-namespace>",
  "key":                "<namespace:key>",
  "value_type":         <u8>,
  "value":              <json-value>,
  "prev_value_digest":  "<64-char lowercase hex SHA-256>",
  "idempotent":         <bool>,
  "ops":                [ { "type": "...", "key": "...", ... } ],
  "event_count":        <u32>,
  "root_digest":        "<64-char lowercase hex SHA-256>",
  "timestamp_ms":       <u64>,
  "digest":             "<64-char lowercase hex SHA-256>",
  "prev_digest":        "<64-char lowercase hex SHA-256>"
}
```

Fields are omitted when not applicable to the event type (canonical JSON, no `null` padding).
All digests are SHA-256 of the canonical JSON representation, encoded as lowercase hex.
The full normative event specification is in [SPEC.md](SPEC.md).
