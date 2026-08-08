# TRI-SYNC Protocol Specification

## 1. Overview
TRI-SYNC is a deterministic runtime protocol designed to guarantee reproducible state, auditable workflows, and cloud-neutral execution. The protocol defines strict invariants for encoding, state management, event logging, and replay.

## 2. Core Invariants
- **Binary State Map** — All numeric values encoded as F64 big-endian with no padding.
- **Canonical JSON Encoding** — Deterministic numeric formatting; no scientific notation; locale-neutral.
- **UTF-8 Lexicographic Key Ordering** — Ensures stable multi-tenant isolation and reproducible state boundaries.
- **SHA-256 Digests** — Every state transition produces a canonical digest.
- **Append-Only Event Log** — Immutable, replayable, portable.
- **Deterministic Replay** — Identical input produces identical output across machines and environments.
- **No Outbound Mutation** — Lens-only, observe-only runtime mode.

## 3. Message Schema
Each event ingested by TRI-SYNC follows this canonical structure:

