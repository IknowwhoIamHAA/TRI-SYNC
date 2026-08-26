# TRI-SYNC Cross-Language Determinism Guide

**Version:** 1.0.0  
**Status:** Normative  
**Audience:** Implementors of TRI-SYNC clients, servers, and tooling in any language.

Any two conforming implementations, starting from the same input, **MUST** produce
byte-for-byte identical canonical JSON, binary state map encodings, and SHA-256
digests. This document provides the precise rules and test vectors needed to verify
cross-language conformance.

---

## 1. Canonical JSON Encoding

All implementations must produce identical canonical JSON for the same input value.

### 1.1 Rules

| Rule | Requirement |
|---|---|
| Object keys | Sorted by raw UTF-8 byte order (equivalent to `memcmp`); no locale, no collation |
| No whitespace | No spaces, tabs, or newlines outside of string values |
| Control characters | U+0000–U+001F **MUST** be escaped as `\uXXXX` with **lowercase** hex digits (RFC 8785 §3.2.2) |
| Specific escapes | `\"` → `\\\"`, `\\` → `\\\\`, U+0008 → `\\b`, U+000C → `\\f`, U+000A → `\\n`, U+000D → `\\r`, U+0009 → `\\t` |
| Other characters | Emitted as-is (UTF-8) |
| No normalization | String values and object keys are stored and encoded as **raw UTF-8 bytes**. NFC, NFD, NFKC, and NFKD normalization are **forbidden** at every layer. |
| No BOM, no trailing newline | The output MUST NOT begin with a byte-order mark or end with a newline |

### 1.2 String Test Vectors

| Input string | Expected canonical JSON output |
|---|---|
| `hello` | `"hello"` |
| `a"b\c` | `"a\"b\\c"` |
| U+0000 (null byte) | `"\u0000"` |
| U+001F (unit separator) | `"\u001f"` |
| U+000A (newline) | `"\n"` |
| U+0009 (tab) | `"\t"` |
| `café` (NFC: U+0063 U+0061 U+0066 U+00E9) | `"café"` |
| `cafe\u0301` (NFD: U+0063 U+0061 U+0066 U+0065 U+0301) | `"cafe\u0301"` — **distinct** from the NFC form |

### 1.3 Object Key Sorting Test Vector

Input (unordered): `{"z": 1, "a": 2, "m": 3}`  
Expected output: `{"a":2,"m":3,"z":1}`

---

## 2. Decimal Canonical Encoding

All numeric values are encoded via `canonicalize_decimal`.

### 2.1 Rules

| Rule | Requirement |
|---|---|
| No leading zeros | `0.5` valid; `00.5` invalid |
| No trailing zeros | `1.5` valid; `1.50` invalid; `2.0` → `2` (integer, not decimal) |
| Always full decimal | Exponent notation is **never** canonical. `1e-7` → `0.0000001`; `1e3` → `1000` |
| No `+` sign | `+1.5` is invalid |
| No negative zero | `-0` and `-0.0` are invalid |
| Digit limit | Values with more than **256 significant digits** (after stripping leading zeros) are rejected with `INVALID_NUMERIC` |
| Infinity / NaN | Not representable; rejected as protocol errors |

### 2.2 Decimal Test Vectors

| Input | Expected canonical output |
|---|---|
| `1.5000` | `1.5` |
| `001.23` | `1.23` |
| `1e3` | `1000` |
| `1e-7` | `0.0000001` |
| `-0.5` | `-0.5` |
| `0` | `0` |
| `-0` | Error: `INVALID_NUMERIC` |
| `1.` | Error: `INVALID_NUMERIC` |
| `256 × '1'` (256 ones) | `111...1` (256 ones) — accepted |
| `257 × '1'` (257 ones) | Error: `INVALID_NUMERIC` |

---

## 3. Binary State Map (BSM) Wire Format

The binary encoding of a `BinaryStateMap` is the input to the SHA-256 root-digest
computation. All conforming implementations **MUST** produce identical bytes.

### 3.1 Top-Level Structure

```
[ entry_count: u32 BE ]
[ entry_0 ]
[ entry_1 ]
...
[ entry_N ]
```

Entries **MUST** be sorted by their key's raw UTF-8 byte sequence in strictly
ascending order (`memcmp`). No duplicate keys are permitted.

### 3.2 Entry Structure

```
[ key_len: u16 BE ]
[ key: key_len bytes of UTF-8 ]
[ type_tag: u8 ]
[ value_payload: variable ]
```

### 3.3 Type Tags and Value Payloads

| Type | Tag | Payload |
|---|---|---|
| `Boolean` | `0x01` | `0x00` (false) or `0x01` (true) — 1 byte |
| `Integer` | `0x02` | signed 64-bit big-endian — 8 bytes |
| `Decimal` | `0x03` | `[len: u32 BE][canonical decimal ASCII bytes: len bytes]` |
| `String` | `0x04` | `[len: u32 BE][UTF-8 bytes: len bytes]` — raw, no normalization |
| `Bytes` | `0x05` | `[len: u32 BE][raw bytes: len bytes]` |
| `Null` | `0x06` | (empty — 0 bytes) |

### 3.4 Empty State Root Digest

An empty `BinaryStateMap` serializes to exactly 4 bytes: `[0x00, 0x00, 0x00, 0x00]`
(entry count = 0, u32 BE).

SHA-256 of `[0x00, 0x00, 0x00, 0x00]` = `df3f619804a92fdb4057192dc43dd748ea778adc52bc498ce80524c014b81119`

### 3.5 Cross-Language Test Vector

State contents:
```
"tenant-a:counter"  →  Integer(42)
"tenant-a:flag"     →  Boolean(true)
"tenant-a:ratio"    →  Decimal("3.14")
```

Binary encoding (hex, with annotations):

```
00 00 00 03                          # entry_count = 3

# entry 0: "tenant-a:counter" → Integer(42)
00 10                                # key_len = 16
74 65 6e 61 6e 74 2d 61 3a 63 6f 6e 74 65 72 ("tenant-a:counter")
02                                   # type_tag = Integer
00 00 00 00 00 00 00 2a              # i64 BE = 42

# entry 1: "tenant-a:flag" → Boolean(true)
00 0d                                # key_len = 13
74 65 6e 61 6e 74 2d 61 3a 66 6c 61 67  ("tenant-a:flag")
01                                   # type_tag = Boolean
01                                   # true

# entry 2: "tenant-a:ratio" → Decimal("3.14")
00 0e                                # key_len = 14
74 65 6e 61 6e 74 2d 61 3a 72 61 74 69 6f  ("tenant-a:ratio")
03                                   # type_tag = Decimal
00 00 00 04                          # len = 4
33 2e 31 34                          # "3.14"
```

Expected root digest (SHA-256 of the above bytes):  
`768e154f65fb12f4419452ac76223006bf9097187294b0d9cec1260e22c664d3`

---

## 4. SHA-256 Digest Rules

| Context | Encoding |
|---|---|
| Event `digest` field | SHA-256 of canonical JSON event bytes with `digest` zeroed (64 × `\0`) |
| BSM root digest | SHA-256 of BSM binary encoding (§3) |
| All digest fields in JSON | Lowercase 64-character hex string, no prefix |
| All digest fields in binary | Raw 32 bytes, big-endian |

The `metadata` field is **excluded** from event digest computation. The `digest`
field itself is set to the zero-hex string (`0000...0000`, 64 zeros) before hashing.

---

## 5. Reserved Namespace

The namespace `trisync-system` is reserved for runtime internal use. Conforming
implementations **MUST** reject any user-supplied namespace equal to `trisync-system`
with `INVALID_NAMESPACE`.

---

## 6. Replay Guards (Cross-Language Requirements)

All conforming implementations **MUST** enforce these guards during replay:

| Guard | Trigger | Action |
|---|---|---|
| `DIGEST_MISMATCH` | Recomputed event digest ≠ stored digest | Fatal halt |
| `SEQ_GAP` | Non-consecutive sequence number | Fatal halt |
| `TICK_SEAL_FAIL` | Root digest mismatch at tick boundary | Fatal halt |
| `NAMESPACE_LEAK` | Event targets a foreign namespace | Fatal halt |
| `TYPE_MISMATCH` | Value type change on existing key | Fatal halt |
| `ORDER_VIOLATION` | BSM keys out of byte-lexicographic order | Fatal halt |
| `DUPLICATE_EVENT` | Non-idempotent event with already-seen digest | Fatal halt |
| `TIMESTAMP_REGRESSION` | `TICK_SEAL.timestamp_ms` < previous seal's timestamp | Fatal halt |
| `COMPACT_FAIL` | `COMPACT.snapshot_digest` ≠ live state root | Fatal halt |

Idempotent duplicate events (where `event.idempotent == true` and the digest was
already seen) emit `WARN_DUPLICATE` and are **skipped** (not halted).

---

## 7. File Locking (for implementations that write to disk)

When appending to an event log file, implementations **MUST** acquire an
OS-level exclusive advisory lock before any read-validate-write operation.
The reference implementation uses a `.lock` sidecar file (e.g., `log.jsonl.lock`)
and `flock(2)` / `LockFileEx` (Windows). Readers do not acquire the lock.

After each append, `SegmentHeader.seq_end` **MUST** be updated atomically
via write-to-`.tmp` + `rename` (POSIX atomic on the same filesystem).

---

## 8. Conformance Checklist

A cross-language implementation is conformant when it passes all of the following:

- [ ] Produces `768e154f…` root digest for the §3.5 test vector state
- [ ] Produces `df3f619804a92fdb4057192dc43dd748ea778adc52bc498ce80524c014b81119` for the empty BSM
- [ ] Produces `"cafe\u0301"` (not `"café"`) for the NFD string `cafe\u0301`
- [ ] Produces `"\u001f"` (lowercase) for U+001F
- [ ] Rejects `-0`, `1.50`, `00.5`, `1e-7` as non-canonical decimal
- [ ] Accepts `0.0000001` as the canonical form of `1e-7`
- [ ] Rejects a decimal with 257 significant digits
- [ ] Rejects `trisync-system` as a namespace
- [ ] Halts replay on `DUPLICATE_EVENT` (non-idempotent)
- [ ] Halts replay on `TIMESTAMP_REGRESSION`
- [ ] Halts replay on `PROTOCOL_ERROR` events
- [ ] Verifies `snapshot_digest` in `COMPACT` events against live state root
