# TRI-SYNC Protocol (Initial)

TRI-SYNC stores events as canonical JSON lines in an append-only log.

## Event schema
Each line is one JSON object:

- `sequence` (`u64`): monotonic index starting at `0`
- `tenant` (`string`): tenant namespace
- `key` (`string`): tenant-local state key
- `kind` (`"set" | "delete"`): operation type
- `value_hex` (`string`, optional): binary value for `set` operations, hex encoded
- `payload_sha256` (`string`): SHA-256 of the canonicalized payload fields

## Canonicalization
Object keys are recursively sorted and emitted without extra whitespace. This ensures byte-for-byte stable serialization and reproducible hashes.

## Replay rules
1. Read events in log order.
2. Verify `sequence` is contiguous (`0..n-1`).
3. Verify `payload_sha256` for each event.
4. Apply operation to state:
   - `set`: insert/replace `(tenant, key) -> value`
   - `delete`: remove `(tenant, key)`

If any rule fails, replay fails.

## Deterministic state
State is a binary map ordered by `(tenant, key)` in lexicographic order, enabling deterministic snapshots and replay outputs.
