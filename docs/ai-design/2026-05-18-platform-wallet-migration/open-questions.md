# Open Questions

**Purpose:** Decisions still needed from the user before or during implementation. Each decision is actionable; the architect's recommendation is called out explicitly.

[← back to README](README.md)

---

These are not design gaps — the architect has a recommendation for each. They are user-authority decisions: scope commitments, UX policy, and migration strategy that the team lead must confirm before the relevant phase starts.

## Decision #4 — `DiskStorageManager` Rebuild UX

**Needed before:** Phase 3 (affects the upgrade path from Phase 2 to Phase 3)

**Context:** The [verification.md § E.2](verification.md#e2--diskstoragemanager-byte-compat) probe may find that `DiskStorageManager`'s on-disk cache is not byte-compatible between `WalletManager<ManagedWalletInfo>` and `WalletManager<PlatformWalletInfo>`. If so, a strategy is needed for existing users upgrading.

**Options:**

| Option | Description | Tradeoffs |
|---|---|---|
| A — Silent re-sync + info banner (recommended) | Detect version-marker mismatch on first launch; call `SpvManager::clear_data_dir()` (`src/spv/manager.rs:800`); display: "Updating wallet data for the new version. This may take a few minutes." | No user decision required; data dir is a cache (not authoritative); consistent with existing "SPV sync in progress" UX |
| B — Explicit user prompt | Show a dialog explaining the cache must be rebuilt | Exposes internal cache concept to users; jargon; outcome is the same regardless of user response |

**Architect recommendation:** Option A — silent re-sync. The data directory is a cache; wallet truth is the encrypted seed + SQLite. Requiring user confirmation for an internal cache rebuild is self-resolving jargon. Do not wipe without the version-marker check to avoid gratuitous re-sync every launch (A04 fail-safe).

**Confirmation needed:** Approve Option A, or specify Option B / a variation.

---

## Decision #5 — DashPay Scope Boundary

**Needed before:** Phase 3 design is frozen

**Context:** The upstream persister owns contact-requests, established-contacts, DashPay profile, and payment history. A "replace all DashPay" reading of the migration would pull avatar processing, auto-accept logic, incoming-payment UI, and profile UI into scope — significantly ballooning effort and risk.

**Proposed hybrid split:**

| Owner | Owns |
|---|---|
| `platform-wallet-storage` persister | Contact requests, established contacts, DashPay profile, payment history |
| dash-evo-tool (unchanged) | Avatar processing, auto-accept, incoming-payment UI, profile UI (`src/backend_task/dashpay/{avatar_processing,auto_accept_*,incoming_payments,profile}.rs`) |

**Architect recommendation:** Confirm the hybrid. Moving the DET-owned items above is out of scope for this migration; they are not addressed by the upstream persister's trait surface.

**Confirmation needed:** Approve the hybrid split as stated, or identify any specific items to re-scope.

---

## Decision #6 — `QualifiedIdentity` Longevity

**Needed before:** Phase 3 (governs dual-write design)

**Context:** The upstream trait doc explicitly defers moving the `QualifiedIdentity` blob to a later milestone ("evo-tool task #130 / Phase 9c"). `QualifiedIdentity` carries DET-only fields (`ManagedIdentity` lacks) — identity status, voter/operator associations, DPNS — so it remains as the UI/display model regardless. The `identity.data` bincode blob in dash-evo-tool's SQLite (`src/database/identities.rs:157`) stays in place through this migration.

**Architect recommendation:** Align with upstream's deferral. Keep the `QualifiedIdentity` blob in DET through this migration; revisit at upstream "Phase 9c". No action needed in Phases 0–4.

**Confirmation needed:** Confirm alignment with upstream deferral, or indicate a different timeline.

---

## Decision #7 — Devnet Fallback Longevity

**Needed before:** Phase 3

**Context:** Upstream has no Devnet timeline. The legacy DET asset-lock identity discovery path for Devnet cannot be removed. Phase 3 will retain two code paths: the upstream `IdentityManager`-based path for Mainnet/Testnet, and the legacy DET path for Devnet. These persist side by side indefinitely until upstream adds Devnet support.

**Architect recommendation:** Accept retaining the legacy Devnet fallback indefinitely. The two paths are branched on `network`, not on `core_backend_mode`, so they compose cleanly without coupling SPV/RPC to Devnet behavior.

**Confirmation needed:** Confirm the two code paths are acceptable indefinitely, or indicate a preferred deprecation trigger (e.g., upstream Devnet support lands).

---

## Decision #3-resid — DIP-14/15 Mismatch Handling Policy

**Needed before:** Phase 4 planning (depends on E.1 probe result from Phase 0)

**Context:** The [E.1 golden-vector probe](verification.md#e1--dip-1415-dashpay-derivation-parity) runs in Phase 0. If it finds a mismatch between dash-evo-tool's hand-rolled derivation and the upstream `key_wallet`-based derivation, Phase 4 deletion is blocked. The question is: what is the policy for handling existing users who have established DashPay contacts with addresses derived by the old path?

| Scenario | What it means |
|---|---|
| E.1 probe green | No mismatch; Phase 4 proceeds normally |
| E.1 probe red | Addresses derived by old and new code differ for some contacts; Phase 4 deletion waits for the migration tool |

**Proposed approach (if red):**

**Phase-4 startup sanity check:** On first boot post-deletion, for each wallet with established DashPay contacts, re-derive the contact payment address via upstream and compare to the persisted address. On mismatch:
- Structured log entry
- Non-blocking `MessageBanner`: "A DashPay contact address could not be re-derived after the upgrade. Your funds are safe; please re-verify contact `Abc123…` before sending."
- Fall back to the persisted address, never the freshly-derived one (A04 fail-safe)
- Never auto-delete; never block startup

**Optional migration tool:** A `det_cli` audit subcommand reporting mismatches across all wallets without mutating state.

**Architect recommendation:** Approve the migration-tool + startup-sanity-check approach as the Phase-4 unblock, accepting that Phase 4 deletion waits until the tool ships if the probe is red.

**Confirmation needed:** Approve this approach, or specify an alternative (e.g., block Phase 4 entirely until upstream alignment; rewrite old addresses in-place).

---

## DIP-14/15 Mismatch Handling Policy

This section consolidates the Phase-0 and Phase-4 handling strategy for completeness. It is referenced from [migration-plan.md Phase 0 and Phase 4](migration-plan.md#phase-table) and [verification.md § E.1](verification.md#e1--dip-1415-dashpay-derivation-parity).

**Phase 0 detection probe:** E.1 golden vectors. Green → proceed to Phase 1. Red → divergence is characterized, not silently shipped; Phase 4 is paused pending migration-tool completion.

**Phase 4 startup sanity check (if probe was red):** As described in Decision #3-resid above — non-blocking banner, fallback to persisted address, never auto-delete.

**Optional migration tool:** `det_cli` audit subcommand; reports mismatches across all wallets; no mutation.

The key invariant: **never silently use a freshly-derived address that differs from the persisted address**. Funds must not be sent to the wrong contact. User funds safety takes priority over code cleanliness.
