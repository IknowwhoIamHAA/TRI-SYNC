# TRI-SYNC Product Overview

**TRI-SYNC** is a compliance-first deterministic, append-only runtime for reproducible AI workflows, unalterable provenance trails, and cryptographically auditable regulated-data pipelines. It gives finance, healthcare, insurance, and AI platform teams a portable foundation for provable state and tamper-evident SHA-256 computation history.

---

## The Problem

Modern AI and automation systems suffer from a fundamental auditability gap:

- **Non-determinism** — the same inputs produce different outputs across runs, machines, and environments.
- **No audit trail** — there is no tamper-proof record of what happened, in what order, and why.
- **State drift** — distributed components diverge silently; debugging requires manual reconciliation.
- **Vendor lock-in** — audit and state systems are tightly coupled to cloud-provider proprietary APIs.

These gaps create operational and assurance failures for teams that need processing integrity, model-risk controls, and defensible audit evidence.

---

## What TRI-SYNC Does

TRI-SYNC is a **deterministic runtime engine** that guarantees:

1. **Identical outputs for identical inputs** — regardless of machine, OS, clock, or execution order within a tick.
2. **Cryptographic audit trail** — every state transition is SHA-256 hashed and chained, forming an immutable, replayable event log.
3. **Protocol-frozen stability** — the v1.0.0 wire format is frozen; any two conforming implementations produce byte-for-byte identical state representations.
4. **Cloud-neutral portability** — runs on laptop, server, container, bare metal, or across clouds with no single-vendor dependency.

---

## Core Features

### Deterministic Replay
Any node can reconstruct current state from the genesis block and the full event log without external coordination. Two nodes starting from the same log always arrive at the same state.

### Canonical State Map (BSM)
A binary-encoded, cryptographically-keyed key-value store. All values are canonically encoded (big-endian, no ambiguity). The root digest is SHA-256 of the entire state — a single hash that proves the complete state is unchanged.

### Append-Only Event Log
Events are written once, never modified. Each event is chained to the previous via `prev_digest`. Tampering with any event invalidates all subsequent digests — instantly detectable.

### SHA-256 Digest Chain
Every event carries a self-digest (SHA-256 of its canonical form) and a `prev_digest` linking it to the prior event. The chain is verified on every replay. Any gap, reorder, or mutation is a protocol violation.

### TICK_SEAL Snapshots
At the end of every logical tick, a `TICK_SEAL` event records the root digest of the complete state. This creates verifiable checkpoints that any downstream node can independently confirm.

### Multi-Tenant Isolation
Each tenant has a dedicated namespace. Namespace keys are strictly prefixed; cross-tenant reads and writes are protocol violations detected and halted at runtime. Enterprise multi-tenant deployment and scaling require a commercial license; a local single-tenant workflow remains free.

### Deterministic JSON Encoding
All event payloads use RFC 8785 canonical JSON: sorted keys, no whitespace, lowercase `\uXXXX` escapes, no locale drift. The same data always produces the same bytes, always produces the same hash.

---

## Who It's For

| Sector | Use Case |
|---|---|
| **Finance** | Agentic SOC 2 Type II Processing Integrity evidence, internal model risk management, and provable computation history |
| **Healthcare** | Cryptographically verified clinical-decision trails, patient state change tracking, and controlled automation evidence |
| **Insurance** | Deterministic claims processing, reproducible underwriting models, fraud detection audit chains |
| **AI Platforms** | Reproducible model inference logs, prompt-response audit trails, multi-agent coordination with deterministic state |
| **Internal Audit / Risk** | Immutable evidence chains, reconciliations, and replayable control validation |

### Frontier-Scale Governance Segment

Frontier-scale risk tracking is a separate positioning track for elite labs and advanced
AI developers operating under emerging governance frameworks such as SB 53. That
segment is distinct from TRI-SYNC's core enterprise narrative, which is anchored on
processing integrity, internal MRM, and cryptographically verified audit trails.

---

## Architecture

```
User Input
    │
    ▼
Canonical JSON Encoding  ←  No whitespace, sorted keys, RFC 8785
    │
    ▼
Binary State Map (BSM)   ←  Big-endian, lexicographically ordered
    │
    ▼
SHA-256 Digest           ←  Self-digest + chain digest
    │
    ▼
Append-Only Event Log    ←  OS-locked, seq-tracked, immutable
    │
    ▼
Deterministic Replay     ←  Identical input → identical output
```

---

## Guarantees (Protocol v1.0.0 — Frozen)

The following guarantees are normative and cannot be broken without a major version bump:

- **Determinism**: Identical ordered event logs produce byte-for-byte identical state on any conforming implementation.
- **Auditability**: Every state transition is recorded in an immutable, cryptographically chained log.
- **Isolation**: Tenant namespaces are strictly partitioned; cross-tenant access is a protocol violation.
- **Portability**: No platform-specific encoding ambiguity; any language can implement a conforming client.
- **Replay Safety**: Any node can reconstruct current state from genesis without external coordination.

## Open-Core Conformance Clarity

TRI-SYNC's open core is intentionally transparent:

- the wire format is frozen at protocol **v1.0.0**
- conformance expectations are documented in [cross-language-determinism.md](cross-language-determinism.md)
- independent implementations can validate identical bytes, identical replay behavior,
  and identical root digests

This makes TRI-SYNC a mathematically verifiable utility rather than a black-box hosted service.

## Access Model

The public core is available without a key: foundational event logging, `tri-sync verify`, `tri-sync replay`, and local single-tenant execution. Enterprise authorization through `TRISYNC_LICENSE_KEY` is required for commercial production mode, automated compliance export reporting, and scaled multi-tenant enterprise deployment. For core enterprise positioning, TRI-SYNC is aligned to Agentic SOC 2 Type II Processing Integrity, internal Model Risk Management workflows, and cryptographically verified audit trails. It does not itself constitute legal advice or a certification.

---

## Getting Started

See [README.md](../README.md) for quickstart instructions, CLI reference, and licensing.

See [docs/differentiation.md](differentiation.md) for cloud-native logging differentiation.

See [docs/licensing.md](licensing.md) for the commercial license flow.

See [docs/cross-language-determinism.md](cross-language-determinism.md) for wire format specification and conformance test vectors.
