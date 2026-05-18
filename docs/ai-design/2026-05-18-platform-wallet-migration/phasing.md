# Phasing

**Purpose:** P0–P5 phase table with goals, gates, effort, and risk; skills/agents/crew assignments; QA matrix; highest-risk assumption verdict.

[← back to README](README.md)

---

Gates G1 and G2 are defined in [README.md § Hard Sequencing Gates](README.md#hard-sequencing-gates) and detailed in [upstream-reality.md § G2 Caveat](upstream-reality.md#g2-caveat--walletfrom_persisted-load-gap).

## G. Phasing (From-Scratch Rewrite)

Each phase is independently reviewable. Do not collapse phases.

**Gate G1:** PR #3625 must merge AND DET must bump its platform pin (`Cargo.toml:21`, currently `54048b9352…`) to a containing rev. Blocks P3+.

**Gate G2:** Upstream `Wallet::from_persisted` / `ClientStartState.wallets load()` gap still OPEN at head. All phases must use the seed-re-registration pattern, not persister-driven wallet rehydration.

### Phase Table

| Phase | Goal | Files | Effort | Risk | Frozen contract |
|---|---|---|---|---|---|
| **P0 Spike & verify** | Stand up `PlatformWalletManager` + upstream `SqliteWalletPersister` in a harness; prove `SpvRuntime` drives sync end-to-end; run DIP-14/15 golden-vector parity probe + `DiskStorageManager` behavior; confirm event surface | `tests/` only, `Cargo.toml` (feature-gated dep) | M | Med | Verified upstream API + probe results |
| **P1 WalletBackend skeleton + EventBridge** | New `src/wallet_backend/` wrapping `PlatformWalletManager`; `EventBridge`: `PlatformEventHandler` → `TaskResult` MPSC; no DET wiring yet (parallel to old path, behind a feature) | New `src/wallet_backend/*`, `src/backend_task/error.rs` (typed variants) | L | Med | `WalletBackend` public method set; `EventBridge`→`TaskResult` mapping |
| **P2 BackendTask rewire** | Point wallet/identity/DashPay task arms at `WalletBackend`; delete `reconcile_spv_wallets`; collapse `CoreBackendMode`; extract Core-RPC mining utility; single-key stub | `src/backend_task/{mod,core,wallet,identity,dashpay}/*`, `src/context/*`, new `src/core_rpc_util.rs` | L | High | `BackendTask` result variants stable (frontend contract) |
| **P3 One-time migration** | `from_`/`into_` adapters; first-launch migrate + backup + drop legacy tables; gated on G1 | New `src/database/migration_pw.rs`, `database/initialization.rs` | L | High | Migration forward-only, fail-safe, idempotent |
| **P4 Destructive deletion** | Delete `src/spv/`, RPC wallet path, dead model/DB/settings, DIP-14/15 derivation (after parity probe green), SPV `ConnectionStatus` plumbing; UI prune | `src/spv/**`, `src/model/wallet/**`, `src/context/**`, `src/ui/**`, `src/database/**` | L | Med | Final state; docs + user-stories updated |
| **P5 Hardening** | Single-key swap-readiness, `ConnectionStatus` adapter polish, full QA matrix incl. migration lane | Cross-cutting | M | Low | Release-ready |

**Sequencing note:** P0–P1 are not blocked by G1 (skeleton can compile against the PR branch in a spike). P2+ require G1. P3 is the highest-risk phase (irreversible data migration) — mandatory backup + restore path + dedicated test lane (see QA matrix below).

---

## I. Skills, Agents, and QA Matrix

### Governing Workflow

Standard Requirements → Architecture (this spec) → Implementation → QA → Review, per phase. No implementation until this spec is user-approved (see [open-questions.md](open-questions.md) — all 8 decisions pending).

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
| **DIP-14/15 golden-vector parity** | P0 (probe), P4 (regression) | Byte-equality of contact xpub + payment addresses + `calculate_account_reference`, both identifier classes (low / full 256-bit), both networks. Red → DIP-14/15 deletion blocked; see [open-questions.md #6](open-questions.md). |
| **One-time-migration lane** | P3 | Synthetic legacy DB (HD + single-key + identities + DashPay) → migrate → assert: wallets re-registered, identities present, contacts present, legacy tables dropped, single-key preserved+flagged, backup file exists, failure path restores. |
| **Backend E2E (testnet, `tests/backend-e2e/`)** | P2, P4 | Wallet load, balance, send, identity register/top-up, DashPay contact — through `WalletBackend`. |
| **ConnectionStatus adapter** | P4, P5 | UI sync-progress visually matches former SPV behavior, fed from `SpvRuntime::sync_progress()`. |
| **Single-key stub** | P2, P5 | Stub returns typed error + correct banner; swap-boundary trait compiles with a no-op alternate impl. |

`docs/user-stories.md` updated at P4 (RPC mode removed, single-key degraded). `claudius:lessons-learned` invoked at each phase close.

---

## Highest-Risk Assumption — Explicit Verdict

**Decision #1 ("platform-wallet owns SPV internally") is CONFIRMED, not contradicted, at PR #3625 head.**

Evidence: `Cargo.toml` direct `dash-spv` dep; `SpvRuntime` constructs/owns `DashSpvClient` and runs its own sync loop (`run`/`spawn_in_background`); `PlatformWalletManager` owns the `SpvRuntime`; no host-feed API exists. DET's `src/spv/` is deletable. The only residue is a thin DET-side `ConnectionStatus` display adapter fed by upstream events — not chain sync.

**Two remaining live risks:**

1. **G1 — sequencing.** The persister PR (#3625) is an unmerged draft on a different platform line. DET cannot pin it yet.
2. **G2 — `Wallet::from_persisted` gap.** Mitigated by the seed-re-registration pattern upstream itself prescribes.

Neither blocks writing or approving this spec. Both block implementation start of P3+.
