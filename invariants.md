# TRI-SYNC Invariants

## Encoding

- All numbers: F64 big-endian (binary state map); full decimal notation in canonical JSON
- All decimal values: `canonicalize_decimal` enforces no leading zeros, no trailing zeros, no
  exponent notation (always expanded), and rejects values exceeding 256 significant digits
- All keys: UTF-8 lexicographic (raw byte order; no normalization)
- All string values and keys: raw UTF-8 bytes — NFC, NFD, NFKC, and NFKD normalization are
  explicitly prohibited at every layer (SPEC §5.3)
- All JSON: canonical encoding — no whitespace, sorted keys, lowercase `\uXXXX` escapes for
  control characters U+0000–U+001F (RFC 8785 §3.2.2)
- All digests: SHA-256, encoded as lowercase hex in all JSON and log contexts

## State

- Binary State Map root digest is SHA-256 of big-endian serialization with keys sorted by
  raw UTF-8 byte order
- `TYPE_MISMATCH`: a key's value type may never change once written
- `ORDER_VIOLATION`: BSM binary encoding must have strictly increasing key byte sequences
- `trisync-system` namespace is reserved; `validate_namespace` rejects it with `INVALID_NAMESPACE`
- `TransactionalStateMap` provides atomic batch mutations via clone-stage-commit; on error,
  the original state is unchanged

## Event Log

- All logs: strictly append-only
- File locking: `append()` acquires an exclusive OS-level lock on a `.lock` sidecar file before
  any write; dropped (not unlocked) after the write completes
- `SegmentHeader.seq_end` is updated atomically after every append via `.tmp` + `rename`
- All replay: deterministic — identical input always produces identical output

## Replay

- `TICK_SEAL` events: `timestamp_ms` is required; timestamps must be monotonically non-decreasing
  across seals (`TIMESTAMP_REGRESSION` is fatal)
- `DUPLICATE_EVENT`: a non-idempotent event whose digest has already been seen is a fatal error
  (halts replay); idempotent duplicates emit `WARN_DUPLICATE` and are skipped
- `COMPACT` events: verify live state root matches `snapshot_digest`; loading from `archive_uri`
  is caller responsibility
- `PROTOCOL_ERROR` events: halt replay immediately with the recorded `error_code`
- All tenants: isolated by namespace-prefixed keys; `NAMESPACE_LEAK` is fatal
