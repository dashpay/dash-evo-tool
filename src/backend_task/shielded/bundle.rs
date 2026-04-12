use crate::backend_task::error::{TaskError, shielded_broadcast_error, shielded_build_error};
use crate::context::AppContext;
use crate::context::shielded::get_proving_key;
use crate::model::fee_estimation::{format_credits_as_dash, shielded_fee_for_actions};
use crate::model::wallet::WalletId;
use crate::model::wallet::shielded::{ShieldedNote, ShieldedWalletState};
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

/// Build a Shield transition without broadcasting (for batch parallel mode).
///
/// Returns the built `StateTransition` so the caller can broadcast in nonce order.
pub fn build_shield_credit(
    app_context: &Arc<AppContext>,
    seed_hash: &WalletId,
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

    let wallet = wallet_arc.read()?;
    let platform_wallet = wallet
        .platform_wallet
        .as_ref()
        .ok_or(TaskError::WalletLocked)?;
    // memo: 36-byte structured memo (4-byte type tag + 32-byte payload); all zeros = empty memo
    build_shield_transition(
        &recipient_addr,
        amount,
        inputs,
        fee_strategy,
        platform_wallet.platform(),
        0,
        &prover,
        [0u8; 36],
        sdk.version(),
    )
    .map_err(|e| shielded_build_error(e.to_string()))
}

/// Build and broadcast a Shield transition (transparent -> shielded pool).
///
/// Uses the DPP builder which handles Orchard bundle construction internally
/// (including Halo 2 proof generation and RedPallas signature application).
pub async fn shield_credits(
    app_context: &Arc<AppContext>,
    seed_hash: &WalletId,
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
        let core_addr = from_address.to_address_with_network(app_context.network);
        let (_balance, db_nonce) = app_context
            .db
            .get_platform_address_info(&wallet.seed_hash(), &core_addr, &app_context.network)
            .map_err(|_| TaskError::PlatformAddressNotFound)?
            .ok_or(TaskError::PlatformAddressNotFound)?;
        db_nonce + 1
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

    let state_transition = {
        let wallet = wallet_arc.read()?;
        let platform_wallet = wallet
            .platform_wallet
            .as_ref()
            .ok_or(TaskError::WalletLocked)?;
        // memo: 36-byte structured memo (4-byte type tag + 32-byte payload); all zeros = empty memo
        build_shield_transition(
            &recipient_addr,
            amount,
            inputs,
            fee_strategy,
            platform_wallet.platform(),
            0,
            &prover,
            [0u8; 36],
            sdk.version(),
        )
        .map_err(|e| shielded_build_error(e.to_string()))?
    };

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
    _seed_hash: &WalletId,
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

    // memo: 36-byte structured memo (4-byte type tag + 32-byte payload); all zeros = empty memo
    let state_transition = build_shielded_transfer_transition(
        spends,
        &recipient_addr,
        amount,
        &change_addr,
        &shielded_state.keys.fvk,
        &shielded_state.keys.ask,
        anchor,
        &prover,
        [0u8; 36],
        Some(exact_fee),
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
    _seed_hash: &WalletId,
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

    // memo: 36-byte structured memo (4-byte type tag + 32-byte payload); all zeros = empty memo
    let state_transition = build_unshield_transition(
        spends,
        to_platform_address,
        amount,
        &change_addr,
        &shielded_state.keys.fvk,
        &shielded_state.keys.ask,
        anchor,
        &prover,
        [0u8; 36],
        Some(exact_fee),
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
/// Creates an asset lock transaction from wallet UTXOs, broadcasts it, waits for
/// an InstantLock/ChainLock proof, then builds and broadcasts a Type 18
/// ShieldFromAssetLock state transition that deposits credits directly into the
/// shielded pool.
pub async fn shield_from_asset_lock(
    app_context: &Arc<AppContext>,
    seed_hash: &WalletId,
    shielded_state: &ShieldedWalletState,
    amount_duffs: u64,
    source_address: Option<&Address>,
) -> Result<u64, TaskError> {
    use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
    use dash_sdk::dpp::shielded::builder::build_shield_from_asset_lock_transition;
    use std::time::Duration;

    let proving_key = crate::context::shielded::get_proving_key();

    let (platform_fee_duffs, _l1_fee_duffs) = app_context
        .fee_estimator()
        .estimate_shield_from_core_fees_duffs();
    let asset_lock_duffs = amount_duffs.saturating_add(platform_fee_duffs);

    // Step 1: Create the asset lock transaction
    let platform_wallet = {
        let wallet_arc = {
            let wallets = app_context.wallets.read()?;
            wallets
                .get(seed_hash)
                .cloned()
                .ok_or(TaskError::WalletNotFound)?
        };

        let wallet = wallet_arc
            .read()
            .map_err(|_| TaskError::LockPoisoned { resource: "wallet" })?;

        wallet
            .platform_wallet
            .clone()
            .ok_or(TaskError::WalletNotFound)?
    };

    let (asset_lock_transaction, _asset_lock_private_key) = platform_wallet
        .asset_locks()
        .build_asset_lock_transaction(
            asset_lock_duffs,
            0,
            platform_wallet::AssetLockFundingType::IdentityRegistration,
            0,
        )
        .await
        .map_err(|e| shielded_build_error(e.to_string()))?;

    let tx_id = asset_lock_transaction.txid();
    let out_point = dash_sdk::dpp::dashcore::OutPoint::new(tx_id, 0);

    // Step 2–5: Register with AssetLockManager, broadcast via DAPI, and wait
    // for finality proof (IS-lock or ChainLock). The manager handles the full
    // lifecycle internally via SPV event subscription.
    platform_wallet.asset_locks().recover_asset_lock_blocking(
        asset_lock_transaction.clone(),
        asset_lock_duffs,
        0,
        platform_wallet::AssetLockFundingType::IdentityRegistration,
        0,
        out_point,
        None,
    );

    let (asset_lock_proof, asset_lock_private_key) = platform_wallet
        .asset_locks()
        .resume_asset_lock(&out_point, Duration::from_secs(300))
        .await
        .map_err(|e| shielded_build_error(e.to_string()))?;

    // Step 7: Build and broadcast the shield-from-asset-lock transition
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

    // memo: 36-byte structured memo (4-byte type tag + 32-byte payload); all zeros = empty memo
    let state_transition = build_shield_from_asset_lock_transition(
        &recipient,
        shield_amount_credits,
        asset_lock_proof,
        asset_lock_private_key.inner.as_ref(),
        &prover,
        [0u8; 36],
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
    _seed_hash: &WalletId,
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

    // memo: 36-byte structured memo (4-byte type tag + 32-byte payload); all zeros = empty memo
    let state_transition = build_shielded_withdrawal_transition(
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
        Some(exact_fee),
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
    let mut fee_estimate = shielded_fee_for_actions(min_actions, platform_version);

    for _ in 0..5 {
        let (notes, total) = select_notes_for_amount(shielded_state, amount, fee_estimate)?;
        let num_actions = notes.len().max(min_actions);
        let exact_fee = shielded_fee_for_actions(num_actions, platform_version);

        if total >= amount.saturating_add(exact_fee) {
            return Ok((notes, total, exact_fee));
        }

        fee_estimate = exact_fee;
    }

    // Final attempt with last computed fee
    let (notes, total) = select_notes_for_amount(shielded_state, amount, fee_estimate)?;
    let num_actions = notes.len().max(min_actions);
    let exact_fee = shielded_fee_for_actions(num_actions, platform_version);
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
