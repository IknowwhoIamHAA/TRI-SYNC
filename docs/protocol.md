# TRI-SYNC Protocol Specification

**Version:** 1.0.0
**Status:** Normative
**Date:** 2026-08-08
**Authors:** TRI-SYNC Runtime Working Group

---

## Table of Contents

1. [Introduction](#1-introduction)
2. [Terminology](#2-terminology)
3. [Binary State Map](#3-binary-state-map)
4. [Canonical JSON Numeric Encoding](#4-canonical-json-numeric-encoding)
5. [UTF-8 Lexicographic Key Ordering](#5-utf-8-lexicographic-key-ordering)
6. [SHA-256 Digest Protocol](#6-sha-256-digest-protocol)
7. [Append-Only Event Log](#7-append-only-event-log)
8. [Deterministic Replay Rules](#8-deterministic-replay-rules)
9. [Multi-Tenant Isolation](#9-multi-tenant-isolation)
10. [Message Semantics](#10-message-semantics)
11. [Error Handling](#11-error-handling)
12. [Versioning and Compatibility](#12-versioning-and-compatibility)
13. [Security Considerations](#13-security-considerations)
14. [Conformance](#14-conformance)

---

## 1. Introduction

TRI-SYNC is a deterministic, append-only distributed runtime designed for
environments that require cryptographic auditability, reproducible state
derivation, and strict multi-tenant isolation. The protocol guarantees that
any two conforming nodes, starting from the same genesis state and processing
the same ordered event log, will arrive at byte-for-byte identical state
representations.

### 1.1 Design Goals

| Goal | Description |
|---|---|
| **Determinism** | Identical inputs always produce identical outputs, regardless of host platform, clock, or execution order within a tick. |
| **Auditability** | Every state transition is recorded in an immutable, cryptographically chained log. |
| **Isolation** | Tenant namespaces are strictly partitioned; cross-tenant reads and writes are protocol violations. |
| **Portability** | Canonical encoding rules eliminate platform-specific representation ambiguity. |
| **Replay Safety** | Any node can reconstruct current state from the genesis block and the full event log without external coordination. |

### 1.2 Scope

This document specifies:

- The canonical binary and JSON representations of TRI-SYNC state
- The structure, ordering, and digesting of state keys
- The append-only event log format and chaining protocol
- Deterministic replay semantics and replay guards
- Multi-tenant namespace isolation rules
- All message types, fields, and processing semantics

This document does not specify network transport, node discovery, leader
election, or physical storage layout, except where these interact directly
with canonical state or log integrity.

### 1.3 Conformance Language

The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHALL**, **SHALL NOT**,
**SHOULD**, **SHOULD NOT**, **RECOMMENDED**, **MAY**, and **OPTIONAL** in this
document are to be interpreted as described in RFC 2119.

---

## 2. Terminology

| Term | Definition |
|---|---|
| **State Map** | The authoritative key-value store representing a tenant's current runtime state. |
| **Event** | An atomic, immutable record of a state transition, written to the event log. |
| **Event Log** | The ordered, append-only sequence of all events for a given namespace. |
| **Genesis Block** | The first entry in an event log; establishes the root digest and initial state. |
| **Digest** | A SHA-256 hash over a canonical serialization of an event or state map. |
| **Tick** | A globally ordered logical clock unit; all events within a tick are processed in canonical key order. |
| **Namespace** | A fully isolated tenant partition within the TRI-SYNC runtime. |
| **Canonical Form** | The unique, deterministic serialization of a value defined by this protocol. |
| **Replay** | The process of reconstructing state by reprocessing the event log from genesis. |
| **Replay Guard** | A set of runtime checks that detect and reject non-deterministic replay paths. |
| **Root Digest** | The SHA-256 digest of the entire canonical state map after all events in a tick are applied. |

---

## 3. Binary State Map

### 3.1 Overview

The Binary State Map (BSM) is the authoritative, in-memory representation of
a namespace's state at any given tick. It is a finite, ordered mapping from
byte-string keys to typed values. The BSM **MUST** be traversed and serialized
in UTF-8 lexicographic key order at all times (see §5).

### 3.2 Key Constraints

- Keys **MUST** be valid UTF-8 byte sequences.
- Keys **MUST NOT** be empty (zero-length keys are a protocol violation).
- Keys **MUST NOT** exceed 512 bytes after UTF-8 encoding.
- Keys **MUST NOT** contain the null byte (`0x00`).
- Keys are case-sensitive; `"Foo"` and `"foo"` are distinct keys.
- Namespace-scoped keys **MUST** be prefixed with the namespace identifier
  followed by a single colon separator (`:`) — e.g., `tenant-a:counter`.

### 3.3 Value Types

The BSM supports exactly six value types. No other value types are permitted.

| Type Tag | Name | Canonical Encoding |
|---|---|---|
| `0x01` | **Boolean** | Single byte: `0x00` (false) or `0x01` (true) |
| `0x02` | **Integer** | Big-endian signed 64-bit (8 bytes) |
| `0x03` | **Decimal** | Canonical JSON numeric string, UTF-8 encoded, length-prefixed (see §4) |
| `0x04` | **String** | UTF-8 bytes, length-prefixed with big-endian uint32 |
| `0x05` | **Bytes** | Raw bytes, length-prefixed with big-endian uint32 |
| `0x06` | **Null** | Zero bytes (type tag alone; no payload) |

### 3.4 Binary Wire Format

Each BSM entry is serialized as:

```
[ key_len: uint16 BE ][ key: UTF-8 bytes ][ type_tag: uint8 ][ value_payload ]
```

The complete BSM serialization is:

```
[ entry_count: uint32 BE ][ entry_0 ][ entry_1 ] ... [ entry_N ]
```

Entries **MUST** appear in ascending UTF-8 lexicographic key order. Any
serialization that violates this ordering is malformed and **MUST** be rejected
by conforming implementations.

### 3.5 State Map Snapshot

A snapshot is a point-in-time, complete serialization of the BSM at tick
boundary. Snapshots are used as replay checkpoints and **MUST** include:

- The namespace identifier (UTF-8, uint16 length-prefixed)
- The tick number (uint64 BE) at which the snapshot was taken
- The root digest of the BSM at that tick (32 bytes; see §6)
- The full binary-encoded BSM (as specified in §3.4)

```
[ namespace_len: uint16 BE ][ namespace: UTF-8 ]
[ tick: uint64 BE ]
[ root_digest: 32 bytes ]
[ bsm_entry_count: uint32 BE ][ bsm_entries ... ]
```

Snapshots **MUST NOT** be used as a substitute for the full event log. They
are acceleration hints only; replay **MUST** always be verifiable from genesis
(see §8.3).

---

## 4. Canonical JSON Numeric Encoding

### 4.1 Motivation

Floating-point representation varies across platforms, compilers, and runtime
environments. TRI-SYNC mandates a single canonical JSON numeric encoding to
ensure that any decimal value serializes to the same byte sequence on every
conforming node.

### 4.2 Integer Values

Integer values (type tag `0x02`) **MUST NOT** include a decimal point or
exponent in their JSON representation.

**Valid:** `42`, `-7`, `0`, `1000000`
**Invalid:** `42.0`, `4.2e1`, `1e6`

### 4.3 Decimal Values

Decimal values (type tag `0x03`) **MUST** conform to the following rules when
serialized to canonical JSON form:

1. **No leading zeros** — except for the single digit before the decimal point
   when the integer part is zero. (`0.5` is valid; `00.5` and `01.5` are invalid.)
2. **No trailing zeros after the decimal point** — `1.5` not `1.50`; `2.0` is
   represented as integer `2`, not decimal.
3. **No positive exponent notation** — values **MUST** be written in full decimal
   notation unless doing so would exceed 34 significant digits (see §4.4).
4. **Negative exponent notation** is permitted only for values where the absolute
   value is less than `1e-6`. In this case, normalized scientific notation
   **MUST** be used: one non-zero digit before the decimal point, followed by
   the fractional digits, followed by `e-N`.
5. **Sign**: Negative zero (`-0`) is forbidden. Positive sign prefix is
   forbidden (`+1.5` is invalid; `1.5` is valid).
6. **Infinity and NaN** are not representable and **MUST** be rejected as
   protocol errors.

### 4.4 High-Precision Decimals

For values requiring more than 34 significant digits, implementations **MUST**
truncate (not round) to 34 significant digits and record a precision-loss flag
in the containing event's metadata field. Precision loss does not invalidate
an event but **MUST** be propagated to subscribers.

### 4.5 Canonical JSON Object Encoding

When an entire BSM or event payload is serialized as a JSON object:

- Keys **MUST** be UTF-8 strings ordered lexicographically (see §5).
- No whitespace (spaces, tabs, newlines) is permitted outside of string values.
- Unicode escape sequences **MUST** use uppercase hex digits: `\uABCD` not `\uabcd`.
- Surrogate pairs **MUST** be represented as a single encoded codepoint where possible.
- The serialization **MUST NOT** include a trailing newline or byte-order mark (BOM).

---

## 5. UTF-8 Lexicographic Key Ordering

### 5.1 Definition

TRI-SYNC uses **UTF-8 byte-order lexicographic comparison** as the canonical
key ordering. Given two keys `A` and `B`, their order is determined by
comparing their UTF-8 encoded byte sequences left-to-right, byte by byte,
treating each byte as an unsigned 8-bit integer.

This ordering is:
- Deterministic across all conforming implementations
- Independent of locale, collation, or Unicode normalization
- Identical to the ordering produced by a standard `memcmp` on UTF-8 byte
  sequences of equal or differing length (shorter sequences that are a prefix
  of a longer sequence sort first)

### 5.2 Ordering Rules

1. Compare bytes left-to-right. The first differing byte determines order: the
   key with the lower unsigned byte value sorts first.
2. If all bytes of the shorter key match the corresponding prefix of the longer
   key, the shorter key sorts first.
3. Keys that are byte-for-byte identical are equal. Equal keys in the same
   namespace are a protocol violation (duplicate keys).

### 5.3 Normalization Prohibition

TRI-SYNC **MUST NOT** apply Unicode normalization (NFC, NFD, NFKC, NFKD) to
keys before comparison or storage. Two keys that differ only in Unicode
normalization form are considered distinct keys. Implementations **MUST NOT**
normalize keys silently.

### 5.4 Example Ordering

The following keys are shown in correct ascending lexicographic order:

```
""           ← prohibited (empty key)
"!"          ← 0x21
"A"          ← 0x41
"B"          ← 0x42
"a"          ← 0x61
"aa"         ← 0x61 0x61
"b"          ← 0x62
"tenant-a:x" ← namespace-scoped
"tenant-b:x" ← different namespace, sorts after
"é"          ← 0xC3 0xA9 (two bytes; sorts after all ASCII)
"ü"          ← 0xC3 0xBC
```

### 5.5 Enforcement

Every component that reads, writes, or digests the BSM **MUST** validate that
keys are in canonical order. An out-of-order key sequence is a fatal protocol
error. The receiving node **MUST** log the violation, reject the message or
snapshot, and emit a `PROTOCOL_ERROR` event to its own namespace log.

---

## 6. SHA-256 Digest Protocol

### 6.1 Digest Scope

TRI-SYNC uses SHA-256 as the exclusive hash function for all integrity
guarantees. No other hash function is permitted. All digests are 32 bytes
(256 bits), encoded as lowercase hexadecimal when represented in JSON or log
metadata.

### 6.2 Event Digest

Each event is assigned a digest computed over its canonical serialization:

```
digest = SHA-256( canonical_event_bytes )
```

The canonical event bytes are the binary encoding of the event as defined in
§10.3, with the `digest` field itself set to 32 zero bytes during computation
(to avoid circular dependency).

### 6.3 Chain Digest

Each event includes a `prev_digest` field containing the digest of the
immediately preceding event in the log. The genesis block sets `prev_digest`
to 32 zero bytes.

This forms an append-only cryptographic chain:

```
genesis_digest = SHA-256( genesis_event_bytes )
event_1_digest = SHA-256( event_1_bytes | prev=genesis_digest )
event_2_digest = SHA-256( event_2_bytes | prev=event_1_digest )
...
```

Any gap, reordering, or mutation in the chain **MUST** be detectable by
recomputing digests from the genesis block forward.

### 6.4 Root Digest

At the end of every tick, after all events for that tick have been applied, a
Root Digest is computed over the canonical BSM serialization (§3.4):

```
root_digest = SHA-256( canonical_bsm_bytes )
```

The Root Digest is appended to the log as a special `TICK_SEAL` event (see
§10.2.4) and **MUST** be verified before processing any event in the
subsequent tick.

### 6.5 Digest Verification Requirements

Conforming implementations **MUST**:

1. Verify `prev_digest` on every received event before applying it.
2. Recompute and verify the `digest` of each event on receipt.
3. Verify the `root_digest` in each `TICK_SEAL` event against a locally
   computed BSM digest.
4. Reject and quarantine any event that fails digest verification.
5. Never apply a quarantined event, even under operator instruction.

### 6.6 Digest Encoding

- In binary contexts: raw 32-byte big-endian value.
- In JSON contexts: lowercase 64-character hexadecimal string.
- In log metadata: lowercase 64-character hexadecimal string, no prefix.

---

## 7. Append-Only Event Log

### 7.1 Log Guarantees

The TRI-SYNC event log is **strictly append-only**. The following operations
are protocol violations:

- Deleting any event from the log
- Modifying any field of any committed event
- Inserting an event at any position other than the tail
- Truncating the log to reclaim space (compaction must use snapshot-based
  archival; see §7.5)

### 7.2 Log Segments

The event log is divided into contiguous, immutable segments. Each segment:

- Contains a contiguous range of events identified by monotonically increasing
  sequence numbers.
- Begins with a segment header (see §7.3).
- Is identified by its first and last sequence numbers and the digest of its
  first event.
- **MUST NOT** be modified once written.

### 7.3 Segment Header Format

```json
{
  "segment_id":   "<uuid-v4>",
  "namespace":    "<namespace-id>",
  "seq_start":    "<uint64>",
  "seq_end":      "<uint64>",
  "first_digest": "<sha256-hex>",
  "prev_segment": "<sha256-hex | null>",
  "created_at":   "<uint64-unix-ms>",
  "protocol_ver": "1.0.0"
}
```

`prev_segment` is the SHA-256 digest of the preceding segment's header. The
genesis segment sets `prev_segment` to `null`.

### 7.4 Event Sequence Numbers

- Sequence numbers are unsigned 64-bit integers, starting at `0` for the
  genesis block.
- Sequence numbers **MUST** be strictly monotonically increasing with no gaps.
- Each namespace maintains its own independent sequence number space.
- Cross-namespace sequence numbers are not comparable and **MUST NOT** be used
  to establish ordering between namespaces.

### 7.5 Log Compaction

To bound storage growth, conforming implementations **MAY** archive log
segments older than a configurable retention window. Archival **MUST**:

1. Produce a verified snapshot (§3.5) at the compaction boundary tick.
2. Preserve the full segment chain from the genesis block or the most recent
   verified snapshot, whichever is more recent.
3. Retain all `TICK_SEAL` events indefinitely; they are never subject to archival.
4. Record the compaction event in the log as a `COMPACT` event (see §10.2.5).

Archived segments **MUST** be retained in cold storage for at least the
operator-configured audit retention period (default: 7 years) and **MUST**
remain retrievable for replay verification.

### 7.6 Write Ordering Invariants

Within a single tick:

1. All `STATE_WRITE` events for the tick are buffered until the tick is closed.
2. Events are committed to the log in ascending canonical key order of their
   target key.
3. A single `TICK_SEAL` event is appended last, sealing the tick and recording
   the root digest.

No event from tick `T+1` may be committed before the `TICK_SEAL` of tick `T`
is durably written.

---

## 8. Deterministic Replay Rules

### 8.1 Replay Invariants

Replay is the process by which a node reconstructs the current BSM by
reprocessing the event log from a known starting point (genesis or a verified
snapshot). Replay **MUST** produce bit-identical BSM state to the original
execution, regardless of:

- The host operating system or hardware architecture
- The time elapsed since original execution
- The implementation language or runtime version (within the same protocol version)

### 8.2 Replay Starting Points

| Starting Point | Precondition |
|---|---|
| **Genesis** | Event log is complete from sequence `0`. |
| **Verified Snapshot** | Snapshot root digest matches the `TICK_SEAL` digest for the snapshot tick. |

Replay from a snapshot skips all events up to and including the snapshot tick.
The snapshot BSM is loaded directly, and replay proceeds from the next event
after the snapshot tick's `TICK_SEAL`.

### 8.3 Full-Chain Verification

Implementations supporting audit mode **MUST** offer full-chain replay from
genesis regardless of available snapshots. Full-chain replay verifies every
event digest and root digest in sequence. Any discrepancy is a chain integrity
violation.

### 8.4 Replay Execution Rules

During replay, the following rules **MUST** be enforced:

1. **No external I/O** — replay may not perform network requests, file system
   writes, or any non-deterministic system call. All state transitions must
   derive exclusively from the event log.
2. **No clock reads** — wall-clock or monotonic clock reads are forbidden
   during replay. Timestamps in events are taken verbatim from the log; they
   are not recomputed.
3. **No randomness** — any PRNG or entropy source **MUST** be seeded from a
   deterministic value derived from the event log, not from system entropy.
4. **Strict event ordering** — events are applied in ascending sequence number
   order. No buffering, reordering, or speculative application is permitted.
5. **Idempotency enforcement** — events marked `idempotent: true` that appear
   more than once in the log (due to at-least-once delivery guarantees) **MUST**
   be applied exactly once. Duplicate detection uses the event `digest` as the
   deduplication key.
6. **Replay guard activation** — all replay guards (§8.5) **MUST** be active
   during replay. Guards may not be disabled by operator configuration during
   replay mode.

### 8.5 Replay Guards

Replay guards are runtime checks that detect and halt replay on
non-deterministic conditions.

| Guard | Trigger Condition | Action |
|---|---|---|
| `DIGEST_MISMATCH` | Recomputed event digest does not match stored digest | Halt, emit `REPLAY_ERROR`, quarantine log segment |
| `SEQ_GAP` | Sequence number is non-consecutive | Halt, emit `REPLAY_ERROR` |
| `TICK_SEAL_FAIL` | Root digest at tick boundary does not match recomputed BSM digest | Halt, emit `REPLAY_ERROR` |
| `NAMESPACE_LEAK` | Event targets a key outside its declared namespace | Halt, emit `PROTOCOL_ERROR` |
| `TYPE_MISMATCH` | Value type tag does not match the declared type for an existing key | Halt, emit `REPLAY_ERROR` |
| `ORDER_VIOLATION` | Key in BSM serialization is out of lexicographic order | Halt, emit `REPLAY_ERROR` |
| `DUPLICATE_EVENT` | Non-idempotent event with a previously seen digest is encountered | Skip event, emit `WARN_DUPLICATE` |

### 8.6 Replay Completion

Replay is complete when the last event in the log has been applied and the
final `TICK_SEAL` root digest has been verified against the locally computed
BSM. The node then transitions from replay mode to live mode.

---

## 9. Multi-Tenant Isolation

### 9.1 Namespace Model

Each tenant is assigned exactly one namespace. A namespace is a string
identifier conforming to the following rules:

- **Format:** `[a-z0-9][a-z0-9\-]{1,61}[a-z0-9]` (lowercase alphanumeric and
  hyphens, 3–63 characters, no leading or trailing hyphen)
- **Uniqueness:** Namespace identifiers **MUST** be unique within a TRI-SYNC cluster.
- **Immutability:** Once assigned, a namespace identifier **MUST NOT** be
  changed or reused, even after a tenant is deprovisioned.

### 9.2 Key-Space Partitioning

All BSM keys **MUST** be prefixed with the owning namespace and a colon separator:

```
<namespace>:<user-defined-key>
```

Keys that do not carry the correct namespace prefix for the processing context
are a protocol violation. Implementations **MUST** reject such keys at the
write path and quarantine any event that contains them.

The `:` separator character is reserved; user-defined key portions **MUST NOT**
begin with `:`.

### 9.3 Isolation Enforcement

| Operation | Rule |
|---|---|
| **Read** | A tenant context may only read keys with its own namespace prefix. |
| **Write** | A tenant context may only write keys with its own namespace prefix. |
| **Event Log** | Each namespace maintains a dedicated, independent event log. Cross-log references are forbidden. |
| **Digest** | Root digests are computed per namespace. There is no cross-namespace composite digest. |
| **Replay** | Replay is always scoped to a single namespace. Cross-namespace replay is not defined. |

### 9.4 Administrative Namespace

The reserved namespace `trisync-system` is used exclusively by the runtime for
system events (cluster membership, compaction records, audit events). Tenant
operations **MUST NOT** target the `trisync-system` namespace. System events
**MUST NOT** reference tenant namespaces.

### 9.5 Namespace Lifecycle Events

| Event Type | Description |
|---|---|
| `NS_CREATE` | Namespace provisioned; genesis block written. |
| `NS_SUSPEND` | Namespace suspended; writes rejected, reads permitted. |
| `NS_RESUME` | Namespace re-activated from suspended state. |
| `NS_DEPROVISION` | Namespace permanently decommissioned; log sealed and archived. |

After `NS_DEPROVISION`, no further events may be written to the namespace log.
The final `TICK_SEAL` and `NS_DEPROVISION` event are the permanent tail of
the log.

### 9.6 Resource Quotas

Each namespace **MAY** have operator-configured resource quotas:

- `max_key_count` — maximum number of active BSM keys (default: unlimited)
- `max_bsm_bytes` — maximum total size of BSM values (default: unlimited)
- `max_events_per_tick` — maximum events in a single tick (default: `65535`)
- `max_log_bytes` — soft limit triggering compaction recommendation (default: unlimited)

Quota violations **MUST** result in a `QUOTA_EXCEEDED` error event; the
offending write is rejected. Quota events do not halt the namespace; subsequent
writes are permitted after the condition is resolved.

---

## 10. Message Semantics

### 10.1 Message Transport Contract

TRI-SYNC messages are transport-agnostic. This specification defines message
structure and processing semantics only. Transport-layer concerns (framing,
ordering guarantees, retry, backpressure) are delegated to the transport
binding specification.

All messages **MUST** be serialized in canonical JSON form (§4.5) for the wire
format, or in the binary format described in §3 when using binary transport
bindings.

### 10.2 Event Types

#### 10.2.1 `STATE_WRITE`

Records a key-value write to the BSM.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | string | **MUST** | `"STATE_WRITE"` |
| `seq` | uint64 | **MUST** | Monotonic sequence number |
| `tick` | uint64 | **MUST** | Logical tick number |
| `namespace` | string | **MUST** | Target namespace |
| `key` | string | **MUST** | Target BSM key (namespace-prefixed) |
| `value_type` | uint8 | **MUST** | Type tag (see §3.3) |
| `value` | varies | **MUST** | Canonical encoded value |
| `prev_value_digest` | hex32 | **SHOULD** | Digest of the previous value, for optimistic concurrency |
| `idempotent` | boolean | **MUST** | Whether this write is safe to apply multiple times |
| `prev_digest` | hex32 | **MUST** | Digest of the preceding event |
| `digest` | hex32 | **MUST** | SHA-256 digest of this event (see §6.2) |
| `metadata` | object | **MAY** | Arbitrary key-value pairs; not included in digest computation |

#### 10.2.2 `STATE_DELETE`

Records a key deletion from the BSM.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | string | **MUST** | `"STATE_DELETE"` |
| `seq` | uint64 | **MUST** | Monotonic sequence number |
| `tick` | uint64 | **MUST** | Logical tick number |
| `namespace` | string | **MUST** | Target namespace |
| `key` | string | **MUST** | Key to delete (namespace-prefixed) |
| `prev_value_digest` | hex32 | **MUST** | Digest of the value being deleted (optimistic concurrency) |
| `idempotent` | boolean | **MUST** | `true` if deleting an already-absent key is a no-op |
| `prev_digest` | hex32 | **MUST** | Digest of the preceding event |
| `digest` | hex32 | **MUST** | SHA-256 digest of this event |

A `STATE_DELETE` targeting a key that does not exist in the BSM is a no-op
when `idempotent` is `true`, and a `KEY_NOT_FOUND` error when `idempotent`
is `false`.

#### 10.2.3 `STATE_BATCH`

An atomic group of `STATE_WRITE` and/or `STATE_DELETE` operations applied as
a single unit.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | string | **MUST** | `"STATE_BATCH"` |
| `seq` | uint64 | **MUST** | Sequence number of the batch event itself |
| `tick` | uint64 | **MUST** | Logical tick number |
| `namespace` | string | **MUST** | Target namespace |
| `ops` | array | **MUST** | Ordered list of `STATE_WRITE` or `STATE_DELETE` payloads (without `seq`, `prev_digest`, `digest`) |
| `prev_digest` | hex32 | **MUST** | Digest of the preceding event |
| `digest` | hex32 | **MUST** | SHA-256 digest of the full batch, including all `ops` |

Within a batch, `ops` are applied in the order specified. If any operation in
the batch fails, the entire batch is rolled back — no partial application is
permitted. Batches are always `idempotent: false` at the batch level;
individual ops inherit their own idempotency flag.

#### 10.2.4 `TICK_SEAL`

Closes a tick and records the root digest of the resulting BSM.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | string | **MUST** | `"TICK_SEAL"` |
| `seq` | uint64 | **MUST** | Sequence number |
| `tick` | uint64 | **MUST** | Tick being sealed |
| `namespace` | string | **MUST** | Target namespace |
| `event_count` | uint32 | **MUST** | Number of events applied in this tick |
| `root_digest` | hex32 | **MUST** | SHA-256 of canonical BSM after all tick events are applied |
| `prev_digest` | hex32 | **MUST** | Digest of the preceding event (last event of the tick) |
| `digest` | hex32 | **MUST** | SHA-256 digest of this `TICK_SEAL` event |
| `timestamp_ms` | uint64 | **MUST** | Unix epoch milliseconds at which the tick was sealed |

`TICK_SEAL` events **MUST NOT** be omitted, even for empty ticks. An empty
tick has `event_count: 0` and a `root_digest` equal to the previous tick's
`root_digest`.

#### 10.2.5 `COMPACT`

Records a log compaction event.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | string | **MUST** | `"COMPACT"` |
| `seq` | uint64 | **MUST** | Sequence number |
| `tick` | uint64 | **MUST** | Tick at which compaction occurred |
| `namespace` | string | **MUST** | Target namespace |
| `snapshot_digest` | hex32 | **MUST** | Root digest of the snapshot taken at compaction |
| `archived_seq_start` | uint64 | **MUST** | First sequence number archived |
| `archived_seq_end` | uint64 | **MUST** | Last sequence number archived |
| `archive_uri` | string | **MUST** | URI of the archived segment in cold storage |
| `prev_digest` | hex32 | **MUST** | Digest of the preceding event |
| `digest` | hex32 | **MUST** | SHA-256 digest of this event |

#### 10.2.6 `PROTOCOL_ERROR`

Records a protocol violation detected by the local node.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | string | **MUST** | `"PROTOCOL_ERROR"` |
| `seq` | uint64 | **MUST** | Sequence number |
| `tick` | uint64 | **MUST** | Tick in which the error was detected |
| `namespace` | string | **MUST** | Namespace context |
| `error_code` | string | **MUST** | One of the defined error codes (see §11) |
| `offending_seq` | uint64 | **MAY** | Sequence number of the violating event |
| `detail` | string | **MAY** | Human-readable description |
| `prev_digest` | hex32 | **MUST** | Digest of the preceding event |
| `digest` | hex32 | **MUST** | SHA-256 digest of this event |

`PROTOCOL_ERROR` events are informational records. They do not close the
namespace or the tick; the offending event is quarantined and the log continues.

### 10.3 Canonical Event Serialization

For digest computation, events are serialized as canonical JSON (§4.5) with
the following additional rules:

1. Fields **MUST** appear in ascending UTF-8 lexicographic key order.
2. The `digest` field **MUST** be set to
   `"0000000000000000000000000000000000000000000000000000000000000000"`
   (64 zeros) during digest computation.
3. The `metadata` field **MUST** be excluded from digest computation entirely.
4. All integer fields **MUST** be encoded as JSON integers (not strings).
5. The serialization **MUST NOT** include trailing whitespace or newlines.

### 10.4 Message Processing Order

Within a tick, messages are processed in the following order:

1. `NS_CREATE` / `NS_RESUME` (if applicable)
2. `STATE_WRITE` and `STATE_BATCH` events, in sequence-number order
3. `STATE_DELETE` events, in sequence-number order
4. `NS_SUSPEND` / `NS_DEPROVISION` (if applicable)
5. `COMPACT` (if applicable)
6. `PROTOCOL_ERROR` (appended as errors are detected)
7. `TICK_SEAL` (always last)

No event type may appear after `TICK_SEAL` within the same tick.

---

## 11. Error Handling

### 11.1 Error Codes

| Code | Description | Severity |
|---|---|---|
| `DIGEST_MISMATCH` | Recomputed digest does not match stored digest | Fatal |
| `SEQ_GAP` | Non-consecutive sequence number detected | Fatal |
| `TICK_SEAL_FAIL` | Root digest mismatch at tick boundary | Fatal |
| `NAMESPACE_LEAK` | Key targets a foreign namespace | Fatal |
| `TYPE_MISMATCH` | Value type inconsistent with existing key type | Fatal |
| `ORDER_VIOLATION` | BSM keys out of lexicographic order | Fatal |
| `KEY_NOT_FOUND` | Delete or read on non-existent key with `idempotent: false` | Error |
| `KEY_TOO_LONG` | Key exceeds 512 bytes | Error |
| `INVALID_KEY` | Key contains null byte or is empty | Error |
| `QUOTA_EXCEEDED` | Write would violate a namespace quota | Error |
| `INVALID_NUMERIC` | Decimal encoding violates canonical rules | Error |
| `BATCH_ROLLBACK` | One or more ops in a `STATE_BATCH` failed | Error |
| `PRECISION_LOSS` | Decimal truncated to 34 significant digits | Warning |
| `WARN_DUPLICATE` | Idempotent event applied more than once | Warning |
| `INVALID_SEGMENT` | Segment header fields are malformed or inconsistent | Fatal |

**Fatal** errors halt the affected replay or processing context and emit a
`PROTOCOL_ERROR` event. **Error** severity rejects the offending message.
**Warning** severity is recorded but does not interrupt processing.

### 11.2 Error Propagation

Errors detected during live message processing **MUST** be:

1. Appended to the event log as a `PROTOCOL_ERROR` event before the current
   tick's `TICK_SEAL`.
2. Returned to the originating client as an error response with the applicable
   error code.
3. Reported to the operator monitoring interface.

Errors detected during replay **MUST** halt replay immediately. The partially
reconstructed BSM **MUST** be discarded. The node **MUST NOT** enter live mode
until the replay error is resolved.

---

## 12. Versioning and Compatibility

### 12.1 Protocol Version

The protocol version is a semantic version string of the form `MAJOR.MINOR.PATCH`.

- **MAJOR** increment: breaking change to the canonical encoding, event format,
  or digest protocol.
- **MINOR** increment: backward-compatible addition (new event types, new
  optional fields).
- **PATCH** increment: clarification or correction with no behavioral change.

The current version is **1.0.0**.

### 12.2 Version Negotiation

The protocol version is declared in:

- Every segment header (`protocol_ver` field)
- Every `NS_CREATE` event (`protocol_ver` field)

Nodes **MUST** reject events from a segment with a MAJOR version higher than
their supported version. Nodes **MAY** process events from segments with a
lower MAJOR version only if a documented migration procedure applies.

### 12.3 Forward Compatibility

Implementations **MUST** ignore unrecognized JSON fields in event payloads at
MINOR version boundaries. Unrecognized fields **MUST NOT** be included in
digest computation (only known, specified fields participate in the digest).

### 12.4 Deprecation

Fields or event types deprecated in a MINOR version will be removed in the
next MAJOR version. A deprecation notice **MUST** be present for at least one
full MINOR version before removal.

---

## 13. Security Considerations

### 13.1 Digest Integrity

The SHA-256 chain provides tamper evidence, not tamper prevention. Operators
**MUST** ensure that:

- Log storage is access-controlled; only the TRI-SYNC runtime may append to
  the log.
- Snapshot storage is read-only after creation.
- Archive storage is immutable (write-once) after the `COMPACT` event is
  committed.

### 13.2 Namespace Isolation

Namespace isolation is enforced at the protocol layer, not solely at the
storage layer. Operators **MUST NOT** rely on shared storage access controls
as the sole isolation mechanism.

### 13.3 Key Confidentiality

TRI-SYNC does not encrypt keys or values in the log. If key or value
confidentiality is required, operators **MUST** apply encryption at the
application layer before writing values to the BSM. TRI-SYNC digests and
chains operate over the raw (potentially encrypted) bytes.

### 13.4 Replay Attack Prevention

The cryptographic chain and monotonically increasing sequence numbers prevent
replay of stale event subsequences. Implementations **MUST** reject any event
whose `seq` is less than or equal to the highest seen `seq` for the namespace,
regardless of digest validity.

### 13.5 Denial of Service

Implementations **SHOULD** enforce:

- Maximum event payload size (recommended: 1 MiB per event)
- Maximum batch size (recommended: 1000 ops per `STATE_BATCH`)
- Rate limits on `PROTOCOL_ERROR` generation to prevent log flooding

---

## 14. Conformance

### 14.1 Conformance Classes

| Class | Description |
|---|---|
| **Core** | Implements §3, §4, §5, §6, §7, §8, §10. Sufficient for a read-write node. |
| **Full** | Implements Core plus §9 (multi-tenant isolation) and §11 (error handling). Required for production deployments. |
| **Audit** | Implements Full plus full-chain replay from genesis (§8.3). Required for audit nodes. |

### 14.2 Mandatory Requirements Summary

Implementations claiming conformance at any class **MUST**:

- [ ] Serialize and deserialize the BSM in UTF-8 lexicographic key order (§5)
- [ ] Use SHA-256 exclusively for all digests (§6.1)
- [ ] Verify `prev_digest` on every received event (§6.5)
- [ ] Verify `root_digest` on every `TICK_SEAL` event (§6.5)
- [ ] Reject events that fail digest verification (§6.5)
- [ ] Enforce append-only semantics on the event log (§7.1)
- [ ] Enforce deterministic replay rules including all replay guards (§8.4, §8.5)
- [ ] Enforce namespace key-prefix isolation (§9.2, §9.3)
- [ ] Produce canonical JSON numeric encoding (§4)
- [ ] Apply canonical event serialization for digest computation (§10.3)
- [ ] Process events in the defined within-tick order (§10.4)

### 14.3 Test Vectors

A companion test-vector suite is provided in `test-vectors/` within this
repository. Conforming implementations **MUST** pass all test vectors for their
claimed conformance class before declaring conformance.

---

*End of TRI-SYNC Protocol Specification v1.0.0*

---

**Document History**

| Version | Date | Change Summary |
|---|---|---|
| 1.0.0 | 2026-08-08 | Initial normative release |
```

---

**What's covered — section by section:**

| Section | Content |
|---|---|
| §3 Binary State Map | Wire format, 6 typed value tags, key constraints, snapshot layout |
| §4 Canonical JSON Numerics | Integer vs. decimal rules, no exponents, no trailing zeros, precision-loss truncation |
| §5 UTF-8 Lex Ordering | `memcmp`-based ordering, normalization prohibition, worked example |
| §6 SHA-256 Digests | Event digest, chain digest, root digest, circular-dependency zero-fill |
| §7 Append-Only Log | Segment headers, sequence numbers, compaction via `COMPACT` event, write ordering |
| §8 Deterministic Replay | No I/O / no clocks / no randomness rules, all 7 replay guards |
| §9 Multi-Tenant Isolation | `<ns>:<key>` partitioning, lifecycle events, quotas, `trisync-system` reservation |
| §10 Message Semantics | Full field tables for `STATE_WRITE`, `STATE_DELETE`, `STATE_BATCH`, `TICK_SEAL`, `COMPACT`, `PROTOCOL_ERROR`; tick ordering |
| §11 Error Handling | 15 error codes with severity levels; propagation rules |
| §12 Versioning | SemVer negotiation, forward-compat field ignoring, deprecation policy |
| §13 Security | Tamper evidence vs. prevention, key confidentiality, DoS mitigations |
| §14 Conformance | Core / Full / Audit classes + checklist + test-vector reference || `0x02` | **Integer** | Big-endian signed 64-bit (8 bytes) |
| `0x03` | **Decimal** | Canonical JSON numeric string, UTF-8 encoded, length-prefixed (see §4) |
| `0x04` | **String** | UTF-8 bytes, length-prefixed with big-endian uint32 |
| `0x05` | **Bytes** | Raw bytes, length-prefixed with big-endian uint32 |
| `0x06` | **Null** | Zero bytes (type tag alone; no payload) |

### 3.4 Binary Wire Format

Each BSM entry is serialized as:

```
[ key_len: uint16 BE ][ key: UTF-8 bytes ][ type_tag: uint8 ][ value_payload ]
```

The complete BSM serialization is:

```
[ entry_count: uint32 BE ][ entry_0 ][ entry_1 ] ... [ entry_N ]
```

Entries **MUST** appear in ascending UTF-8 lexicographic key order. Any
serialization that violates this ordering is malformed and **MUST** be rejected
by conforming implementations.

### 3.5 State Map Snapshot

A snapshot is a point-in-time, complete serialization of the BSM at tick
boundary. Snapshots are used as replay checkpoints and **MUST** include:

- The namespace identifier (UTF-8, uint16 length-prefixed)
- The tick number (uint64 BE) at which the snapshot was taken
- The root digest of the BSM at that tick (32 bytes; see §6)
- The full binary-encoded BSM (as specified in §3.4)

```
[ namespace_len: uint16 BE ][ namespace: UTF-8 ]
[ tick: uint64 BE ]
[ root_digest: 32 bytes ]
[ bsm_entry_count: uint32 BE ][ bsm_entries ... ]
```

Snapshots **MUST NOT** be used as a substitute for the full event log. They
are acceleration hints only; replay **MUST** always be verifiable from genesis
(see §8.3).

---

## 4. Canonical JSON Numeric Encoding

### 4.1 Motivation

Floating-point representation varies across platforms, compilers, and runtime
environments. TRI-SYNC mandates a single canonical JSON numeric encoding to
ensure that any decimal value serializes to the same byte sequence on every
conforming node.

### 4.2 Integer Values

Integer values (type tag `0x02`) **MUST NOT** include a decimal point or
exponent in their JSON representation.

**Valid:** `42`, `-7`, `0`, `1000000`
**Invalid:** `42.0`, `4.2e1`, `1e6`

### 4.3 Decimal Values

Decimal values (type tag `0x03`) **MUST** conform to the following rules when
serialized to canonical JSON form:

1. **No leading zeros** — except for the single digit before the decimal point
   when the integer part is zero. (`0.5` is valid; `00.5` and `01.5` are invalid.)
2. **No trailing zeros after the decimal point** — `1.5` not `1.50`; `2.0` is
   represented as integer `2`, not decimal.
3. **No positive exponent notation** — values **MUST** be written in full decimal
   notation unless doing so would exceed 34 significant digits (see §4.4).
4. **Negative exponent notation** is permitted only for values where the absolute
   value is less than `1e-6`. In this case, normalized scientific notation
   **MUST** be used: one non-zero digit before the decimal point, followed by
   the fractional digits, followed by `e-N`.
5. **Sign**: Negative zero (`-0`) is forbidden. Positive sign prefix is
   forbidden (`+1.5` is invalid; `1.5` is valid).
6. **Infinity and NaN** are not representable and **MUST** be rejected as
   protocol errors.

### 4.4 High-Precision Decimals

For values requiring more than 34 significant digits, implementations **MUST**
truncate (not round) to 34 significant digits and record a precision-loss flag
in the containing event's metadata field. Precision loss does not invalidate
an event but **MUST** be propagated to subscribers.

### 4.5 Canonical JSON Object Encoding

When an entire BSM or event payload is serialized as a JSON object:

- Keys **MUST** be UTF-8 strings ordered lexicographically (see §5).
- No whitespace (spaces, tabs, newlines) is permitted outside of string values.
- Unicode escape sequences **MUST** use uppercase hex digits: `\uABCD` not `\uabcd`.
- Surrogate pairs **MUST** be represented as a single encoded codepoint where possible.
- The serialization **MUST NOT** include a trailing newline or byte-order mark (BOM).

---

## 5. UTF-8 Lexicographic Key Ordering

### 5.1 Definition

TRI-SYNC uses **UTF-8 byte-order lexicographic comparison** as the canonical
key ordering. Given two keys `A` and `B`, their order is determined by
comparing their UTF-8 encoded byte sequences left-to-right, byte by byte,
treating each byte as an unsigned 8-bit integer.

This ordering is:
- Deterministic across all conforming implementations
- Independent of locale, collation, or Unicode normalization
- Identical to the ordering produced by a standard `memcmp` on UTF-8 byte
  sequences of equal or differing length (shorter sequences that are a prefix
  of a longer sequence sort first)

### 5.2 Ordering Rules

1. Compare bytes left-to-right. The first differing byte determines order: the
   key with the lower unsigned byte value sorts first.
2. If all bytes of the shorter key match the corresponding prefix of the longer
   key, the shorter key sorts first.
3. Keys that are byte-for-byte identical are equal. Equal keys in the same
   namespace are a protocol violation (duplicate keys).

### 5.3 Normalization Prohibition

TRI-SYNC **MUST NOT** apply Unicode normalization (NFC, NFD, NFKC, NFKD) to
keys before comparison or storage. Two keys that differ only in Unicode
normalization form are considered distinct keys. Implementations **MUST NOT**
normalize keys silently.

### 5.4 Example Ordering

The following keys are shown in correct ascending lexicographic order:

```
""           ← prohibited (empty key)
"!"          ← 0x21
"A"          ← 0x41
"B"          ← 0x42
"a"          ← 0x61
"aa"         ← 0x61 0x61
"b"          ← 0x62
"tenant-a:x" ← namespace-scoped
"tenant-b:x" ← different namespace, sorts after
"é"          ← 0xC3 0xA9 (two bytes; sorts after all ASCII)
"ü"          ← 0xC3 0xBC
```

### 5.5 Enforcement

Every component that reads, writes, or digests the BSM **MUST** validate that
keys are in canonical order. An out-of-order key sequence is a fatal protocol
error. The receiving node **MUST** log the violation, reject the message or
snapshot, and emit a `PROTOCOL_ERROR` event to its own namespace log.

---

## 6. SHA-256 Digest Protocol

### 6.1 Digest Scope

TRI-SYNC uses SHA-256 as the exclusive hash function for all integrity
guarantees. No other hash function is permitted. All digests are 32 bytes
(256 bits), encoded as lowercase hexadecimal when represented in JSON or log
metadata.

### 6.2 Event Digest

Each event is assigned a digest computed over its canonical serialization:

```
digest = SHA-256( canonical_event_bytes )
```

The canonical event bytes are the binary encoding of the event as defined in
§10.3, with the `digest` field itself set to 32 zero bytes during computation
(to avoid circular dependency).

### 6.3 Chain Digest

Each event includes a `prev_digest` field containing the digest of the
immediately preceding event in the log. The genesis block sets `prev_digest`
to 32 zero bytes.

This forms an append-only cryptographic chain:

```
genesis_digest = SHA-256( genesis_event_bytes )
event_1_digest = SHA-256( event_1_bytes | prev=genesis_digest )
event_2_digest = SHA-256( event_2_bytes | prev=event_1_digest )
...
```

Any gap, reordering, or mutation in the chain **MUST** be detectable by
recomputing digests from the genesis block forward.

### 6.4 Root Digest

At the end of every tick, after all events for that tick have been applied, a
Root Digest is computed over the canonical BSM serialization (§3.4):

```
root_digest = SHA-256( canonical_bsm_bytes )
```

The Root Digest is appended to the log as a special `TICK_SEAL` event (see
§10.2.4) and **MUST** be verified before processing any event in the
subsequent tick.

### 6.5 Digest Verification Requirements

Conforming implementations **MUST**:

1. Verify `prev_digest` on every received event before applying it.
2. Recompute and verify the `digest` of each event on receipt.
3. Verify the `root_digest` in each `TICK_SEAL` event against a locally
   computed BSM digest.
4. Reject and quarantine any event that fails digest verification.
5. Never apply a quarantined event, even under operator instruction.

### 6.6 Digest Encoding

- In binary contexts: raw 32-byte big-endian value.
- In JSON contexts: lowercase 64-character hexadecimal string.
- In log metadata: lowercase 64-character hexadecimal string, no prefix.

---

## 7. Append-Only Event Log

### 7.1 Log Guarantees

The TRI-SYNC event log is **strictly append-only**. The following operations
are protocol violations:

- Deleting any event from the log
- Modifying any field of any committed event
- Inserting an event at any position other than the tail
- Truncating the log to reclaim space (compaction must use snapshot-based
  archival; see §7.5)

### 7.2 Log Segments

The event log is divided into contiguous, immutable segments. Each segment:

- Contains a contiguous range of events identified by monotonically increasing
  sequence numbers.
- Begins with a segment header (see §7.3).
- Is identified by its first and last sequence numbers and the digest of its
  first event.
- **MUST NOT** be modified once written.

### 7.3 Segment Header Format

```json
{
  "segment_id":   "<uuid-v4>",
  "namespace":    "<namespace-id>",
  "seq_start":    "<uint64>",
  "seq_end":      "<uint64>",
  "first_digest": "<sha256-hex>",
  "prev_segment": "<sha256-hex | null>",
  "created_at":   "<uint64-unix-ms>",
  "protocol_ver": "1.0.0"
}
```

`prev_segment` is the SHA-256 digest of the preceding segment's header. The
genesis segment sets `prev_segment` to `null`.

### 7.4 Event Sequence Numbers

- Sequence numbers are unsigned 64-bit integers, starting at `0` for the
  genesis block.
- Sequence numbers **MUST** be strictly monotonically increasing with no gaps.
- Each namespace maintains its own independent sequence number space.
- Cross-namespace sequence numbers are not comparable and **MUST NOT** be used
  to establish ordering between namespaces.

### 7.5 Log Compaction

To bound storage growth, conforming implementations **MAY** archive log
segments older than a configurable retention window. Archival **MUST**:

1. Produce a verified snapshot (§3.5) at the compaction boundary tick.
2. Preserve the full segment chain from the genesis block or the most recent
   verified snapshot, whichever is more recent.
3. Retain all `TICK_SEAL` events indefinitely; they are never subject to archival.
4. Record the compaction event in the log as a `COMPACT` event (see §10.2.5).

Archived segments **MUST** be retained in cold storage for at least the
operator-configured audit retention period (default: 7 years) and **MUST**
remain retrievable for replay verification.

### 7.6 Write Ordering Invariants

Within a single tick:

1. All `STATE_WRITE` events for the tick are buffered until the tick is closed.
2. Events are committed to the log in ascending canonical key order of their
   target key.
3. A single `TICK_SEAL` event is appended last, sealing the tick and recording
   the root digest.

No event from tick `T+1` may be committed before the `TICK_SEAL` of tick `T`
is durably written.

---

## 8. Deterministic Replay Rules

### 8.1 Replay Invariants

Replay is the process by which a node reconstructs the current BSM by
reprocessing the event log from a known starting point (genesis or a verified
snapshot). Replay **MUST** produce bit-identical BSM state to the original
execution, regardless of:

- The host operating system or hardware architecture
- The time elapsed since original execution
- The implementation language or runtime version (within the same protocol version)

### 8.2 Replay Starting Points

| Starting Point | Precondition |
|---|---|
| **Genesis** | Event log is complete from sequence `0`. |
| **Verified Snapshot** | Snapshot root digest matches the `TICK_SEAL` digest for the snapshot tick. |

Replay from a snapshot skips all events up to and including the snapshot tick.
The snapshot BSM is loaded directly, and replay proceeds from the next event
after the snapshot tick's `TICK_SEAL`.

### 8.3 Full-Chain Verification

Implementations supporting audit mode **MUST** offer full-chain replay from
genesis regardless of available snapshots. Full-chain replay verifies every
event digest and root digest in sequence. Any discrepancy is a chain integrity
violation.

### 8.4 Replay Execution Rules

During replay, the following rules **MUST** be enforced:

1. **No external I/O** — replay may not perform network requests, file system
   writes, or any non-deterministic system call. All state transitions must
   derive exclusively from the event log.
2. **No clock reads** — wall-clock or monotonic clock reads are forbidden
   during replay. Timestamps in events are taken verbatim from the log; they
   are not recomputed.
3. **No randomness** — any PRNG or entropy source **MUST** be seeded from a
   deterministic value derived from the event log, not from system entropy.
4. **Strict event ordering** — events are applied in ascending sequence number
   order. No buffering, reordering, or speculative application is permitted.
5. **Idempotency enforcement** — events marked `idempotent: true` that appear
   more than once in the log (due to at-least-once delivery guarantees) **MUST**
   be applied exactly once. Duplicate detection uses the event `digest` as the
   deduplication key.
6. **Replay guard activation** — all replay guards (§8.5) **MUST** be active
   during replay. Guards may not be disabled by operator configuration during
   replay mode.

### 8.5 Replay Guards

Replay guards are runtime checks that detect and halt replay on
non-deterministic conditions.

| Guard | Trigger Condition | Action |
|---|---|---|
| `DIGEST_MISMATCH` | Recomputed event digest does not match stored digest | Halt, emit `REPLAY_ERROR`, quarantine log segment |
| `SEQ_GAP` | Sequence number is non-consecutive | Halt, emit `REPLAY_ERROR` |
| `TICK_SEAL_FAIL` | Root digest at tick boundary does not match recomputed BSM digest | Halt, emit `REPLAY_ERROR` |
| `NAMESPACE_LEAK` | Event targets a key outside its declared namespace | Halt, emit `PROTOCOL_ERROR` |
| `TYPE_MISMATCH` | Value type tag does not match the declared type for an existing key | Halt, emit `REPLAY_ERROR` |
| `ORDER_VIOLATION` | Key in BSM serialization is out of lexicographic order | Halt, emit `REPLAY_ERROR` |
| `DUPLICATE_EVENT` | Non-idempotent event with a previously seen digest is encountered | Skip event, emit `WARN_DUPLICATE` |

### 8.6 Replay Completion

Replay is complete when the last event in the log has been applied and the
final `TICK_SEAL` root digest has been verified against the locally computed
BSM. The node then transitions from replay mode to live mode.

---

## 9. Multi-Tenant Isolation

### 9.1 Namespace Model

Each tenant is assigned exactly one namespace. A namespace is a string
identifier conforming to the following rules:

- **Format:** `[a-z0-9][a-z0-9\-]{1,61}[a-z0-9]` (lowercase alphanumeric and
  hyphens, 3–63 characters, no leading or trailing hyphen)
- **Uniqueness:** Namespace identifiers **MUST** be unique within a TRI-SYNC cluster.
- **Immutability:** Once assigned, a namespace identifier **MUST NOT** be
  changed or reused, even after a tenant is deprovisioned.

### 9.2 Key-Space Partitioning

All BSM keys **MUST** be prefixed with the owning namespace and a colon separator:

```
<namespace>:<user-defined-key>
```

Keys that do not carry the correct namespace prefix for the processing context
are a protocol violation. Implementations **MUST** reject such keys at the
write path and quarantine any event that contains them.

The `:` separator character is reserved; user-defined key portions **MUST NOT**
begin with `:`.

### 9.3 Isolation Enforcement

| Operation | Rule |
|---|---|
| **Read** | A tenant context may only read keys with its own namespace prefix. |
| **Write** | A tenant context may only write keys with its own namespace prefix. |
| **Event Log** | Each namespace maintains a dedicated, independent event log. Cross-log references are forbidden. |
| **Digest** | Root digests are computed per namespace. There is no cross-namespace composite digest. |
| **Replay** | Replay is always scoped to a single namespace. Cross-namespace replay is not defined. |

### 9.4 Administrative Namespace

The reserved namespace `trisync-system` is used exclusively by the runtime for
system events (cluster membership, compaction records, audit events). Tenant
operations **MUST NOT** target the `trisync-system` namespace. System events
**MUST NOT** reference tenant namespaces.

### 9.5 Namespace Lifecycle Events

| Event Type | Description |
|---|---|
| `NS_CREATE` | Namespace provisioned; genesis block written. |
| `NS_SUSPEND` | Namespace suspended; writes rejected, reads permitted. |
| `NS_RESUME` | Namespace re-activated from suspended state. |
| `NS_DEPROVISION` | Namespace permanently decommissioned; log sealed and archived. |

After `NS_DEPROVISION`, no further events may be written to the namespace log.
The final `TICK_SEAL` and `NS_DEPROVISION` event are the permanent tail of
the log.

### 9.6 Resource Quotas

Each namespace **MAY** have operator-configured resource quotas:

- `max_key_count` — maximum number of active BSM keys (default: unlimited)
- `max_bsm_bytes` — maximum total size of BSM values (default: unlimited)
- `max_events_per_tick` — maximum events in a single tick (default: `65535`)
- `max_log_bytes` — soft limit triggering compaction recommendation (default: unlimited)

Quota violations **MUST** result in a `QUOTA_EXCEEDED` error event; the
offending write is rejected. Quota events do not halt the namespace; subsequent
writes are permitted after the condition is resolved.

---

## 10. Message Semantics

### 10.1 Message Transport Contract

TRI-SYNC messages are transport-agnostic. This specification defines message
structure and processing semantics only. Transport-layer concerns (framing,
ordering guarantees, retry, backpressure) are delegated to the transport
binding specification.

All messages **MUST** be serialized in canonical JSON form (§4.5) for the wire
format, or in the binary format described in §3 when using binary transport
bindings.

### 10.2 Event Types

#### 10.2.1 `STATE_WRITE`

Records a key-value write to the BSM.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | string | **MUST** | `"STATE_WRITE"` |
| `seq` | uint64 | **MUST** | Monotonic sequence number |
| `tick` | uint64 | **MUST** | Logical tick number |
| `namespace` | string | **MUST** | Target namespace |
| `key` | string | **MUST** | Target BSM key (namespace-prefixed) |
| `value_type` | uint8 | **MUST** | Type tag (see §3.3) |
| `value` | varies | **MUST** | Canonical encoded value |
| `prev_value_digest` | hex32 | **SHOULD** | Digest of the previous value, for optimistic concurrency |
| `idempotent` | boolean | **MUST** | Whether this write is safe to apply multiple times |
| `prev_digest` | hex32 | **MUST** | Digest of the preceding event |
| `digest` | hex32 | **MUST** | SHA-256 digest of this event (see §6.2) |
| `metadata` | object | **MAY** | Arbitrary key-value pairs; not included in digest computation |

#### 10.2.2 `STATE_DELETE`

Records a key deletion from the BSM.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | string | **MUST** | `"STATE_DELETE"` |
| `seq` | uint64 | **MUST** | Monotonic sequence number |
| `tick` | uint64 | **MUST** | Logical tick number |
| `namespace` | string | **MUST** | Target namespace |
| `key` | string | **MUST** | Key to delete (namespace-prefixed) |
| `prev_value_digest` | hex32 | **MUST** | Digest of the value being deleted (optimistic concurrency) |
| `idempotent` | boolean | **MUST** | `true` if deleting an already-absent key is a no-op |
| `prev_digest` | hex32 | **MUST** | Digest of the preceding event |
| `digest` | hex32 | **MUST** | SHA-256 digest of this event |

A `STATE_DELETE` targeting a key that does not exist in the BSM is a no-op
when `idempotent` is `true`, and a `KEY_NOT_FOUND` error when `idempotent`
is `false`.

#### 10.2.3 `STATE_BATCH`

An atomic group of `STATE_WRITE` and/or `STATE_DELETE` operations applied as
a single unit.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | string | **MUST** | `"STATE_BATCH"` |
| `seq` | uint64 | **MUST** | Sequence number of the batch event itself |
| `tick` | uint64 | **MUST** | Logical tick number |
| `namespace` | string | **MUST** | Target namespace |
| `ops` | array | **MUST** | Ordered list of `STATE_WRITE` or `STATE_DELETE` payloads (without `seq`, `prev_digest`, `digest`) |
| `prev_digest` | hex32 | **MUST** | Digest of the preceding event |
| `digest` | hex32 | **MUST** | SHA-256 digest of the full batch, including all `ops` |

Within a batch, `ops` are applied in the order specified. If any operation in
the batch fails, the entire batch is rolled back — no partial application is
permitted. Batches are always `idempotent: false` at the batch level;
individual ops inherit their own idempotency flag.

#### 10.2.4 `TICK_SEAL`

Closes a tick and records the root digest of the resulting BSM.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | string | **MUST** | `"TICK_SEAL"` |
| `seq` | uint64 | **MUST** | Sequence number |
| `tick` | uint64 | **MUST** | Tick being sealed |
| `namespace` | string | **MUST** | Target namespace |
| `event_count` | uint32 | **MUST** | Number of events applied in this tick |
| `root_digest` | hex32 | **MUST** | SHA-256 of canonical BSM after all tick events are applied |
| `prev_digest` | hex32 | **MUST** | Digest of the preceding event (last event of the tick) |
| `digest` | hex32 | **MUST** | SHA-256 digest of this `TICK_SEAL` event |
| `timestamp_ms` | uint64 | **MUST** | Unix epoch milliseconds at which the tick was sealed |

`TICK_SEAL` events **MUST NOT** be omitted, even for empty ticks. An empty
tick has `event_count: 0` and a `root_digest` equal to the previous tick's
`root_digest`.

#### 10.2.5 `COMPACT`

Records a log compaction event.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | string | **MUST** | `"COMPACT"` |
| `seq` | uint64 | **MUST** | Sequence number |
| `tick` | uint64 | **MUST** | Tick at which compaction occurred |
| `namespace` | string | **MUST** | Target namespace |
| `snapshot_digest` | hex32 | **MUST** | Root digest of the snapshot taken at compaction |
| `archived_seq_start` | uint64 | **MUST** | First sequence number archived |
| `archived_seq_end` | uint64 | **MUST** | Last sequence number archived |
| `archive_uri` | string | **MUST** | URI of the archived segment in cold storage |
| `prev_digest` | hex32 | **MUST** | Digest of the preceding event |
| `digest` | hex32 | **MUST** | SHA-256 digest of this event |

#### 10.2.6 `PROTOCOL_ERROR`

Records a protocol violation detected by the local node.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | string | **MUST** | `"PROTOCOL_ERROR"` |
| `seq` | uint64 | **MUST** | Sequence number |
| `tick` | uint64 | **MUST** | Tick in which the error was detected |
| `namespace` | string | **MUST** | Namespace context |
| `error_code` | string | **MUST** | One of the defined error codes (see §11) |
| `offending_seq` | uint64 | **MAY** | Sequence number of the violating event |
| `detail` | string | **MAY** | Human-readable description |
| `prev_digest` | hex32 | **MUST** | Digest of the preceding event |
| `digest` | hex32 | **MUST** | SHA-256 digest of this event |

`PROTOCOL_ERROR` events are informational records. They do not close the
namespace or the tick; the offending event is quarantined and the log continues.

### 10.3 Canonical Event Serialization

For digest computation, events are serialized as canonical JSON (§4.5) with
the following additional rules:

1. Fields **MUST** appear in ascending UTF-8 lexicographic key order.
2. The `digest` field **MUST** be set to
   `"0000000000000000000000000000000000000000000000000000000000000000"`
   (64 zeros) during digest computation.
3. The `metadata` field **MUST** be excluded from digest computation entirely.
4. All integer fields **MUST** be encoded as JSON integers (not strings).
5. The serialization **MUST NOT** include trailing whitespace or newlines.

### 10.4 Message Processing Order

Within a tick, messages are processed in the following order:

1. `NS_CREATE` / `NS_RESUME` (if applicable)
2. `STATE_WRITE` and `STATE_BATCH` events, in sequence-number order
3. `STATE_DELETE` events, in sequence-number order
4. `NS_SUSPEND` / `NS_DEPROVISION` (if applicable)
5. `COMPACT` (if applicable)
6. `PROTOCOL_ERROR` (appended as errors are detected)
7. `TICK_SEAL` (always last)

No event type may appear after `TICK_SEAL` within the same tick.

---

## 11. Error Handling

### 11.1 Error Codes

| Code | Description | Severity |
|---|---|---|
| `DIGEST_MISMATCH` | Recomputed digest does not match stored digest | Fatal |
| `SEQ_GAP` | Non-consecutive sequence number detected | Fatal |
| `TICK_SEAL_FAIL` | Root digest mismatch at tick boundary | Fatal |
| `NAMESPACE_LEAK` | Key targets a foreign namespace | Fatal |
| `TYPE_MISMATCH` | Value type inconsistent with existing key type | Fatal |
| `ORDER_VIOLATION` | BSM keys out of lexicographic order | Fatal |
| `KEY_NOT_FOUND` | Delete or read on non-existent key with `idempotent: false` | Error |
| `KEY_TOO_LONG` | Key exceeds 512 bytes | Error |
| `INVALID_KEY` | Key contains null byte or is empty | Error |
| `QUOTA_EXCEEDED` | Write would violate a namespace quota | Error |
| `INVALID_NUMERIC` | Decimal encoding violates canonical rules | Error |
| `BATCH_ROLLBACK` | One or more ops in a `STATE_BATCH` failed | Error |
| `PRECISION_LOSS` | Decimal truncated to 34 significant digits | Warning |
| `WARN_DUPLICATE` | Idempotent event applied more than once | Warning |
| `INVALID_SEGMENT` | Segment header fields are malformed or inconsistent | Fatal |

**Fatal** errors halt the affected replay or processing context and emit a
`PROTOCOL_ERROR` event. **Error** severity rejects the offending message.
**Warning** severity is recorded but does not interrupt processing.

### 11.2 Error Propagation

Errors detected during live message processing **MUST** be:

1. Appended to the event log as a `PROTOCOL_ERROR` event before the current
   tick's `TICK_SEAL`.
2. Returned to the originating client as an error response with the applicable
   error code.
3. Reported to the operator monitoring interface.

Errors detected during replay **MUST** halt replay immediately. The partially
reconstructed BSM **MUST** be discarded. The node **MUST NOT** enter live mode
until the replay error is resolved.

---

## 12. Versioning and Compatibility

### 12.1 Protocol Version

The protocol version is a semantic version string of the form `MAJOR.MINOR.PATCH`.

- **MAJOR** increment: breaking change to the canonical encoding, event format,
  or digest protocol.
- **MINOR** increment: backward-compatible addition (new event types, new
  optional fields).
- **PATCH** increment: clarification or correction with no behavioral change.

The current version is **1.0.0**.

### 12.2 Version Negotiation

The protocol version is declared in:

- Every segment header (`protocol_ver` field)
- Every `NS_CREATE` event (`protocol_ver` field)

Nodes **MUST** reject events from a segment with a MAJOR version higher than
their supported version. Nodes **MAY** process events from segments with a
lower MAJOR version only if a documented migration procedure applies.

### 12.3 Forward Compatibility

Implementations **MUST** ignore unrecognized JSON fields in event payloads at
MINOR version boundaries. Unrecognized fields **MUST NOT** be included in
digest computation (only known, specified fields participate in the digest).

### 12.4 Deprecation

Fields or event types deprecated in a MINOR version will be removed in the
next MAJOR version. A deprecation notice **MUST** be present for at least one
full MINOR version before removal.

---

## 13. Security Considerations

### 13.1 Digest Integrity

The SHA-256 chain provides tamper evidence, not tamper prevention. Operators
**MUST** ensure that:

- Log storage is access-controlled; only the TRI-SYNC runtime may append to
  the log.
- Snapshot storage is read-only after creation.
- Archive storage is immutable (write-once) after the `COMPACT` event is
  committed.

### 13.2 Namespace Isolation

Namespace isolation is enforced at the protocol layer, not solely at the
storage layer. Operators **MUST NOT** rely on shared storage access controls
as the sole isolation mechanism.

### 13.3 Key Confidentiality

TRI-SYNC does not encrypt keys or values in the log. If key or value
confidentiality is required, operators **MUST** apply encryption at the
application layer before writing values to the BSM. TRI-SYNC digests and
chains operate over the raw (potentially encrypted) bytes.

### 13.4 Replay Attack Prevention

The cryptographic chain and monotonically increasing sequence numbers prevent
replay of stale event subsequences. Implementations **MUST** reject any event
whose `seq` is less than or equal to the highest seen `seq` for the namespace,
regardless of digest validity.

### 13.5 Denial of Service

Implementations **SHOULD** enforce:

- Maximum event payload size (recommended: 1 MiB per event)
- Maximum batch size (recommended: 1000 ops per `STATE_BATCH`)
- Rate limits on `PROTOCOL_ERROR` generation to prevent log flooding

---

## 14. Conformance

### 14.1 Conformance Classes

| Class | Description |
|---|---|
| **Core** | Implements §3, §4, §5, §6, §7, §8, §10. Sufficient for a read-write node. |
| **Full** | Implements Core plus §9 (multi-tenant isolation) and §11 (error handling). Required for production deployments. |
| **Audit** | Implements Full plus full-chain replay from genesis (§8.3). Required for audit nodes. |

### 14.2 Mandatory Requirements Summary

Implementations claiming conformance at any class **MUST**:

- [ ] Serialize and deserialize the BSM in UTF-8 lexicographic key order (§5)
- [ ] Use SHA-256 exclusively for all digests (§6.1)
- [ ] Verify `prev_digest` on every received event (§6.5)
- [ ] Verify `root_digest` on every `TICK_SEAL` event (§6.5)
- [ ] Reject events that fail digest verification (§6.5)
- [ ] Enforce append-only semantics on the event log (§7.1)
- [ ] Enforce deterministic replay rules including all replay guards (§8.4, §8.5)
- [ ] Enforce namespace key-prefix isolation (§9.2, §9.3)
- [ ] Produce canonical JSON numeric encoding (§4)
- [ ] Apply canonical event serialization for digest computation (§10.3)
- [ ] Process events in the defined within-tick order (§10.4)

### 14.3 Test Vectors

A companion test-vector suite is provided in `test-vectors/` within this
repository. Conforming implementations **MUST** pass all test vectors for their
claimed conformance class before declaring conformance.

---

*End of TRI-SYNC Protocol Specification v1.0.0*

---

**Document History**

| Version | Date | Change Summary |
|---|---|---|
| 1.0.0 | 2026-08-08 | Initial normative release |
```

---

**What's covered — section by section:**

| Section | Content |
|---|---|
| §3 Binary State Map | Wire format, 6 typed value tags, key constraints, snapshot layout |
| §4 Canonical JSON Numerics | Integer vs. decimal rules, no exponents, no trailing zeros, precision-loss truncation |
| §5 UTF-8 Lex Ordering | `memcmp`-based ordering, normalization prohibition, worked example |
| §6 SHA-256 Digests | Event digest, chain digest, root digest, circular-dependency zero-fill |
| §7 Append-Only Log | Segment headers, sequence numbers, compaction via `COMPACT` event, write ordering |
| §8 Deterministic Replay | No I/O / no clocks / no randomness rules, all 7 replay guards |
| §9 Multi-Tenant Isolation | `<ns>:<key>` partitioning, lifecycle events, quotas, `trisync-system` reservation |
| §10 Message Semantics | Full field tables for `STATE_WRITE`, `STATE_DELETE`, `STATE_BATCH`, `TICK_SEAL`, `COMPACT`, `PROTOCOL_ERROR`; tick ordering |
| §11 Error Handling | 15 error codes with severity levels; propagation rules |
| §12 Versioning | SemVer negotiation, forward-compat field ignoring, deprecation policy |
| §13 Security | Tamper evidence vs. prevention, key confidentiality, DoS mitigations |
| §14 Conformance | Core / Full / Audit classes + checklist + test-vector reference |
