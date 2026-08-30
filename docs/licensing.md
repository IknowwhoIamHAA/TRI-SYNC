# TRI-SYNC Licensing

TRI-SYNC is commercially licensed software. Use in production requires a valid license key.

---

## How Licensing Works

TRI-SYNC uses a **key-based activation model**:

1. **Pay** — purchase a commercial license (see below).
2. **Receive key** — you receive a license key string (e.g., `TRI-XXXXXXXX-XXXXXXXX-XXXXXXXX`).
3. **Set environment variable** — export your key before running any `tri-sync` command.
4. **Use TRI-SYNC** — run normally in production.

If the key is missing, empty, or invalid, `tri-sync` prints a clear error and exits immediately. No protocol state is modified.

---

## Activation Steps

### Step 1 — Obtain a License Key

**Purchase a 1-month license key ($29/month):**

```
https://buy.stripe.com/eVq3cxalw3RbgRL4FCfEk05
```

After completing the purchase, you will receive a license key by email.

### Step 2 — Install TRI-SYNC

**Download pre-built binary (Linux x86-64):**
```bash
curl -L https://github.com/IknowwhoIamHAA/TRI-SYNC/releases/latest/download/tri-sync-linux-x86_64 \
     -o tri-sync && chmod +x tri-sync
```

**Build from source:**
```bash
git clone https://github.com/IknowwhoIamHAA/TRI-SYNC
cd TRI-SYNC
cargo build --release
# Binary is at: target/release/tri-sync
```

### Step 3 — Set Your License Key

```bash
export TRISYNC_LICENSE_KEY=TRI-XXXXXXXX-XXXXXXXX-XXXXXXXX
```

For persistent activation, add the export to your shell profile (`~/.bashrc`, `~/.zshrc`) or your service's environment configuration.

### Step 4 — Verify Activation

```bash
tri-sync digest --input "hello"
```

If your key is valid, you will see a SHA-256 digest. If the key is invalid, you will see a clear error message describing the problem and how to resolve it.

---

## License Key Store

TRI-SYNC resolves the valid-keys file in this order:

| Priority | Path |
|---|---|
| 1 | `$TRISYNC_LICENSE_KEYS_FILE` (if set) |
| 2 | `$HOME/.trisync/license_keys` |
| 3 | `/etc/trisync/license_keys` (Linux/macOS system-wide) |

The key-store file is a plain text file with one key per line. Lines beginning with `#` are comments and are ignored.

**Example `~/.trisync/license_keys`:**
```
# TRI-SYNC license keys — do not share this file
TRI-XXXXXXXX-XXXXXXXX-XXXXXXXX
```

For multi-node or containerized deployments, the recommended approach is to inject `TRISYNC_LICENSE_KEY` as a secret environment variable via your secret manager (Kubernetes Secrets, AWS Secrets Manager, HashiCorp Vault, etc.).

---

## License Tiers

| Tier | Use Case |
|---|---|
| **Developer** | Single developer, non-production use, evaluation |
| **Team** | Up to 10 developers, internal tooling, staging environments |
| **Enterprise** | Unlimited developers, production deployments, SLA support |
| **OEM** | Redistribution rights, embedded use in third-party products |

Contact [the TRI-SYNC team](https://buy.stripe.com/eVq3cxalw3RbgRL4FCfEk05) to discuss pricing and terms for your use case.

---

## Commercial License Summary

TRI-SYNC is provided under a **Commercial License**. Key terms:

- Production use requires a paid commercial license.
- The source code is available for evaluation and review.
- You may not redistribute TRI-SYNC or build products based on TRI-SYNC without an OEM license.
- The protocol specification (SPEC.md) is provided for interoperability purposes.

Full terms are in [COMMERCIAL_LICENSE.md](../COMMERCIAL_LICENSE.md).

---

## FAQ

**Q: Can I evaluate TRI-SYNC without a license key?**  
A: You can build and run TRI-SYNC in a development environment with a Developer license key. Contact us for a free evaluation key.

**Q: Does the license key expire?**  
A: License keys may have expiry dates depending on your tier. Annual licenses are renewed each year; perpetual licenses do not expire.

**Q: Is TRI-SYNC open source?**  
A: The source code is available for review and evaluation. Production use requires a commercial license. The wire protocol is open (SPEC.md is public) to allow interoperability with other conforming implementations.

**Q: Can I run TRI-SYNC in a Docker container?**  
A: Yes. Pass the license key as an environment variable: `docker run -e TRISYNC_LICENSE_KEY=... your-image`.

**Q: What happens if my license expires during a running process?**  
A: The license is checked only at startup. A running process is not interrupted by key expiry. The key is re-checked on the next invocation.
