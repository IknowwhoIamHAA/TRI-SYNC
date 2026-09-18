# TRI-SYNC Invariants

## Encoding

- All numbers: typed binary encoding in the BSM wire format (`Integer` = i64 big-endian 8 bytes;
  `Decimal` = u32 big-endian byte length + UTF-8 bytes); full decimal notation in canonical JSON
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
- File locking: the filesystem backend acquires an exclusive OS-level lock on a `.lock` sidecar
  file before writes via `fs2::FileExt::lock_exclusive()`; the lock is released when the lock
  file handle is dropped after the append completes
- Active segment tail metadata (`seq_end`, `last_digest`, `event_count`, `size_bytes`) is
  persisted atomically in `<log>.catalog.json` via `.catalog.tmp` + `rename`; segment files
  themselves remain append-only
- All replay: deterministic — identical input always produces identical output

## Replay

- `SEQ_GAP`: replay halts when a sequence number is non-consecutive
- `SEQUENCE_COLLISION`: replay/log append reject a duplicate sequence number within the same log
- `DIGEST_MISMATCH`: replay halts when `prev_digest` continuity or the stored event digest fails
  validation
- `INVALID_EVENT_FORMAT`: replay halts when a required field is missing or malformed;
  tenant-namespace validation surfaces as `INVALID_NAMESPACE`
- `STATE_MISMATCH`: replay/verify halt when state evolution, deletes, or checkpoint roots do not
  match the log
- `MISSING_TICK_SEAL`: `verify --checkpoint-root` fails when no matching `TICK_SEAL` exists
- `TICK_SEAL` events: `timestamp_ms` is required; timestamps must be monotonically non-decreasing
  across seals (`TIMESTAMP_REGRESSION` is fatal)
- `TICK_REGRESSION`: replay halts when an event's `tick` moves backward relative to the prior max
  tick
- `verify --checkpoint-root`: the checkpoint root must parse as a 32-byte hex
  digest before snapshot-cache path lookup; cached snapshots are trusted only if
  the matching `TICK_SEAL` still validates its digest and required timestamp
- `DUPLICATE_EVENT`: a non-idempotent event whose digest has already been seen is a fatal error
  (halts replay); idempotent duplicates emit `WARN_DUPLICATE` and are skipped
- `COMPACT` events: replay verifies that live state root matches `snapshot_digest`; `archive_uri`
  is recorded on the event but replay/verify do not load from it
- `PROTOCOL_ERROR` is the event type recorded in the log; encountering it halts replay with error
  code `PROTOCOL_ERROR_EVENT`
- All tenants: isolated by namespace-prefixed keys; cross-namespace replay is fatal with
  `NAMESPACE_BREACH`, while invalid key prefixes surface as `NAMESPACE_LEAK` during validation

## Exit Codes

- Exit code `4`: sequence/digest/format/duplicate/protocol-error failures
  (`SEQ_GAP`, `DIGEST_MISMATCH`, `INVALID_EVENT_FORMAT`/`INVALID_NAMESPACE`, `DUPLICATE_EVENT`,
  `PROTOCOL_ERROR_EVENT`)
- Exit code `5`: namespace/sequence-collision failures (`NAMESPACE_BREACH`, `SEQUENCE_COLLISION`)
- Exit code `6`: checkpoint/state continuity failures (`STATE_MISMATCH`, `MISSING_TICK_SEAL`,
  `TIMESTAMP_REGRESSION`, `TICK_REGRESSION`, `COMPACT_MISMATCH`)
