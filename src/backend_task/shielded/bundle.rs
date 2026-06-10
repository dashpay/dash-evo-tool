use crate::backend_task::error::{TaskError, shielded_broadcast_error, shielded_build_error};
use crate::context::AppContext;
use crate::context::shielded::get_proving_key;
use crate::model::fee_estimation::{format_credits_as_dash, shielded_fee_for_actions};
use crate::model::wallet::WalletSeedHash;
use crate::model::wallet::shielded::{ShieldedNote, ShieldedWalletState};
use crate::wallet_backend::{DetPlatformSigner, PlatformPathIndex, RememberPolicy, SecretScope};
use dash_sdk::dpp::address_funds::{
    AddressFundsFeeStrategy, AddressFundsFeeStrategyStep, OrchardAddress, PlatformAddress,
};
use dash_sdk::dpp::dashcore::Address;
use dash_sdk::dpp::identity::core_script::CoreScript;
use dash_sdk::dpp::shielded::builder::{
    OrchardProver, SpendableNote, build_shield_transition, build_shielded_transfer_transition,
    build_shielded_withdrawal_transition, build_unshield_transition,
};
use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::dpp::withdrawal::Pooling;
use dash_sdk::grovedb_commitment_tree::{
    Anchor, ClientPersistentCommitmentTree, Nullifier, PaymentAddress, ProvingKey,
};
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

/// Wrapper around a cached `ProvingKey` that implements `OrchardProver`.
struct CachedProver {
    key: &'static ProvingKey,
}

impl OrchardProver for CachedProver {
    fn proving_key(&self) -> &ProvingKey {
        self.key
    }
}

/// Progress stage for a shield credits operation (used by batch UI).
#[derive(Clone, Debug)]
pub enum ShieldStage {
    Queued,
    BuildingProof {
        nonce: u32,
    },
    WaitingToBroadcast,
    Broadcasting,
    Complete,
    Failed {
        error: String,
        st_json: Option<String>,
    },
}

impl ShieldStage {
    pub fn is_terminal(&self) -> bool {
        matches!(self, ShieldStage::Complete | ShieldStage::Failed { .. })
    }

    pub fn progress_fraction(&self) -> f32 {
        match self {
            ShieldStage::Queued => 0.0,
            ShieldStage::BuildingProof { .. } => 0.4,
            ShieldStage::WaitingToBroadcast => 0.6,
            ShieldStage::Broadcasting => 0.8,
            ShieldStage::Complete => 1.0,
            ShieldStage::Failed { .. } => 1.0,
        }
    }

    pub fn label(&self) -> String {
        match self {
            ShieldStage::Queued => "Queued".to_string(),
            ShieldStage::BuildingProof { nonce } => {
                format!("Building proof... (nonce: {})", nonce)
            }
            ShieldStage::WaitingToBroadcast => "Waiting to broadcast...".to_string(),
            ShieldStage::Broadcasting => {
                "Broadcasting & waiting for nonce confirmation...".to_string()
            }
            ShieldStage::Complete => "Complete".to_string(),
            ShieldStage::Failed { error, .. } => format!("Failed: {}", error),
        }
    }
}

/// Resolve the wallet's HD seed once and keep it in the session cache for the
/// rest of the app session, so a batch of [`build_shield_credit`] calls prompts
/// for the passphrase at most once. Call [`forget_batch_seed`] when the batch
/// finishes to drop the cached seed early.
pub async fn warm_seed_for_batch(
    app_context: &Arc<AppContext>,
    seed_hash: &WalletSeedHash,
) -> Result<(), TaskError> {
    let backend = app_context.wallet_backend()?;
    let scope = SecretScope::HdSeed {
        seed_hash: *seed_hash,
    };
    backend
        .secret_access()
        .with_secret_session(&scope, async |session| {
            session
                .plaintext()
                .expose_hd_seed()
                .ok_or(TaskError::WalletLocked)?;
            backend.secret_access().remember_session(
                &scope,
                session.plaintext(),
                RememberPolicy::UntilAppClose,
            );
            Ok(())
        })
        .await
}

/// Drop the batch-cached HD seed promoted by [`warm_seed_for_batch`].
pub fn forget_batch_seed(app_context: &Arc<AppContext>, seed_hash: &WalletSeedHash) {
    if let Ok(backend) = app_context.wallet_backend() {
        backend.secret_access().forget(&SecretScope::HdSeed {
            seed_hash: *seed_hash,
        });
    }
}

/// Build a Shield transition without broadcasting (for batch parallel mode).
///
/// Returns the built `StateTransition` so the caller can broadcast in nonce
/// order. The HD seed is fetched through the JIT chokepoint; warm the session
/// cache once before a batch so the prompt fires at most once for all builds.
pub async fn build_shield_credit(
    app_context: &Arc<AppContext>,
    seed_hash: &WalletSeedHash,
    recipient_payment_address: &PaymentAddress,
    amount: u64,
    from_address: PlatformAddress,
    nonce: u32,
) -> Result<dash_sdk::dpp::state_transition::StateTransition, TaskError> {
    let sdk = { app_context.sdk.load().as_ref().clone() };

    let prover = CachedProver {
        key: get_proving_key(),
    };
    let recipient_addr = payment_address_to_orchard(recipient_payment_address)?;

    let wallet_arc = {
        let wallets = app_context.wallets.read()?;
        wallets
            .get(seed_hash)
            .cloned()
            .ok_or(TaskError::WalletNotFound)?
    };

    let mut inputs = BTreeMap::new();
    inputs.insert(from_address, (nonce, amount));

    let fee_strategy: AddressFundsFeeStrategy =
        vec![AddressFundsFeeStrategyStep::DeductFromInput(0)];

    // Build the pure address→path index before touching the secret. The read
    // guard is dropped here so the seed scope below holds no wallet lock.
    let network = app_context.network;
    let path_index = {
        let wallet = wallet_arc.read()?;
        PlatformPathIndex::from_wallet(&wallet, network)
    };

    let backend = app_context.wallet_backend()?;
    let seed_hash = *seed_hash;
    // memo: 36-byte structured memo (4-byte type tag + 32-byte payload); all zeros = empty memo.
    // The seed is borrowed for this one build via `DetPlatformSigner` and
    // zeroizes when the scope returns.
    backend
        .secret_access()
        .with_secret_session(&SecretScope::HdSeed { seed_hash }, async |session| {
            let plaintext = session.plaintext();
            let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;
            let signer = DetPlatformSigner::from_held(seed, network, &path_index);
            build_shield_transition(
                &recipient_addr,
                amount,
                inputs,
                fee_strategy,
                &signer,
                0,
                &prover,
                [0u8; 36],
                sdk.version(),
            )
            .await
            .map_err(|e| shielded_build_error(e.to_string()))
        })
        .await
}

/// Build and broadcast a Shield transition (transparent -> shielded pool).
///
/// Uses the DPP builder which handles Orchard bundle construction internally
/// (including Halo 2 proof generation and RedPallas signature application).
pub async fn shield_credits(
    app_context: &Arc<AppContext>,
    seed_hash: &WalletSeedHash,
    recipient_payment_address: &PaymentAddress,
    amount: u64,
    from_address: PlatformAddress,
    nonce_override: Option<u32>,
    stage: Option<Arc<Mutex<ShieldStage>>>,
) -> Result<(), TaskError> {
    let sdk = { app_context.sdk.load().as_ref().clone() };

    let prover = CachedProver {
        key: get_proving_key(),
    };

    let recipient_addr = payment_address_to_orchard(recipient_payment_address)?;

    let wallet_arc = {
        let wallets = app_context.wallets.read()?;
        wallets
            .get(seed_hash)
            .cloned()
            .ok_or(TaskError::WalletNotFound)?
    };

    let nonce: u32 = if let Some(n) = nonce_override {
        n
    } else {
        let wallet = wallet_arc.read()?;
        wallet
            .platform_address_info
            .iter()
            .find_map(|(addr, info)| {
                let platform_addr = PlatformAddress::try_from(addr.clone()).ok()?;
                if platform_addr == from_address {
                    Some(info.nonce + 1)
                } else {
                    None
                }
            })
            .ok_or(TaskError::PlatformAddressNotFound)?
    };

    let mut inputs = BTreeMap::new();
    inputs.insert(from_address, (nonce, amount));

    let fee_strategy: AddressFundsFeeStrategy =
        vec![AddressFundsFeeStrategyStep::DeductFromInput(0)];

    tracing::info!(
        "Shield credits: {} ({} credits), nonce={}, building proof...",
        format_credits_as_dash(amount),
        amount,
        nonce,
    );

    if let Some(s) = &stage {
        *s.lock()? = ShieldStage::BuildingProof { nonce };
    }

    // Build the pure address→path index before the secret scope; the read
    // guard never crosses an await.
    let network = app_context.network;
    let path_index = {
        let wallet = wallet_arc.read()?;
        PlatformPathIndex::from_wallet(&wallet, network)
    };

    let backend = app_context.wallet_backend()?;
    let seed_hash = *seed_hash;
    // memo: 36-byte structured memo (4-byte type tag + 32-byte payload); all zeros = empty memo.
    // Sign the shield input through a JIT platform signer that borrows the HD
    // seed only for this build; the seed zeroizes when the scope returns.
    let state_transition = backend
        .secret_access()
        .with_secret_session(&SecretScope::HdSeed { seed_hash }, async |session| {
            let plaintext = session.plaintext();
            let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;
            let signer = DetPlatformSigner::from_held(seed, network, &path_index);
            build_shield_transition(
                &recipient_addr,
                amount,
                inputs,
                fee_strategy,
                &signer,
                0,
                &prover,
                [0u8; 36],
                sdk.version(),
            )
            .await
            .map_err(|e| shielded_build_error(e.to_string()))
        })
        .await?;

    if let Some(s) = &stage {
        *s.lock()? = ShieldStage::Broadcasting;
    }

    tracing::trace!("Shield credits: state transition built, broadcasting...");

    state_transition
        .broadcast(&sdk, None)
        .await
        .map_err(shielded_broadcast_error)?;

    state_transition
        .wait_for_response::<StateTransitionProofResult>(&sdk, None)
        .await
        .map_err(|e| {
            tracing::warn!("Shield credits broadcast succeeded but confirmation wait failed: {e}");
        })
        .ok();

    tracing::info!(
        "Shield credits broadcast succeeded: {}",
        format_credits_as_dash(amount),
    );

    Ok(())
}

/// Build and broadcast a ShieldedTransfer transition (pool -> pool).
///
/// Returns the nullifiers of the notes that were spent.
pub async fn shielded_transfer(
    app_context: &Arc<AppContext>,
    _seed_hash: &WalletSeedHash,
    shielded_state: &ShieldedWalletState,
    amount: u64,
    recipient_address_bytes: &[u8],
) -> Result<Vec<Nullifier>, TaskError> {
    let sdk = { app_context.sdk.load().as_ref().clone() };

    let prover = CachedProver {
        key: get_proving_key(),
    };

    let recipient_bytes: [u8; 43] = recipient_address_bytes
        .try_into()
        .map_err(|_| TaskError::ShieldedInvalidRecipientAddress)?;
    let recipient_addr = OrchardAddress::from_raw_bytes(&recipient_bytes)
        .map_err(|_| TaskError::ShieldedInvalidRecipientAddress)?;

    let (spendable_notes, total_input_value, exact_fee) =
        select_notes_with_fee(shielded_state, amount, 2, sdk.version())?;
    let change_amount = total_input_value
        .saturating_sub(amount)
        .saturating_sub(exact_fee);

    tracing::info!(
        "Shielded transfer: sending {} ({} credits), fee {} ({} credits), spending {} input note(s) totalling {} ({} credits), change: {} ({} credits)",
        format_credits_as_dash(amount),
        amount,
        format_credits_as_dash(exact_fee),
        exact_fee,
        spendable_notes.len(),
        format_credits_as_dash(total_input_value),
        total_input_value,
        format_credits_as_dash(change_amount),
        change_amount,
    );

    let spent_nullifiers: Vec<Nullifier> = spendable_notes.iter().map(|n| n.nullifier).collect();

    let (spends, anchor) = {
        let tree = shielded_state.commitment_tree.lock()?;
        extract_spends_and_anchor(&tree, &spendable_notes)?
    };

    let change_addr = payment_address_to_orchard(&shielded_state.keys.default_address)?;

    // memo: 36-byte structured memo (4-byte type tag + 32-byte payload); all zeros = empty memo.
    // The fee is no longer a caller argument: consensus pins a transfer's
    // `value_balance` to exactly `compute_minimum_shielded_fee`, so the builder
    // computes it internally and returns the applied fee alongside the transition.
    let (state_transition, _applied_fee) = build_shielded_transfer_transition(
        spends,
        &recipient_addr,
        amount,
        &change_addr,
        &shielded_state.keys.fvk,
        &shielded_state.keys.ask,
        anchor,
        &prover,
        [0u8; 36],
        sdk.version(),
    )
    .map_err(|e| shielded_build_error(e.to_string()))?;

    tracing::trace!("Shielded transfer: state transition built, broadcasting...");

    state_transition
        .broadcast(&sdk, None)
        .await
        .map_err(shielded_broadcast_error)?;

    state_transition
        .wait_for_response::<StateTransitionProofResult>(&sdk, None)
        .await
        .map_err(|e| {
            tracing::warn!(
                "Shielded transfer broadcast succeeded but confirmation wait failed: {e}"
            );
        })
        .ok();

    tracing::info!(
        "Shielded transfer broadcast succeeded: {} nullifiers created, change={}",
        spent_nullifiers.len(),
        change_amount > 0,
    );

    Ok(spent_nullifiers)
}

/// Build and broadcast an Unshield transition (shielded pool -> platform address).
///
/// Returns the nullifiers of the notes that were spent.
pub async fn unshield_credits(
    app_context: &Arc<AppContext>,
    _seed_hash: &WalletSeedHash,
    shielded_state: &ShieldedWalletState,
    amount: u64,
    to_platform_address: PlatformAddress,
) -> Result<Vec<Nullifier>, TaskError> {
    let sdk = { app_context.sdk.load().as_ref().clone() };

    let prover = CachedProver {
        key: get_proving_key(),
    };

    let (spendable_notes, total_input_value, exact_fee) =
        select_notes_with_fee(shielded_state, amount, 1, sdk.version())?;
    let change_amount = total_input_value
        .saturating_sub(amount)
        .saturating_sub(exact_fee);

    tracing::info!(
        "Unshield credits: {} ({} credits), fee {} ({} credits), spending {} input note(s) totalling {} ({} credits), change: {} ({} credits)",
        format_credits_as_dash(amount),
        amount,
        format_credits_as_dash(exact_fee),
        exact_fee,
        spendable_notes.len(),
        format_credits_as_dash(total_input_value),
        total_input_value,
        format_credits_as_dash(change_amount),
        change_amount,
    );

    let spent_nullifiers: Vec<Nullifier> = spendable_notes.iter().map(|n| n.nullifier).collect();

    let (spends, anchor) = {
        let tree = shielded_state.commitment_tree.lock()?;
        extract_spends_and_anchor(&tree, &spendable_notes)?
    };

    let change_addr = payment_address_to_orchard(&shielded_state.keys.default_address)?;

    // memo: 36-byte structured memo (4-byte type tag + 32-byte payload); all zeros = empty memo.
    // The builder now computes the consensus-pinned fee internally and returns
    // it alongside the transition, so the fee is no longer passed in.
    let (state_transition, _applied_fee) = build_unshield_transition(
        spends,
        to_platform_address,
        amount,
        &change_addr,
        &shielded_state.keys.fvk,
        &shielded_state.keys.ask,
        anchor,
        &prover,
        [0u8; 36],
        sdk.version(),
    )
    .map_err(|e| shielded_build_error(e.to_string()))?;

    tracing::trace!("Unshield credits: state transition built, broadcasting...");

    state_transition
        .broadcast(&sdk, None)
        .await
        .map_err(shielded_broadcast_error)?;

    state_transition
        .wait_for_response::<StateTransitionProofResult>(&sdk, None)
        .await
        .map_err(|e| {
            tracing::warn!(
                "Unshield credits broadcast succeeded but confirmation wait failed: {e}"
            );
        })
        .ok();

    tracing::info!(
        "Unshield credits broadcast succeeded: {} nullifiers created, change={}",
        spent_nullifiers.len(),
        change_amount > 0,
    );

    Ok(spent_nullifiers)
}

/// Build and broadcast a ShieldFromAssetLock transition (core DASH -> shielded pool via asset lock).
///
/// The asset lock is built, broadcast, and tracked to an InstantLock/ChainLock
/// proof by the upstream wallet (`WalletBackend::create_asset_lock_proof` with
/// [`AssetLockFundingType::AssetLockShieldedAddressTopUp`]) — coin selection
/// runs against the upstream authoritative live UTXO set at construction time,
/// with store-before-broadcast crash safety owned upstream. This function then
/// builds and broadcasts the Type 18 ShieldFromAssetLock state transition that
/// deposits credits directly into the shielded pool.
///
/// [`AssetLockFundingType::AssetLockShieldedAddressTopUp`]: platform_wallet::AssetLockFundingType::AssetLockShieldedAddressTopUp
pub async fn shield_from_asset_lock(
    app_context: &Arc<AppContext>,
    seed_hash: &WalletSeedHash,
    shielded_state: &ShieldedWalletState,
    amount_duffs: u64,
) -> Result<u64, TaskError> {
    use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
    use dash_sdk::dpp::shielded::builder::build_shield_from_asset_lock_transition;
    use platform_wallet::AssetLockFundingType;

    let proving_key = crate::context::shielded::get_proving_key();

    let (platform_fee_duffs, _l1_fee_duffs) = app_context
        .fee_estimator()
        .estimate_shield_from_core_fees_duffs();
    let asset_lock_duffs = amount_duffs.saturating_add(platform_fee_duffs);

    // Build + broadcast + track-to-finality the asset lock via the upstream
    // wallet. Selection, persistence-before-broadcast, and proof wait are all
    // upstream-authoritative — DET performs no coin selection here.
    let (asset_lock_proof, asset_lock_private_key, _tx_id) = app_context
        .wallet_backend()?
        .create_asset_lock_proof(
            seed_hash,
            asset_lock_duffs,
            AssetLockFundingType::AssetLockShieldedAddressTopUp,
            0,
        )
        .await?;

    // Build and broadcast the shield-from-asset-lock transition
    let sdk = { app_context.sdk.load().as_ref().clone() };

    let recipient = payment_address_to_orchard(&shielded_state.keys.default_address)?;
    let prover = CachedProver { key: proving_key };

    let shield_amount_credits =
        amount_duffs
            .checked_mul(CREDITS_PER_DUFF)
            .ok_or(TaskError::CreditCalculationOverflow {
                amount: amount_duffs,
                credits_per_duff: CREDITS_PER_DUFF,
            })?;

    tracing::info!(
        "Shield from asset lock: building state transition for {} ({} credits)",
        format_credits_as_dash(shield_amount_credits),
        shield_amount_credits,
    );

    // memo: 36-byte structured memo (4-byte type tag + 32-byte payload); all zeros = empty memo.
    // `surplus_output = None`: the asset-lock surplus (lock value − shield amount
    // − fee) folds into the fee pools rather than going to a separate address.
    let state_transition = build_shield_from_asset_lock_transition(
        &recipient,
        shield_amount_credits,
        asset_lock_proof,
        asset_lock_private_key.inner.as_ref(),
        &prover,
        [0u8; 36],
        None,
        sdk.version(),
    )
    .map_err(|e| shielded_build_error(e.to_string()))?;

    tracing::trace!("Shield from asset lock: state transition built, broadcasting...");

    state_transition
        .broadcast(&sdk, None)
        .await
        .map_err(shielded_broadcast_error)?;

    state_transition
        .wait_for_response::<StateTransitionProofResult>(&sdk, None)
        .await
        .map_err(|e| {
            tracing::warn!(
                "Shield from asset lock broadcast succeeded but confirmation wait failed: {e}"
            );
        })
        .ok();

    tracing::info!(
        "Shield from asset lock broadcast succeeded: {}",
        format_credits_as_dash(shield_amount_credits),
    );

    Ok(shield_amount_credits)
}

/// Build and broadcast a ShieldedWithdrawal transition (shielded pool -> core L1 address).
///
/// Returns the nullifiers of the notes that were spent.
pub async fn shielded_withdrawal(
    app_context: &Arc<AppContext>,
    _seed_hash: &WalletSeedHash,
    shielded_state: &ShieldedWalletState,
    amount: u64,
    to_core_address: Address,
) -> Result<Vec<Nullifier>, TaskError> {
    let sdk = { app_context.sdk.load().as_ref().clone() };

    let prover = CachedProver {
        key: get_proving_key(),
    };

    let output_script = CoreScript::from_bytes(to_core_address.script_pubkey().to_bytes());

    let (spendable_notes, total_input_value, exact_fee) =
        select_notes_with_fee(shielded_state, amount, 1, sdk.version())?;
    let change_amount = total_input_value
        .saturating_sub(amount)
        .saturating_sub(exact_fee);

    tracing::info!(
        "Shielded withdrawal: {} ({} credits) to core address, fee {} ({} credits), spending {} input note(s) totalling {} ({} credits), change: {} ({} credits)",
        format_credits_as_dash(amount),
        amount,
        format_credits_as_dash(exact_fee),
        exact_fee,
        spendable_notes.len(),
        format_credits_as_dash(total_input_value),
        total_input_value,
        format_credits_as_dash(change_amount),
        change_amount,
    );

    let spent_nullifiers: Vec<Nullifier> = spendable_notes.iter().map(|n| n.nullifier).collect();

    let (spends, anchor) = {
        let tree = shielded_state.commitment_tree.lock()?;
        extract_spends_and_anchor(&tree, &spendable_notes)?
    };

    let change_addr = payment_address_to_orchard(&shielded_state.keys.default_address)?;

    // memo: 36-byte structured memo (4-byte type tag + 32-byte payload); all zeros = empty memo.
    // The builder now computes the consensus-pinned fee internally and returns
    // it alongside the transition, so the fee is no longer passed in.
    let (state_transition, _applied_fee) = build_shielded_withdrawal_transition(
        spends,
        amount,
        output_script,
        1, // core_fee_per_byte
        Pooling::Standard,
        &change_addr,
        &shielded_state.keys.fvk,
        &shielded_state.keys.ask,
        anchor,
        &prover,
        [0u8; 36],
        sdk.version(),
    )
    .map_err(|e| shielded_build_error(e.to_string()))?;

    tracing::trace!("Shielded withdrawal: state transition built, broadcasting...");

    state_transition
        .broadcast(&sdk, None)
        .await
        .map_err(shielded_broadcast_error)?;

    state_transition
        .wait_for_response::<StateTransitionProofResult>(&sdk, None)
        .await
        .map_err(|e| {
            tracing::warn!(
                "Shielded withdrawal broadcast succeeded but confirmation wait failed: {e}"
            );
        })
        .ok();

    tracing::info!(
        "Shielded withdrawal broadcast succeeded: {} nullifiers created, change={}",
        spent_nullifiers.len(),
        change_amount > 0,
    );

    Ok(spent_nullifiers)
}

/// Select notes sufficient to cover `amount` plus the exact shielded fee.
///
/// Uses an iterative approach:
/// 1. Estimate fee for `min_actions` (the builder's minimum action count)
/// 2. Select notes for amount + estimated fee
/// 3. Compute exact fee from actual note count
/// 4. If insufficient, re-select with exact fee; repeat (converges in 2-3 iterations)
///
/// Returns the selected notes, total input value, and the exact fee.
fn select_notes_with_fee<'a>(
    shielded_state: &'a ShieldedWalletState,
    amount: u64,
    min_actions: usize,
    platform_version: &PlatformVersion,
) -> Result<
    (
        Vec<&'a crate::model::wallet::shielded::ShieldedNote>,
        u64,
        u64,
    ),
    TaskError,
> {
    let mut fee_estimate = shielded_fee_for_actions(min_actions, platform_version)
        .map_err(|source| TaskError::ShieldedFeeComputationFailed { source })?;

    for _ in 0..5 {
        let (notes, total) = select_notes_for_amount(shielded_state, amount, fee_estimate)?;
        let num_actions = notes.len().max(min_actions);
        let exact_fee = shielded_fee_for_actions(num_actions, platform_version)
            .map_err(|source| TaskError::ShieldedFeeComputationFailed { source })?;

        if total >= amount.saturating_add(exact_fee) {
            return Ok((notes, total, exact_fee));
        }

        fee_estimate = exact_fee;
    }

    // Final attempt with last computed fee
    let (notes, total) = select_notes_for_amount(shielded_state, amount, fee_estimate)?;
    let num_actions = notes.len().max(min_actions);
    let exact_fee = shielded_fee_for_actions(num_actions, platform_version)
        .map_err(|source| TaskError::ShieldedFeeComputationFailed { source })?;
    if total < amount.saturating_add(exact_fee) {
        return Err(TaskError::ShieldedInsufficientBalance {
            available: total,
            required: amount.saturating_add(exact_fee),
        });
    }
    Ok((notes, total, exact_fee))
}

/// Select unspent notes to cover `amount + fee_headroom` using a greedy algorithm.
///
/// The `fee_headroom` ensures selected inputs cover both the send amount
/// and the transition fee. Without it, sending the full balance fails
/// because the DPP builder adds fees on top of the selected amount.
///
/// The `required` amount in error messages includes the fee so the user
/// understands the total cost.
fn select_notes_for_amount(
    shielded_state: &ShieldedWalletState,
    amount: u64,
    fee_headroom: u64,
) -> Result<(Vec<&crate::model::wallet::shielded::ShieldedNote>, u64), TaskError> {
    let unspent: Vec<_> = shielded_state.unspent_notes();

    if unspent.is_empty() {
        return Err(TaskError::ShieldedNoUnspentNotes);
    }

    let required = amount.saturating_add(fee_headroom);
    let total_available: u64 = unspent.iter().map(|n| n.value).sum();
    if total_available < required {
        return Err(TaskError::ShieldedInsufficientBalance {
            available: total_available,
            required,
        });
    }

    let mut sorted: Vec<_> = unspent;
    sorted.sort_by(|a, b| b.value.cmp(&a.value));

    let mut selected = Vec::new();
    let mut accumulated = 0u64;

    for note in sorted {
        selected.push(note);
        accumulated += note.value;
        if accumulated >= required {
            break;
        }
    }

    Ok((selected, accumulated))
}

/// Extract spendable notes with Merkle witnesses and the tree anchor.
///
/// Locks the commitment tree, computes a Merkle path for each selected note,
/// and returns them alongside the current tree anchor for proof construction.
fn extract_spends_and_anchor(
    tree: &MutexGuard<'_, ClientPersistentCommitmentTree>,
    notes: &[&ShieldedNote],
) -> Result<(Vec<SpendableNote>, Anchor), TaskError> {
    let spends = notes
        .iter()
        .map(|note| {
            let merkle_path = tree
                .witness(note.position, 0)
                .map_err(|e| TaskError::ShieldedMerkleWitnessUnavailable {
                    detail: e.to_string(),
                })?
                .ok_or(TaskError::ShieldedMerkleWitnessUnavailable {
                    detail: "No Merkle path available for note".into(),
                })?;
            Ok(SpendableNote {
                note: note.note,
                merkle_path,
            })
        })
        .collect::<Result<Vec<_>, TaskError>>()?;

    let anchor = tree
        .anchor()
        .map_err(|e| TaskError::ShieldedMerkleWitnessUnavailable {
            detail: e.to_string(),
        })?;
    Ok((spends, anchor))
}

/// Convert a PaymentAddress to an OrchardAddress for the builder functions.
fn payment_address_to_orchard(addr: &PaymentAddress) -> Result<OrchardAddress, TaskError> {
    let raw = addr.to_raw_address_bytes();
    OrchardAddress::from_raw_bytes(&raw).map_err(|_| TaskError::ShieldedInvalidRecipientAddress)
}
