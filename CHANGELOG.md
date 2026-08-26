# Changelog

All notable changes to this project will be documented in this file.

## [1.0.0] - 2026-08-26

### Added
- Public API freeze declaration for TRI-SYNC Runtime v1.0.0.
- Stable protocol reference specification in `SPEC.md`.
- `normalize_hex()` utility — validates and normalizes hex strings to lowercase.
- `TransactionalStateMap` — wraps `BinaryStateMap` in a `Mutex`; provides atomic
  batch mutations via clone-stage-commit; automatic rollback on error.
- `--tick` flag on `apply` and `delete` CLI subcommands (default `0`).
- `docs/cross-language-determinism.md` — wire-format byte vectors and encoding
  rules for cross-language conformance.

### Changed (Audit Fixes — required before 1.0.0 freeze)

#### hex.rs
- `normalize_hex()` validates and lowercases all hex input; `encode_hex` and
  `decode_hex` were already lowercase-only and are unchanged.

#### decimal.rs
- Added `MAX_DECIMAL_DIGITS = 256`; `canonicalize_decimal` now rejects values
  with more than 256 significant digits with `INVALID_NUMERIC`.
- SPEC §4.3–4.4 updated: always-full-decimal (no scientific notation ever);
  256-digit hard reject (no truncation, no precision-loss flag).

#### canonical_json.rs
- Removed NFC normalization from `write_json_string`. Raw UTF-8 bytes are
  preserved at all layers, matching SPEC §5.3.
- `hex_digit` produces lowercase `a–f` for `\uXXXX` control-character escapes,
  aligning with RFC 8785 §3.2.2.
- Added 9 RFC 8785/JCS conformance tests and 2 raw-UTF-8 no-normalization tests.
- SPEC §4.5 updated: escape case changed to lowercase.

#### key.rs
- `validate_namespace` now rejects `"trisync-system"` (the reserved runtime
  namespace) with `INVALID_NAMESPACE`.

#### state_map.rs
- Added `TransactionalStateMap` providing mutex-guarded atomic batch writes.

#### event.rs / replay.rs
- `TICK_SEAL` events now require `timestamp_ms`; replay enforces monotonically
  non-decreasing timestamps (`TIMESTAMP_REGRESSION` is fatal).
- Non-idempotent `DUPLICATE_EVENT` detection changed from WARN to fatal error,
  halting replay immediately.
- `COMPACT` events: verify `snapshot_digest` against live state root
  (`COMPACT_FAIL` is fatal).
- `PROTOCOL_ERROR` events: halt replay immediately with the recorded error code.
- SPEC §8.5 updated: `DUPLICATE_EVENT` action changed to "Halt, emit REPLAY_ERROR".

#### event_log.rs
- `append()` acquires an exclusive OS-level lock (via `fs2`) on a `.lock`
  sidecar file before every write; advisory locking prevents concurrent corruption.
- `SegmentHeader.seq_end` is updated atomically after every append via
  write-to-`.tmp` + `rename`.

#### main.rs
- `apply` and `delete` CLI subcommands now accept `--tick <u64>` (default `0`);
  the bound value is threaded to the event constructors.

### Guarantees
- Deterministic replay from identical ordered event logs.
- Append-only event chaining with SHA-256 digest verification.
- Canonical JSON: deterministic numeric encoding, sorted keys, lowercase
  `\uXXXX` control-character escapes, no Unicode normalization.
- Multi-tenant namespace isolation via namespace-prefixed keys; `trisync-system`
  is reserved for runtime use.
- Cross-language binary state map determinism pinned by test vector `768e154f…`
  (see `docs/cross-language-determinism.md`).

### Compatibility
- `v1.0.0` is the frozen public API baseline.
- Future changes must remain backward-compatible with v1.0.0.

### Breaking Changes from Pre-1.0.0 snapshots
- `validate_namespace("trisync-system")` now returns `Err`.
- `TICK_SEAL` events without `timestamp_ms` are rejected during replay.
- Non-idempotent duplicate events during replay are now fatal errors.
- `COMPACT` events verify `snapshot_digest` against live state.
- `PROTOCOL_ERROR` events during replay now halt with `Err`.
- Decimal values exceeding 256 significant digits are rejected.
- `\uXXXX` control-character escapes in canonical JSON use lowercase hex.
- Unicode strings in canonical JSON are stored as raw UTF-8 (no NFC normalization).
