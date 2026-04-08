# Backend E2E Test Case Specifications

**Date:** 2026-04-08
**Target:** ~68 test cases across 8 BackendTask groups
**Framework:** `tests/backend-e2e/`, `#[tokio_shared_rt::test(shared)]`, `#[ignore]`, serial execution

---

## 1. CoreTask Tests (`core_tasks.rs`)

### TC-001: RefreshWalletInfo — Core only
- **BackendTask variant**: `BackendTask::CoreTask(CoreTask::RefreshWalletInfo(wallet, false))`
- **Group**: CoreTask
- **Preconditions**: Framework wallet restored and SPV-synced with known balance
- **Steps**:
  1. Setup: obtain `Arc<RwLock<Wallet>>` from harness
  2. Execute: `run_task(ctx, BackendTask::CoreTask(CoreTask::RefreshWalletInfo(wallet.clone(), false)))`
  3. Assert: result matches `BackendTaskSuccessResult::RefreshedWallet { warning }`, warning is `None`
  4. Assert: wallet balance (read lock) is > 0 and matches pre-refresh known balance
- **Expected outcome**: `RefreshedWallet { warning: None }`
- **Shared fixture dependency**: Framework wallet (harness)
- **Estimated runtime**: 5s

### TC-002: RefreshWalletInfo — Core + Platform
- **BackendTask variant**: `BackendTask::CoreTask(CoreTask::RefreshWalletInfo(wallet, true))`
- **Group**: CoreTask
- **Preconditions**: Framework wallet restored, SPV-synced
- **Steps**:
  1. Setup: obtain wallet from harness
  2. Execute: `run_task(ctx, BackendTask::CoreTask(CoreTask::RefreshWalletInfo(wallet.clone(), true)))`
  3. Assert: result matches `RefreshedWallet { .. }` (warning may or may not be present depending on Platform state)
- **Expected outcome**: `RefreshedWallet { .. }` without panic
- **Shared fixture dependency**: Framework wallet
- **Estimated runtime**: 10s

### TC-003: RefreshSingleKeyWalletInfo
- **BackendTask variant**: `BackendTask::CoreTask(CoreTask::RefreshSingleKeyWalletInfo(skw))`
- **Group**: CoreTask
- **Preconditions**: A `SingleKeyWallet` created from a known private key and registered in the database
- **Steps**:
  1. Setup: create a `SingleKeyWallet` from a test private key, insert into DB
  2. Execute: `run_task(ctx, BackendTask::CoreTask(CoreTask::RefreshSingleKeyWalletInfo(skw.clone())))`
  3. Assert: result matches `RefreshedWallet { .. }`
  4. Assert: wallet's `balance()` is a valid amount (may be 0 for a fresh key)
- **Expected outcome**: `RefreshedWallet { .. }`
- **Shared fixture dependency**: None (creates its own fixture)
- **Estimated runtime**: 5s

### TC-004: CreateRegistrationAssetLock
- **BackendTask variant**: `BackendTask::CoreTask(CoreTask::CreateRegistrationAssetLock(wallet, credits, identity_index))`
- **Group**: CoreTask
- **Preconditions**: Funded framework wallet with >= 0.01 tDASH
- **Steps**:
  1. Setup: obtain wallet, choose `identity_index = 99` (unused index to avoid collision)
  2. Execute: `run_task(ctx, BackendTask::CoreTask(CoreTask::CreateRegistrationAssetLock(wallet, 100_000_000, 99)))`
  3. Assert: result matches `CoreItem(CoreItem::InstantLockedTransaction(tx, outputs, islock))`
  4. Assert: `tx` has at least one output, `outputs` is non-empty, `islock` signature is non-zero
- **Expected outcome**: `CoreItem(CoreItem::InstantLockedTransaction(...))`
- **Shared fixture dependency**: Framework wallet
- **Estimated runtime**: 30s

### TC-005: CreateTopUpAssetLock
- **BackendTask variant**: `BackendTask::CoreTask(CoreTask::CreateTopUpAssetLock(wallet, credits, identity_index, topup_index))`
- **Group**: CoreTask
- **Preconditions**: Funded framework wallet, existing identity at index 0
- **Steps**:
  1. Setup: obtain wallet, use identity_index=0 (SHARED_IDENTITY's index), topup_index=1
  2. Execute: `run_task(ctx, BackendTask::CoreTask(CoreTask::CreateTopUpAssetLock(wallet, 50_000_000, 0, 1)))`
  3. Assert: result matches `CoreItem(CoreItem::InstantLockedTransaction(tx, outputs, islock))`
  4. Assert: transaction output value approximately matches requested credits converted to duffs
- **Expected outcome**: `CoreItem(CoreItem::InstantLockedTransaction(...))`
- **Shared fixture dependency**: Framework wallet, SHARED_IDENTITY (for valid index)
- **Estimated runtime**: 30s

### TC-006: RecoverAssetLocks
- **BackendTask variant**: `BackendTask::CoreTask(CoreTask::RecoverAssetLocks(wallet))`
- **Group**: CoreTask
- **Preconditions**: Funded framework wallet with existing UTXOs
- **Steps**:
  1. Setup: obtain wallet from harness
  2. Execute: `run_task(ctx, BackendTask::CoreTask(CoreTask::RecoverAssetLocks(wallet.clone())))`
  3. Assert: result matches `RecoveredAssetLocks { recovered_count, total_amount }`
  4. Assert: `recovered_count` >= 0 (may be 0 if no asset locks exist), `total_amount` >= 0
- **Expected outcome**: `RecoveredAssetLocks { .. }` (0 recoveries is valid)
- **Shared fixture dependency**: Framework wallet
- **Estimated runtime**: 10s

### TC-007: GetBestChainLock
- **BackendTask variant**: `BackendTask::CoreTask(CoreTask::GetBestChainLock)`
- **Group**: CoreTask
- **Preconditions**: SPV synced
- **Steps**:
  1. Execute: `run_task(ctx, BackendTask::CoreTask(CoreTask::GetBestChainLock))`
  2. Assert: result matches `CoreItem(CoreItem::ChainLock(cl, network))`
  3. Assert: `cl.block_height > 0`, `network` matches testnet
- **Expected outcome**: `CoreItem(CoreItem::ChainLock(...))`
- **Shared fixture dependency**: None
- **Estimated runtime**: 5s

### TC-008: GetBestChainLocks
- **BackendTask variant**: `BackendTask::CoreTask(CoreTask::GetBestChainLocks)`
- **Group**: CoreTask
- **Preconditions**: SPV synced
- **Steps**:
  1. Execute: `run_task(ctx, BackendTask::CoreTask(CoreTask::GetBestChainLocks))`
  2. Assert: result matches `CoreItem(CoreItem::ChainLocks(testnet_cl, mainnet_cl))`
  3. Assert: testnet chain lock is `Some` with `block_height > 0`
- **Expected outcome**: `CoreItem(CoreItem::ChainLocks(Some(..), ..))`
- **Shared fixture dependency**: None
- **Estimated runtime**: 5s

### TC-009: SendSingleKeyWalletPayment
- **BackendTask variant**: `BackendTask::CoreTask(CoreTask::SendSingleKeyWalletPayment { wallet, request })`
- **Group**: CoreTask
- **Preconditions**: A funded `SingleKeyWallet` (requires pre-funding or skip if unfunded)
- **Steps**:
  1. Setup: create `SingleKeyWallet` from a funded test key, build `WalletPaymentRequest` with a small amount (1000 duffs) to a known test address
  2. Execute: `run_task(ctx, BackendTask::CoreTask(CoreTask::SendSingleKeyWalletPayment { wallet: skw, request }))`
  3. Assert: result matches `WalletPayment { txid, recipients, total_amount }`
  4. Assert: `txid` is a valid 64-char hex string, `total_amount` matches requested amount
- **Expected outcome**: `WalletPayment { .. }`
- **Shared fixture dependency**: None (requires its own funded key)
- **Estimated runtime**: 30s

### TC-010: ListCoreWallets (conditional)
- **BackendTask variant**: `BackendTask::CoreTask(CoreTask::ListCoreWallets)`
- **Group**: CoreTask
- **Preconditions**: `E2E_CORE_RPC_URL` environment variable set
- **Steps**:
  1. Guard: skip if `E2E_CORE_RPC_URL` is not set
  2. Execute: `run_task(ctx, BackendTask::CoreTask(CoreTask::ListCoreWallets))`
  3. Assert: result matches `CoreWalletsList(wallets)` where `wallets` is a `Vec<String>`
- **Expected outcome**: `CoreWalletsList(Vec<String>)`
- **Shared fixture dependency**: None
- **Estimated runtime**: 3s

### TC-011: CoreTask error — invalid payment address
- **BackendTask variant**: `BackendTask::CoreTask(CoreTask::SendWalletPayment { .. })`
- **Group**: CoreTask (error path)
- **Preconditions**: Framework wallet
- **Steps**:
  1. Setup: build `WalletPaymentRequest` with an invalid/malformed address string
  2. Execute: `run_task(ctx, BackendTask::CoreTask(CoreTask::SendWalletPayment { wallet, request }))`
  3. Assert: result is `Err(TaskError::...)` — not a panic
  4. Assert: the error is a typed `TaskError` variant (e.g. `WalletError` or `AddressError`)
- **Expected outcome**: `Err(TaskError::...)` with specific variant
- **Shared fixture dependency**: Framework wallet
- **Estimated runtime**: 2s

---

## 2. WalletTask Tests (`wallet_tasks.rs`)

### TC-012: GenerateReceiveAddress
- **BackendTask variant**: `BackendTask::WalletTask(WalletTask::GenerateReceiveAddress { seed_hash })`
- **Group**: WalletTask
- **Preconditions**: Framework wallet loaded in context
- **Steps**:
  1. Setup: obtain `seed_hash` from framework wallet
  2. Execute: `run_task(ctx, BackendTask::WalletTask(WalletTask::GenerateReceiveAddress { seed_hash }))`
  3. Assert: result matches `GeneratedReceiveAddress { seed_hash: sh, address }` where `sh == seed_hash`
  4. Assert: `address` is a valid Dash testnet address (starts with `y` or `8`)
  5. Execute again: second call returns a different address (key derivation advances)
- **Expected outcome**: `GeneratedReceiveAddress { .. }` with valid testnet address
- **Shared fixture dependency**: Framework wallet
- **Estimated runtime**: 2s

### TC-013: FetchPlatformAddressBalances — no platform addresses
- **BackendTask variant**: `BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances { seed_hash })`
- **Group**: WalletTask
- **Preconditions**: Framework wallet with no platform addresses funded
- **Steps**:
  1. Setup: obtain `seed_hash` from framework wallet
  2. Execute: `run_task(ctx, BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances { seed_hash }))`
  3. Assert: result matches `PlatformAddressBalances { seed_hash: sh, balances, network }`
  4. Assert: `balances` may be empty or have zero-balance entries, `network` is testnet
- **Expected outcome**: `PlatformAddressBalances { .. }`
- **Shared fixture dependency**: Framework wallet
- **Estimated runtime**: 5s

### TC-014: FundPlatformAddressFromWalletUtxos
- **BackendTask variant**: `BackendTask::WalletTask(WalletTask::FundPlatformAddressFromWalletUtxos { .. })`
- **Group**: WalletTask
- **Preconditions**: Funded framework wallet with >= 0.01 tDASH
- **Steps**:
  1. Setup: derive a `PlatformAddress` from the wallet at index 0, set `amount = 1_000_000` duffs (0.01 DASH)
  2. Execute: `run_task(ctx, BackendTask::WalletTask(WalletTask::FundPlatformAddressFromWalletUtxos { seed_hash, amount: 1_000_000, destination: platform_addr, fee_deduct_from_output: true }))`
  3. Assert: result matches `PlatformAddressFunded { seed_hash: sh }` where `sh == seed_hash`
  4. Verify: call `FetchPlatformAddressBalances` and confirm the funded address has credits > 0
- **Expected outcome**: `PlatformAddressFunded { .. }` + verifiable balance increase
- **Shared fixture dependency**: Framework wallet
- **Estimated runtime**: 60s (asset lock + wait for proof)

### TC-015: FetchPlatformAddressBalances — after funding
- **BackendTask variant**: `BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances { seed_hash })`
- **Group**: WalletTask
- **Preconditions**: TC-014 completed (platform address funded)
- **Steps**:
  1. Execute: `run_task(ctx, BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances { seed_hash }))`
  2. Assert: result matches `PlatformAddressBalances { balances, .. }`
  3. Assert: at least one entry in `balances` has credits > 0
  4. Assert: the funded address's balance is approximately `1_000_000 * 1000` credits (duffs to credits conversion)
- **Expected outcome**: `PlatformAddressBalances { .. }` with non-zero balance
- **Shared fixture dependency**: TC-014 output
- **Estimated runtime**: 5s

### TC-016: TransferPlatformCredits
- **BackendTask variant**: `BackendTask::WalletTask(WalletTask::TransferPlatformCredits { .. })`
- **Group**: WalletTask
- **Preconditions**: At least one funded platform address (from TC-014), a second platform address derived from the same wallet
- **Steps**:
  1. Setup: derive a second `PlatformAddress` at index 1; build `inputs` map with source address and half its balance; build `outputs` map with destination address
  2. Execute: `run_task(ctx, BackendTask::WalletTask(WalletTask::TransferPlatformCredits { seed_hash, inputs, outputs, fee_payer_index: 0 }))`
  3. Assert: result matches `PlatformCreditsTransferred { seed_hash: sh }`
  4. Verify: `FetchPlatformAddressBalances` shows both addresses with credits
- **Expected outcome**: `PlatformCreditsTransferred { .. }`
- **Shared fixture dependency**: TC-014 output
- **Estimated runtime**: 30s

### TC-017: WithdrawFromPlatformAddress
- **BackendTask variant**: `BackendTask::WalletTask(WalletTask::WithdrawFromPlatformAddress { .. })`
- **Group**: WalletTask
- **Preconditions**: Funded platform address (from TC-014/TC-016)
- **Steps**:
  1. Setup: build `inputs` with remaining platform address balance minus fee margin; derive `CoreScript` from a wallet receive address; set `core_fee_per_byte = 1`
  2. Execute: `run_task(ctx, BackendTask::WalletTask(WalletTask::WithdrawFromPlatformAddress { seed_hash, inputs, output_script, core_fee_per_byte: 1, fee_payer_index: 0 }))`
  3. Assert: result matches `PlatformAddressWithdrawal { seed_hash: sh }`
  4. Verify: `FetchPlatformAddressBalances` shows reduced balance on the source address
- **Expected outcome**: `PlatformAddressWithdrawal { .. }`
- **Shared fixture dependency**: Funded platform address
- **Estimated runtime**: 30s

### TC-018: FundPlatformAddressFromAssetLock
- **BackendTask variant**: `BackendTask::WalletTask(WalletTask::FundPlatformAddressFromAssetLock { .. })`
- **Group**: WalletTask
- **Preconditions**: A pre-built asset lock proof (from `CoreTask::CreateRegistrationAssetLock`)
- **Steps**:
  1. Setup: call `CoreTask::CreateRegistrationAssetLock` to get `(tx, outputs, islock)`, construct `AssetLockProof` from these
  2. Setup: derive a `PlatformAddress` at a fresh index
  3. Execute: `run_task(ctx, BackendTask::WalletTask(WalletTask::FundPlatformAddressFromAssetLock { seed_hash, asset_lock_proof: Box::new(proof), asset_lock_address, outputs }))`
  4. Assert: result matches `PlatformAddressFunded { .. }`
- **Expected outcome**: `PlatformAddressFunded { .. }`
- **Shared fixture dependency**: Framework wallet
- **Estimated runtime**: 60s

### TC-019: WalletTask error — unknown seed hash
- **BackendTask variant**: `BackendTask::WalletTask(WalletTask::GenerateReceiveAddress { seed_hash })`
- **Group**: WalletTask (error path)
- **Preconditions**: None
- **Steps**:
  1. Setup: construct a `WalletSeedHash` from arbitrary bytes (not matching any loaded wallet)
  2. Execute: `run_task(ctx, BackendTask::WalletTask(WalletTask::GenerateReceiveAddress { seed_hash: fake_hash }))`
  3. Assert: result is `Err(TaskError::...)` — a typed error, not a panic
- **Expected outcome**: `Err(TaskError::WalletNotFound)` or similar
- **Shared fixture dependency**: None
- **Estimated runtime**: 1s

---

## 3. IdentityTask Tests (`identity_tasks.rs`)

### TC-020: TopUpIdentity
- **BackendTask variant**: `BackendTask::IdentityTask(IdentityTask::TopUpIdentity(info))`
- **Group**: IdentityTask
- **Preconditions**: SHARED_IDENTITY registered, funded framework wallet
- **Steps**:
  1. Setup: build `IdentityTopUpInfo { qualified_identity: SHARED_IDENTITY, wallet, identity_funding_method: FundWithWallet(50_000_000, 0, 0) }`
  2. Record SHARED_IDENTITY's balance before top-up
  3. Execute: `run_task(ctx, BackendTask::IdentityTask(IdentityTask::TopUpIdentity(info)))`
  4. Assert: result matches `ToppedUpIdentity(qi, fee_result)` where `qi.identity.id() == SHARED_IDENTITY.id()`
  5. Assert: `fee_result.actual_fee > 0`
  6. Verify: re-fetch identity from Platform and confirm balance increased
- **Expected outcome**: `ToppedUpIdentity(_, FeeResult { .. })`
- **Shared fixture dependency**: SHARED_IDENTITY, Framework wallet
- **Estimated runtime**: 60s

### TC-021: TopUpIdentityFromPlatformAddresses
- **BackendTask variant**: `BackendTask::IdentityTask(IdentityTask::TopUpIdentityFromPlatformAddresses { .. })`
- **Group**: IdentityTask
- **Preconditions**: SHARED_IDENTITY, funded platform address (from WalletTask tests or separate setup)
- **Steps**:
  1. Setup: fund a platform address via `FundPlatformAddressFromWalletUtxos` if not already funded
  2. Build `inputs` map with platform address and desired credit amount
  3. Execute: `run_task(ctx, BackendTask::IdentityTask(IdentityTask::TopUpIdentityFromPlatformAddresses { identity: SHARED_IDENTITY, inputs, wallet_seed_hash }))`
  4. Assert: result matches `ToppedUpIdentity(qi, fee_result)`
  5. Verify: identity balance increased on Platform
- **Expected outcome**: `ToppedUpIdentity(_, FeeResult { .. })`
- **Shared fixture dependency**: SHARED_IDENTITY, funded platform address
- **Estimated runtime**: 60s

### TC-022: AddKeyToIdentity
- **BackendTask variant**: `BackendTask::IdentityTask(IdentityTask::AddKeyToIdentity(qi, key, private_key_bytes))`
- **Group**: IdentityTask
- **Preconditions**: SHARED_IDENTITY with sufficient credits
- **Steps**:
  1. Setup: derive a new `QualifiedIdentityPublicKey` (ECDSA_SECP256K1, AUTHENTICATION, HIGH)
  2. Generate a random 32-byte private key
  3. Execute: `run_task(ctx, BackendTask::IdentityTask(IdentityTask::AddKeyToIdentity(SHARED_IDENTITY.clone(), new_key, private_key_bytes)))`
  4. Assert: result matches `AddedKeyToIdentity(fee_result)` with `fee_result.actual_fee > 0`
  5. Verify: re-fetch identity, confirm new key exists in `identity.public_keys()`
- **Expected outcome**: `AddedKeyToIdentity(FeeResult { .. })`
- **Shared fixture dependency**: SHARED_IDENTITY
- **Estimated runtime**: 30s

### TC-023: Transfer (credits to another identity)
- **BackendTask variant**: `BackendTask::IdentityTask(IdentityTask::Transfer(qi, recipient_id, credits, key_id))`
- **Group**: IdentityTask
- **Preconditions**: SHARED_IDENTITY with credits, a second identity (use SHARED_DASHPAY_PAIR or create a new one)
- **Steps**:
  1. Setup: register or obtain a second identity (`recipient`)
  2. Record recipient's balance
  3. Execute: `run_task(ctx, BackendTask::IdentityTask(IdentityTask::Transfer(SHARED_IDENTITY.clone(), recipient.identity.id(), 10_000_000, None)))`
  4. Assert: result matches `TransferredCredits(fee_result)`
  5. Verify: re-fetch recipient identity, confirm balance increased by ~10_000_000 (minus any fees on recipient side)
- **Expected outcome**: `TransferredCredits(FeeResult { .. })`
- **Shared fixture dependency**: SHARED_IDENTITY, second identity
- **Estimated runtime**: 30s

### TC-024: TransferToAddresses
- **BackendTask variant**: `BackendTask::IdentityTask(IdentityTask::TransferToAddresses { .. })`
- **Group**: IdentityTask
- **Preconditions**: SHARED_IDENTITY with credits, a platform address
- **Steps**:
  1. Setup: derive a platform address from framework wallet
  2. Build `outputs` map: `{ platform_addr => 5_000_000 }`
  3. Execute: `run_task(ctx, BackendTask::IdentityTask(IdentityTask::TransferToAddresses { identity: SHARED_IDENTITY.clone(), outputs, key_id: None }))`
  4. Assert: result matches `TransferredCredits(fee_result)`
  5. Verify: `FetchPlatformAddressBalances` shows credits on the destination address
- **Expected outcome**: `TransferredCredits(FeeResult { .. })`
- **Shared fixture dependency**: SHARED_IDENTITY, Framework wallet
- **Estimated runtime**: 30s

### TC-025: RefreshIdentity
- **BackendTask variant**: `BackendTask::IdentityTask(IdentityTask::RefreshIdentity(qi))`
- **Group**: IdentityTask
- **Preconditions**: SHARED_IDENTITY registered
- **Steps**:
  1. Execute: `run_task(ctx, BackendTask::IdentityTask(IdentityTask::RefreshIdentity(SHARED_IDENTITY.clone())))`
  2. Assert: result matches `RefreshedIdentity(qi)` where `qi.identity.id() == SHARED_IDENTITY.id()`
  3. Assert: `qi.identity.balance() > 0`
- **Expected outcome**: `RefreshedIdentity(QualifiedIdentity { .. })`
- **Shared fixture dependency**: SHARED_IDENTITY
- **Estimated runtime**: 5s

### TC-026: RefreshLoadedIdentitiesOwnedDPNSNames
- **BackendTask variant**: `BackendTask::IdentityTask(IdentityTask::RefreshLoadedIdentitiesOwnedDPNSNames)`
- **Group**: IdentityTask
- **Preconditions**: At least one identity with a DPNS name loaded in the database
- **Steps**:
  1. Setup: ensure SHARED_IDENTITY (with a DPNS name) is in the database
  2. Execute: `run_task(ctx, BackendTask::IdentityTask(IdentityTask::RefreshLoadedIdentitiesOwnedDPNSNames))`
  3. Assert: result matches `RefreshedOwnedDpnsNames`
- **Expected outcome**: `RefreshedOwnedDpnsNames`
- **Shared fixture dependency**: SHARED_IDENTITY with DPNS name
- **Estimated runtime**: 10s

### TC-027: LoadIdentity
- **BackendTask variant**: `BackendTask::IdentityTask(IdentityTask::LoadIdentity(input))`
- **Group**: IdentityTask
- **Preconditions**: SHARED_IDENTITY registered on Platform
- **Steps**:
  1. Setup: build `IdentityInputToLoad` with `identity_id_input = SHARED_IDENTITY.identity.id().to_string(Encoding::Base58)`, `identity_type = IdentityType::User`, empty keys, `derive_keys_from_wallets = true`, `selected_wallet_seed_hash = Some(framework_wallet_seed_hash)`
  2. Execute: `run_task(ctx, BackendTask::IdentityTask(IdentityTask::LoadIdentity(input)))`
  3. Assert: result matches `LoadedIdentity(qi)` where `qi.identity.id() == SHARED_IDENTITY.id()`
  4. Assert: loaded identity has the expected public keys
- **Expected outcome**: `LoadedIdentity(QualifiedIdentity { .. })`
- **Shared fixture dependency**: SHARED_IDENTITY
- **Estimated runtime**: 5s

### TC-028: SearchIdentityFromWallet
- **BackendTask variant**: `BackendTask::IdentityTask(IdentityTask::SearchIdentityFromWallet(wallet_ref, index))`
- **Group**: IdentityTask
- **Preconditions**: Framework wallet with identity at index 0
- **Steps**:
  1. Setup: obtain `WalletArcRef` from framework wallet, use index 0
  2. Execute: `run_task(ctx, BackendTask::IdentityTask(IdentityTask::SearchIdentityFromWallet(wallet_ref, 0)))`
  3. Assert: result matches `RegisteredIdentity(qi, _)` or `LoadedIdentity(qi)` where the identity was found
- **Expected outcome**: Identity found or `Message("No identities found")` if none at that index
- **Shared fixture dependency**: Framework wallet with registered identity
- **Estimated runtime**: 10s

### TC-029: SearchIdentitiesUpToIndex
- **BackendTask variant**: `BackendTask::IdentityTask(IdentityTask::SearchIdentitiesUpToIndex(wallet_ref, max_index))`
- **Group**: IdentityTask
- **Preconditions**: Framework wallet with at least one identity
- **Steps**:
  1. Setup: obtain `WalletArcRef`, set `max_index = 5`
  2. Execute: `run_task(ctx, BackendTask::IdentityTask(IdentityTask::SearchIdentitiesUpToIndex(wallet_ref, 5)))`
  3. Assert: result is not an error (may be `Progress` messages followed by final result)
  4. Assert: at least one identity is found if SHARED_IDENTITY was registered from this wallet
- **Expected outcome**: `Message(...)` or `Progress { .. }` results, no error
- **Shared fixture dependency**: Framework wallet
- **Estimated runtime**: 30s

### TC-030: IdentityTask error — load nonexistent identity
- **BackendTask variant**: `BackendTask::IdentityTask(IdentityTask::LoadIdentity(input))`
- **Group**: IdentityTask (error path)
- **Preconditions**: None
- **Steps**:
  1. Setup: build `IdentityInputToLoad` with a random/nonexistent identity ID (valid Base58 but never registered)
  2. Execute: `run_task(ctx, BackendTask::IdentityTask(IdentityTask::LoadIdentity(input)))`
  3. Assert: result is `Err(TaskError::...)` — typed error variant, not a panic
- **Expected outcome**: `Err(TaskError::...)` indicating identity not found
- **Shared fixture dependency**: None
- **Estimated runtime**: 5s

---

## 4. DashPayTask Tests (`dashpay_tasks.rs`)

### TC-031: LoadProfile — identity with no profile
- **BackendTask variant**: `BackendTask::DashPayTask(Box::new(DashPayTask::LoadProfile { identity }))`
- **Group**: DashPayTask
- **Preconditions**: SHARED_IDENTITY (may not have a DashPay profile yet)
- **Steps**:
  1. Execute: `run_task(ctx, BackendTask::DashPayTask(Box::new(DashPayTask::LoadProfile { identity: SHARED_IDENTITY.clone() })))`
  2. Assert: result matches `DashPayProfile(profile)` where `profile` is `None` (no profile yet) or `Some((name, bio, url))`
- **Expected outcome**: `DashPayProfile(None)` or `DashPayProfile(Some(...))`
- **Shared fixture dependency**: SHARED_IDENTITY
- **Estimated runtime**: 5s

### TC-032: UpdateProfile
- **BackendTask variant**: `BackendTask::DashPayTask(Box::new(DashPayTask::UpdateProfile { .. }))`
- **Group**: DashPayTask
- **Preconditions**: SHARED_DASHPAY_PAIR[0] — identity with DashPay keys and sufficient credits
- **Steps**:
  1. Setup: use identity A from SHARED_DASHPAY_PAIR
  2. Execute: `run_task(ctx, BackendTask::DashPayTask(Box::new(DashPayTask::UpdateProfile { identity: A, display_name: Some("E2E Test User".into()), bio: Some("Backend E2E test profile".into()), avatar_url: None })))`
  3. Assert: result matches `DashPayProfileUpdated(id)` where `id == A.identity.id()`
  4. Verify: call `LoadProfile` and confirm display_name = "E2E Test User"
- **Expected outcome**: `DashPayProfileUpdated(Identifier)`
- **Shared fixture dependency**: SHARED_DASHPAY_PAIR
- **Estimated runtime**: 30s

### TC-033: SearchProfiles
- **BackendTask variant**: `BackendTask::DashPayTask(Box::new(DashPayTask::SearchProfiles { search_query }))`
- **Group**: DashPayTask
- **Preconditions**: At least one identity with a DPNS name on testnet
- **Steps**:
  1. Setup: use the DPNS name registered for SHARED_DASHPAY_PAIR[0]
  2. Execute: `run_task(ctx, BackendTask::DashPayTask(Box::new(DashPayTask::SearchProfiles { search_query: known_username.clone() })))`
  3. Assert: result matches `DashPayProfileSearchResults(results)` where `results.len() >= 1`
  4. Assert: at least one result contains the expected username
- **Expected outcome**: `DashPayProfileSearchResults(Vec<...>)` with at least one match
- **Shared fixture dependency**: SHARED_DASHPAY_PAIR (for known username)
- **Estimated runtime**: 10s

### TC-034: LoadContacts — empty
- **BackendTask variant**: `BackendTask::DashPayTask(Box::new(DashPayTask::LoadContacts { identity }))`
- **Group**: DashPayTask
- **Preconditions**: Identity with DashPay keys but no established contacts
- **Steps**:
  1. Execute: `run_task(ctx, BackendTask::DashPayTask(Box::new(DashPayTask::LoadContacts { identity: SHARED_IDENTITY.clone() })))`
  2. Assert: result matches `DashPayContacts(contacts)` or `DashPayContactsWithInfo(contacts)` where contacts may be empty
- **Expected outcome**: `DashPayContacts([])` or `DashPayContactsWithInfo([])`
- **Shared fixture dependency**: SHARED_IDENTITY
- **Estimated runtime**: 5s

### TC-035: LoadContactRequests — empty
- **BackendTask variant**: `BackendTask::DashPayTask(Box::new(DashPayTask::LoadContactRequests { identity }))`
- **Group**: DashPayTask
- **Preconditions**: Identity with DashPay keys
- **Steps**:
  1. Execute: `run_task(ctx, BackendTask::DashPayTask(Box::new(DashPayTask::LoadContactRequests { identity: SHARED_IDENTITY.clone() })))`
  2. Assert: result matches `DashPayContactRequests { incoming, outgoing }` where both may be empty
- **Expected outcome**: `DashPayContactRequests { incoming: [], outgoing: [] }`
- **Shared fixture dependency**: SHARED_IDENTITY
- **Estimated runtime**: 5s

### TC-036: FetchContactProfile
- **BackendTask variant**: `BackendTask::DashPayTask(Box::new(DashPayTask::FetchContactProfile { identity, contact_id }))`
- **Group**: DashPayTask
- **Preconditions**: A known identity ID on testnet with a DashPay profile
- **Steps**:
  1. Setup: use SHARED_DASHPAY_PAIR[0] as identity, SHARED_DASHPAY_PAIR[1]'s ID as contact_id
  2. Execute: `run_task(ctx, BackendTask::DashPayTask(Box::new(DashPayTask::FetchContactProfile { identity: A, contact_id: B_id })))`
  3. Assert: result matches `DashPayContactProfile(profile)` — `profile` may be `None` if B has no DashPay profile
- **Expected outcome**: `DashPayContactProfile(Option<Document>)`
- **Shared fixture dependency**: SHARED_DASHPAY_PAIR
- **Estimated runtime**: 5s

### TC-037: SendContactRequest
- **BackendTask variant**: `BackendTask::DashPayTask(Box::new(DashPayTask::SendContactRequest { .. }))`
- **Group**: DashPayTask
- **Preconditions**: SHARED_DASHPAY_PAIR — two identities (A, B) both with DashPay keys and DPNS names
- **Steps**:
  1. Setup: use identity A as sender, B's DPNS username as `to_username`
  2. Obtain A's encryption signing key
  3. Execute: `run_task(ctx, BackendTask::DashPayTask(Box::new(DashPayTask::SendContactRequest { identity: A, signing_key, to_username: B_username, account_label: None })))`
  4. Assert: result matches `DashPayContactRequestSent(username)` where `username == B_username`
- **Expected outcome**: `DashPayContactRequestSent(String)`
- **Shared fixture dependency**: SHARED_DASHPAY_PAIR
- **Estimated runtime**: 30s

### TC-038: LoadContactRequests — after sending
- **BackendTask variant**: `BackendTask::DashPayTask(Box::new(DashPayTask::LoadContactRequests { identity }))`
- **Group**: DashPayTask
- **Preconditions**: TC-037 completed (contact request sent from A to B)
- **Steps**:
  1. Execute: `run_task(ctx, BackendTask::DashPayTask(Box::new(DashPayTask::LoadContactRequests { identity: B })))`
  2. Assert: result matches `DashPayContactRequests { incoming, outgoing }`
  3. Assert: `incoming.len() >= 1`, at least one request is from A
- **Expected outcome**: `DashPayContactRequests { incoming: [..], .. }` with A's request
- **Shared fixture dependency**: TC-037 output
- **Estimated runtime**: 5s

### TC-039: AcceptContactRequest
- **BackendTask variant**: `BackendTask::DashPayTask(Box::new(DashPayTask::AcceptContactRequest { identity, request_id }))`
- **Group**: DashPayTask
- **Preconditions**: TC-038 — pending incoming request from A to B
- **Steps**:
  1. Setup: obtain `request_id` from TC-038's incoming requests
  2. Execute: `run_task(ctx, BackendTask::DashPayTask(Box::new(DashPayTask::AcceptContactRequest { identity: B, request_id })))`
  3. Assert: result matches `DashPayContactRequestAccepted(id)` where `id == request_id`
  4. Verify: `LoadContacts` for B shows A in the contacts list
- **Expected outcome**: `DashPayContactRequestAccepted(Identifier)`
- **Shared fixture dependency**: TC-038 output
- **Estimated runtime**: 30s

### TC-040: RegisterDashPayAddresses
- **BackendTask variant**: `BackendTask::DashPayTask(Box::new(DashPayTask::RegisterDashPayAddresses { identity }))`
- **Group**: DashPayTask
- **Preconditions**: Identity with DashPay keys and an established contact (from TC-039)
- **Steps**:
  1. Execute: `run_task(ctx, BackendTask::DashPayTask(Box::new(DashPayTask::RegisterDashPayAddresses { identity: B })))`
  2. Assert: result matches `Message(_)` or a success variant (DashPay address registration is a local + network operation)
- **Expected outcome**: Success result (variant depends on implementation)
- **Shared fixture dependency**: SHARED_DASHPAY_PAIR with established contact
- **Estimated runtime**: 10s

### TC-041: LoadPaymentHistory — empty
- **BackendTask variant**: `BackendTask::DashPayTask(Box::new(DashPayTask::LoadPaymentHistory { identity }))`
- **Group**: DashPayTask
- **Preconditions**: Identity with DashPay keys
- **Steps**:
  1. Execute: `run_task(ctx, BackendTask::DashPayTask(Box::new(DashPayTask::LoadPaymentHistory { identity: A })))`
  2. Assert: result matches `DashPayPaymentHistory(history)` where `history` may be empty
- **Expected outcome**: `DashPayPaymentHistory(Vec<...>)`
- **Shared fixture dependency**: SHARED_DASHPAY_PAIR
- **Estimated runtime**: 5s

### TC-042: UpdateContactInfo
- **BackendTask variant**: `BackendTask::DashPayTask(Box::new(DashPayTask::UpdateContactInfo { .. }))`
- **Group**: DashPayTask
- **Preconditions**: Established contact pair (A and B from TC-039)
- **Steps**:
  1. Execute: `run_task(ctx, BackendTask::DashPayTask(Box::new(DashPayTask::UpdateContactInfo { identity: B, contact_id: A_id, nickname: Some("Test Nickname".into()), note: Some("E2E note".into()), is_hidden: false, accepted_accounts: vec![0] })))`
  2. Assert: result matches `DashPayContactInfoUpdated(id)` where `id == A_id`
- **Expected outcome**: `DashPayContactInfoUpdated(Identifier)`
- **Shared fixture dependency**: SHARED_DASHPAY_PAIR with established contact
- **Estimated runtime**: 5s

### TC-043: RejectContactRequest
- **BackendTask variant**: `BackendTask::DashPayTask(Box::new(DashPayTask::RejectContactRequest { identity, request_id }))`
- **Group**: DashPayTask
- **Preconditions**: A fresh contact request sent (requires a third identity or re-send from A to B after relationship is established — more practically, send from B to a third identity C, have C reject)
- **Steps**:
  1. Setup: create a fresh contact request from SHARED_DASHPAY_PAIR[0] to a third DashPay identity
  2. Load incoming requests for the third identity, obtain `request_id`
  3. Execute: `run_task(ctx, BackendTask::DashPayTask(Box::new(DashPayTask::RejectContactRequest { identity: C, request_id })))`
  4. Assert: result matches `DashPayContactRequestRejected(id)`
- **Expected outcome**: `DashPayContactRequestRejected(Identifier)`
- **Shared fixture dependency**: Third DashPay identity
- **Estimated runtime**: 60s (includes creating third identity + sending request)

### TC-044: DashPayTask error — send contact request to nonexistent username
- **BackendTask variant**: `BackendTask::DashPayTask(Box::new(DashPayTask::SendContactRequest { .. }))`
- **Group**: DashPayTask (error path)
- **Preconditions**: Identity with DashPay keys
- **Steps**:
  1. Setup: use a nonexistent username (e.g., `"zzz_nonexistent_e2e_test_user_999"`)
  2. Execute: `run_task(ctx, BackendTask::DashPayTask(Box::new(DashPayTask::SendContactRequest { identity: A, signing_key, to_username: "zzz_nonexistent_e2e_test_user_999".into(), account_label: None })))`
  3. Assert: result is `Err(TaskError::...)` — not a panic
  4. Assert: error indicates user/identity not found
- **Expected outcome**: `Err(TaskError::...)` typed variant
- **Shared fixture dependency**: SHARED_DASHPAY_PAIR
- **Estimated runtime**: 5s

---

## 5. TokenTask Tests (`token_tasks.rs`)

### TC-045: RegisterTokenContract
- **BackendTask variant**: `BackendTask::TokenTask(Box::new(TokenTask::RegisterTokenContract { .. }))`
- **Group**: TokenTask
- **Preconditions**: SHARED_IDENTITY with sufficient credits, a signing key
- **Steps**:
  1. Setup: build `RegisterTokenContract` with owner-permissive rules: `manual_minting_rules` allows the identity, `freeze_rules` allows the identity, `marketplace_trade_mode = 1` (direct purchase), `base_supply = 1_000_000`, `max_supply = Some(100_000_000)`
  2. Execute: `run_task(ctx, BackendTask::TokenTask(Box::new(TokenTask::RegisterTokenContract { .. })))`
  3. Assert: result matches `RegisteredTokenContract`
  4. Verify: fetch the contract from Platform by ID and confirm it exists
- **Expected outcome**: `RegisteredTokenContract`
- **Shared fixture dependency**: SHARED_IDENTITY (stored as SHARED_TOKEN)
- **Estimated runtime**: 60s

### TC-046: QueryMyTokenBalances
- **BackendTask variant**: `BackendTask::TokenTask(Box::new(TokenTask::QueryMyTokenBalances))`
- **Group**: TokenTask
- **Preconditions**: Identities loaded in DB (SHARED_IDENTITY with token from TC-045)
- **Steps**:
  1. Execute: `run_task(ctx, BackendTask::TokenTask(Box::new(TokenTask::QueryMyTokenBalances)))`
  2. Assert: result matches `FetchedTokenBalances`
- **Expected outcome**: `FetchedTokenBalances`
- **Shared fixture dependency**: SHARED_IDENTITY, SHARED_TOKEN
- **Estimated runtime**: 10s

### TC-047: QueryIdentityTokenBalance
- **BackendTask variant**: `BackendTask::TokenTask(Box::new(TokenTask::QueryIdentityTokenBalance(iti)))`
- **Group**: TokenTask
- **Preconditions**: SHARED_TOKEN registered, SHARED_IDENTITY has base_supply tokens
- **Steps**:
  1. Setup: build `IdentityTokenIdentifier` with SHARED_IDENTITY's ID and SHARED_TOKEN's token ID
  2. Execute: `run_task(ctx, BackendTask::TokenTask(Box::new(TokenTask::QueryIdentityTokenBalance(iti))))`
  3. Assert: result matches `FetchedTokenBalances` (or a typed variant showing the balance)
- **Expected outcome**: `FetchedTokenBalances`
- **Shared fixture dependency**: SHARED_TOKEN
- **Estimated runtime**: 5s

### TC-048: FetchTokenByContractId
- **BackendTask variant**: `BackendTask::TokenTask(Box::new(TokenTask::FetchTokenByContractId(contract_id)))`
- **Group**: TokenTask
- **Preconditions**: SHARED_TOKEN registered
- **Steps**:
  1. Setup: use SHARED_TOKEN's contract ID
  2. Execute: `run_task(ctx, BackendTask::TokenTask(Box::new(TokenTask::FetchTokenByContractId(contract_id))))`
  3. Assert: result matches `FetchedContractWithTokenPosition(contract, position)`
  4. Assert: `contract.id() == contract_id`, `position == 0`
- **Expected outcome**: `FetchedContractWithTokenPosition(DataContract, 0)`
- **Shared fixture dependency**: SHARED_TOKEN
- **Estimated runtime**: 5s

### TC-049: FetchTokenByTokenId
- **BackendTask variant**: `BackendTask::TokenTask(Box::new(TokenTask::FetchTokenByTokenId(token_id)))`
- **Group**: TokenTask
- **Preconditions**: SHARED_TOKEN registered
- **Steps**:
  1. Setup: compute token_id from SHARED_TOKEN's contract ID + position
  2. Execute: `run_task(ctx, BackendTask::TokenTask(Box::new(TokenTask::FetchTokenByTokenId(token_id))))`
  3. Assert: result matches `FetchedContractWithTokenPosition(contract, position)`
- **Expected outcome**: `FetchedContractWithTokenPosition(DataContract, 0)`
- **Shared fixture dependency**: SHARED_TOKEN
- **Estimated runtime**: 5s

### TC-050: SaveTokenLocally
- **BackendTask variant**: `BackendTask::TokenTask(Box::new(TokenTask::SaveTokenLocally(token_info)))`
- **Group**: TokenTask
- **Preconditions**: A `TokenInfo` struct built from SHARED_TOKEN
- **Steps**:
  1. Setup: build `TokenInfo` from SHARED_TOKEN's contract
  2. Execute: `run_task(ctx, BackendTask::TokenTask(Box::new(TokenTask::SaveTokenLocally(token_info))))`
  3. Assert: result matches `SavedToken`
  4. Verify: query local DB to confirm token was persisted
- **Expected outcome**: `SavedToken`
- **Shared fixture dependency**: SHARED_TOKEN
- **Estimated runtime**: 1s

### TC-051: QueryDescriptionsByKeyword
- **BackendTask variant**: `BackendTask::TokenTask(Box::new(TokenTask::QueryDescriptionsByKeyword(keyword, start)))`
- **Group**: TokenTask
- **Preconditions**: Token contract registered with keywords
- **Steps**:
  1. Setup: use a keyword from SHARED_TOKEN's `contract_keywords`
  2. Execute: `run_task(ctx, BackendTask::TokenTask(Box::new(TokenTask::QueryDescriptionsByKeyword(keyword, None))))`
  3. Assert: result matches `DescriptionsByKeyword(results, _)` (may be empty if Platform indexing hasn't caught up)
- **Expected outcome**: `DescriptionsByKeyword(Vec<ContractDescriptionInfo>, Option<Start>)`
- **Shared fixture dependency**: SHARED_TOKEN
- **Estimated runtime**: 10s

### TC-052: QueryTokenPricing
- **BackendTask variant**: `BackendTask::TokenTask(Box::new(TokenTask::QueryTokenPricing(token_id)))`
- **Group**: TokenTask
- **Preconditions**: SHARED_TOKEN registered
- **Steps**:
  1. Setup: compute token_id
  2. Execute: `run_task(ctx, BackendTask::TokenTask(Box::new(TokenTask::QueryTokenPricing(token_id))))`
  3. Assert: result matches `TokenPricing { token_id: tid, prices }` where `tid == token_id`
- **Expected outcome**: `TokenPricing { .. }`
- **Shared fixture dependency**: SHARED_TOKEN
- **Estimated runtime**: 5s

### TC-053: MintTokens
- **BackendTask variant**: `BackendTask::TokenTask(Box::new(TokenTask::MintTokens { .. }))`
- **Group**: TokenTask
- **Preconditions**: SHARED_TOKEN with owner-permissive minting rules, SHARED_IDENTITY as owner
- **Steps**:
  1. Setup: build `MintTokens` with `amount = 500_000`, `recipient_id = None` (mint to self), `group_info = None`
  2. Execute: `run_task(ctx, BackendTask::TokenTask(Box::new(TokenTask::MintTokens { sending_identity: IDENTITY, data_contract: CONTRACT, token_position: 0, signing_key, public_note: Some("E2E mint".into()), amount: 500_000, recipient_id: None, group_info: None })))`
  3. Assert: result matches `MintedTokens(fee_result)` with `fee_result.actual_fee > 0`
  4. Verify: query token balance, confirm increase of 500_000
- **Expected outcome**: `MintedTokens(FeeResult { .. })`
- **Shared fixture dependency**: SHARED_TOKEN, SHARED_IDENTITY
- **Estimated runtime**: 30s

### TC-054: BurnTokens
- **BackendTask variant**: `BackendTask::TokenTask(Box::new(TokenTask::BurnTokens { .. }))`
- **Group**: TokenTask
- **Preconditions**: SHARED_IDENTITY holds tokens (from TC-053 or base supply)
- **Steps**:
  1. Setup: build `BurnTokens` with `amount = 100`, `group_info = None`
  2. Execute: `run_task(ctx, ...)`
  3. Assert: result matches `BurnedTokens(fee_result)`
  4. Verify: token balance decreased by 100
- **Expected outcome**: `BurnedTokens(FeeResult { .. })`
- **Shared fixture dependency**: SHARED_TOKEN, SHARED_IDENTITY
- **Estimated runtime**: 30s

### TC-055: TransferTokens
- **BackendTask variant**: `BackendTask::TokenTask(Box::new(TokenTask::TransferTokens { .. }))`
- **Group**: TokenTask
- **Preconditions**: SHARED_IDENTITY holds tokens, second identity exists
- **Steps**:
  1. Setup: build `TransferTokens` with `recipient_id = second_identity.id()`, `amount = 100`
  2. Execute: `run_task(ctx, ...)`
  3. Assert: result matches `TransferredTokens(fee_result)`
  4. Verify: recipient's token balance increased
- **Expected outcome**: `TransferredTokens(FeeResult { .. })`
- **Shared fixture dependency**: SHARED_TOKEN, SHARED_IDENTITY, second identity
- **Estimated runtime**: 30s

### TC-056: FreezeTokens
- **BackendTask variant**: `BackendTask::TokenTask(Box::new(TokenTask::FreezeTokens { .. }))`
- **Group**: TokenTask
- **Preconditions**: SHARED_TOKEN with freeze rules, a target identity that holds tokens
- **Steps**:
  1. Setup: build `FreezeTokens` with `freeze_identity = second_identity.id()`
  2. Execute: `run_task(ctx, ...)`
  3. Assert: result matches `FrozeTokens(fee_result)`
- **Expected outcome**: `FrozeTokens(FeeResult { .. })`
- **Shared fixture dependency**: SHARED_TOKEN, second identity
- **Estimated runtime**: 30s

### TC-057: UnfreezeTokens
- **BackendTask variant**: `BackendTask::TokenTask(Box::new(TokenTask::UnfreezeTokens { .. }))`
- **Group**: TokenTask
- **Preconditions**: TC-056 completed (identity frozen)
- **Steps**:
  1. Setup: build `UnfreezeTokens` with `unfreeze_identity = second_identity.id()`
  2. Execute: `run_task(ctx, ...)`
  3. Assert: result matches `UnfrozeTokens(fee_result)`
- **Expected outcome**: `UnfrozeTokens(FeeResult { .. })`
- **Shared fixture dependency**: TC-056 state
- **Estimated runtime**: 30s

### TC-058: DestroyFrozenFunds
- **BackendTask variant**: `BackendTask::TokenTask(Box::new(TokenTask::DestroyFrozenFunds { .. }))`
- **Group**: TokenTask
- **Preconditions**: A frozen identity with token balance (re-freeze after TC-057 or use a different identity)
- **Steps**:
  1. Setup: freeze a target identity (call FreezeTokens first), then build `DestroyFrozenFunds` with that identity
  2. Execute: `run_task(ctx, ...)`
  3. Assert: result matches `DestroyedFrozenFunds(fee_result)`
  4. Verify: target identity's token balance is 0
- **Expected outcome**: `DestroyedFrozenFunds(FeeResult { .. })`
- **Shared fixture dependency**: SHARED_TOKEN, target identity
- **Estimated runtime**: 60s (freeze + destroy)

### TC-059: PauseTokens
- **BackendTask variant**: `BackendTask::TokenTask(Box::new(TokenTask::PauseTokens { .. }))`
- **Group**: TokenTask
- **Preconditions**: SHARED_TOKEN with emergency_action_rules permitting identity
- **Steps**:
  1. Execute: `run_task(ctx, BackendTask::TokenTask(Box::new(TokenTask::PauseTokens { actor_identity: IDENTITY, data_contract: CONTRACT, token_position: 0, signing_key, public_note: None, group_info: None })))`
  2. Assert: result matches `PausedTokens(fee_result)`
- **Expected outcome**: `PausedTokens(FeeResult { .. })`
- **Shared fixture dependency**: SHARED_TOKEN
- **Estimated runtime**: 30s

### TC-060: ResumeTokens
- **BackendTask variant**: `BackendTask::TokenTask(Box::new(TokenTask::ResumeTokens { .. }))`
- **Group**: TokenTask
- **Preconditions**: TC-059 completed (token paused)
- **Steps**:
  1. Execute: `run_task(ctx, BackendTask::TokenTask(Box::new(TokenTask::ResumeTokens { actor_identity: IDENTITY, data_contract: CONTRACT, token_position: 0, signing_key, public_note: None, group_info: None })))`
  2. Assert: result matches `ResumedTokens(fee_result)`
- **Expected outcome**: `ResumedTokens(FeeResult { .. })`
- **Shared fixture dependency**: TC-059 state
- **Estimated runtime**: 30s

### TC-061: SetDirectPurchasePrice
- **BackendTask variant**: `BackendTask::TokenTask(Box::new(TokenTask::SetDirectPurchasePrice { .. }))`
- **Group**: TokenTask
- **Preconditions**: SHARED_TOKEN with marketplace rules permitting identity
- **Steps**:
  1. Setup: build a `TokenPricingSchedule` with a simple fixed price (e.g., 1000 credits per token)
  2. Execute: `run_task(ctx, BackendTask::TokenTask(Box::new(TokenTask::SetDirectPurchasePrice { identity: IDENTITY, data_contract: CONTRACT, token_position: 0, signing_key, token_pricing_schedule: Some(pricing), public_note: None, group_info: None })))`
  3. Assert: result matches `SetTokenPrice(fee_result)`
- **Expected outcome**: `SetTokenPrice(FeeResult { .. })`
- **Shared fixture dependency**: SHARED_TOKEN
- **Estimated runtime**: 30s

### TC-062: PurchaseTokens
- **BackendTask variant**: `BackendTask::TokenTask(Box::new(TokenTask::PurchaseTokens { .. }))`
- **Group**: TokenTask
- **Preconditions**: TC-061 completed (price set), second identity with credits
- **Steps**:
  1. Setup: build `PurchaseTokens` with `amount = 10`, `total_agreed_price = 10_000` (matching price schedule)
  2. Execute with second identity as purchaser
  3. Assert: result matches `PurchasedTokens(fee_result)`
  4. Verify: purchaser's token balance increased by 10
- **Expected outcome**: `PurchasedTokens(FeeResult { .. })`
- **Shared fixture dependency**: SHARED_TOKEN, second identity with credits
- **Estimated runtime**: 30s

### TC-063: UpdateTokenConfig
- **BackendTask variant**: `BackendTask::TokenTask(Box::new(TokenTask::UpdateTokenConfig { .. }))`
- **Group**: TokenTask
- **Preconditions**: SHARED_TOKEN, identity that owns the contract with appropriate config change rules
- **Steps**:
  1. Setup: build `IdentityTokenInfo` from SHARED_TOKEN, choose a `TokenConfigurationChangeItem` (e.g., change max_supply)
  2. Execute: `run_task(ctx, ...)`
  3. Assert: result matches `UpdatedTokenConfig(description, fee_result)` where `description` describes the change
- **Expected outcome**: `UpdatedTokenConfig(String, FeeResult { .. })`
- **Shared fixture dependency**: SHARED_TOKEN
- **Estimated runtime**: 30s

### TC-064: EstimatePerpetualTokenRewardsWithExplanation
- **BackendTask variant**: `BackendTask::TokenTask(Box::new(TokenTask::EstimatePerpetualTokenRewardsWithExplanation { identity_id, token_id }))`
- **Group**: TokenTask
- **Preconditions**: SHARED_TOKEN with perpetual distribution configured (or any token — returns zero/error gracefully)
- **Steps**:
  1. Setup: use SHARED_IDENTITY's ID and SHARED_TOKEN's token ID
  2. Execute: `run_task(ctx, ...)`
  3. Assert: result matches `TokenEstimatedNonClaimedPerpetualDistributionAmountWithExplanation(iti, amount, explanation)` or an appropriate error if no distribution configured
- **Expected outcome**: Success variant or graceful error
- **Shared fixture dependency**: SHARED_TOKEN
- **Estimated runtime**: 5s

### TC-065: TokenTask error — mint with unauthorized identity
- **BackendTask variant**: `BackendTask::TokenTask(Box::new(TokenTask::MintTokens { .. }))`
- **Group**: TokenTask (error path)
- **Preconditions**: SHARED_TOKEN, a different identity that is NOT authorized to mint
- **Steps**:
  1. Setup: build `MintTokens` using an identity that is not the token owner and not in any authorized group
  2. Execute: `run_task(ctx, ...)`
  3. Assert: result is `Err(TaskError::...)` — a typed error, not a panic
- **Expected outcome**: `Err(TaskError::...)` indicating unauthorized
- **Shared fixture dependency**: SHARED_TOKEN, unauthorized identity
- **Estimated runtime**: 10s

---

## 6. BroadcastStateTransition Tests (`broadcast_st_tasks.rs`)

### TC-066: BroadcastStateTransition — identity update
- **BackendTask variant**: `BackendTask::BroadcastStateTransition(state_transition)`
- **Group**: BroadcastStateTransition
- **Preconditions**: SHARED_IDENTITY with keys, SDK available
- **Steps**:
  1. Setup: build a minimal valid `StateTransition` — an identity update that adds a new public key (build using the SDK's state transition builder with correct nonce)
  2. Sign the state transition with the identity's master key
  3. Execute: `run_task(ctx, BackendTask::BroadcastStateTransition(st))`
  4. Assert: result matches `BroadcastedStateTransition`
  5. Verify: re-fetch identity and confirm the new key exists
- **Expected outcome**: `BroadcastedStateTransition`
- **Shared fixture dependency**: SHARED_IDENTITY
- **Estimated runtime**: 30s

### TC-067: BroadcastStateTransition error — invalid state transition
- **BackendTask variant**: `BackendTask::BroadcastStateTransition(invalid_st)`
- **Group**: BroadcastStateTransition (error path)
- **Preconditions**: None
- **Steps**:
  1. Setup: construct an intentionally invalid `StateTransition` (e.g., unsigned, or with wrong nonce)
  2. Execute: `run_task(ctx, BackendTask::BroadcastStateTransition(invalid_st))`
  3. Assert: result is `Err(TaskError::...)` — typed error
- **Expected outcome**: `Err(TaskError::...)` from SDK broadcast failure
- **Shared fixture dependency**: None
- **Estimated runtime**: 5s

---

## 7. MnListTask Tests (`mnlist_tasks.rs`)

### TC-068: FetchEndDmlDiff
- **BackendTask variant**: `BackendTask::MnListTask(MnListTask::FetchEndDmlDiff { .. })`
- **Group**: MnListTask
- **Preconditions**: SPV synced, two known testnet block heights/hashes obtained at runtime
- **Steps**:
  1. Setup: use `mnlist_helpers::get_current_block_info(ctx)` to get tip `(height, hash)`. Use `(height - 100, hash_at_height_minus_100)` as base.
  2. Execute: `run_task(ctx, BackendTask::MnListTask(MnListTask::FetchEndDmlDiff { base_block_height: h-100, base_block_hash: bh_base, block_height: h, block_hash: bh, validate_quorums: false }))`
  3. Assert: result matches `MnListFetchedDiff { base_height, height, diff }`
  4. Assert: `base_height == h-100`, `height == h`, `diff` has at least some masternode entries
- **Expected outcome**: `MnListFetchedDiff { .. }`
- **Shared fixture dependency**: None (runtime block info)
- **Estimated runtime**: 15s

### TC-069: FetchEndQrInfo
- **BackendTask variant**: `BackendTask::MnListTask(MnListTask::FetchEndQrInfo { .. })`
- **Group**: MnListTask
- **Preconditions**: SPV synced, known block hash
- **Steps**:
  1. Setup: get current block hash, use genesis hash as `known_block_hashes`
  2. Execute: `run_task(ctx, BackendTask::MnListTask(MnListTask::FetchEndQrInfo { known_block_hashes: vec![genesis_hash], block_hash: tip_hash }))`
  3. Assert: result matches `MnListFetchedQrInfo { qr_info }`
  4. Assert: `qr_info` contains valid masternode list data
- **Expected outcome**: `MnListFetchedQrInfo { .. }`
- **Shared fixture dependency**: None
- **Estimated runtime**: 30s

### TC-070: FetchEndQrInfoWithDmls
- **BackendTask variant**: `BackendTask::MnListTask(MnListTask::FetchEndQrInfoWithDmls { .. })`
- **Group**: MnListTask
- **Preconditions**: Same as TC-069
- **Steps**:
  1. Setup: same as TC-069
  2. Execute: `run_task(ctx, BackendTask::MnListTask(MnListTask::FetchEndQrInfoWithDmls { known_block_hashes: vec![genesis_hash], block_hash: tip_hash }))`
  3. Assert: result matches `MnListFetchedQrInfo { qr_info }`
- **Expected outcome**: `MnListFetchedQrInfo { .. }`
- **Shared fixture dependency**: None
- **Estimated runtime**: 30s

### TC-071: FetchDiffsChain
- **BackendTask variant**: `BackendTask::MnListTask(MnListTask::FetchDiffsChain { chain })`
- **Group**: MnListTask
- **Preconditions**: SPV synced, a sequence of known (height, hash) pairs
- **Steps**:
  1. Setup: get tip and two earlier heights (e.g., tip-200, tip-100, tip). Build `chain` as `vec![(h-200, bh_200, h-100, bh_100), (h-100, bh_100, h, bh)]`
  2. Execute: `run_task(ctx, BackendTask::MnListTask(MnListTask::FetchDiffsChain { chain }))`
  3. Assert: result matches `MnListFetchedDiffs { items }` where `items.len() == 2`
  4. Assert: each item has valid height ranges
- **Expected outcome**: `MnListFetchedDiffs { items }` with correct count
- **Shared fixture dependency**: None
- **Estimated runtime**: 30s

### TC-072: FetchChainLocks (conditional)
- **BackendTask variant**: `BackendTask::MnListTask(MnListTask::FetchChainLocks { .. })`
- **Group**: MnListTask
- **Preconditions**: `E2E_CORE_RPC_URL` set, Core RPC accessible
- **Steps**:
  1. Guard: skip if `E2E_CORE_RPC_URL` is not set
  2. Setup: use current tip height and `base_block_height = tip - 10`
  3. Execute: `run_task(ctx, BackendTask::MnListTask(MnListTask::FetchChainLocks { base_block_height: tip-10, block_height: tip }))`
  4. Assert: result matches `MnListChainLockSigs { entries }` where entries is non-empty
- **Expected outcome**: `MnListChainLockSigs { entries }` with block hash + optional signature pairs
- **Shared fixture dependency**: None
- **Estimated runtime**: 10s

### TC-073: MnListTask error — invalid block hash
- **BackendTask variant**: `BackendTask::MnListTask(MnListTask::FetchEndDmlDiff { .. })`
- **Group**: MnListTask (error path)
- **Preconditions**: None
- **Steps**:
  1. Setup: use an all-zeros `BlockHash` (invalid/nonexistent)
  2. Execute: `run_task(ctx, BackendTask::MnListTask(MnListTask::FetchEndDmlDiff { base_block_height: 0, base_block_hash: zero_hash, block_height: 1, block_hash: zero_hash, validate_quorums: false }))`
  3. Assert: result is `Err(TaskError::...)` — P2P error
- **Expected outcome**: `Err(TaskError::...)` indicating P2P/network failure
- **Shared fixture dependency**: None
- **Estimated runtime**: 10s

---

## 8. ShieldedTask Tests (`shielded_tasks.rs`)

All tests in this group are skipped when `E2E_SKIP_SHIELDED=1` is set.

### TC-074: WarmUpProvingKey
- **BackendTask variant**: `BackendTask::ShieldedTask(ShieldedTask::WarmUpProvingKey)`
- **Group**: ShieldedTask
- **Preconditions**: None (may download proving key on first run)
- **Steps**:
  1. Guard: skip if `E2E_SKIP_SHIELDED=1`
  2. Execute: `run_task(ctx, BackendTask::ShieldedTask(ShieldedTask::WarmUpProvingKey))`
  3. Assert: result matches `ProvingKeyReady`
- **Expected outcome**: `ProvingKeyReady`
- **Shared fixture dependency**: None
- **Estimated runtime**: 30-60s (first run may download ~100MB)

### TC-075: InitializeShieldedWallet
- **BackendTask variant**: `BackendTask::ShieldedTask(ShieldedTask::InitializeShieldedWallet { seed_hash })`
- **Group**: ShieldedTask
- **Preconditions**: Framework wallet
- **Steps**:
  1. Guard: skip if `E2E_SKIP_SHIELDED=1`
  2. Execute: `run_task(ctx, BackendTask::ShieldedTask(ShieldedTask::InitializeShieldedWallet { seed_hash }))`
  3. Assert: result matches `ShieldedInitialized { seed_hash: sh, balance }` where `sh == seed_hash`
  4. Assert: `balance >= 0` (likely 0 for fresh wallet)
- **Expected outcome**: `ShieldedInitialized { .. }`
- **Shared fixture dependency**: Framework wallet
- **Estimated runtime**: 5s

### TC-076: SyncNotes
- **BackendTask variant**: `BackendTask::ShieldedTask(ShieldedTask::SyncNotes { seed_hash })`
- **Group**: ShieldedTask
- **Preconditions**: TC-075 completed (shielded wallet initialized)
- **Steps**:
  1. Execute: `run_task(ctx, BackendTask::ShieldedTask(ShieldedTask::SyncNotes { seed_hash }))`
  2. Assert: result matches `ShieldedNotesSynced { seed_hash: sh, new_notes, balance }`
  3. Assert: `new_notes >= 0`, `balance >= 0`
- **Expected outcome**: `ShieldedNotesSynced { .. }`
- **Shared fixture dependency**: TC-075
- **Estimated runtime**: 10s

### TC-077: CheckNullifiers
- **BackendTask variant**: `BackendTask::ShieldedTask(ShieldedTask::CheckNullifiers { seed_hash })`
- **Group**: ShieldedTask
- **Preconditions**: TC-075 completed
- **Steps**:
  1. Execute: `run_task(ctx, BackendTask::ShieldedTask(ShieldedTask::CheckNullifiers { seed_hash }))`
  2. Assert: result matches `ShieldedNullifiersChecked { seed_hash: sh, spent_count }`
  3. Assert: `spent_count >= 0` (likely 0 for fresh wallet)
- **Expected outcome**: `ShieldedNullifiersChecked { .. }`
- **Shared fixture dependency**: TC-075
- **Estimated runtime**: 5s

### TC-078: ShieldFromAssetLock
- **BackendTask variant**: `BackendTask::ShieldedTask(ShieldedTask::ShieldFromAssetLock { seed_hash, amount_duffs, source_address })`
- **Group**: ShieldedTask
- **Preconditions**: TC-074 (proving key ready), TC-075 (wallet initialized), funded framework wallet
- **Steps**:
  1. Setup: `amount_duffs = 500_000` (0.005 DASH), `source_address = None`
  2. Execute: `run_task(ctx, BackendTask::ShieldedTask(ShieldedTask::ShieldFromAssetLock { seed_hash, amount_duffs: 500_000, source_address: None }))`
  3. Assert: result matches `ShieldedFromAssetLock { seed_hash: sh, amount }` where `amount > 0`
  4. Verify: `SyncNotes` shows increased balance
- **Expected outcome**: `ShieldedFromAssetLock { .. }`
- **Shared fixture dependency**: Framework wallet, TC-074, TC-075
- **Estimated runtime**: 90s (asset lock + ZK proof)

### TC-079: ShieldCredits
- **BackendTask variant**: `BackendTask::ShieldedTask(ShieldedTask::ShieldCredits { .. })`
- **Group**: ShieldedTask
- **Preconditions**: TC-074, TC-075, funded platform address
- **Steps**:
  1. Setup: fund a platform address (via WalletTask), then shield from it with `amount = 200_000_000` credits
  2. Execute: `run_task(ctx, BackendTask::ShieldedTask(ShieldedTask::ShieldCredits { seed_hash, amount: 200_000_000, from_address: platform_addr, nonce_override: None }))`
  3. Assert: result matches `ShieldedCreditsShielded { seed_hash: sh, amount }`
- **Expected outcome**: `ShieldedCreditsShielded { .. }`
- **Shared fixture dependency**: TC-074, TC-075, funded platform address
- **Estimated runtime**: 60s

### TC-080: ShieldedTransfer
- **BackendTask variant**: `BackendTask::ShieldedTask(ShieldedTask::ShieldedTransfer { .. })`
- **Group**: ShieldedTask
- **Preconditions**: TC-078 or TC-079 completed (shielded balance > 0)
- **Steps**:
  1. Setup: derive a recipient Orchard address (can be the same wallet's address for simplicity), serialize to bytes
  2. Execute: `run_task(ctx, BackendTask::ShieldedTask(ShieldedTask::ShieldedTransfer { seed_hash, amount: 50_000, recipient_address_bytes }))`
  3. Assert: result matches `ShieldedTransferComplete { seed_hash: sh, amount }`
- **Expected outcome**: `ShieldedTransferComplete { .. }`
- **Shared fixture dependency**: Shielded balance from TC-078/TC-079
- **Estimated runtime**: 60s (ZK proof)

### TC-081: UnshieldCredits
- **BackendTask variant**: `BackendTask::ShieldedTask(ShieldedTask::UnshieldCredits { .. })`
- **Group**: ShieldedTask
- **Preconditions**: Shielded balance > 0
- **Steps**:
  1. Setup: derive a `PlatformAddress` as destination
  2. Execute: `run_task(ctx, BackendTask::ShieldedTask(ShieldedTask::UnshieldCredits { seed_hash, amount: 30_000, to_platform_address: platform_addr }))`
  3. Assert: result matches `ShieldedCreditsUnshielded { seed_hash: sh, amount }`
  4. Verify: `FetchPlatformAddressBalances` shows credits on the destination
- **Expected outcome**: `ShieldedCreditsUnshielded { .. }`
- **Shared fixture dependency**: Shielded balance
- **Estimated runtime**: 60s

### TC-082: ShieldedWithdrawal
- **BackendTask variant**: `BackendTask::ShieldedTask(ShieldedTask::ShieldedWithdrawal { .. })`
- **Group**: ShieldedTask
- **Preconditions**: Shielded balance > 0
- **Steps**:
  1. Setup: derive a Core L1 testnet address from framework wallet
  2. Execute: `run_task(ctx, BackendTask::ShieldedTask(ShieldedTask::ShieldedWithdrawal { seed_hash, amount: 20_000, to_core_address: core_addr }))`
  3. Assert: result matches `ShieldedWithdrawalComplete { seed_hash: sh, amount }`
- **Expected outcome**: `ShieldedWithdrawalComplete { .. }`
- **Shared fixture dependency**: Shielded balance
- **Estimated runtime**: 60s

### TC-083: ShieldedTask error — uninitialized wallet
- **BackendTask variant**: `BackendTask::ShieldedTask(ShieldedTask::SyncNotes { seed_hash })`
- **Group**: ShieldedTask (error path)
- **Preconditions**: Shielded wallet NOT initialized for the given seed_hash
- **Steps**:
  1. Setup: use a `WalletSeedHash` for a wallet that has not had `InitializeShieldedWallet` called
  2. Execute: `run_task(ctx, BackendTask::ShieldedTask(ShieldedTask::SyncNotes { seed_hash: uninitialized_hash }))`
  3. Assert: result is `Err(TaskError::...)` — typed error indicating wallet not initialized
- **Expected outcome**: `Err(TaskError::...)` with specific variant
- **Shared fixture dependency**: None
- **Estimated runtime**: 2s

---

## Shared Fixtures Summary

| Fixture | Initialization | Used by |
|---|---|---|
| Framework wallet (harness) | `tests/backend-e2e/framework/harness.rs` — singleton | All groups |
| `SHARED_IDENTITY` | `OnceCell` — register identity at index 0 from framework wallet | CoreTask (TC-005), IdentityTask (TC-020..TC-029), DashPayTask (TC-031..TC-035), TokenTask (TC-045..TC-065), BroadcastST (TC-066) |
| `SHARED_TOKEN` | `OnceCell` — register token contract owned by SHARED_IDENTITY | TokenTask (TC-046..TC-065) |
| `SHARED_DASHPAY_PAIR` | `OnceCell` — two DashPay-keyed identities with DPNS names | DashPayTask (TC-032..TC-044) |

---

## Test File Organization

| File | Group | Test cases | Dependencies |
|---|---|---|---|
| `core_tasks.rs` | CoreTask | TC-001 to TC-011 | Framework wallet |
| `wallet_tasks.rs` | WalletTask | TC-012 to TC-019 | Framework wallet |
| `identity_tasks.rs` | IdentityTask | TC-020 to TC-030 | SHARED_IDENTITY |
| `dashpay_tasks.rs` | DashPayTask | TC-031 to TC-044 | SHARED_DASHPAY_PAIR |
| `token_tasks.rs` | TokenTask | TC-045 to TC-065 | SHARED_IDENTITY, SHARED_TOKEN |
| `broadcast_st_tasks.rs` | BroadcastStateTransition | TC-066 to TC-067 | SHARED_IDENTITY |
| `mnlist_tasks.rs` | MnListTask | TC-068 to TC-073 | SPV sync |
| `shielded_tasks.rs` | ShieldedTask | TC-074 to TC-083 | Framework wallet, `E2E_SKIP_SHIELDED` guard |

---

## Execution Order Within Groups

**Core tasks**: TC-001, TC-002, TC-003, TC-004, TC-005, TC-006, TC-007, TC-008, TC-009, TC-010, TC-011 (independent)

**Wallet tasks**: TC-012, TC-013, TC-014 -> TC-015 -> TC-016 -> TC-017, TC-018, TC-019

**Identity tasks**: TC-020, TC-021, TC-022, TC-023, TC-024, TC-025, TC-026, TC-027, TC-028, TC-029, TC-030 (SHARED_IDENTITY initialized first, then independent)

**DashPay tasks**: TC-031, TC-032 -> TC-033, TC-034, TC-035, TC-036, TC-037 -> TC-038 -> TC-039 -> TC-040, TC-041, TC-042, TC-043, TC-044

**Token tasks**: TC-045 -> TC-046..TC-052 (queries, independent), TC-053 -> TC-054, TC-055, TC-056 -> TC-057, TC-058, TC-059 -> TC-060, TC-061 -> TC-062, TC-063, TC-064, TC-065

**MnList tasks**: TC-068..TC-073 (independent except TC-072 conditional)

**Shielded tasks**: TC-074 -> TC-075 -> TC-076, TC-077, TC-078 -> TC-080 -> TC-081, TC-082, TC-079, TC-083

---

## Coverage Summary

| Group | Variants in scope | Test cases | Error tests | Total |
|---|---|---|---|---|
| CoreTask | 10 | 10 | 1 | 11 |
| WalletTask | 6 | 7 | 1 | 8 |
| IdentityTask | 13 | 10 | 1 | 11 |
| DashPayTask | 11 | 13 | 1 | 14 |
| TokenTask | 19 | 20 | 1 | 21 |
| BroadcastST | 1 | 1 | 1 | 2 |
| MnListTask | 4+1 | 5 | 1 | 6 |
| ShieldedTask | 9 | 9 | 1 | 10 |
| **Total** | **73+1** | **75** | **8** | **83** |

Note: Total exceeds the initial 68 estimate because some variants benefit from multiple test cases (e.g., RefreshWalletInfo with and without Platform sync, FetchPlatformAddressBalances before and after funding). The additional tests verify observable side effects as required by acceptance criteria.
