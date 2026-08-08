# TRI-SYNC Runtime Architecture

**Version:** 1.0.0  
**Status:** Normative  
**Date:** 2026-08-08

This document describes the internal structure of the TRI-SYNC runtime.
It is an implementation-level reference, not a marketing overview.

---

## Component Map

```
┌─────────────────────────────────────────────────────────────────┐
│  CLI  (tri-sync)                                                │
│  apply | delete | run | replay | digest | example               │
└────┬──────────┬────────────┬────────────┬────────────┬──────────┘
     │          │            │            │            │
     ▼          ▼            ▼            ▼            ▼
 Event       Event       Workflow     Replay       Digest
 Builder     Log          Runner      Engine       Module
     │          │            │            │
     └──────────┴────────────┘            │
                │                         │
                ▼                         ▼
          AppendOnly               BinaryStateMap
          EventLog                 (BTreeMap<TenantKey, Vec<u8>>)
          (flat text, one          sorted by (tenant, key)
           canonical JSON          UTF-8 lexicographic
           line per event)
                │
                ▼
          Canonical JSON
          Encoder
          (key-sorted, no
           whitespace)
```

---

## Modules

### `canonical_json` (`src/canonical_json.rs`)

Converts any `serde_json::Value` into its canonical string form:

- Object keys sorted recursively in UTF-8 lexicographic order.
- No extra whitespace.
- Numbers serialised by `serde_json`'s default IEEE 754 rules.

Used by every module that produces a digest or writes to the event log.

---

### `digest` (`src/digest.rs`)

Single public function: `sha256_hex(data: &[u8]) -> String`.

- Wraps the `sha2` crate's `Sha256` hasher.
- Returns a 64-character lowercase hex string.
- Has no state; every call is independent.

---

### `hex` (`src/hex.rs`)

Helpers for converting `&[u8]` ↔ lowercase hex strings.

- `encode_hex` — used when storing binary values in JSON events.
- `decode_hex` — used when reading values back from events.

---

### `key` (`src/key.rs`)

Defines `TenantKey { tenant: String, key: String }`.

- Derives `Ord` so `BTreeMap<TenantKey, _>` is automatically sorted
  by `(tenant, key)` in UTF-8 lexicographic order.
- This is the only type used as a state map key.

---

### `event` (`src/event.rs`)

Defines the `Event` struct and `EventKind` enum (`Set` / `Delete`).

Fields:
| Field | Type | Description |
|-------|------|-------------|
| `sequence` | `u64` | Monotonically increasing position in the log |
| `tenant` | `String` | Tenant namespace |
| `key` | `String` | Key within the tenant |
| `kind` | `EventKind` | `set` or `delete` |
| `value_hex` | `Option<String>` | Hex-encoded value bytes (Set only) |
| `payload_sha256` | `String` | SHA-256 of canonical payload JSON |

Constructor methods:
- `Event::new_set(sequence, tenant, key, value: &[u8])` — hex-encodes the value
  and computes the digest.
- `Event::new_delete(sequence, tenant, key)` — no value; digest covers `null`.

Validation:
- `validate_digest()` recomputes the expected digest and compares it to the stored
  one. Called by the replay engine before applying each event.

---

### `event_log` (`src/event_log.rs`)

`AppendOnlyEventLog` wraps a file path and provides:

- `append(event)` — serialises the event to canonical JSON and appends one line.
- `load()` — reads all lines, deserialises, and returns `Vec<Event>`.
- `next_sequence()` — returns `last_sequence + 1` (or `0` for an empty log).

The log file is a plain UTF-8 text file. Each non-empty line is one event.
The file is opened in append mode; no seek or overwrite is performed.

---

### `state_map` (`src/state_map.rs`)

`BinaryStateMap` is a `BTreeMap<TenantKey, Vec<u8>>`.

- `set(tenant, key, value)` / `set_tenant_key(key, value)` — upsert.
- `delete(tenant, key)` / `delete_tenant_key(key)` — remove.
- `get(tenant, key)` — lookup.
- `entries()` — ordered iterator over all `(TenantKey, Vec<u8>)` pairs.
- `to_nested_hex_json()` — emits a `serde_json::Value` of shape
  `{ tenant: { key: hex_value } }`, used for output and digest computation.

---

### `replay` (`src/replay.rs`)

`ReplayEngine::replay(events: &[Event]) -> Result<BinaryStateMap, String>`

Algorithm:
1. Start with `expected_sequence = 0`.
2. For each event:
   a. Assert `event.sequence == expected_sequence`; error otherwise.
   b. Call `event.validate_digest()`; error on mismatch.
   c. Apply the event to the state map (set or delete).
   d. Increment `expected_sequence`.
3. Return the final state map.

The enhanced replay (`tri-sync replay --verbose`) additionally:
- Prints each event as it is applied.
- Shows the state diff (added/changed/removed keys) after each event.
- Prints the `payload_sha256` for each event.
- Emits a separator when the tenant changes between consecutive events.

---

### `workflow` (`src/workflow.rs`)

Defines the workflow JSON format and `WorkflowRunner`.

**Workflow JSON schema:**
```json
{
  "id": "<string>",
  "description": "<string>",
  "steps": [
    {
      "id": "<string>",
      "tenant": "<string>",
      "ops": [
        { "op": "set",      "key": "<k>", "value": "<string>" },
        { "op": "delete",   "key": "<k>" },
        { "op": "add",      "key": "<k>", "operand": <f64> },
        { "op": "multiply", "key": "<k>", "operand": <f64> }
      ]
    }
  ]
}
```

`WorkflowRunner::run(workflow, log)`:
1. Loads the current log to determine the next sequence number.
2. For each step, in order:
   - Applies the current state (from log) to resolve `add`/`multiply` operands.
   - For each op, produces an `Event` and appends it to the log.
3. Returns the final state after all steps.

---

## CLI (`src/main.rs`)

The `tri-sync` binary exposes these subcommands:

| Subcommand | Purpose |
|------------|---------|
| `apply --log <f> --tenant <t> --key <k> --value <v>` | Append a `Set` event |
| `delete --log <f> --tenant <t> --key <k>` | Append a `Delete` event |
| `run <workflow.json> --log <f>` | Execute a workflow file |
| `replay --log <f>` | Replay and print final state (canonical JSON) |
| `replay --log <f> --verbose` | Replay with per-event diffs and digest chain |
| `digest --input <s>` | Print SHA-256 hex of a string |
| `example --log <f>` | Run the built-in example workflow |

---

## Data Flow: `tri-sync run`

```
workflow.json
      │
      ▼
WorkflowRunner::run()
      │  reads current log state
      │  iterates steps in order
      │  for each op → Event::new_set / new_delete
      │  AppendOnlyEventLog::append(event)
      ▼
event.log  ←── append-only
      │
      ▼
ReplayEngine::replay()
      │  sequence check
      │  digest check
      │  apply set/delete
      ▼
BinaryStateMap
      │
      ▼
to_nested_hex_json()  →  canonical_json  →  stdout
```

---

## Data Flow: `tri-sync replay --verbose`

```
event.log
      │
      ▼
AppendOnlyEventLog::load()  →  Vec<Event>
      │
      ▼
ReplayEngine::replay_verbose()
  for each event:
    1. verify sequence
    2. verify digest  →  print digest (chain entry)
    3. capture state snapshot before
    4. apply event
    5. capture state snapshot after
    6. compute diff (added / changed / removed)
    7. print event summary + diff
    8. if tenant changed from previous event: print boundary marker
      ▼
final BinaryStateMap
```

---

## Invariant Enforcement Points

| Invariant | Enforced in |
|-----------|------------|
| Sequence monotonicity | `ReplayEngine::replay` |
| Canonical JSON encoding | `canonical_json::to_canonical_string` |
| Big-endian F64 (numeric state values) | `state_map` + protocol convention |
| SHA-256 payload digest | `Event::validate_digest` |
| Append-only log | `AppendOnlyEventLog::append` (O_APPEND) |
| Deterministic replay | `ReplayEngine::replay` |
| Tenant isolation | `BinaryStateMap` + `TenantKey` ordering |
| Workflow step determinism | `WorkflowRunner::run` |
