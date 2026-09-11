# TRI-SYNC Differentiation

## Why TRI-SYNC Is Not Just CloudTrail or Object Lock

TRI-SYNC solves a different problem than cloud-native integrity features such as
**AWS CloudTrail Log File Integrity Validation** or **S3 Object Lock**.

Those services are useful within their own layer:

- **CloudTrail** records AWS control-plane and service API activity.
- **S3 Object Lock** prevents deletion or mutation of stored objects for a
  retention period.

TRI-SYNC operates at the **application protocol layer** instead:

- it records **semantic business/application events**, not just infrastructure calls
- it reconstructs **deterministic state** from those events
- it seals each logical checkpoint with **`TICK_SEAL` root digests**
- it verifies replay correctness with a **portable, open wire format**
- it can run **locally, on bare metal, or across clouds** without tying auditability
  to a single vendor

---

## Layer Comparison

| Capability | AWS CloudTrail Integrity / S3 Object Lock | TRI-SYNC |
|---|---|---|
| Primary scope | AWS infrastructure activity and object retention | Application-level state transitions and audit semantics |
| Vendor portability | AWS-only | Cloud-neutral and local-first |
| Replayable state | No deterministic state engine | Yes — deterministic replay with verifiable root digests |
| Checkpoint model | Service-specific integrity/retention controls | `TICK_SEAL` checkpoints over the full state map |
| Multi-cloud / bare metal | No | Yes |
| Open-core conformance | No public cross-language protocol utility | Yes — transparent wire format and conformance vectors |

---

## CloudTrail vs TRI-SYNC

CloudTrail is designed to answer questions like:

- Which IAM principal called an AWS API?
- When was a bucket policy changed?
- Which infrastructure action happened in an account?

TRI-SYNC is designed to answer questions like:

- Which application decision changed a regulated record?
- Which sequence of model or agent actions produced the current state?
- Can another implementation replay the same event log and get the exact same root digest?

CloudTrail helps prove **infrastructure activity** inside AWS.
TRI-SYNC helps prove **application behavior and state evolution** anywhere it runs.

---

## Object Lock vs TRI-SYNC

S3 Object Lock helps preserve bytes once written. It does **not** define:

- a semantic event model
- deterministic replay rules
- tenant namespace isolation rules
- checkpoint digests over application state
- cross-language conformance behavior

TRI-SYNC provides those protocol-level guarantees directly. Object retention can
complement TRI-SYNC, but it does not replace a deterministic audit runtime.

---

## Enterprise Implication

For enterprise teams, the distinction matters:

- use provider-native controls when you need platform retention or account-level API logging
- use TRI-SYNC when you need **portable, cryptographically verifiable,
  application-level audit trails**
- combine both when infrastructure evidence and application evidence must align

TRI-SYNC is therefore positioned as a **zero-trust audit and deterministic-state
utility**, not as a wrapper around a single cloud provider's logging stack.

---

## Related Documents

- [README.md](../README.md)
- [docs/product.md](product.md)
- [docs/cross-language-determinism.md](cross-language-determinism.md)
- [docs/protocol.md](protocol.md)
