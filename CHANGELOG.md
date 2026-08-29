# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased] — v1.1.0

> **Backward-compatible additions only.** The v1.0.0 wire protocol is unchanged.  
> All new features are opt-in. No breaking changes.

### Added

#### License
- Replaced Apache 2.0 `LICENSE` file with the correct TRI-SYNC commercial license notice.
- Added `NOTICE` file attributing Apache 2.0 third-party Cargo dependencies.

#### CLI
- `inspect --log <path>` — Human-readable event log dump: prints each event's seq, type,
  namespace, key, value, and digest in aligned columns. Useful for auditing and debugging.
- `status --log <path>` — Single-line summary of a log: event count, head digest,
  whether the log ends with a `TICK_SEAL`, and whether replay passes.

#### Replay Guard
- Tick regression detection across all event types. If a non-zero `tick` in any event is
  strictly less than the highest non-zero `tick` seen so far, replay halts with
  `TICK_REGRESSION`. This is an additional guard beyond the existing `TIMESTAMP_REGRESSION`
  check on `TICK_SEAL` events.

#### State Map
- `BinaryStateMap::diff(a, b) -> Vec<StateDiff>` — structured comparison of two snapshots.
  Returns one `StateDiff` per key that differs: `Added`, `Removed`, or `Changed { from, to }`.
  Useful for audit tooling, snapshot promotion workflows, and change-verification pipelines.

#### Documentation
- Fixed stale test count in `CHANGELOG.md` and `README.md` (63 → 100).
- Fixed stale wire-format description in `protocol.md` §2 and `invariants.md`
  (`F64 big-endian` → correct typed encoding summary).
- Removed orphan `src/cli/` and `src/workflow/` directories (never compiled, referenced
  non-existent API methods).

#### CI
- Added Clippy job (enforces `deny(warnings)` with MSRV-aware lint checks).
- Added MSRV job (pins `rust-version = "1.85"` in `Cargo.toml`; verified with
  `incompatible_msrv` clippy lint).
- Added multi-platform matrix: ubuntu-latest, macos-latest, windows-latest.
- Added cross-language digest conformance job (Node.js TypeScript client vs. Rust).

#### Tests
- 37 new tests covering wire vectors + BsmValue variants + ordering violations + digest stability
  raised the count from **74 → 100**.
- 10 additional conformance tests for v1.1.0 features: 7 for `BinaryStateMap::diff` and 3 for
  the tick regression guard, raising the total to **110 tests**.

### Changed
- `Cargo.toml`: added `rust-version = "1.85"` (MSRV pin).
- `src/hex.rs`: replaced `usize::is_multiple_of(2)` (stabilized Rust 1.87) with
  `% 2 != 0` (MSRV-safe).
- `src/event.rs`: `#[allow(clippy::too_many_arguments)]` on `state_write` and `compact`
  (deliberate high-arity constructors matching the protocol's event fields).
- Various clippy auto-fixes: `needless_as_bytes`, `collapsible_if`.

### Fixed
- `README.md` + `docs/licensing.md`: corrected license key format from
  `TRISYNC-XXXX-XXXX-XXXX` to `TRI-XXXXXXXX-XXXXXXXX-XXXXXXXX` (5 occurrences).
- `protocol.md`: replaced stale physics schema stub with the actual event schema.

### Compatibility
- Wire format: **unchanged**. All v1.0.0 encoded logs are fully replayable by v1.1.0.
- `BinaryStateMap::from_binary` / `to_binary`: byte-identical to v1.0.0.
- All existing CLI subcommands (`apply`, `delete`, `replay`, `verify`, `export`, `digest`,
  `example`) are unchanged.

---

## [1.0.0] - 2026-08-26 — **Protocol Frozen. Production-Ready.**

> **This is the stable, commercially-licensed release of TRI-SYNC.**  
> The v1.0.0 wire protocol is frozen. All future versions will be backward-compatible.  
> 100 tests pass. CodeQL: 0 alerts. No TODOs or FIXMEs in protocol-critical code.

### Added
- Public API freeze declaration for TRI-SYNC Runtime v1.0.0.
- Stable protocol reference specification in `SPEC.md`.
- `normalize_hex()` utility — validates and normalizes hex strings to lowercase.
- `TransactionalStateMap` — wraps `BinaryStateMap` in a `Mutex`; provides atomic
  batch mutations via clone-stage-commit; automatic rollback on error.
- `--tick` flag on `apply` and `delete` CLI subcommands (default `0`).
- `docs/cross-language-determinism.md` — wire-format byte vectors and encoding
  rules for cross-language conformance.
- `docs/product.md` — market-facing product overview, use cases, and guarantees.
- `docs/licensing.md` — commercial licensing flow, tier descriptions, and FAQ.
- `COMMERCIAL_LICENSE.md` — commercial license terms.
- `src/license.rs` — key-based license activation; reads `TRISYNC_LICENSE_KEY`
  from the environment; validates against a key-store file; clear error + exit
  on missing or invalid key.

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
