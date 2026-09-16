# **TRI‑SYNC**
### *The compliance-first deterministic runtime for auditable AI and regulated workflows.*

TRI‑SYNC is the definitive standalone Rust runtime for reproducible state, immutable provenance, tamper-evident SHA-256 digest logs, and independent audit verification. It is self-contained, offline-first, and runs with zero server requirements.

> **v1.3.0 — Protocol frozen. Production-ready.**  
> The wire format is stable. Any two conforming implementations produce byte-for-byte identical state.
>
> **Release status:** This is the authoritative zero-infrastructure production release of TRI-SYNC. Pre-1.0 experimental and serverless iterations are retired and should not be used for new deployments.

---

## Quick Start

### 1 — Build from source

**Build from source (requires Rust 1.85+):**
```bash
git clone https://github.com/IknowwhoIamHAA/TRI-SYNC
cd TRI-SYNC
cargo build --release
# Binary: target/release/tri-sync
```

### 2 — Use the community core

The open community core includes deterministic logging, digest generation, verification, replay, inspect, and status workflows without any license or network dependency.

```bash
./target/release/tri-sync digest --input "hello"
# 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
```

### 3 — Unlock enterprise features offline

Enterprise-only capabilities such as `--production` execution and compliance reporting are unlocked by a signed offline license document supplied through `TRISYNC_LICENSE` or a local license file.

```bash
export TRISYNC_LICENSE='{"license_version":1,"license_id":"lic_example","holder":"Example Corp","tier":"enterprise","features":["commercial-production","compliance-reporting"],"issued_at":1757913600,"expires_at":null,"signature":"<hex-ed25519-signature>"}'
```

You can also store the same JSON document at `~/.trisync/license.json`, `./trisync-license.json`, or point `TRISYNC_LICENSE_FILE` to a different path.

---

## CLI Reference

`verify`, `replay`, `digest`, `inspect`, `status`, and local single-tenant workflows run in community mode. Enterprise production mode and automated compliance reporting require a valid offline `TRISYNC_LICENSE` document.

```bash
# Write a value to the append-only log
# Local single-tenant execution is free. Add --production for licensed commercial use.
tri-sync apply \
  --log events.jsonl \
  --namespace tenant-a \
  --key job-status \
  --value "running" \
  --tick 1

# Delete a key
tri-sync delete \
  --log events.jsonl \
  --namespace tenant-a \
  --key job-status \
  --tick 2

# Verify the log and print the final root digest
tri-sync verify --log events.jsonl

# Verify against a trusted prior TICK_SEAL checkpoint root
tri-sync verify --log events.jsonl \
  --checkpoint-root 768e154f...

# Replay the log and print final state as canonical JSON
tri-sync replay --log events.jsonl

# Compute SHA-256 of any input
tri-sync digest --input "hello world"

# Write and replay a complete example workflow
tri-sync example --log /tmp/example.jsonl

# Generate a licensed automated compliance report
tri-sync report --log events.jsonl
```

`--tick` (default `0`) sets the logical tick number on the event. Ticks must be monotonically non-decreasing within a namespace.

### `verify` output

```
OK
log=events.jsonl
events=3
root_digest=768e154f...
```

Exit code `0` means the log is valid.

Protocol violations are emitted to `stderr` as JSON with distinct exit codes:

| Exit code | Category | Examples |
|---|---|---|
| `4` | Sequence / digest / format | `SequenceGap`, `DigestMismatch`, `InvalidEventFormat` |
| `5` | Namespace isolation | `NamespaceBreach` |
| `6` | Checkpoint / replayed state | `StateMismatch`, `MissingTickSeal` |

---

## Licensing

**TRI-SYNC's open-source core is offline-first and always available. Enterprise-only capabilities are unlocked by a signed license document verified locally with an embedded public key.**

| Step | Action |
|---|---|
| 1 | Use `tri-sync verify`, `tri-sync replay`, `tri-sync digest`, and local single-tenant workflows for free |
| 2 | Obtain a signed TRI-SYNC license document for enterprise capabilities |
| 3 | Supply the document through `TRISYNC_LICENSE` or a local `license.json` file |
| 4 | Run `tri-sync --production ...` or `tri-sync report ...` completely offline |

If no license is supplied, TRI-SYNC stays in community mode. If an enterprise feature is requested without a valid license, `tri-sync` prints a clear error and exits before modifying state.

**License document lookup order** (checked in order):
1. `$TRISYNC_LICENSE`
2. Path in `$TRISYNC_LICENSE_FILE`
3. `$HOME/.trisync/license.json`
4. `./trisync-license.json`

For containers or air-gapped systems, mount the same signed JSON document locally or inject it directly through `TRISYNC_LICENSE`.

**Full licensing details:** [docs/licensing.md](docs/licensing.md)  
**Commercial terms:** [COMMERCIAL_LICENSE.md](COMMERCIAL_LICENSE.md)  
**Repository license notice:** [LICENSE](LICENSE)

### Historical status

- `v1.0.0` is the frozen public protocol baseline.
- Pre-1.0 experimental/serverless repository states are retired and preserved only as historical development milestones.
- The current maintained architecture is the standalone Rust runtime with local append-only storage and offline Ed25519 license verification.

---

## Download Binary

Pre-built static binaries are published with each release:

| Platform | Download |
|---|---|
| Linux x86-64 | `tri-sync-linux-x86_64` |
| Linux ARM64 | `tri-sync-linux-aarch64` |
| macOS x86-64 | `tri-sync-darwin-x86_64` |
| macOS ARM64 (M-series) | `tri-sync-darwin-aarch64` |

All binaries are statically linked (no libc dependency on Linux when built with `musl`).

**Build a static Linux binary:**
```bash
# Install musl target
rustup target add x86_64-unknown-linux-musl

# Build
cargo build --release --target x86_64-unknown-linux-musl
# Binary: target/x86_64-unknown-linux-musl/release/tri-sync
```

---

## Use as a Library

TRI-SYNC is also usable as a Rust library for embedding deterministic state into your own applications.

```toml
# Cargo.toml
[dependencies]
tri-sync = { git = "https://github.com/IknowwhoIamHAA/TRI-SYNC", tag = "v1.3.0" }
```

```rust
use tri_sync::event::{Event, ZERO_DIGEST_HEX};
use tri_sync::event_log::AppendOnlyEventLog;
use tri_sync::replay::ReplayEngine;
use tri_sync::state_map::BsmValue;

let log = AppendOnlyEventLog::open("events.jsonl");
let event = Event::state_write(
    0, 0, "tenant-a", "tenant-a:counter",
    BsmValue::Integer(42), false, ZERO_DIGEST_HEX, None,
)?;
log.append(&event)?;

let state = ReplayEngine::replay(&log.load()?)?;
println!("root_digest = {}", state.root_digest_hex()?);
```

### Custom storage backends

`tri_sync::backend::EventLogBackend` isolates replay and verification logic from storage concerns. TRI-SYNC includes:

- `FileSystemBackend` — wraps the current append-only JSONL file log
- `InMemoryBackend` — lightweight backend for tests and rapid iteration

Custom backends only need to implement:

```rust
use tri_sync::backend::EventLogBackend;
use tri_sync::error::ProtocolViolationError;
use tri_sync::event::Event;

struct CustomBackend;

impl EventLogBackend for CustomBackend {
    fn append(&self, _event: &Event) -> Result<(), ProtocolViolationError> { Ok(()) }
    fn load(&self) -> Result<Vec<Event>, ProtocolViolationError> { Ok(Vec::new()) }
    fn next_sequence(&self) -> Result<u64, ProtocolViolationError> { Ok(0) }
    fn lock_for_write(&self) -> Result<(), ProtocolViolationError> { Ok(()) }
}
```

Batch ingestion is available through `append_batch(&[Event])`. For the filesystem backend, TRI-SYNC holds the lock once, streams all batch events, flushes segment lines, and then persists catalog metadata at the end of the batch.

Checkpoint verification persists trusted `TICK_SEAL` snapshots at append time in `<log>.snapshots/`. During `verify --checkpoint-root <digest>`, TRI-SYNC attempts to load the cached snapshot first and falls back to replaying from genesis through the checkpoint when no cache is available.

---

## Key Features

| Feature | Description |
|---|---|
| **Deterministic replay** | Identical ordered logs → identical state, any machine, any time |
| **SHA-256 digest chain** | Every event is self-hashed and chained; tampering is instantly detectable |
| **Canonical JSON** | Deterministic canonical JSON with UTF-8 byte-order keys and no locale drift |
| **Binary state map** | Big-endian, lexicographically ordered; root digest proves complete state |
| **TICK_SEAL checkpoints** | Root digest snapshots after every logical tick for independent verification |
| **Multi-tenant isolation** | Namespace-prefixed keys; cross-tenant access is a protocol violation |
| **File locking** | Concurrent appends are safe via OS-level exclusive locks |
| **Transactional writes** | `TransactionalStateMap` for atomic multi-key batch mutations |
| **Offline licensing** | Enterprise capabilities are unlocked locally with Ed25519-signed JSON licenses |
| **Protocol frozen** | v1.0.0 wire format will not change; future versions are additive only |

---

## Commercial Use

TRI-SYNC is purpose-built for regulated and high-assurance environments:

- **Finance** — Agentic SOC 2 Type II Processing Integrity evidence, internal model risk management, auditable order books
- **Healthcare** — cryptographically verified clinical decision trails and controlled automation audit logs
- **Insurance** — deterministic claims processing, reproducible underwriting
- **AI Platforms** — reproducible inference logs, multi-agent coordination

Frontier-scale AI risk tracking is a separate segment for elite labs operating under specialized governance frameworks; general enterprise positioning remains centered on processing integrity, internal MRM, and portable auditability.

**Learn more:** [docs/product.md](docs/product.md)

---

## Documentation

| Document | Description |
|---|---|
| [SPEC.md](SPEC.md) | Full normative protocol specification |
| [docs/product.md](docs/product.md) | Product overview, use cases, guarantees |
| [docs/differentiation.md](docs/differentiation.md) | TRI-SYNC vs CloudTrail, Object Lock, and vendor-native integrity features |
| [docs/licensing.md](docs/licensing.md) | Offline license format, activation flow, FAQ |
| [docs/cross-language-determinism.md](docs/cross-language-determinism.md) | Wire format, test vectors, conformance checklist |
| [invariants.md](invariants.md) | All protocol invariants |
| [architecture.md](architecture.md) | Runtime layer architecture |
| [CHANGELOG.md](CHANGELOG.md) | Release history |
| [COMMERCIAL_LICENSE.md](COMMERCIAL_LICENSE.md) | Commercial license terms |

---

## Project Status

**v1.3.0 — Protocol frozen. Production-ready.**

- ✅ Wire format frozen — no breaking changes after v1.0.0
- ✅ Rust test suite expanded for checkpoint replay, backends, and CLI compliance errors
- ✅ CodeQL: 0 security alerts
- ✅ No TODOs or FIXMEs in protocol-critical code
- ✅ Cross-language determinism test vector pinned: `768e154f…`
- ✅ `verify` subcommand — replay-based audit tool, exits 1 on any protocol violation
- ✅ Zero-overhead offline licensing — no cloud or serverless control plane required

---

## License

TRI-SYNC provides an open-source core engine with optional commercial licensing for enterprise-only capabilities. See [COMMERCIAL_LICENSE.md](COMMERCIAL_LICENSE.md) for terms and [docs/licensing.md](docs/licensing.md) for offline activation details.
