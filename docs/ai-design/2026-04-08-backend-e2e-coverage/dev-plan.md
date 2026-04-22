# Backend E2E Test Coverage — Development Plan

**Date:** 2026-04-08
**Branch:** `test/backend-e2e-coverage` on top of PR #814
**Constraint:** Test-only changes — no production code modifications
**Total:** 83 test cases across 8 groups, 8 new test files, 5 new framework helper files

---

## 1. Architecture

### 1.1 File Layout

```
tests/backend-e2e/
├── main.rs                          # Module declarations (MODIFIED — add 8 new modules)
├── framework/
│   ├── mod.rs                       # (MODIFIED — add 4 new submodules)
│   ├── harness.rs                   # (EXISTING — unchanged)
│   ├── task_runner.rs               # (EXISTING — unchanged)
│   ├── wait.rs                      # (EXISTING — unchanged)
│   ├── identity_helpers.rs          # (EXISTING — unchanged)
│   ├── funding.rs                   # (EXISTING — unchanged)
│   ├── cleanup.rs                   # (EXISTING — unchanged)
│   ├── fixtures.rs                  # NEW — OnceCell shared fixtures
│   ├── dashpay_helpers.rs           # NEW — DashPay identity creation, contact request helpers
│   ├── token_helpers.rs             # NEW — Token contract registration, minting helpers
│   ├── mnlist_helpers.rs            # NEW — Block info retrieval from SPV
│   └── shielded_helpers.rs          # NEW — Proving key warmup, shielded wallet init
│
├── core_tasks.rs                    # NEW — TC-001 to TC-011 (11 tests)
├── wallet_tasks.rs                  # NEW — TC-012 to TC-019 (8 tests)
├── identity_tasks.rs                # NEW — TC-020 to TC-030 (11 tests)
├── dashpay_tasks.rs                 # NEW — TC-031 to TC-044 (14 tests)
├── token_tasks.rs                   # NEW — TC-045 to TC-065 (21 tests)
├── broadcast_st_tasks.rs            # NEW — TC-066 to TC-067 (2 tests)
├── mnlist_tasks.rs                  # NEW — TC-068 to TC-073 (6 tests)
└── shielded_tasks.rs                # NEW — TC-074 to TC-083 (10 tests)
```

### 1.2 Module Structure (`main.rs` changes)

Add 8 new module declarations after existing ones:

```rust
mod core_tasks;
mod wallet_tasks;
mod identity_tasks;
mod dashpay_tasks;
mod token_tasks;
mod broadcast_st_tasks;
mod mnlist_tasks;
mod shielded_tasks;
```

Add 4 new submodules to `framework/mod.rs`:

```rust
pub mod fixtures;
pub mod dashpay_helpers;
pub mod token_helpers;
pub mod mnlist_helpers;
pub mod shielded_helpers;
```

### 1.3 Shared Fixtures Design (`framework/fixtures.rs`)

All expensive setup (identity registration, token contract deployment, DashPay pair creation) uses `tokio::sync::OnceCell` for lazy, one-time initialization within the shared runtime. Each fixture accessor is an `async fn` returning `&'static T`.

```rust
use tokio::sync::OnceCell;

// --- SHARED_IDENTITY ---
// A single registered identity reused across identity/token/broadcast tests.
// Initialized by registering a new identity from the framework wallet at index 0.
static SHARED_IDENTITY: OnceCell<SharedIdentity> = OnceCell::const_new();

pub struct SharedIdentity {
    pub qualified_identity: QualifiedIdentity,
    pub wallet_arc: Arc<RwLock<Wallet>>,
    pub wallet_seed_hash: WalletSeedHash,
    pub signing_key: IdentityPublicKey,     // master auth key
    pub signing_key_bytes: Vec<u8>,         // private key bytes
}

pub async fn shared_identity() -> &'static SharedIdentity { ... }

// --- SHARED_TOKEN ---
// Token contract + position registered by SHARED_IDENTITY.
// Initialized by deploying a token contract with permissive rules.
static SHARED_TOKEN: OnceCell<SharedToken> = OnceCell::const_new();

pub struct SharedToken {
    pub data_contract: Arc<DataContract>,
    pub token_position: TokenContractPosition,
    pub token_id: Identifier,
}

pub async fn shared_token() -> &'static SharedToken { ... }

// --- SHARED_DASHPAY_PAIR ---
// Two identities (A, B) with DashPay keys and DPNS names.
// Used for contact request / accept / reject flow tests.
static SHARED_DASHPAY_PAIR: OnceCell<SharedDashPayPair> = OnceCell::const_new();

pub struct SharedDashPayPair {
    pub identity_a: QualifiedIdentity,  // sender
    pub identity_b: QualifiedIdentity,  // receiver
    pub username_a: String,
    pub username_b: String,
    pub signing_key_a: (IdentityPublicKey, Vec<u8>),
    pub signing_key_b: (IdentityPublicKey, Vec<u8>),
    pub wallet_a: Arc<RwLock<Wallet>>,
    pub wallet_b: Arc<RwLock<Wallet>>,
}

pub async fn shared_dashpay_pair() -> &'static SharedDashPayPair { ... }
```

Key design decisions:
- Each `OnceCell` is initialized by the first test that calls the accessor.
- Initialization reuses existing `harness::ctx()` for `AppContext`.
- `SharedIdentity` creates a dedicated funded test wallet (2M duffs) rather than using the framework wallet, to isolate identity-index usage.
- `SharedDashPayPair` creates two separate funded wallets (3M duffs each — identity + DashPay keys are more expensive).
- `SharedToken` depends on `SharedIdentity` (calls `shared_identity()` first).

---

## 2. Implementation Tasks

### Task 0: Framework Helpers & Fixtures

**Files created:**
- `tests/backend-e2e/framework/fixtures.rs`
- `tests/backend-e2e/framework/dashpay_helpers.rs`
- `tests/backend-e2e/framework/token_helpers.rs`
- `tests/backend-e2e/framework/mnlist_helpers.rs`
- `tests/backend-e2e/framework/shielded_helpers.rs`

**Files modified:**
- `tests/backend-e2e/framework/mod.rs` (add 5 new `pub mod` lines)
- `tests/backend-e2e/main.rs` (add 8 new `mod` lines for test files — can also be done in Task 0 since all test files will be empty stubs until their task)

**Contents:**

| File | Functions | Lines (est.) | Used by TCs |
|------|-----------|------|-------------|
| `fixtures.rs` | `SharedIdentity`, `shared_identity()`, `SharedToken`, `shared_token()`, `SharedDashPayPair`, `shared_dashpay_pair()` | ~250 | All groups except CoreTask, WalletTask, MnListTask |
| `dashpay_helpers.rs` | `create_dashpay_identity(ctx, wallet, seed_hash) -> QualifiedIdentity`, `get_dashpay_signing_key(qi) -> (IdentityPublicKey, Vec<u8>)`, `get_encryption_key(qi) -> (IdentityPublicKey, Vec<u8>)` | ~120 | TC-031..TC-044 |
| `token_helpers.rs` | `build_token_contract_registration(identity, signing_key) -> RegisterTokenContract fields`, `mint_tokens(ctx, identity, contract, position, signing_key, amount)` | ~150 | TC-045..TC-065 |
| `mnlist_helpers.rs` | `get_current_block_info(ctx) -> (u32, BlockHash)`, `get_block_hash_at_height(ctx, height) -> BlockHash` | ~60 | TC-068..TC-073 |
| `shielded_helpers.rs` | `skip_if_shielded_disabled()`, `warm_up_and_init(ctx, seed_hash)` | ~50 | TC-074..TC-083 |

**Estimated total: ~630 lines**
**Agent:** `developer-bilby` (opus — complex fixture initialization logic, async OnceCell patterns)
**Conflicts:** Modifies `main.rs` and `framework/mod.rs` (all other tasks only create new files, but they need the module declarations). Task 0 adds all `mod` declarations upfront.

---

### Task 1: Core Task Tests (`core_tasks.rs`)

**Test cases:** TC-001, TC-002, TC-003, TC-004, TC-005, TC-006, TC-007, TC-008, TC-009, TC-010, TC-011
**Files created:** `tests/backend-e2e/core_tasks.rs`
**Framework deps:** `harness::ctx()`, `task_runner::run_task()`, `fixtures::shared_identity()` (for TC-005 only)
**New helpers needed:** None (all use existing framework)
**Estimated lines:** ~350
**Agent:** `developer-bilby` (sonnet — straightforward dispatch + assert)
**Conflicts:** None (independent file)

Notes:
- TC-003 (RefreshSingleKeyWalletInfo) needs a `SingleKeyWallet` fixture created inline — not worth a shared helper since only one test uses it.
- TC-009 (SendSingleKeyWalletPayment) requires a funded single-key wallet. May need to fund it from the framework wallet first.
- TC-010 (ListCoreWallets) uses env-var guard: `if std::env::var("E2E_CORE_RPC_URL").is_err() { return; }`

---

### Task 2: Wallet Task Tests (`wallet_tasks.rs`)

**Test cases:** TC-012, TC-013, TC-014, TC-015, TC-016, TC-017, TC-018, TC-019
**Files created:** `tests/backend-e2e/wallet_tasks.rs`
**Framework deps:** `harness::ctx()`, `task_runner::run_task()`, `wait::*`
**New helpers needed:** None — wallet/platform address operations use BackendTask variants directly
**Estimated lines:** ~400
**Agent:** `developer-bilby` (sonnet — sequential flow with assertion verification)
**Conflicts:** None

Notes:
- TC-014 through TC-017 form a sequence (fund -> verify balance -> transfer -> withdraw). Each test must be self-contained with its own setup, but since tests run serially within the file, they can share state via module-level `OnceCell` for the funded platform address.
- TC-018 (FundPlatformAddressFromAssetLock) requires calling `CoreTask::CreateRegistrationAssetLock` first as setup.

---

### Task 3: Identity Task Tests (`identity_tasks.rs`)

**Test cases:** TC-020, TC-021, TC-022, TC-023, TC-024, TC-025, TC-026, TC-027, TC-028, TC-029, TC-030
**Files created:** `tests/backend-e2e/identity_tasks.rs`
**Framework deps:** `harness::ctx()`, `task_runner::run_task()`, `fixtures::shared_identity()`, `identity_helpers::*`
**New helpers needed:** None
**Estimated lines:** ~450
**Agent:** `developer-bilby` (sonnet — SHARED_IDENTITY fixture handles complexity; tests are dispatch + verify)
**Conflicts:** None

Notes:
- TC-021 (TopUpIdentityFromPlatformAddresses) requires a funded platform address — self-setup within the test via `FundPlatformAddressFromWalletUtxos`.
- TC-023 (Transfer) requires a second identity — create a fresh one in-test or use SHARED_DASHPAY_PAIR. Prefer creating a minimal second identity in-test to avoid coupling with DashPay fixture.
- TC-028, TC-029 (SearchIdentityFromWallet, SearchIdentitiesUpToIndex) need `WalletArcRef` construction. Check production code for how `WalletArcRef` is built.

---

### Task 4: DashPay Task Tests (`dashpay_tasks.rs`)

**Test cases:** TC-031, TC-032, TC-033, TC-034, TC-035, TC-036, TC-037, TC-038, TC-039, TC-040, TC-041, TC-042, TC-043, TC-044
**Files created:** `tests/backend-e2e/dashpay_tasks.rs`
**Framework deps:** `harness::ctx()`, `task_runner::run_task()`, `fixtures::shared_dashpay_pair()`, `dashpay_helpers::*`
**New helpers needed:** Uses `dashpay_helpers` created in Task 0
**Estimated lines:** ~600
**Agent:** `developer-bilby` (opus — complex multi-step flows: send request -> load -> accept -> verify contacts; DashPay key handling)
**Conflicts:** None

Notes:
- TC-037 through TC-042 form a sequential flow (send contact request -> load requests -> accept -> register addresses -> load contacts -> update info). Module-level `OnceCell` stores intermediate state (e.g., `request_id` from TC-038).
- TC-043 (RejectContactRequest) requires a third DashPay identity (C). Create it in-test with a fresh wallet. This makes TC-043 the most expensive DashPay test (~60s).
- TC-044 (error: nonexistent username) is independent.

---

### Task 5: Token Task Tests (`token_tasks.rs`)

**Test cases:** TC-045, TC-046, TC-047, TC-048, TC-049, TC-050, TC-051, TC-052, TC-053, TC-054, TC-055, TC-056, TC-057, TC-058, TC-059, TC-060, TC-061, TC-062, TC-063, TC-064, TC-065
**Files created:** `tests/backend-e2e/token_tasks.rs`
**Framework deps:** `harness::ctx()`, `task_runner::run_task()`, `fixtures::shared_identity()`, `fixtures::shared_token()`, `token_helpers::*`
**New helpers needed:** Uses `token_helpers` created in Task 0
**Estimated lines:** ~800
**Agent:** `developer-bilby` (opus — 21 tests, complex token lifecycle: mint -> burn/transfer/freeze/unfreeze/destroy/pause/resume/purchase; token contract construction with specific rules)
**Conflicts:** None

Notes:
- TC-045 (RegisterTokenContract) initializes `SHARED_TOKEN` via `shared_token()`.
- TC-053 (MintTokens) must run before TC-054..TC-058 (they depend on minted balance). Use a module-level `OnceCell<bool>` to track whether minting has happened.
- TC-055 (TransferTokens) requires a second identity as recipient. Create a minimal identity in-test.
- TC-056 -> TC-057 (Freeze -> Unfreeze) and TC-058 (DestroyFrozenFunds) need a freezable target. Use the second identity from TC-055 or create another.
- TC-059 -> TC-060 (Pause -> Resume) are sequential.
- TC-061 -> TC-062 (SetPrice -> Purchase) require second identity with credits.
- TC-064 (EstimatePerpetualRewards) may return graceful error if no distribution configured — assert `Ok` or specific error variant.

---

### Task 6: Broadcast State Transition Tests (`broadcast_st_tasks.rs`)

**Test cases:** TC-066, TC-067
**Files created:** `tests/backend-e2e/broadcast_st_tasks.rs`
**Framework deps:** `harness::ctx()`, `task_runner::run_task()`, `fixtures::shared_identity()`
**New helpers needed:** None
**Estimated lines:** ~120
**Agent:** `developer-bilby` (sonnet — 2 tests, but TC-066 requires building a valid StateTransition programmatically which needs SDK familiarity)
**Conflicts:** None

Notes:
- TC-066: Build an `IdentityUpdateTransition` adding a new key. Must fetch current identity nonce from Platform first. Use `dash-sdk` builder APIs.
- TC-067: Build an unsigned / wrong-nonce state transition. Assert `Err(TaskError::...)`.

---

### Task 7: MnList Task Tests (`mnlist_tasks.rs`)

**Test cases:** TC-068, TC-069, TC-070, TC-071, TC-072, TC-073
**Files created:** `tests/backend-e2e/mnlist_tasks.rs`
**Framework deps:** `harness::ctx()`, `task_runner::run_task()`, `mnlist_helpers::*`
**New helpers needed:** Uses `mnlist_helpers` created in Task 0
**Estimated lines:** ~250
**Agent:** `developer-bilby` (sonnet — read-only P2P queries, block hash retrieval)
**Conflicts:** None

Notes:
- TC-072 (FetchChainLocks) uses env-var guard for `E2E_CORE_RPC_URL`.
- Block hash retrieval from SPV: `mnlist_helpers::get_current_block_info()` reads from SPV's chain state. Need to inspect `SpvManager` API for block-at-height lookups.
- TC-073 (error: invalid block hash) uses all-zeros `BlockHash`.

---

### Task 8: Shielded Task Tests (`shielded_tasks.rs`)

**Test cases:** TC-074, TC-075, TC-076, TC-077, TC-078, TC-079, TC-080, TC-081, TC-082, TC-083
**Files created:** `tests/backend-e2e/shielded_tasks.rs`
**Framework deps:** `harness::ctx()`, `task_runner::run_task()`, `shielded_helpers::*`, `wait::*`
**New helpers needed:** Uses `shielded_helpers` created in Task 0
**Estimated lines:** ~350
**Agent:** `developer-bilby` (opus — ZK proving, shielded pool operations, asset locks, complex flow sequencing)
**Conflicts:** None

Notes:
- All tests guarded by `shielded_helpers::skip_if_shielded_disabled()` at the top.
- TC-074 (WarmUpProvingKey) may take 30-60s on first run due to proving key download.
- TC-078 -> TC-080 -> TC-081/TC-082 form the shielded lifecycle chain. Use module-level `OnceCell<bool>` to track whether shielding has occurred.
- TC-079 (ShieldCredits) requires a funded platform address — self-setup via `FundPlatformAddressFromWalletUtxos`.
- TC-083 (error: uninitialized wallet) uses a fresh `WalletSeedHash` that has not been initialized.

---

## 3. Dependency Order

```
Task 0 (Framework + Fixtures)
    │
    ├──── Task 1 (CoreTask)         ─┐
    ├──── Task 2 (WalletTask)        │
    ├──── Task 3 (IdentityTask)      │
    ├──── Task 4 (DashPayTask)       ├── All parallel (independent files)
    ├──── Task 5 (TokenTask)         │
    ├──── Task 6 (BroadcastST)       │
    ├──── Task 7 (MnListTask)        │
    └──── Task 8 (ShieldedTask)     ─┘
```

**Task 0 must complete first** — it creates the shared files (`main.rs` module declarations, `framework/mod.rs`, fixture definitions, helper modules). All subsequent tasks depend on it.

**Tasks 1-8 are fully parallel** — each creates exactly one new test file and reads only from the framework helpers established in Task 0. No cross-file conflicts exist.

**File conflict matrix:**

| File | Task 0 | Task 1-8 |
|------|--------|----------|
| `main.rs` | WRITE (add `mod` lines) | READ only |
| `framework/mod.rs` | WRITE (add `pub mod` lines) | READ only |
| `framework/fixtures.rs` | CREATE | READ only |
| `framework/*_helpers.rs` | CREATE | READ only |
| `core_tasks.rs` | — | Task 1 CREATE |
| `wallet_tasks.rs` | — | Task 2 CREATE |
| `identity_tasks.rs` | — | Task 3 CREATE |
| `dashpay_tasks.rs` | — | Task 4 CREATE |
| `token_tasks.rs` | — | Task 5 CREATE |
| `broadcast_st_tasks.rs` | — | Task 6 CREATE |
| `mnlist_tasks.rs` | — | Task 7 CREATE |
| `shielded_tasks.rs` | — | Task 8 CREATE |

---

## 4. Agent Assignments

| Task | Agent | Model | Rationale |
|------|-------|-------|-----------|
| Task 0: Framework Helpers + Fixtures | `developer-bilby` | **opus** | Complex async OnceCell initialization, DashPay key derivation, token contract construction — requires deep understanding of production code patterns |
| Task 1: Core Task Tests | `developer-bilby` | **sonnet** | Straightforward dispatch-and-assert; SingleKeyWallet construction is the only nuance |
| Task 2: Wallet Task Tests | `developer-bilby` | **sonnet** | Sequential flow with platform address operations — well-documented in test specs |
| Task 3: Identity Task Tests | `developer-bilby` | **sonnet** | Relies on SHARED_IDENTITY fixture; most tests are single-dispatch with re-fetch verification |
| Task 4: DashPay Task Tests | `developer-bilby` | **opus** | Multi-step contact request flow, DashPay key handling, third identity for reject test, encryption key derivation |
| Task 5: Token Task Tests | `developer-bilby` | **opus** | 21 tests covering full token lifecycle; complex token contract construction with specific rules (minting, freezing, marketplace); cross-test state dependencies |
| Task 6: Broadcast ST Tests | `developer-bilby` | **sonnet** | 2 tests; building a StateTransition requires SDK knowledge but the spec is precise |
| Task 7: MnList Task Tests | `developer-bilby` | **sonnet** | Read-only P2P queries; main challenge is retrieving block hashes from SPV |
| Task 8: Shielded Task Tests | `developer-bilby` | **opus** | ZK proving, asset lock → shield → transfer → unshield chain; timing-sensitive; compute-intensive operations |

---

## 5. Framework Helpers Inventory

### `framework/fixtures.rs`

| Function/Struct | Description | Used by TCs | Production parallel |
|------|-------------|-------------|---------------------|
| `SharedIdentity` struct | Holds registered identity + wallet + signing key | TC-020..TC-030, TC-045..TC-067 | `QualifiedIdentity` in `src/model/qualified_identity/` |
| `shared_identity()` | OnceCell accessor; registers identity at index 0 | Same as above | `IdentityTask::RegisterIdentity` in `src/backend_task/identity/mod.rs` |
| `SharedToken` struct | Holds token contract + position + token ID | TC-045..TC-065 | Token state in `src/ui/screens/tokens/` |
| `shared_token()` | OnceCell accessor; deploys token contract | Same as above | `TokenTask::RegisterTokenContract` in `src/backend_task/token/mod.rs` |
| `SharedDashPayPair` struct | Two DashPay-keyed identities with usernames | TC-031..TC-044 | DashPay contact model in `src/model/dashpay/` |
| `shared_dashpay_pair()` | OnceCell accessor; registers 2 identities + DPNS names | Same as above | `IdentityTask::RegisterDpnsName` in `src/backend_task/identity/mod.rs` |

**TODO annotations required:**
```
// TODO(production-reuse): This fixture duplicates identity registration logic from
//   `src/backend_task/identity/mod.rs::run_register_identity_task()`.
// Source basis: src/backend_task/identity/mod.rs:run_register_identity_task
// Staleness warning: Before extracting to production, diff against
//   `src/backend_task/identity/mod.rs:run_register_identity_task` — the original
//   may have changed since this helper was written (created 2026-04-08 based on commit XXXX)
```

### `framework/dashpay_helpers.rs`

| Function | Description | Used by TCs | Production parallel |
|------|-------------|-------------|---------------------|
| `create_dashpay_identity(ctx, wallet, seed_hash)` | Register identity with DashPay encryption/decryption keys | TC-031..TC-044 | `src/backend_task/identity/mod.rs::default_identity_key_specs()` + contract-bound key derivation |
| `get_dashpay_signing_key(qi)` | Extract the DashPay signing key from a QualifiedIdentity | TC-032, TC-037 | `src/backend_task/dashpay/mod.rs` — key selection logic |
| `get_encryption_key(qi)` | Extract encryption public key for contact requests | TC-037 | `src/backend_task/dashpay/mod.rs::run_send_contact_request()` |

**TODO annotations:** Reference `src/backend_task/identity/mod.rs:default_identity_key_specs` and `src/backend_task/dashpay/mod.rs:run_send_contact_request` as source basis.

### `framework/token_helpers.rs`

| Function | Description | Used by TCs | Production parallel |
|------|-------------|-------------|---------------------|
| `build_token_contract_registration(identity, signing_key)` | Build a token data contract with permissive minting/freeze/marketplace rules | TC-045 (via fixtures) | `src/ui/screens/tokens/register_token_screen.rs` — UI-driven contract construction |
| `mint_tokens(ctx, identity, contract, position, signing_key, amount)` | Mint tokens via `TokenTask::MintTokens` | TC-053, TC-055, TC-056, TC-058 | `src/backend_task/token/mod.rs::run_mint_tokens()` |

**TODO annotations:** Reference `src/backend_task/token/mod.rs:run_register_token_contract_task` and token rules construction.

### `framework/mnlist_helpers.rs`

| Function | Description | Used by TCs | Production parallel |
|------|-------------|-------------|---------------------|
| `get_current_block_info(ctx)` | Get tip height + hash from SPV chain state | TC-068..TC-073 | `src/spv/mod.rs` — chain tip access |
| `get_block_hash_at_height(ctx, height)` | Look up block hash at a given height from SPV | TC-068, TC-071 | `src/spv/mod.rs` — block header store |

**TODO annotations:** Reference `src/spv/mod.rs` chain state accessors.

### `framework/shielded_helpers.rs`

| Function | Description | Used by TCs | Production parallel |
|------|-------------|-------------|---------------------|
| `skip_if_shielded_disabled()` | Check `E2E_SKIP_SHIELDED` env var, return early if set | TC-074..TC-083 | N/A (test-only) |
| `warm_up_and_init(ctx, seed_hash)` | Run WarmUpProvingKey + InitializeShieldedWallet in sequence | TC-078..TC-082 | `src/backend_task/shielded/mod.rs` — proving key + init flow |

**TODO annotations:** Reference `src/backend_task/shielded/mod.rs:run_warm_up_proving_key` and `run_initialize_shielded_wallet`.

---

## 6. Risk Assessment

### High-risk test groups (most likely to flake)

| Group | Risk | Flake vector | Mitigation |
|-------|------|--------------|------------|
| **ShieldedTask** | HIGH | ZK proof generation is compute-intensive (30-60s per proof). Proving key download may timeout. Network propagation of shielded STs is slow. | `E2E_SKIP_SHIELDED` env var. 90s per-test timeout. Run last in test ordering. |
| **DashPayTask** | HIGH | Contact request flow is multi-step with network propagation between steps. DashPay keys must be properly contract-bound. Profile updates may not be immediately queryable. | Add 5s sleep between send-request and load-requests. Use `tokio::time::timeout` with 120s for multi-step flows. |
| **TokenTask** | MEDIUM | Token contract registration is expensive (~60s). Freeze/unfreeze/pause/resume depend on specific contract rules matching the identity. Marketplace operations need proper pricing configuration. | Careful contract rule construction in `token_helpers`. Use SHARED_TOKEN to amortize registration cost. |
| **MnListTask** | MEDIUM | P2P connections may be flaky. Block hash lookups depend on SPV having sufficient chain history. | Retry wrapper for P2P calls (up to 3 attempts with 5s backoff). Use recent blocks (tip - 100) to ensure data availability. |
| **WalletTask** | MEDIUM | Platform address funding requires asset lock proofs, which depend on chain confirmation. Credits take time to appear. | `wait_for_platform_credits` helper with 120s timeout (if needed, can be added inline). |
| **CoreTask** | LOW | Most are read-only queries. Asset lock creation depends on wallet having spendable UTXOs. | Framework wallet guaranteed funded (10+ tDASH). |
| **IdentityTask** | LOW | SHARED_IDENTITY amortizes setup cost. Most tests are re-fetch + verify. | OnceCell ensures identity exists. |
| **BroadcastST** | LOW | Building valid STs requires correct nonce. | Fetch nonce from Platform before building ST. |

### Retry/skip strategies

1. **Environment-gated tests**: `E2E_SKIP_SHIELDED`, `E2E_CORE_RPC_URL` — allow CI to skip expensive or infra-dependent tests.
2. **Timeout per test**: The `#[tokio_shared_rt::test]` macro does not support built-in timeout. Use `tokio::time::timeout(Duration::from_secs(300), async { ... })` wrapping the entire test body for tests over 60s expected runtime.
3. **No automatic retries**: Tests are `#[ignore]` and run manually. Retries add non-determinism. Instead, ensure each test is idempotent and uses unique on-chain identifiers (random wallet seeds).
4. **Cleanup resilience**: The existing `cleanup_test_wallets` in harness handles orphaned wallets. New tests creating identities/tokens leave them on-chain (immutable) but return credits via `WithdrawFromIdentity` where possible.

### Total estimated runtime

| Group | Tests | Est. time |
|-------|-------|-----------|
| Framework init (SPV sync) | — | ~120s |
| CoreTask | 11 | ~135s |
| WalletTask | 8 | ~200s |
| IdentityTask | 11 | ~300s |
| DashPayTask | 14 | ~350s |
| TokenTask | 21 | ~550s |
| BroadcastST | 2 | ~35s |
| MnListTask | 6 | ~125s |
| ShieldedTask | 10 | ~500s |
| **Total** | **83** | **~38 min** |

This is within the 45-minute budget specified in acceptance criteria. The first run with proving key download may exceed this; subsequent runs will be faster.

---

## Appendix: Task Checklist for Agent Prompts

Each agent prompt for Tasks 1-8 should include:

1. The full test spec (TC-IDs) from `test-specs.md` for their group
2. The file path to create and its module declaration (already in `main.rs`)
3. The list of imports from framework helpers
4. The test function boilerplate:
   ```rust
   #[ignore]
   #[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
   async fn test_name() { ... }
   ```
5. Assertion patterns for each `BackendTaskSuccessResult` variant
6. Instruction to run `cargo clippy` and `cargo +nightly fmt` before finishing
7. Instruction NOT to modify any production code
