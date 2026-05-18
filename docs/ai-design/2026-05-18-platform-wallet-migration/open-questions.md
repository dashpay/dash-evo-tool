# Open Questions

**Purpose:** Eight decisions still needed from the user before implementation begins. Each is actionable; the architect's recommendation or assumption is called out explicitly.

[← back to README](README.md)

---

No implementation starts until all decisions are confirmed (or explicitly deferred with a documented rationale). See [phasing.md](phasing.md) for how each decision gates a specific phase.

---

## Decision #1 — G1 Timing

**Needed before:** P2 (P0–P1 can proceed)

**Context:** PR #3625 is an open draft on `v3.1-dev`. DET's platform pin (`Cargo.toml:21`, `54048b9352…`) predates it. DET cannot reference the persister crate until #3625 merges and a containing platform rev is pinnable.

**Options:**

| Option | Description |
|---|---|
| **(a) Wait for #3625 merge + release** | Safer; spec assumes this for shipping |
| **(b) Temporarily pin to PR branch for P0–P2** | Faster iteration on the spike; acceptable for exploratory work; not for production |

**Architect assumption:** (a) for shipping; (b) tolerated for spike only.

**Confirmation needed:** Confirm (a), or approve (b) for the spike with an understanding that the pin reverts before P3.

---

## Decision #2 — G2 Seed-Re-Registration UX

**Needed before:** P3 migration design is finalized

**Context:** Because `Wallet::from_persisted` does not yet exist upstream (Gate G2), DET must decrypt the seed and re-register each wallet on every launch. Password-protected wallets prompt on launch today — this behavior continues. See [upstream-reality.md § G2 Caveat](upstream-reality.md#g2-caveat--walletfrom_persisted-load-gap).

**Options:**

| Option | Description |
|---|---|
| **(a) Accept seed-re-registration (current behavior)** | No UX regression; prescribed upstream pattern |
| **(b) Wait for upstream `Wallet::from_persisted`** | Defers P3 start until upstream ships the constructor |

**Architect recommendation:** (a) — this is what upstream prescribes and what DET does today.

**Confirmation needed:** Confirm (a) acceptable, or indicate a preference to wait for (b).

---

## Decision #3 — ZMQ Listener

**Needed before:** P4 (deletion pass)

**Context:** `components/core_zmq_listener` feeds non-wallet Core events (e.g. ChainLock notifications, InstantSend events). Once wallet no longer uses Core RPC, it may be droppable — but it may have consumers outside the wallet path. A usage audit is needed before P4 removes it.

**Architect recommendation:** Audit usages before P4; likely droppable; confirm scope.

**Confirmation needed:** Confirm ZMQ listener scope — retained, dropped, or "audit first."

---

## Decision #4 — Devnet Identity Discovery

**Needed before:** P2 (IdentityTask rewire)

**Context:** Upstream `AssetLockManager` and identity discovery cover Mainnet/Testnet only. DET's DAPI-based Devnet path (`discover_identities.rs`) has no upstream equivalent and no upstream timeline.

**Architect recommendation:** Confirm DET-permanent. The Devnet path is isolated in `discover_identities.rs` and branches on `network`, not on `CoreBackendMode`, so it coexists cleanly with the new backend.

**Confirmation needed:** Confirm DET-permanent Devnet path is acceptable indefinitely.

---

## Decision #5 — DashPay Scope Boundary

**Needed before:** P2 (DashPayTask rewire)

**Context:** The upstream export surface includes the full DashPay type set and derivation functions. A "replace all DashPay" reading would pull avatar processing, auto-accept proof, and incoming-payment detection into migration scope — significantly expanding risk and effort.

**Proposed hybrid split:**

| Owner | Owns |
|---|---|
| Upstream (`IdentityWallet<B>`, derivation functions) | Contact-request/established-contact state, DashPay profile, derivation crypto |
| DET (unchanged) | Avatar I/O (`avatar_processing.rs`), auto-accept proof, incoming-payment detection, payment-history cache |

**Architect recommendation:** Confirm the hybrid. The DET-owned items above are not addressed by the upstream persister's trait surface.

**Confirmation needed:** Approve hybrid split as stated, or identify items to re-scope.

---

## Decision #6 — DIP-14/15 Parity Policy

**Needed before:** P4 (deletion of `dip14_derivation.rs` / `hd_derivation.rs`)

**Context:** DET's `index_to_child_number` (`dip14_derivation.rs:213-240`) collapses a 256-bit child index to 31 bits for legacy `ChildNumber` storage. Upstream uses native `ChildNumber::Normal256` (full 256-bit, no lossy collapse). The P0 golden-vector parity probe (DIP-14/15 lane — see [phasing.md QA matrix](phasing.md#qa-matrix)) determines whether the on-curve derived addresses are byte-identical.

**If probe is green:** DET derivation deleted in P4; upstream functions used exclusively.

**If probe is red — proposed fallback:**

Keep DET derivation for existing contacts; use upstream for new contacts. On first boot post-deletion, re-derive each contact payment address and compare to persisted. On mismatch: structured log + non-blocking banner:

> "A DashPay contact address could not be re-derived after the upgrade. Your funds are safe; please re-verify contact `Abc123…` before sending."

Never auto-delete; never use freshly-derived address over persisted; never block startup (A04 fail-safe). Optional `det_cli` audit subcommand reports mismatches without mutation. The key invariant: **never silently use a freshly-derived address that differs from the persisted address.**

**Architect recommendation:** Run the P0 probe; approve the fallback approach now so P4 planning is not gated on the probe result.

**Confirmation needed:** Approve fallback policy, or specify an alternative (e.g. block P4 deletion until upstream alignment; rewrite addresses in-place).

---

## Decision #7 — Single-Key Timeline

**Needed before:** P2 (single-key stub shipped to users)

**Context:** Single-key wallets are mocked: operations return `TaskError::SingleKeyWalletsUnsupported`; existing data is preserved and surfaced read-only with a clear message. The `SingleKeyBackend` trait boundary makes a future swap a one-file change when upstream ships a non-HD wallet type. See [single-key-mock.md](single-key-mock.md).

**Architect recommendation:** "Mock now, swap later" — single-key users get read-only + a clear, calm message for at least one release.

**Confirmation needed:** Confirm "mock now, swap later" is acceptable to ship. If not, state the constraint (e.g. must ship full single-key support in the same release).

---

## Decision #8 — One-Release No-Op Grace for Removed Tasks

**Needed before:** P2 (tasks removed or demoted)

**Context:** `CoreTask::RecoverAssetLocks` and `CoreTask::ListCoreWallets` are removed. `AssetLockManager` continuous tracking makes explicit recovery obsolete; named Core wallets have no meaning without RPC mode. Two options:

| Option | Description |
|---|---|
| **(a) Immediate removal** | Task variants deleted in P2; UI entry points removed or guarded |
| **(b) One-release no-op grace** | Task variants return graceful no-op success in P2, removed in P5; old UI entry points degrade gracefully rather than error |

**Architect recommendation:** (b) for `RecoverAssetLocks` (users may have the old UI cached); (a) for `ListCoreWallets` (Core-wallet picker is being removed from the UI in P2 regardless).

**Confirmation needed:** Approve this split, or choose (a) or (b) uniformly.
