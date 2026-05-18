# Phasing

**Purpose:** P0–P5 phase table with goals, gates, effort, and risk; skills/agents/crew assignments; QA matrix; highest-risk assumption verdict.

[← back to README](README.md)

---

Gates G1 and G2 are defined in [README.md § Gate Posture](README.md#gate-posture-updated) and detailed in [upstream-reality.md § G2 Caveat](upstream-reality.md#g2-caveat--walletfrom_persisted-load-gap).

## Combined Gate Posture

With Decision #1 (pin to #3625 head now) and Decision #2 (G2 downgraded via `PersistedWalletLoader` seam), **implementation is no longer upstream-blocked**.

**G1 — release-hardening track, not a start blocker.**
DET is pinned to PR #3625 head. P0–P2 start immediately. G1 resolves to: track #3625 until it merges, bump pin to a released rev before shipping P3+. Not a gate on any phase start.

**G2 — deferred swap, not a gate.**
`SeedReregistrationLoader` ships in P1 with correct behavior. `UpstreamFromPersisted` is reserved for when upstream `Wallet::from_persisted` lands — one-line construction swap, zero blast radius. See [g2-mock-boundary.md](g2-mock-boundary.md).

**Only release-blocking gate: Decision #6 DIP-14/15 migration/hard-stop QA lane.**
The per-contact migrate-or-quarantine path must be implemented (P3) and QA-proven (P3+P4) before the release ships. See [dip14-migration-hardstop.md](dip14-migration-hardstop.md).

## G. Phasing (From-Scratch Rewrite)

Each phase is independently reviewable. Do not collapse phases.

### Phase Table

| Phase | Goal | Files | Effort | Risk | Frozen contract |
|---|---|---|---|---|---|
| **P0 Spike & verify** (DONE — PROCEED) | Stand up `PlatformWalletManager` + upstream `SqliteWalletPersister` in a harness; prove `SpvRuntime` drives sync end-to-end; run DIP-14/15 golden-vector parity probe + `DiskStorageManager` behavior; confirm event surface. Pin to #3625 head (Decision #1). G2 confirmed open (load() returns empty `ClientStartState.wallets`) — `PersistedWalletLoader`/`SeedReregistrationLoader` premise validated. **Harness-shape constraint: pre-P0.5 spike harnesses MUST be standalone crates, not `tests/*.rs`** — those link the SDK-drift-broken lib. | Standalone crate harness only; `Cargo.toml` (feature-gated dep) | M | Med | Verified upstream API + probe results; DIP-14/15 divergence recorded as release-blocking finding |
| **P0.5 Compile Floor** | Atomically bump `dash-sdk` + `rs-sdk-trusted-context-provider` `54048b…`→`738091f734…`; add `platform-wallet` (feature `serde`) + `platform-wallet-storage` git deps at `738091f…`; then DELETE/STUB/FIXUP exactly enough of the old wallet stack to reach green `cargo build` + `cargo clippy --all-features --all-targets -- -D warnings`. Tests need NOT pass — failing tests are left failing or marked `#[ignore]` + `// TODO(P0.5): re-enable in P{1,2,3}`. No production wallet behavior is expected; wallet ops are inert or removed. **Co-land constraint:** the pin bump is NOT separable from the deletions — no compiling intermediate exists. P0.5 IS the atomic floor commit (or a tight commit series on the branch). P1+ build green on top of it. See [P0.5 Compile-Floor Task List](#p05-compile-floor-task-list) below. | `Cargo.toml:21+`, `src/spv/**`, `src/context/wallet_lifecycle.rs:619-985`, `src/backend_task/core/mod.rs` (heavy), `src/model/wallet/mod.rs` (heavy), `src/model/qualified_identity/mod.rs` (fixup only), `src/backend_task/shielded/bundle.rs` (fixup only), identity/contract tasks (fixup only) | M–L | Medium (over-deletion / under-stubbing) | Workspace compiles; clippy-clean; P1+ build on this floor |
| **P1 WalletBackend skeleton + EventBridge** | New `src/wallet_backend/` wrapping `PlatformWalletManager`; `EventBridge`: `PlatformEventHandler` → `TaskResult` MPSC; `PersistedWalletLoader` trait + `SeedReregistrationLoader` impl (G2 seam — see [g2-mock-boundary.md](g2-mock-boundary.md)); no DET wiring yet (parallel to old path, behind a feature). Builds on the P0.5 green floor. | New `src/wallet_backend/*`, `src/backend_task/error.rs` (typed variants) | L | Med | `WalletBackend` public method set; `EventBridge`→`TaskResult` mapping; `PersistedWalletLoader` seam |
| **P2 BackendTask rewire** | Point wallet/identity/DashPay task arms at `WalletBackend`; replace P0.5 stubs with real `WalletBackend` calls; extract Core-RPC mining utility. | `src/backend_task/{mod,core,wallet,identity,dashpay}/*`, `src/context/*`, new `src/core_rpc_util.rs` | L | High | `BackendTask` result variants stable (frontend contract) |
| **P3 One-time migration** | `from_`/`into_` adapters; first-launch migrate + backup + drop legacy tables; gated on G1. | New `src/database/migration_pw.rs`, `database/initialization.rs` | L | High | Migration forward-only, fail-safe, idempotent |
| **P4 Cleanup** | Most destructive deletion was already done in P0.5 to reach compile. P4 = remove remaining dead code; UI prune (RPC-mode toggle, Core-wallet picker, local-node settings); ZMQ-listener usage audit + drop if no non-wallet consumer; finalize dead-settings-column removal. Conditioned on migration executed + DIP-14/15 hard-stop path proven per [dip14-migration-hardstop.md §6.5](dip14-migration-hardstop.md#65--p0-probe-and-phasing-interaction). | `src/ui/**`, `src/database/**`, `src/context/**`, remaining dead code from P0.5 stubs | L | Med | Final state; docs + user-stories updated |
| **P5 Hardening** | Single-key swap-readiness, `ConnectionStatus` adapter polish, full QA matrix including migration lane. | Cross-cutting | M | Low | Release-ready |

**Sequencing:** P0 done (PROCEED). P0.5 is the atomic compile floor — must land before any P1 work. P1–P2 build green on the floor. P3 is the highest-risk phase (irreversible data migration) — mandatory backup + restore path + dedicated test lane. P3 ships only after G1 resolves to a released rev. The only release-blocking gate is the #6 DIP-14/15 migration/hard-stop QA lane (P3+P4).

---

## P0.5 Compile-Floor Task List

This section is the authoritative checklist for P0.5. Work through the seven clusters in order. The pin bump (Step 0) must land in the same atomic commit (or series) as the deletions — no intermediate that compiles with the old deps and the old code.

### Step 0 — Dependency Bump

`Cargo.toml:21+` (P0 confirmed zero version conflicts, no `[patch]`, all DET SDK features still present):

- Bump `dash-sdk` + `rs-sdk-trusted-context-provider` from `54048b…` to `738091f734…`.
- Add `platform-wallet` (feature `serde`) + `platform-wallet-storage` git deps at `738091f…`.

### Cluster A — `src/spv/` (8 errors in manager.rs)

**Classification: DELETE**

DELETE the entire `src/spv/` module tree: `manager.rs`, `error.rs`, `mod.rs`, `tests.rs`. Remove `mod spv;` declaration and all `crate::spv::*` imports throughout the workspace.

Rationale: chain sync is owned by `platform-wallet`'s `SpvRuntime`. No DET sync code is needed.

### Cluster B — `src/context/wallet_lifecycle.rs:619-985` (3 errors)

**Classification: DELETE**

Delete the following functions from `wallet_lifecycle.rs:619-985`:
- `reconcile_spv_wallets`
- `sync_spv_account_addresses`
- `spv_setup_finality_listener`
- `spv_setup_reconcile_listener`
- `handle_spv_finality_event`

Also delete the `spv_manager()` accessor and its field wiring in `context/mod.rs:97,295-360`.

### Cluster C — `src/backend_task/core/mod.rs` (8 errors)

**Classification: DELETE / STUB (mixed)**

**Delete:**
- `send_wallet_payment_via_spv`
- `build_spv_unsigned_transaction_multi` (`core/mod.rs:677`)
- `sign_spv_transaction` (`core/mod.rs:900`)
- `send_wallet_payment_via_rpc`
- `CoreBackendMode` enum and all `core_backend_mode()` branch sites
- `core_client_for_wallet` for wallet ops (`context/mod.rs:686`)
- RPC arms of `refresh_wallet_info` and `recover_asset_locks`

**Stub** (return `TaskError::WalletBackendNotYetWired`):

All retained dispatch arms whose implementation is being replaced in P2 return the new typed variant from P0.5 onward:

```rust
#[error("This action is being upgraded and is temporarily unavailable. \
    Please use the previous version of the app to transact, \
    or wait for the next update.")]
WalletBackendNotYetWired,
```

Specifically: `run_wallet_task` / `send_wallet_payment` / `RefreshWalletInfo` / `GenerateReceiveAddress` / `CreateAssetLock` / `FundPlatformAddress*`.

**Keep as-is:**
- `CoreTask::MineBlocks` on thin Core-RPC client (Regtest/Devnet only).

**Single-key arms:** return `TaskError::SingleKeyWalletsUnsupported` from P0.5 onward (see [single-key-mock.md](single-key-mock.md)).

### Cluster D — `src/model/wallet/mod.rs` (9 errors)

**Classification: DELETE / RETAIN-MINIMAL (mixed)**

**Delete:**
- Dead balance/UTXO/tx surface
- `utxos.rs`
- `update_spv_balances`
- `reconcile` maps

**Retain minimal** (required by P3 migration + `database/wallet.rs` read path):
- Seed handle
- `WalletSeed` / `ClosedKeyItem`
- Alias, `is_main`, `seed_hash`, network

**Highest over-deletion risk.** If the retained skeleton is wider or narrower than needed, P3 migration will fail. Flag any uncertainty as a blocking finding before proceeding.

### Cluster E — `src/model/qualified_identity/mod.rs` (4 errors)

**Classification: SDK-DRIFT-FIXUP — RETAIN, do NOT stub or delete**

Fix the following mechanically:
- `sign` / `sign_create_witness` lifetime annotations
- `AddressProvider` member updates
- Missing associated types `Tag` / `Address`

These are upstream API drift fixes, not migration deletions. Mis-classifying this cluster as a stub candidate would cause silent capability loss (A04 violation). If any fix proves too expensive within budget, escalate as a BLOCKING finding — do not replace with `unimplemented!()`.

### Cluster F — `src/backend_task/shielded/bundle.rs` (6 errors)

**Classification: SDK-DRIFT-FIXUP — RETAIN, do NOT stub or delete**

Fix the following:
- `AddressKey` → `AddressOps` rename
- `async ?`-on-`Pin<Box<dyn Future>>` usage
- Trait signature updates

If any fix is unfixable within budget: escalate as a BLOCKING finding. Do NOT stub shielded ops — silent capability loss is forbidden (A04). Never use `unimplemented!()` for retained code.

### Cluster G — Identity / Contract Tasks (4 errors)

**Classification: SDK-DRIFT-FIXUP — RETAIN, do NOT stub or delete**

Mechanical signature and import updates only. Never stub identity or contract tasks.

---

### Disambiguation Summary

Three distinct operations that MUST NOT be confused:

a. **DELETE** (Clusters A, B, C-delete, D-delete): code that belongs to the old SPV/RPC wallet stack, fully replaced by `platform-wallet`. Recoverable via git history on the branch — no tombstones, no commented-out code (M-NO-TOMBSTONES).

b. **STUB** (Cluster C-stub): retained dispatch arms that will be rewired in P2, rendered temporarily inert with a typed error. Stubs exist from P0.5 onward; they disappear in P2 when real `WalletBackend` calls replace them.

c. **SDK-DRIFT-FIXUP** (Clusters E, F, G): code that is out of the migration scope entirely, broken only because upstream API signatures changed. Fix these; never delete or stub them. **This is the highest-risk mis-classification** — if E/F/G code ends up behind `unimplemented!()`, capability is silently lost.

**Data recoverability:** P0.5 and P4 deletions are recoverable via git history on the branch. No commented-out code is left in place (M-NO-TOMBSTONES). P0.5 and P4 touch code only — no DB schema change before P3, no user data is at risk. P3 adds `*.db.premigration` + DIP-14/15 quarantine retention. Consistent with A04 fail-safe ordering.

---

## I. Skills, Agents, and QA Matrix

### Governing Workflow

Standard Requirements → Architecture (this spec) → Implementation → QA → Review, per phase. All 8 decisions resolved (see [open-questions.md](open-questions.md)). Implementation is unblocked.

### Crew Assignments

| Phase | Lead crew | Mandatory reviewers | Skills enforced |
|---|---|---|---|
| P0 | Research/spike agent + Architect | — | rust-best-practices (M-PRIOR-ART, M-STATIC-VERIFICATION) |
| P0.5 | Rust impl agent | Architect (delete/stub/fixup classification review) | rust-best-practices (M-NO-TOMBSTONES); security (A04 over-deletion check) |
| P1 | Rust impl agent | Architect (boundary/frozen-contract review) | rust-best-practices (M-DONT-LEAK-TYPES, C-NEWTYPE-HIDE, M-SERVICES-CLONE) |
| P2 | Rust impl agent | Architect (BackendTask contract) | rust-best-practices (error taxonomy, M-APP-ERROR); security (A09 error wrapping) |
| P3 | Rust impl agent | Data-integrity reviewer — mandatory | security (A08 safe deserialization, A04 fail-safe migration, ASVS V14.2 secret boundary — seeds never enter persister) |
| P4 | Rust impl agent | Correctness reviewer — mandatory (DIP-14/15 parity gate green) | rust-best-practices (M-NO-TOMBSTONES, test quality) |
| P5 | Rust impl agent + Architect | — | Full static verification |

### QA Matrix

All phases run the standard baseline:
- `cargo clippy --all-features --all-targets -- -D warnings`
- `cargo +nightly fmt --all`
- `cargo test --all-features --workspace`

Plus targeted lanes:

| Lane | Phases | What |
|---|---|---|
| **Compile-floor verification** | P0.5 | `cargo build` + `cargo clippy` green. Tests need not pass — failing tests left with `#[ignore]` + `// TODO(P0.5): re-enable in P{1,2,3}`. |
| **SpvRuntime-owned-sync verification** | P0 | Prove `PlatformWalletManager.start()` → blocks sync → balances/tx appear via `PlatformEventHandler` with **no DET sync code running**. This is the load-bearing assumption of the whole spec — it must be confirmed before P1. |
| **DIP-14/15 golden-vector parity** | P0 (probe), P4 (regression) | Byte-equality of contact xpub + payment addresses + `calculate_account_reference`, both identifier classes (low / full 256-bit), both networks. Full-256-bit divergence = **release-blocking finding** — forces upstream dip14.rs fix or explicit acceptance that runtime migrate-or-hard-stop is sole safety net. See [dip14-migration-hardstop.md §6.5](dip14-migration-hardstop.md#65--p0-probe-and-phasing-interaction). |
| **`PersistedWalletLoader` seam** | P1, P5 (regression) | Mock yields exactly N `WalletRegistration` for N seed-store wallets; backend registers all N and `start()` succeeds. Swap-boundary compiles with alternate `StubFromPersisted`. Seed-decrypt failure surfaces existing typed `TaskError`. See [g2-mock-boundary.md §G2.6](g2-mock-boundary.md#g26--phasing-and-qa). |
| **DIP-14/15 migrate-or-quarantine lane (release-blocking)** | P3, P4 | Fixtures with low-index (expect migratable), deliberately-divergent full-256-bit (expect quarantine), mixed set. Asserts: migratable → persister byte-identical; divergent → quarantined + legacy DashPay/contact tables retained + `*.db.premigration` preserved + blocking banner + structured diagnostic + app starts non-DashPay intact + DashPay to quarantined contacts blocked. See [dip14-migration-hardstop.md §6.5](dip14-migration-hardstop.md#65--p0-probe-and-phasing-interaction). |
| **One-time-migration lane** | P3 | Synthetic legacy DB (HD + single-key + identities + DashPay) → migrate → assert: wallets re-registered via `SeedReregistrationLoader`, identities present, contacts present (or quarantined + retained), legacy HD/UTXO/SPV tables dropped, single-key preserved+flagged, backup file exists, failure path restores. |
| **Backend E2E (testnet, `tests/backend-e2e/`)** | P2, P4 | Wallet load, balance, send, identity register/top-up, DashPay contact — through `WalletBackend`. |
| **ConnectionStatus adapter** | P4, P5 | UI sync-progress visually matches former SPV behavior, fed from `SpvRuntime::sync_progress()`. |
| **Single-key stub** | P2, P5 | Stub returns typed error + correct banner; swap-boundary trait compiles with a no-op alternate impl. |

`docs/user-stories.md` updated at P4 (RPC mode removed, single-key degraded). `claudius:lessons-learned` invoked at each phase close.

---

## Highest-Risk Assumption — Explicit Verdict

**Decision #1 ("platform-wallet owns SPV internally") is CONFIRMED, not contradicted, at PR #3625 head.**

Evidence: `Cargo.toml` direct `dash-spv` dep; `SpvRuntime` constructs/owns `DashSpvClient` and runs its own sync loop (`run`/`spawn_in_background`); `PlatformWalletManager` owns the `SpvRuntime`; no host-feed API exists. DET's `src/spv/` is deletable. The only residue is a thin DET-side `ConnectionStatus` display adapter fed by upstream events — not chain sync.

**Remaining live risks (updated for resolved decisions):**

1. **G1 — release-hardening track.** DET is now pinned to #3625 head (Decision #1). G1 is no longer a start blocker; it resolves at release time to a merged + released platform rev for P3+.
2. **G2 — deferred swap.** Mitigated by `PersistedWalletLoader` seam ([g2-mock-boundary.md](g2-mock-boundary.md)). Not a gate.
3. **DIP-14/15 migration — release-blocking QA lane.** The per-contact migrate-or-quarantine path must be implemented and QA-proven. P0 full-256-bit probe divergence becomes a release-blocking finding requiring upstream fix or explicit acceptance. See [dip14-migration-hardstop.md](dip14-migration-hardstop.md).
4. **P0.5 mis-classification risk.** Highest risk: treating Clusters E/F/G (SDK-drift fixups) as stub candidates. Any `unimplemented!()` in retained code is a silent capability loss. Escalate expensive fixups; never stub retained code.
