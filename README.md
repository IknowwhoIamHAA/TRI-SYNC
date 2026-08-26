# **TRI‑SYNC**
### *The deterministic AI auditability runtime. Protocol-frozen. Production-ready.*

TRI‑SYNC is a portable, commercially-licensed Rust runtime that guarantees reproducible state, cryptographic audit trails, and cloud-neutral execution for AI systems, regulated data pipelines, and multi-tenant workflows. Every state transition is hashed, chained, and replayable — forever.

> **v1.0.0 — Protocol frozen. Production-ready.**  
> The wire format is stable. Any two conforming implementations produce byte-for-byte identical state.

---

## Quick Start

### 1 — Obtain a License Key

TRI-SYNC requires a commercial license for production use.

```
https://github.com/IknowwhoIamHAA/TRI-SYNC
```

Once you have your key:

```bash
export TRISYNC_LICENSE_KEY=TRISYNC-XXXX-XXXX-XXXX
```

### 2 — Download or Build

**Download pre-built binary (Linux x86-64):**
```bash
curl -L https://github.com/IknowwhoIamHAA/TRI-SYNC/releases/latest/download/tri-sync-linux-x86_64 \
     -o tri-sync && chmod +x tri-sync
```

**Build from source (requires Rust 1.85+):**
```bash
git clone https://github.com/IknowwhoIamHAA/TRI-SYNC
cd TRI-SYNC
cargo build --release
# Binary: target/release/tri-sync
```

### 3 — Verify

```bash
export TRISYNC_LICENSE_KEY=TRISYNC-XXXX-XXXX-XXXX
./tri-sync digest --input "hello"
# ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
```

---

## CLI Reference

All commands require a valid `TRISYNC_LICENSE_KEY` environment variable.

```bash
# Write a value to the append-only log
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

# Replay the log and print final state as canonical JSON
tri-sync replay --log events.jsonl

# Compute SHA-256 of any input
tri-sync digest --input "hello world"

# Write and replay a complete example workflow
tri-sync example --log /tmp/example.jsonl
```

`--tick` (default `0`) sets the logical tick number on the event. Ticks must be monotonically non-decreasing within a namespace.

---

## Licensing

**TRI-SYNC requires a commercial license for production use.**

| Step | Action |
|---|---|
| 1 | Pay → receive license key |
| 2 | `export TRISYNC_LICENSE_KEY=<your-key>` |
| 3 | Run `tri-sync <command>` |

If the key is missing or invalid, `tri-sync` prints a clear error and exits. No state is modified.

**Key store locations** (checked in order):
1. `$TRISYNC_LICENSE_KEYS_FILE`
2. `$HOME/.trisync/license_keys`
3. `/etc/trisync/license_keys`

For Docker/Kubernetes: inject `TRISYNC_LICENSE_KEY` as a secret environment variable.

**Full licensing details:** [docs/licensing.md](docs/licensing.md)  
**Commercial terms:** [COMMERCIAL_LICENSE.md](COMMERCIAL_LICENSE.md)

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
tri-sync = { git = "https://github.com/IknowwhoIamHAA/TRI-SYNC", tag = "v1.0.0" }
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

Library use requires a commercial license. See [docs/licensing.md](docs/licensing.md).

---

## Key Features

| Feature | Description |
|---|---|
| **Deterministic replay** | Identical ordered logs → identical state, any machine, any time |
| **SHA-256 digest chain** | Every event is self-hashed and chained; tampering is instantly detectable |
| **Canonical JSON** | RFC 8785 encoding — no locale drift, no ambiguity, no surprises |
| **Binary state map** | Big-endian, lexicographically ordered; root digest proves complete state |
| **TICK_SEAL checkpoints** | Root digest snapshots after every logical tick for independent verification |
| **Multi-tenant isolation** | Namespace-prefixed keys; cross-tenant access is a protocol violation |
| **File locking** | Concurrent appends are safe via OS-level exclusive locks |
| **Transactional writes** | `TransactionalStateMap` for atomic multi-key batch mutations |
| **Protocol frozen** | v1.0.0 wire format will not change; future versions are additive only |

---

## Commercial Use

TRI-SYNC is purpose-built for regulated and high-assurance environments:

- **Finance** — auditable order books, settlement reconciliation, regulatory reporting
- **Healthcare** — HIPAA-compliant AI audit logs, clinical decision trails
- **Insurance** — deterministic claims processing, reproducible underwriting
- **Government** — tamper-proof record systems, verifiable processing pipelines
- **AI Platforms** — reproducible inference logs, multi-agent coordination

**Learn more:** [docs/product.md](docs/product.md)

---

## Documentation

| Document | Description |
|---|---|
| [SPEC.md](SPEC.md) | Full normative protocol specification |
| [docs/product.md](docs/product.md) | Product overview, use cases, guarantees |
| [docs/licensing.md](docs/licensing.md) | Licensing flow, tiers, FAQ |
| [docs/cross-language-determinism.md](docs/cross-language-determinism.md) | Wire format, test vectors, conformance checklist |
| [invariants.md](invariants.md) | All protocol invariants |
| [architecture.md](architecture.md) | Runtime layer architecture |
| [CHANGELOG.md](CHANGELOG.md) | Release history |
| [COMMERCIAL_LICENSE.md](COMMERCIAL_LICENSE.md) | Commercial license terms |

---

## Project Status

**v1.0.0 — Protocol frozen. Production-ready.**

- ✅ Wire format frozen — no breaking changes after v1.0.0
- ✅ 63 tests pass
- ✅ CodeQL: 0 security alerts
- ✅ No TODOs or FIXMEs in protocol-critical code
- ✅ Cross-language determinism test vector pinned: `768e154f…`

---

## License

TRI-SYNC is commercially licensed. See [COMMERCIAL_LICENSE.md](COMMERCIAL_LICENSE.md) for full terms.

For licensing inquiries: https://github.com/IknowwhoIamHAA/TRI-SYNC
