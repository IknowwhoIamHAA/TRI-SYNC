# TRI-SYNC Invariants

- All numbers: F64 big-endian
- All keys: UTF-8 lexicographic
- All JSON: canonical numeric encoding
- All digests: SHA-256 lowercase hex
- All logs: append-only
- All replay: deterministic
- All tenants: isolated by key ordering
