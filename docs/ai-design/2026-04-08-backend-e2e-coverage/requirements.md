# Backend E2E Test Coverage — Requirements & Scope

**Date:** 2026-04-08  
**Scope:** Extending `tests/backend-e2e/` to cover all BackendTask variant groups  
**Constraint:** Test-only changes, SPV/testnet mode, existing framework

---

## 1. Testable Scope

### 1.1 CoreTask (12 variants — 1 currently tested)

| Variant | Status | Notes |
|---|---|---|
| `SendWalletPayment` | ✅ Tested | Covered by `send_funds.rs` and harness internals |
| `RefreshWalletInfo` | ✅ Testable | Call on a wallet with known balance, verify no error |
| `RefreshSingleKeyWalletInfo` | ✅ Testable | Requires a `SingleKeyWallet` fixture |
| `CreateRegistrationAssetLock` | ✅ Testable | Prerequisite for identity creation (partially exercised by `identity_create.rs`) |
| `CreateTopUpAssetLock` | ✅ Testable | Requires an existing identity for the top-up index |
| `RecoverAssetLocks` | ✅ Testable | Can test with a wallet that has existing UTXOs |
| `ListCoreWallets` | ⚠️ Testable with caveats | Calls Core RPC `listwallets`; SPV mode uses SPV, not Core RPC. Will fail if Core RPC is unavailable. Must gracefully handle the "no Core RPC" case. |
| `GetBestChainLock` | ✅ Testable | Read-only query; dead_code attr but callable |
| `GetBestChainLocks` | ✅ Testable | Read-only query across networks |
| `SendSingleKeyWalletPayment` | ✅ Testable | Requires a funded `SingleKeyWallet` |
| `StartDashQT` | ❌ Not testable | Spawns external process (Dash Qt GUI binary). No headless equivalent. |
| `MineBlocks` | ❌ Not testable | Regtest/Devnet only. Testnet does not accept `generate` RPC commands. |

**Net testable:** 10 variants (add 9 new)

---

### 1.2 WalletTask (6 variants — 0 currently tested)

| Variant | Status | Notes |
|---|---|---|
| `GenerateReceiveAddress` | ✅ Testable | Pure key derivation + DB write, no network |
| `FetchPlatformAddressBalances` | ✅ Testable | Platform read query after funding a platform address |
| `FundPlatformAddressFromWalletUtxos` | ✅ Testable | Creates asset lock, waits for proof, funds address — requires funded wallet |
| `FundPlatformAddressFromAssetLock` | ✅ Testable | Requires a pre-built asset lock proof (use `CoreTask::CreateRegistrationAssetLock` first) |
| `TransferPlatformCredits` | ✅ Testable | Requires two funded platform addresses on the same wallet |
| `WithdrawFromPlatformAddress` | ✅ Testable | Reverse of fund; requires funded platform address |

**Net testable:** 6 variants (add 6 new)

---

### 1.3 IdentityTask (13 variants — 4 currently tested)

Currently tested: `RegisterIdentity`, `RegisterDpnsName`, `SearchIdentityByDpnsName`, `WithdrawFromIdentity`

| Variant | Status | Notes |
|---|---|---|
| `RegisterIdentity` | ✅ Tested | `identity_create.rs` |
| `TopUpIdentity` | ✅ Testable | Requires existing identity; top-up via wallet |
| `TopUpIdentityFromPlatformAddresses` | ✅ Testable | Requires funded platform address + existing identity |
| `AddKeyToIdentity` | ✅ Testable | Requires existing identity; derive a new key |
| `WithdrawFromIdentity` | ✅ Tested | `identity_withdraw.rs` |
| `Transfer` (credits to identity) | ✅ Testable | Requires two identities (sender, receiver) |
| `TransferToAddresses` | ✅ Testable | Requires identity with credits + platform address |
| `RegisterDpnsName` | ✅ Tested | `register_dpns.rs` |
| `RefreshIdentity` | ✅ Testable | Read-only refresh of existing identity state |
| `RefreshLoadedIdentitiesOwnedDPNSNames` | ✅ Testable | Requires identities loaded in DB |
| `LoadIdentity` | ✅ Testable | Load by ID + private key; already exercised indirectly |
| `SearchIdentityFromWallet` | ✅ Testable | dead_code attr but functional; search by wallet + index |
| `SearchIdentitiesUpToIndex` | ✅ Testable | Scans multiple identity indices in a wallet |
| `SearchIdentityByDpnsName` | ✅ Tested | `register_dpns.rs` |

**Net testable:** 13 variants (add 9 new)

---

### 1.4 DashPayTask (14 variants — 0 currently tested)

All DashPay variants require:
- A registered identity with **DashPay keys** (encryption/decryption keys, contract-bound)
- DashPay contract deployed on testnet (it is — it's a system contract)

| Variant | Status | Notes |
|---|---|---|
| `LoadProfile` | ✅ Testable | Read-only; works with any DashPay-registered identity |
| `UpdateProfile` | ✅ Testable | Requires identity + DashPay signing key |
| `SearchProfiles` | ✅ Testable | Read-only DPNS+DashPay query |
| `LoadContacts` | ✅ Testable | Read-only; empty result is valid |
| `LoadContactRequests` | ✅ Testable | Read-only; empty result is valid |
| `FetchContactProfile` | ✅ Testable | Requires known contact identity ID |
| `SendContactRequest` | ✅ Testable | Requires two identities (sender, receiver) |
| `AcceptContactRequest` | ✅ Testable | Requires a pending incoming contact request |
| `RejectContactRequest` | ✅ Testable | Requires a pending incoming contact request |
| `LoadPaymentHistory` | ✅ Testable | Read-only; empty result is valid |
| `RegisterDashPayAddresses` | ✅ Testable | Derives and registers extended public keys |
| `UpdateContactInfo` | ⚠️ Testable with caveats | Local DB write + optional network update; requires established contact |
| `SendPaymentToContact` | ⚠️ Testable with caveats | Requires two established DashPay contacts with payment channels; complex to set up |
| `SendContactRequestWithProof` | ⚠️ Testable with caveats | Requires building a valid `AutoAcceptProofData`; complex internal type |

**Net testable:** 14 variants (11 straightforward, 3 complex)

---

### 1.5 TokenTask (21 variants — 0 currently tested)

All token state-mutation variants require a registered token contract owned by a test identity.

| Variant | Status | Notes |
|---|---|---|
| `RegisterTokenContract` | ✅ Testable | Creates + broadcasts a new token data contract |
| `QueryMyTokenBalances` | ✅ Testable | Read-only; requires identities in DB |
| `QueryIdentityTokenBalance` | ✅ Testable | Read-only |
| `QueryDescriptionsByKeyword` | ✅ Testable | Read-only keyword search |
| `FetchTokenByContractId` | ✅ Testable | Read-only; use a known testnet token contract ID |
| `FetchTokenByTokenId` | ✅ Testable | Read-only; use known token ID |
| `SaveTokenLocally` | ✅ Testable | Pure DB write; no network |
| `QueryTokenPricing` | ✅ Testable | Read-only |
| `MintTokens` | ✅ Testable | Requires token contract with minting rules permitting identity |
| `BurnTokens` | ✅ Testable | Requires minted token balance |
| `TransferTokens` | ✅ Testable | Requires token balance + recipient identity |
| `FreezeTokens` | ✅ Testable | Requires freeze rules + target identity |
| `UnfreezeTokens` | ✅ Testable | Requires previously frozen identity |
| `DestroyFrozenFunds` | ✅ Testable | Requires frozen identity with balance |
| `PauseTokens` | ✅ Testable | Requires pause rules |
| `ResumeTokens` | ✅ Testable | Requires paused token |
| `ClaimTokens` | ⚠️ Testable with caveats | Requires distribution rules with claimable rewards; perpetual distribution needs time to accrue |
| `EstimatePerpetualTokenRewardsWithExplanation` | ⚠️ Testable with caveats | Read-only but needs a token with perpetual distribution configured |
| `UpdateTokenConfig` | ✅ Testable | Requires a config change rule that permits the identity |
| `PurchaseTokens` | ✅ Testable | Requires token with marketplace trade mode enabled + priced |
| `SetDirectPurchasePrice` | ✅ Testable | Requires token with marketplace rules permitting identity |

**Net testable:** 21 variants (19 straightforward, 2 complex)

---

### 1.6 BroadcastStateTransition (1 variant — 0 currently tested)

| Variant | Status | Notes |
|---|---|---|
| `BroadcastStateTransition` | ✅ Testable | Build a minimal valid ST (e.g. identity update with a new key nonce) and broadcast directly |

---

### 1.7 MnListTask (5 variants — 0 currently tested)

All MnList variants use `CoreP2PHandler` for P2P connections (no SPV, no Core RPC) except `FetchChainLocks`.

| Variant | Status | Notes |
|---|---|---|
| `FetchEndDmlDiff` | ✅ Testable | Needs two known testnet block hashes/heights |
| `FetchEndQrInfo` | ✅ Testable | Needs known block hashes |
| `FetchEndQrInfoWithDmls` | ✅ Testable | Same as above |
| `FetchChainLocks` | ⚠️ Testable with caveats | Calls Core RPC `getblockbyhash`; requires Core RPC connectivity. Skip in pure-SPV environments. |
| `FetchDiffsChain` | ✅ Testable | Needs a chain of known (height, hash) pairs |

**Net testable:** 4 full + 1 conditional  
**Block hashes:** Must be obtained at test runtime via SPV chain data or hardcoded testnet constants.

---

### 1.8 ShieldedTask (9 variants — 0 currently tested)

Shielded tasks require the shielded pool feature. All depend on `InitializeShieldedWallet` first.

| Variant | Status | Notes |
|---|---|---|
| `WarmUpProvingKey` | ✅ Testable | Background ZK key warmup; ~30s; no wallet needed |
| `InitializeShieldedWallet` | ✅ Testable | Key derivation + DB init |
| `SyncNotes` | ✅ Testable | Trial decrypt from Platform; may return 0 notes |
| `CheckNullifiers` | ✅ Testable | Platform read; may return empty |
| `ShieldFromAssetLock` | ✅ Testable | Locks Core DASH, shields directly; requires funded wallet |
| `ShieldCredits` | ✅ Testable | Shields from platform address; requires funded platform address |
| `ShieldedTransfer` | ✅ Testable | Internal shielded pool transfer; requires shielded balance |
| `UnshieldCredits` | ✅ Testable | Unshield to platform address; requires shielded balance |
| `ShieldedWithdrawal` | ✅ Testable | Shielded pool to Core L1 address; requires shielded balance |

**Net testable:** 9 variants  
**Note:** Full shielded chain (`ShieldFromAssetLock` → `ShieldedTransfer` → `UnshieldCredits` → `ShieldedWithdrawal`) is the intended integration path. ZK proving is compute-intensive (~30-60s per proof).

---

## 2. Test Dependencies

```
SPV sync + framework wallet (always first)
    │
    ├── CoreTask tests
    │       └── SendWalletPayment → already tested
    │       └── RefreshWalletInfo, RefreshSingleKeyWalletInfo, RecoverAssetLocks
    │           └── funded test wallet required
    │
    ├── WalletTask tests
    │       └── GenerateReceiveAddress (no deps)
    │       └── FundPlatformAddressFromWalletUtxos (funded wallet)
    │           └── FetchPlatformAddressBalances
    │               └── TransferPlatformCredits (two funded platform addresses)
    │               └── WithdrawFromPlatformAddress
    │       └── FundPlatformAddressFromAssetLock (CoreTask::CreateRegistrationAssetLock first)
    │
    ├── IdentityTask tests
    │       └── RegisterIdentity (funded wallet + asset lock)
    │           └── TopUpIdentity, AddKeyToIdentity, RefreshIdentity
    │           └── RegisterDpnsName → SearchIdentityByDpnsName (already tested)
    │           └── Transfer (two identities)
    │           └── TransferToAddresses (funded platform address)
    │           └── WithdrawFromIdentity (already tested)
    │           └── TopUpIdentityFromPlatformAddresses (funded platform address)
    │
    ├── DashPayTask tests
    │       └── RegisterIdentity with DashPay keys
    │           └── UpdateProfile, LoadProfile, SearchProfiles
    │           └── RegisterDashPayAddresses
    │           └── SendContactRequest (two DashPay identities)
    │               └── LoadContactRequests (receiver)
    │               └── AcceptContactRequest → LoadContacts
    │               └── RejectContactRequest
    │               └── UpdateContactInfo
    │               └── SendPaymentToContact (established contact)
    │
    ├── TokenTask tests
    │       └── RegisterIdentity (funded)
    │           └── RegisterTokenContract (identity as owner, minting rules permitting identity)
    │               └── QueryMyTokenBalances, FetchTokenByContractId, FetchTokenByTokenId
    │               └── MintTokens
    │                   └── BurnTokens, TransferTokens, FreezeTokens
    │                   └── FreezeTokens → UnfreezeTokens
    │                   └── FreezeTokens → DestroyFrozenFunds
    │                   └── PauseTokens → ResumeTokens
    │                   └── SetDirectPurchasePrice → PurchaseTokens (second identity)
    │               └── UpdateTokenConfig
    │               └── ClaimTokens (if distribution configured)
    │
    ├── MnListTask tests
    │       └── SPV synced (for chain height/hash data)
    │           └── FetchEndDmlDiff, FetchEndQrInfo, FetchEndQrInfoWithDmls, FetchDiffsChain
    │
    └── ShieldedTask tests
            └── WarmUpProvingKey (no deps, run first for perf)
            └── funded wallet
                └── InitializeShieldedWallet
                    └── SyncNotes, CheckNullifiers
                    └── ShieldFromAssetLock → ShieldedTransfer → UnshieldCredits/ShieldedWithdrawal
                    └── FundPlatformAddress → ShieldCredits → (same as above)
```

---

## 3. Framework Helpers Needed

### New helper functions

1. **`identity_helpers::create_dashpay_identity(ctx, funded_wallet) -> QualifiedIdentity`**  
   Registers an identity that includes encryption/decryption keys bound to the DashPay contract. Builds on the existing `build_identity_registration` helper.

2. **`identity_helpers::get_or_create_dpns_identity(ctx, wallet, name) -> QualifiedIdentity`**  
   Convenience: register identity + DPNS name in one call. Useful for DashPay tests.

3. **`wait::wait_for_platform_credits(ctx, wallet_hash, address, min_credits, timeout) -> u64`**  
   Polls `WalletTask::FetchPlatformAddressBalances` until the address has at least `min_credits`. Mirrors `wait_for_spendable_balance` but for platform addresses.

4. **`token_helpers` module**  
   - `register_test_token(ctx, identity, signing_key) -> (Arc<DataContract>, TokenContractPosition)`  
     Registers a simple token with owner-permissive minting/freeze rules. Returns contract + position for use in subsequent token operation tests.
   - `mint_test_tokens(ctx, identity, signing_key, contract, amount) -> FeeResult`

5. **`mnlist_helpers::get_current_block_info(ctx) -> (u32, BlockHash)`**  
   Retrieves current testnet tip height and hash from SPV state, for use in MnList requests.

6. **`shielded_helpers::warm_up_and_init(ctx, wallet_hash)`**  
   Runs `WarmUpProvingKey` + `InitializeShieldedWallet` in sequence, ensuring proving key is ready before any shielded operation.

### Shared test fixtures (lazy/once-initialized)

Some test groups share expensive setup (registered identity, deployed token contract). Introduce `once_cell`-based shared fixtures in a `fixtures` module:

- `SHARED_IDENTITY: OnceCell<QualifiedIdentity>` — a single identity reused across identity/DashPay/token tests where mutation is not expected
- `SHARED_TOKEN: OnceCell<(Arc<DataContract>, TokenContractPosition)>` — token contract for query-only token tests

---

## 4. Environment Requirements

### Existing requirements (unchanged)
- `E2E_WALLET_MNEMONIC` — pre-funded testnet wallet with ≥10 tDASH
- Live Dash testnet + Platform connectivity
- `--test-threads=1` (serial execution)

### Additional requirements for new test groups

| Requirement | Needed by | Notes |
|---|---|---|
| ≥20 tDASH in framework wallet | ShieldedTask, TokenTask, DashPayTask | ZK operations + multiple identities/tokens are expensive in credits |
| Testnet DashPay contract deployed | DashPayTask | Always true on official testnet |
| Platform has fee headroom | TokenTask (mutating ops) | Testnet occasionally has credit shortages; tests may flake |
| Core RPC accessible | MnListTask::FetchChainLocks, CoreTask::ListCoreWallets | Only if those variants are included; mark as skip-if-unavailable |
| ZK proving key download | ShieldedTask | `WarmUpProvingKey` may download ~100MB proving key on first run; requires internet + disk space |

### Optional environment variables (new)

- `E2E_SKIP_SHIELDED=1` — skip all ShieldedTask tests (they are slow and ZK-compute-heavy)
- `E2E_SKIP_DASHPAY=1` — skip DashPayTask tests (they require maintaining DashPay-keyed identities)
- `E2E_CORE_RPC_URL` — if set, enables MnListTask::FetchChainLocks and CoreTask::ListCoreWallets tests

---

## 5. Exclusions

| Variant | Reason |
|---|---|
| `CoreTask::StartDashQT` | Spawns an external GUI process. No headless equivalent; cannot assert success without process inspection. |
| `CoreTask::MineBlocks` | Only valid on Regtest/Devnet. Broadcasting a `generate` command to testnet will fail. |
| `MnListTask::FetchChainLocks` (conditional) | Requires Core RPC (`getblockbyhash`). Excluded from default run; only enabled when `E2E_CORE_RPC_URL` is set. |
| `CoreTask::ListCoreWallets` (conditional) | Calls `listwallets` via Core RPC. SPV mode has no Core RPC. Same condition as above. |
| `BackendTask::SwitchNetwork` | Infrastructure-level task that replaces the entire `AppContext`. Incompatible with the singleton harness model; testing it would destroy the shared context. |
| `BackendTask::ReinitCoreClientAndSdk` | Requires valid Core RPC credentials. Tests run in SPV mode. |
| `BackendTask::DiscoverDapiNodes` | Network-level bootstrap; not a user feature. Already verified implicitly (DAPI discovery runs during `AppContext::new`). |
| `BackendTask::SystemTask` | Contains theme preferences and other GUI-state tasks. No observable network behavior. |
| `BackendTask::GroveSTARKTask` | Experimental ZK proof generation. Not production-ready; requires specific proof data inputs. Exclude from initial coverage. |
| `BackendTask::ContestedResourceTask` | DPNS contest voting. Requires active contests on testnet and a voting identity (masternode). Non-deterministic testnet state makes reliable assertions difficult. Defer. |
| `BackendTask::DocumentTask` | Requires a deployed data contract with documents. Testable in principle but low priority given no existing contract fixture. Defer. |
| `BackendTask::ContractTask` | Contract registration is covered implicitly by `TokenTask::RegisterTokenContract`. Dedicated contract tests (update, fetch nonce) are lower priority. Defer. |
| `DashPayTask::SendContactRequestWithProof` | Requires constructing valid `AutoAcceptProofData` (internal type with cryptographic proof). Needs dedicated proof-construction helper that doesn't exist yet. Defer to Phase 2. |
| `TokenTask::ClaimTokens` (perpetual distribution) | Requires a token with a perpetual distribution schedule that has accrued rewards. Testnet timing makes this non-deterministic. Skip unless a known token with claimable rewards exists. |

---

## 6. Acceptance Criteria

**"Full coverage" is defined as:**

1. **Every non-excluded variant has at least one test** that:
   - Invokes the variant via `run_task()`
   - Asserts the correct `BackendTaskSuccessResult` variant is returned
   - Does not panic on the happy path against a live testnet

2. **Mutation tests verify observable side effects**, not just return values:
   - Identity operations: re-fetch identity from Platform and confirm change
   - Token operations: re-query token balance/state after mutation
   - DashPay operations: re-query contact requests/profile after mutation
   - Fund transfers: verify balance change via wait helpers

3. **All tests pass independently** when run in isolation (not only as part of a sequence)
   - Each test sets up its own prerequisites (funded wallet, identity, etc.) or uses shared fixtures via `OnceCell`

4. **Error paths have at least one test per group:**
   - One test per group verifies that an invalid input produces a typed `TaskError` variant (not a panic)

5. **Coverage metric:**
   - CoreTask: 10/12 (2 excluded)
   - WalletTask: 6/6
   - IdentityTask: 13/13
   - DashPayTask: 11/14 (3 deferred)
   - TokenTask: 19/21 (2 conditional)
   - BroadcastStateTransition: 1/1
   - MnListTask: 4/5 (1 conditional)
   - ShieldedTask: 9/9

6. **Test runtime budget:** individual tests must complete within 5 minutes. The full suite must complete within 45 minutes on a reasonable internet connection.

7. **No test pollutes global state** — each test that creates identities/tokens on-chain uses its own test wallet and cleans up credits (via `WithdrawFromIdentity` or by funding a wallet that gets swept by `cleanup_test_wallets`).

---

## Appendix: Variant Count Summary

| Group | Total | Testable | Excluded | New tests needed |
|---|---|---|---|---|
| CoreTask | 12 | 10 | 2 | 9 |
| WalletTask | 6 | 6 | 0 | 6 |
| IdentityTask | 13 | 13 | 0 | 9 |
| DashPayTask | 14 | 11+3 deferred | 0 | 11 (14 eventually) |
| TokenTask | 21 | 19+2 conditional | 0 | 19 (21 eventually) |
| BroadcastStateTransition | 1 | 1 | 0 | 1 |
| MnListTask | 5 | 4+1 conditional | 0 | 4 (5 conditionally) |
| ShieldedTask | 9 | 9 | 0 | 9 |
| **Total** | **81** | **73+6 deferred/conditional** | **2** | **68** |
