# TRI-SYNC Licensing

TRI-SYNC ships as an open-source core deterministic runtime with optional enterprise-only capabilities unlocked by an offline signed license document. No hosted service, webhook, or online activation step is required.

---

## How Licensing Works

1. **Community mode is the default** — verification, replay, digesting, inspection, status, and local single-tenant workflows run without a license.
2. **Enterprise mode is offline** — restricted capabilities such as commercial production execution and automated compliance reporting unlock only when TRI-SYNC verifies a signed local license document.
3. **Verification is local** — `src/license.rs` verifies the license signature with an embedded Ed25519 public key.

If no license is present, TRI-SYNC continues in community mode. If an enterprise feature is requested without a valid license, the CLI prints a structured error and exits before modifying state.

---

## Activation Steps

### Step 1 — Obtain a signed license document

Enterprise customers receive a signed JSON license document containing:

- `license_version`
- `license_id`
- `holder`
- `tier`
- `features`
- `issued_at`
- optional `expires_at`
- `signature`

### Step 2 — Install TRI-SYNC

```bash
git clone https://github.com/IknowwhoIamHAA/TRI-SYNC
cd TRI-SYNC
cargo build --release
```

### Step 3 — Supply the license locally

Use either the environment variable:

```bash
export TRISYNC_LICENSE='{"license_version":1,"license_id":"lic_example","holder":"Example Corp","tier":"enterprise","features":["commercial-production","compliance-reporting"],"issued_at":1757913600,"expires_at":null,"signature":"<hex-ed25519-signature>"}'
```

Or a local file:

- `$TRISYNC_LICENSE_FILE`
- `$HOME/.trisync/license.json`
- `./trisync-license.json`

### Step 4 — Run enterprise-only commands offline

```bash
tri-sync apply --log events.jsonl --namespace tenant-a --key job-status --value ready --production
tri-sync report --log events.jsonl
```

---

## Lookup Order

TRI-SYNC checks license sources in this order:

| Priority | Source |
|---|---|
| 1 | `$TRISYNC_LICENSE` |
| 2 | File path in `$TRISYNC_LICENSE_FILE` |
| 3 | `$HOME/.trisync/license.json` |
| 4 | `./trisync-license.json` |

The first present document is parsed as JSON and verified locally against TRI-SYNC's embedded Ed25519 public key.

---

## Feature Access

| Feature | Community mode | Enterprise license |
|---|---|---|
| SHA-256 digest, `verify`, `replay`, `inspect`, `status` | Yes | Yes |
| Local single-tenant execution | Yes | Yes |
| Commercial production execution (`apply` or `delete` with `--production`) | No | Yes |
| Automated compliance report (`tri-sync report`) | No | Yes |

TRI-SYNC's immutable provenance, SHA-256 digest chain, and independent verification support audit workflows. They do not by themselves certify compliance with any law or regulation.

---

## FAQ

**Q: Do I need internet access to activate TRI-SYNC?**  
A: No. Activation is fully offline and uses a signed JSON license document verified locally.

**Q: What happens if I do not provide a license?**  
A: TRI-SYNC stays in community mode and continues to support its open-core verification and replay workflows.

**Q: What happens if the license is invalid or expired?**  
A: Community-mode features still work. Enterprise-only commands fail fast with a structured license error.

**Q: Can I use a file instead of an environment variable?**  
A: Yes. Put the signed JSON document at `~/.trisync/license.json`, `./trisync-license.json`, or point `TRISYNC_LICENSE_FILE` at any local path.

**Q: Can I run TRI-SYNC in a container or air-gapped environment?**  
A: Yes. Mount the signed JSON file locally or inject the same document through `TRISYNC_LICENSE`.
