# Changelog

All notable changes to this project will be documented in this file.

## [1.0.0] - 2026-08-25

### Added
- Public API freeze declaration for TRI-SYNC Runtime v1.0.0.
- Stable protocol reference specification in `SPEC.md`.

### Guarantees
- Deterministic replay from identical ordered event logs.
- Append-only event chaining with SHA-256 digest verification.
- Canonical JSON numeric encoding and deterministic key ordering.
- Multi-tenant namespace isolation via namespace-prefixed keys.

### Compatibility
- `v1.0.0` is the frozen public API baseline.
- Future changes must remain backward-compatible with v1.0.0.
