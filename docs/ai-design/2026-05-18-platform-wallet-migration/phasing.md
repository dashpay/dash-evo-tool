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
| **P0 Spike & verify** | Stand up `PlatformWalletManager` + upstream `SqliteWalletPersister` in a harness; prove `SpvRuntime` drives sync end-to-end; run DIP-14/15 golden-vector parity probe (full-256-bit divergence = release-blocking finding) + `DiskStorageManager` behavior; confirm event surface. Pin to #3625 head (Decision #1). | `tests/` only, `Cargo.toml` (feature-gated dep) | M | Med | Verified upstream API + probe results |
| **P1 WalletBackend skeleton + EventBridge** | New `src/wallet_backend/` wrapping `PlatformWalletManager`; `EventBridge`: `PlatformEventHandler` → `TaskResult` MPSC; `PersistedWalletLoader` trait + `SeedReregistrationLoader` impl (G2 seam — see [g2-mock-boundary.md](g2-mock-boundary.md)); no DET wiring yet (parallel to old path, behind a feature) | New `src/wallet_backend/*`, `src/backend_task/error.rs` (typed variants) | L | Med | `WalletBackend` public method set; `EventBridge`→`TaskResult` mapping; `PersistedWalletLoader` seam |
| **P2 BackendTask rewire** | Point wallet/identity/DashPay task arms at `WalletBackend`; delete `reconcile_spv_wallets`; collapse `CoreBackendMode`; extract Core-RPC mining utility; single-key stub | `src/backend_task/{mod,core,wallet,identity,dashpay}/*`, `src/context/*`, new `src/core_rpc_util.rs` | L | High | `BackendTask` result variants stable (frontend contract) |
| **P3 One-time migration** | `from_`/`into_` adapters; first-launch migrate + backup + drop legacy tables; gated on G1 | New `src/database/migration_pw.rs`, `database/initialization.rs` | L | High | Migration forward-only, fail-safe, idempotent |
| **P4 Destructive deletion** | Delete `src/spv/`, RPC wallet path, dead model/DB/settings, DIP-14/15 derivation (conditioned on migration executed + hard-stop path proven per [dip14-migration-hardstop.md §6.5](dip14-migration-hardstop.md#65--p0-probe-and-phasing-interaction)), SPV `ConnectionStatus` plumbing; UI prune | `src/spv/**`, `src/model/wallet/**`, `src/context/**`, `src/ui/**`, `src/database/**` | L | Med | Final state; docs + user-stories updated |
| **P5 Hardening** | Single-key swap-readiness, `ConnectionStatus` adapter polish, full QA matrix incl. migration lane | Cross-cutting | M | Low | Release-ready |

**Sequencing note:** P0–P2 start immediately against the pinned #3625 head (Decision #1 — G1 is no longer a start blocker). P3 is the highest-risk phase (irreversible data migration) — mandatory backup + restore path + dedicated test lane (see QA matrix below). P3 ships only after G1 resolves to a released rev. The only release-blocking gate is the #6 DIP-14/15 migration/hard-stop QA lane (P3+P4).

---

## I. Skills, Agents, and QA Matrix

### Governing Workflow

Standard Requirements → Architecture (this spec) → Implementation → QA → Review, per phase. All 8 decisions resolved (see [open-questions.md](open-questions.md)). Implementation is unblocked.

### Crew Assignments

| Phase | Lead crew | Mandatory reviewers | Skills enforced |
|---|---|---|---|
| P0 | Research/spike agent + Architect | — | rust-best-practices (M-PRIOR-ART, M-STATIC-VERIFICATION) |
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
