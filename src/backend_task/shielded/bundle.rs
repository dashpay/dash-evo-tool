use crate::context::AppContext;
use crate::context::shielded::get_proving_key;
use crate::model::wallet::WalletSeedHash;
use crate::model::wallet::shielded::ShieldedWalletState;
use dash_sdk::dpp::address_funds::{
    AddressFundsFeeStrategy, AddressFundsFeeStrategyStep, OrchardAddress, PlatformAddress,
};
use dash_sdk::dpp::dashcore::Address;
use dash_sdk::dpp::identity::core_script::CoreScript;
use dash_sdk::dpp::shielded::builder::{
    OrchardProver, SpendableNote, build_shield_transition, build_shielded_transfer_transition,
    build_shielded_withdrawal_transition, build_unshield_transition,
};
use dash_sdk::dpp::withdrawal::Pooling;
use dash_sdk::grovedb_commitment_tree::{Nullifier, PaymentAddress, ProvingKey};
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

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
    seed_hash: &WalletSeedHash,
    recipient_payment_address: &PaymentAddress,
    amount: u64,
    from_address: PlatformAddress,
    nonce: u32,
) -> Result<dash_sdk::dpp::state_transition::StateTransition, String> {
    let sdk = { app_context.sdk.load().as_ref().clone() };

    let prover = CachedProver { key: get_proving_key() };
    let recipient_addr = payment_address_to_orchard(recipient_payment_address)?;

    let wallet_arc = {
        let wallets = app_context.wallets.read().unwrap();
        wallets.get(seed_hash).cloned().ok_or("Wallet not found")?
    };

    let mut inputs = BTreeMap::new();
    inputs.insert(from_address, (nonce, amount));

    let fee_strategy: AddressFundsFeeStrategy =
        vec![AddressFundsFeeStrategyStep::DeductFromInput(0)];

    let wallet = wallet_arc.read().unwrap();
    build_shield_transition(
        &recipient_addr,
        amount,
        inputs,
        fee_strategy,
        &*wallet,
        0,
        &prover,
        [0u8; 36],
        sdk.version(),
    )
    .map_err(|e| format!("Failed to build shield transition: {e}"))
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
) -> Result<(), String> {
    let sdk = { app_context.sdk.load().as_ref().clone() };

    let prover = CachedProver { key: get_proving_key() };

    // Build recipient Orchard address from our default payment address
    let recipient_addr = payment_address_to_orchard(recipient_payment_address)?;

    // Get the wallet for signing and nonce lookup
    let wallet_arc = {
        let wallets = app_context.wallets.read().unwrap();
        wallets.get(seed_hash).cloned().ok_or("Wallet not found")?
    };

    // Get the nonce for the input address from the wallet's platform address info,
    // unless a nonce override was provided (batch parallel mode).
    let nonce: u32 = if let Some(n) = nonce_override {
        n
    } else {
        let wallet = wallet_arc.read().unwrap();
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
            .ok_or("Platform address not found in wallet")?
    };

    let mut inputs = BTreeMap::new();
    inputs.insert(from_address, (nonce, amount));

    let fee_strategy: AddressFundsFeeStrategy =
        vec![AddressFundsFeeStrategyStep::DeductFromInput(0)];

    if let Some(s) = &stage {
        *s.lock().unwrap() = ShieldStage::BuildingProof { nonce };
    }

    // Use the DPP builder which handles bundle construction internally
    let state_transition = {
        let wallet = wallet_arc.read().unwrap();
        build_shield_transition(
            &recipient_addr,
            amount,
            inputs,
            fee_strategy,
            &*wallet,
            0,
            &prover,
            [0u8; 36],
            sdk.version(),
        )
        .map_err(|e| format!("Failed to build shield transition: {e}"))?
    };

    if let Some(s) = &stage {
        *s.lock().unwrap() = ShieldStage::Broadcasting;
    }

    state_transition
        .broadcast(&sdk, None)
        .await
        .map_err(|e| format!("Failed to broadcast shield transition: {e}"))?;

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
) -> Result<Vec<Nullifier>, String> {
    let sdk = { app_context.sdk.load().as_ref().clone() };

    let prover = CachedProver { key: get_proving_key() };

    // Parse recipient address
    let recipient_bytes: [u8; 43] = recipient_address_bytes
        .try_into()
        .map_err(|_| "Invalid recipient address length, expected 43 bytes")?;
    let recipient_addr = OrchardAddress::from_raw_bytes(&recipient_bytes)
        .map_err(|e| format!("Invalid recipient address: {e}"))?;

    // Select notes to spend
    let (spendable_notes, _total_value) = select_notes_for_amount(shielded_state, amount)?;

    // Collect nullifiers of the notes we're about to spend
    let spent_nullifiers: Vec<Nullifier> = spendable_notes.iter().map(|n| n.nullifier).collect();

    // Get Merkle witness and anchor (lock scoped to avoid holding across await)
    let (spends, anchor) = {
        let tree = shielded_state.commitment_tree.lock().unwrap();
        let spends = spendable_notes
            .iter()
            .map(|note| {
                let merkle_path = tree
                    .witness(note.position, 0)
                    .map_err(|e| format!("Failed to get Merkle witness: {e}"))?
                    .ok_or("No Merkle path available for note")?;
                Ok(SpendableNote {
                    note: note.note,
                    merkle_path,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;

        let anchor = tree
            .anchor()
            .map_err(|e| format!("Failed to get tree anchor: {e}"))?;
        (spends, anchor)
    };

    let change_addr = payment_address_to_orchard(&shielded_state.keys.default_address)?;

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
        None,
        sdk.version(),
    )
    .map_err(|e| format!("Failed to build shielded transfer: {e}"))?;

    state_transition
        .broadcast(&sdk, None)
        .await
        .map_err(|e| format!("Failed to broadcast shielded transfer: {e}"))?;

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
) -> Result<Vec<Nullifier>, String> {
    let sdk = { app_context.sdk.load().as_ref().clone() };

    let prover = CachedProver { key: get_proving_key() };

    // Select notes to spend
    let (spendable_notes, _total_value) = select_notes_for_amount(shielded_state, amount)?;

    // Collect nullifiers of the notes we're about to spend
    let spent_nullifiers: Vec<Nullifier> = spendable_notes.iter().map(|n| n.nullifier).collect();

    // Get Merkle witness and anchor (lock scoped to avoid holding across await)
    let (spends, anchor) = {
        let tree = shielded_state.commitment_tree.lock().unwrap();
        let spends = spendable_notes
            .iter()
            .map(|note| {
                let merkle_path = tree
                    .witness(note.position, 0)
                    .map_err(|e| format!("Failed to get Merkle witness: {e}"))?
                    .ok_or("No Merkle path available for note")?;
                Ok(SpendableNote {
                    note: note.note,
                    merkle_path,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;

        let anchor = tree
            .anchor()
            .map_err(|e| format!("Failed to get tree anchor: {e}"))?;
        (spends, anchor)
    };

    let change_addr = payment_address_to_orchard(&shielded_state.keys.default_address)?;

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
        None,
        sdk.version(),
    )
    .map_err(|e| format!("Failed to build unshield transition: {e}"))?;

    state_transition
        .broadcast(&sdk, None)
        .await
        .map_err(|e| format!("Failed to broadcast unshield transition: {e}"))?;

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
    seed_hash: &WalletSeedHash,
    shielded_state: &ShieldedWalletState,
    amount_duffs: u64,
) -> Result<u64, String> {
    use dash_sdk::dashcore_rpc::RpcApi;
    use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
    use dash_sdk::dpp::prelude::AssetLockProof;
    use dash_sdk::dpp::shielded::builder::build_shield_from_asset_lock_transition;
    use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
    use std::time::Duration;

    let proving_key = crate::context::shielded::get_proving_key();

    // Platform charges a processing fee on top of the shielded amount, so we must
    // inflate the asset lock to cover both the shield amount and the fee.
    let platform_fee_credits = app_context
        .fee_estimator()
        .min_fees()
        .address_funding_asset_lock_cost;
    // Add a 20% safety buffer to the platform fee estimate
    let platform_fee_duffs = (platform_fee_credits / CREDITS_PER_DUFF).saturating_mul(120) / 100;
    let asset_lock_duffs = amount_duffs.saturating_add(platform_fee_duffs);

    // Step 1: Create the asset lock transaction
    let (asset_lock_transaction, asset_lock_private_key, _asset_lock_address, used_utxos) = {
        let wallet_arc = {
            let wallets = app_context.wallets.read().unwrap();
            wallets
                .get(seed_hash)
                .cloned()
                .ok_or_else(|| "Wallet not found".to_string())?
        };

        let mut wallet = wallet_arc.write().map_err(|e| e.to_string())?;

        // Try to create the asset lock transaction, reload UTXOs if needed
        match wallet.generic_asset_lock_transaction(
            app_context.network,
            asset_lock_duffs,
            false,
            Some(app_context.as_ref()),
        ) {
            Ok((tx, private_key, address, _change, utxos)) => (tx, private_key, address, utxos),
            Err(_) => {
                // Reload UTXOs and try again
                wallet
                    .reload_utxos(
                        &app_context
                            .core_client
                            .read()
                            .expect("Core client lock was poisoned"),
                        app_context.network,
                        Some(app_context.as_ref()),
                    )
                    .map_err(|e| e.to_string())?;

                let (tx, private_key, address, _change, utxos) = wallet
                    .generic_asset_lock_transaction(
                        app_context.network,
                        asset_lock_duffs,
                        false,
                        Some(app_context.as_ref()),
                    )?;
                (tx, private_key, address, utxos)
            }
        }
    };

    let tx_id = asset_lock_transaction.txid();

    // Step 2: Register this transaction as waiting for finality
    {
        let mut proofs = app_context
            .transactions_waiting_for_finality
            .lock()
            .unwrap();
        proofs.insert(tx_id, None);
    }

    // Step 3: Broadcast the transaction
    app_context
        .core_client
        .read()
        .expect("Core client lock was poisoned")
        .send_raw_transaction(&asset_lock_transaction)
        .map_err(|e| format!("Failed to broadcast asset lock transaction: {}", e))?;

    // Step 4: Remove used UTXOs from wallet
    {
        let wallet_arc = {
            let wallets = app_context.wallets.read().unwrap();
            wallets
                .get(seed_hash)
                .cloned()
                .ok_or_else(|| "Wallet not found".to_string())?
        };

        let mut wallet = wallet_arc.write().map_err(|e| e.to_string())?;
        wallet.utxos.retain(|_, utxo_map| {
            utxo_map.retain(|outpoint, _| !used_utxos.contains_key(outpoint));
            !utxo_map.is_empty()
        });

        for utxo in used_utxos.keys() {
            app_context
                .db
                .drop_utxo(utxo, &app_context.network.to_string())
                .map_err(|e| e.to_string())?;
        }

        wallet.recalculate_affected_address_balances(&used_utxos, app_context.as_ref())?;
    }

    // Step 5: Wait for asset lock proof (InstantLock or ChainLock) with timeout
    let asset_lock_proof: AssetLockProof;
    let timeout = tokio::time::sleep(Duration::from_secs(300)); // 5 minute timeout
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            _ = &mut timeout => {
                if let Ok(mut proofs) = app_context.transactions_waiting_for_finality.try_lock() {
                    proofs.remove(&tx_id);
                }

                if app_context.core_backend_mode() == crate::spv::CoreBackendMode::Rpc
                    && let Some(wallet_arc) = app_context.wallets.read().ok()
                        .and_then(|w| w.get(seed_hash).cloned())
                {
                    let ctx = Arc::clone(app_context);
                    tokio::task::spawn_blocking(move || {
                        if let Err(e) = ctx.refresh_wallet_info(wallet_arc) {
                            tracing::warn!("Failed to auto-refresh wallet after timeout: {}", e);
                        }
                    });
                }

                return Err("Timeout waiting for asset lock proof — no InstantLock or ChainLock received within 5 minutes".to_string());
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => {
                let proofs = app_context.transactions_waiting_for_finality.lock().unwrap();
                if let Some(Some(proof)) = proofs.get(&tx_id) {
                    asset_lock_proof = proof.clone();
                    break;
                }
            }
        }
    }

    // Step 6: Clean up the finality tracking
    {
        let mut proofs = app_context
            .transactions_waiting_for_finality
            .lock()
            .unwrap();
        proofs.remove(&tx_id);
    }

    // Step 7: Build and broadcast the shield-from-asset-lock transition
    let sdk = { app_context.sdk.load().as_ref().clone() };

    let recipient = payment_address_to_orchard(&shielded_state.keys.default_address)?;
    let prover = CachedProver { key: proving_key };

    // Shield only the user's requested amount; the extra duffs in the asset lock
    // cover the platform processing fee.
    let shield_amount_credits = amount_duffs.checked_mul(CREDITS_PER_DUFF).ok_or_else(|| {
        format!(
            "Overflow converting {} duffs to credits (CREDITS_PER_DUFF = {})",
            amount_duffs, CREDITS_PER_DUFF
        )
    })?;

    let state_transition = build_shield_from_asset_lock_transition(
        &recipient,
        shield_amount_credits,
        asset_lock_proof,
        asset_lock_private_key.inner.as_ref(),
        &prover,
        [0u8; 36],
        sdk.version(),
    )
    .map_err(|e| format!("Failed to build shield-from-asset-lock transition: {e}"))?;

    state_transition
        .broadcast(&sdk, None)
        .await
        .map_err(|e| format!("Failed to broadcast shield-from-asset-lock transition: {e}"))?;

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
) -> Result<Vec<Nullifier>, String> {
    let sdk = { app_context.sdk.load().as_ref().clone() };

    let prover = CachedProver { key: get_proving_key() };

    let output_script = CoreScript::from_bytes(to_core_address.script_pubkey().to_bytes());

    let (spendable_notes, _total_value) = select_notes_for_amount(shielded_state, amount)?;
    let spent_nullifiers: Vec<Nullifier> = spendable_notes.iter().map(|n| n.nullifier).collect();

    let (spends, anchor) = {
        let tree = shielded_state.commitment_tree.lock().unwrap();
        let spends = spendable_notes
            .iter()
            .map(|note| {
                let merkle_path = tree
                    .witness(note.position, 0)
                    .map_err(|e| format!("Failed to get Merkle witness: {e}"))?
                    .ok_or("No Merkle path available for note")?;
                Ok(SpendableNote {
                    note: note.note,
                    merkle_path,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;

        let anchor = tree
            .anchor()
            .map_err(|e| format!("Failed to get tree anchor: {e}"))?;
        (spends, anchor)
    };

    let change_addr = payment_address_to_orchard(&shielded_state.keys.default_address)?;

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
        None,
        sdk.version(),
    )
    .map_err(|e| format!("Failed to build shielded withdrawal transition: {e}"))?;

    state_transition
        .broadcast(&sdk, None)
        .await
        .map_err(|e| format!("Failed to broadcast shielded withdrawal transition: {e}"))?;

    Ok(spent_nullifiers)
}

/// Select notes to cover the requested amount using a greedy algorithm.
fn select_notes_for_amount(
    shielded_state: &ShieldedWalletState,
    amount: u64,
) -> Result<(Vec<&crate::model::wallet::shielded::ShieldedNote>, u64), String> {
    let unspent: Vec<_> = shielded_state.unspent_notes();

    if unspent.is_empty() {
        return Err("No unspent shielded notes available".to_string());
    }

    let total_available: u64 = unspent.iter().map(|n| n.value).sum();
    if total_available < amount {
        return Err(format!(
            "Insufficient shielded balance: have {}, need {}",
            total_available, amount
        ));
    }

    let mut sorted: Vec<_> = unspent;
    sorted.sort_by(|a, b| b.value.cmp(&a.value));

    let mut selected = Vec::new();
    let mut accumulated = 0u64;

    for note in sorted {
        selected.push(note);
        accumulated += note.value;
        if accumulated >= amount {
            break;
        }
    }

    Ok((selected, accumulated))
}

/// Convert a PaymentAddress to an OrchardAddress for the builder functions.
fn payment_address_to_orchard(addr: &PaymentAddress) -> Result<OrchardAddress, String> {
    let raw = addr.to_raw_address_bytes();
    OrchardAddress::from_raw_bytes(&raw).map_err(|e| format!("Invalid orchard address: {e}"))
}
