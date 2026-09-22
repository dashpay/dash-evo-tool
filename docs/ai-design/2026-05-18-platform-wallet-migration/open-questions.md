# Open Questions

**Purpose:** Record of the eight decisions needed before implementation. All eight are now RESOLVED.

[← back to README](README.md)

---

All 8 decisions resolved. Implementation is unblocked. See [phasing.md § Combined Gate Posture](phasing.md#combined-gate-posture) for the updated gate state.

---

## Decision #1 — G1 Timing

**RESOLVED: Pin to PR branch now.**

DET pins its platform dep to dashpay/platform PR #3625 head and starts P0–P2 immediately, accepting rework if the PR changes before merge. G1 is reclassified from a start blocker to a release-hardening item: track #3625 until it merges, then bump the pin to a released rev before shipping.

~~Options (a) wait / (b) pin~~: option (b) confirmed for all phases; option (a) applies only at release time.

---

## Decision #2 — G2 Seed-Re-Registration UX

**RESOLVED: Mock it via `PersistedWalletLoader` seam.**

The `PersistedWalletLoader` DET-internal trait provides the seam. `SeedReregistrationLoader` (ships now) performs seed-re-registration exactly as upstream prescribes — behavior is identical to today. `UpstreamFromPersisted` (ships when `Wallet::from_persisted` lands) is a one-line construction swap. G2 is downgraded from a hard gate to a deferred swap-in.

Full design: [g2-mock-boundary.md](g2-mock-boundary.md).

---

## Decision #3 — ZMQ Listener

**RESOLVED: Audit before P4; delete only if no non-wallet consumer.**

`components/core_zmq_listener` usage audit runs before P4. If no non-wallet consumer is found, the listener is deleted in P4. If a non-wallet consumer exists (e.g. ChainLock notifications for UI), it is retained with scope trimmed to that consumer. No decision possible before the audit; the audit is a P4 precondition. See [removal-inventory.md § RETAIN](removal-inventory.md#retain).

---

## Decision #4 — Devnet Identity Discovery

**RESOLVED: Keep DET path permanently.**

`discover_identities.rs` Devnet branch stays DET-owned indefinitely. Upstream has no Devnet timeline. The branch is on `network`, not `CoreBackendMode`, so it coexists cleanly with the new backend.

---

## Decision #5 — DashPay Scope Boundary

**RESOLVED: Hybrid split confirmed.**

| Owner | Owns |
|---|---|
| Upstream (`IdentityWallet<B>`, derivation functions) | Contact-request/established-contact state, DashPay profile, derivation crypto |
| DET (unchanged) | Avatar I/O (`avatar_processing.rs`), auto-accept proof, incoming-payment detection, payment-history cache |

See [backendtask-contract.md § DashPayTask](backendtask-contract.md) for the task-level mapping.

---

## Decision #6 — DIP-14/15 Parity Policy

**RESOLVED 2026-05-18 — drop backwards compatibility; upstream-only DashPay derivation. The migrate-or-quarantine/hard-stop apparatus is WITHDRAWN. Accepted fund-accessibility trade-off recorded in data-model-and-migration.md with a mandatory one-time user notice. Supersedes the prior 'migrate-or-hard-stop' resolution and dip14-migration-hardstop.md.**

The soft fallback ("keep DET derivation for existing contacts, use upstream for new") is WITHDRAWN. Dual-derivation coexistence is not permitted.

The per-contact migrate-or-quarantine apparatus (previously documented in [dip14-migration-hardstop.md](dip14-migration-hardstop.md)) is WITHDRAWN. DashPay contacts are re-established on upstream derivation unconditionally. The fund-accessibility trade-off this introduces is accepted and documented in [data-model-and-migration.md](data-model-and-migration.md) — "Accepted fund-accessibility trade-off (user decision, 2026-05-18)". The mandatory one-time informational notice to migrated users is the sole compensating control.

Historical quarantine design: see [dip14-migration-hardstop.md](dip14-migration-hardstop.md) (retained as historical record only).

---

## Decision #7 — Single-Key Timeline

**RESOLVED: Ship mock now, swap later (confirmed).**

Single-key wallets ship as read-only + clear message for at least one release. Data preserved. `SingleKeyBackend` trait boundary makes the future swap a one-line change. See [single-key-mock.md](single-key-mock.md).

---

## Decision #8 — One-Release No-Op Grace for Removed Tasks

**RESOLVED: Hard-remove immediately.**

`CoreTask::RecoverAssetLocks` and `CoreTask::ListCoreWallets` and their UI entry points are deleted in the same release — no one-release no-op grace. `AssetLockManager` continuous tracking makes explicit recovery obsolete; named Core wallets have no meaning without RPC mode. See [backendtask-contract.md](backendtask-contract.md) (updated rows) and [removal-inventory.md](removal-inventory.md).
