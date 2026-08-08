# TRI-SYNC Deterministic Invariants

**Version:** 1.0.0  
**Status:** Normative  
**Date:** 2026-08-08

These invariants define the correctness contract of the TRI-SYNC runtime.
Any implementation that violates one of these rules is non-conformant.

---

## 1. Sequence Ordering

Every event in the log carries a monotonically increasing `sequence` field starting at `0`.

- The first event must have `sequence == 0`.
- Each subsequent event must have `sequence == previous_sequence + 1`.
- A gap, duplication, or out-of-order sequence number is a fatal replay error.

**Rationale:** Strict sequential ordering makes replay deterministic and prevents
event injection attacks or accidental log corruption.

---

## 2. Canonical JSON Encoding

All JSON serialised by TRI-SYNC must conform to the canonical form:

- Object keys are sorted in UTF-8 lexicographic (byte-level) order, recursively.
- No trailing commas, no comments, no whitespace outside string values.
- Numbers are encoded as per IEEE 754 F64 with no scientific notation for values
  representable in normal decimal form (`1.5`, not `1.5e0`).
- Strings are UTF-8; no surrogate pairs.

The canonical form is used for:
- Event payload serialisation before digest computation.
- State serialisation for comparison and output.

**Rationale:** Two systems encoding the same logical value must produce byte-for-byte
identical output to arrive at the same SHA-256 digest.

---

## 3. Binary State Map Encoding

Numeric state values are stored as IEEE 754 64-bit floating-point in big-endian
byte order (8 bytes per value).

- No platform-specific byte ordering is permitted.
- NaN and ±Infinity are not valid state values.

String and raw binary state values are stored as raw UTF-8 bytes with no length
prefix or null terminator in the on-disk format; they are hex-encoded for JSON
serialisation.

**Rationale:** Big-endian F64 ensures identical binary representation regardless
of host CPU architecture.

---

## 4. SHA-256 Digest Chain

Each event carries a `payload_sha256` field computed over the canonical JSON of its
payload fields:

```
digest = SHA-256(canonical_json({sequence, tenant, key, kind, value_hex}))
```

- The digest is encoded as lowercase hex (64 characters).
- `value_hex` is `null` for delete events and is included in the canonical JSON as
  the JSON `null` value.
- The `payload_sha256` field itself is excluded from the digest input.
- Replay MUST verify every digest before applying any state change.

**Rationale:** Per-event digests provide tamper evidence. An altered event is
detectable before its state transition is applied.

---

## 5. Append-Only Event Log

The event log is a flat text file where each line is one canonical JSON event.

- Events are never modified or deleted.
- New events are only ever appended.
- Empty lines are ignored on read.
- Any line that does not deserialise to a valid `Event` is a fatal read error.

**Rationale:** Append-only semantics guarantee that any observer holding a log
prefix will reach the same state as any observer holding the full log, up to the
shared prefix.

---

## 6. Deterministic Replay Rules

Replay is the process of reconstructing state from the event log.

- Replay always starts from an empty state map.
- Events are applied in strict sequence order (invariant 1).
- Each event's digest is verified before it is applied (invariant 4).
- `Set` events write `(tenant, key) → value_bytes` into the state map.
- `Delete` events remove `(tenant, key)` from the state map.
- Given the same log, any two compliant implementations must produce byte-identical
  state maps.

**Rationale:** Deterministic replay is the foundation of auditability, compliance,
and debugging. Any non-determinism breaks the audit guarantee.

---

## 7. Multi-Tenant Key Ordering

The state map key space is the product `tenant × key`, sorted lexicographically:

1. Primary sort: tenant name, UTF-8 byte order.
2. Secondary sort: key name within tenant, UTF-8 byte order.

Tenant namespaces are strictly isolated:
- A `Set` or `Delete` in tenant `A` never affects keys owned by tenant `B`.
- Cross-tenant key iteration is forbidden in single-tenant API surfaces.

**Rationale:** Deterministic ordering allows reproducible serialisation of the
entire state map. Isolation prevents accidental cross-tenant state corruption.

---

## 8. Workflow Step Determinism

When running a workflow (`tri-sync run`):

- Steps are executed in the order they appear in the `steps` array.
- Each step produces zero or more events that are appended to the log before the
  next step begins.
- Step operations (`set`, `delete`, `add`, `multiply`) are deterministic: given the
  same prior state, the same step always produces the same events.
- The `add` and `multiply` operations read the current state, apply the operation,
  and write the result as a new `Set` event.
- If a key referenced by `add` or `multiply` does not exist, the operation treats the
  current value as `0.0`.

**Rationale:** Workflows are just a structured way to produce a sequence of events.
Their determinism follows directly from the determinism of the underlying event log.

---

## 9. Digest Chain Across Steps

In a workflow run, the sequence of `payload_sha256` values forms a digest chain.
The digest of event `N` implicitly commits to the digests of all previous events via
the `sequence` field:

```
chain[0] = SHA-256(canonical_payload(event[0]))
chain[N] = SHA-256(canonical_payload(event[N]))  // sequence field encodes position
```

A consumer can verify the chain by replaying from sequence 0 and checking that each
digest matches.

---

## Summary Table

| # | Invariant | Violation consequence |
|---|-----------|----------------------|
| 1 | Sequence monotonicity | Fatal replay error |
| 2 | Canonical JSON encoding | Digest mismatch → fatal |
| 3 | Big-endian F64 binary encoding | Incorrect numeric state |
| 4 | SHA-256 payload digest | Fatal replay error |
| 5 | Append-only event log | Undefined state |
| 6 | Deterministic replay | Non-reproducible state |
| 7 | Tenant isolation and ordering | Cross-tenant corruption |
| 8 | Workflow step determinism | Non-reproducible workflow |
| 9 | Digest chain integrity | Undetectable tampering |
