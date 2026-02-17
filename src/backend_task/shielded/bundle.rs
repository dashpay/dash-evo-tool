use crate::context::AppContext;
use crate::context::shielded::get_proving_key;
use crate::model::wallet::WalletSeedHash;
use crate::model::wallet::shielded::ShieldedWalletState;
use dash_sdk::dpp::address_funds::{AddressFundsFeeStrategy, OrchardAddress, PlatformAddress};
use dash_sdk::dpp::shielded::builder::{
    SpendableNote, build_shield_transition, build_shielded_transfer_transition,
    build_unshield_transition,
};
use dash_sdk::grovedb_commitment_tree::PaymentAddress;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Build and broadcast a Shield transition (transparent -> shielded pool).
///
/// Uses the DPP builder which handles Orchard bundle construction internally
/// (including Halo 2 proof generation and RedPallas signature application).
pub async fn shield_credits(
    app_context: &Arc<AppContext>,
    seed_hash: &WalletSeedHash,
    shielded_state: &ShieldedWalletState,
    amount: u64,
    from_address: PlatformAddress,
) -> Result<(), String> {
    let sdk = {
        let guard = app_context.sdk.read().unwrap();
        guard.clone()
    };

    let proving_key = get_proving_key();

    // Build recipient Orchard address from our default payment address
    let recipient_addr = payment_address_to_orchard(&shielded_state.keys.default_address);

    // Get the wallet for signing and nonce lookup
    let wallet_arc = {
        let wallets = app_context.wallets.read().unwrap();
        wallets.get(seed_hash).cloned().ok_or("Wallet not found")?
    };

    // Get the nonce for the input address from the wallet's platform address info
    let (nonce, _balance) = {
        let wallet = wallet_arc.read().unwrap();
        wallet
            .platform_address_info
            .iter()
            .find_map(|(addr, info)| {
                let platform_addr = PlatformAddress::try_from(addr.clone()).ok()?;
                if platform_addr == from_address {
                    Some((info.nonce + 1, info.balance))
                } else {
                    None
                }
            })
            .ok_or("Platform address not found in wallet")?
    };

    let mut inputs = BTreeMap::new();
    inputs.insert(from_address, (nonce, amount));

    let fee_strategy: AddressFundsFeeStrategy = vec![];

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
            proving_key,
            [0u8; 36],
            sdk.version(),
        )
        .map_err(|e| format!("Failed to build shield transition: {e}"))?
    };

    state_transition
        .broadcast(&sdk, None)
        .await
        .map_err(|e| format!("Failed to broadcast shield transition: {e}"))?;

    Ok(())
}

/// Build and broadcast a ShieldedTransfer transition (pool -> pool).
pub async fn shielded_transfer(
    app_context: &Arc<AppContext>,
    _seed_hash: &WalletSeedHash,
    shielded_state: &ShieldedWalletState,
    amount: u64,
    recipient_address_bytes: &[u8],
) -> Result<(), String> {
    let sdk = {
        let guard = app_context.sdk.read().unwrap();
        guard.clone()
    };

    let proving_key = get_proving_key();

    // Parse recipient address
    let recipient_bytes: [u8; 43] = recipient_address_bytes
        .try_into()
        .map_err(|_| "Invalid recipient address length, expected 43 bytes")?;
    let recipient_addr = OrchardAddress::from_raw_bytes(&recipient_bytes);

    // Select notes to spend
    let (spendable_notes, _total_value) = select_notes_for_amount(shielded_state, amount)?;

    // Get Merkle witness for each note
    let spends = spendable_notes
        .iter()
        .map(|note| {
            let merkle_path = shielded_state
                .commitment_tree
                .witness(note.position, 0)
                .map_err(|e| format!("Failed to get Merkle witness: {e}"))?
                .ok_or("No Merkle path available for note")?;
            Ok(SpendableNote {
                note: note.note,
                merkle_path,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let anchor = shielded_state
        .commitment_tree
        .anchor()
        .map_err(|e| format!("Failed to get tree anchor: {e}"))?;

    let change_addr = payment_address_to_orchard(&shielded_state.keys.default_address);

    let state_transition = build_shielded_transfer_transition(
        spends,
        &recipient_addr,
        amount,
        &change_addr,
        &shielded_state.keys.fvk,
        &shielded_state.keys.ask,
        anchor,
        proving_key,
        [0u8; 36],
        sdk.version(),
    )
    .map_err(|e| format!("Failed to build shielded transfer: {e}"))?;

    state_transition
        .broadcast(&sdk, None)
        .await
        .map_err(|e| format!("Failed to broadcast shielded transfer: {e}"))?;

    Ok(())
}

/// Build and broadcast an Unshield transition (shielded pool -> platform address).
pub async fn unshield_credits(
    app_context: &Arc<AppContext>,
    _seed_hash: &WalletSeedHash,
    shielded_state: &ShieldedWalletState,
    amount: u64,
    to_platform_address: PlatformAddress,
) -> Result<(), String> {
    let sdk = {
        let guard = app_context.sdk.read().unwrap();
        guard.clone()
    };

    let proving_key = get_proving_key();

    // Select notes to spend
    let (spendable_notes, _total_value) = select_notes_for_amount(shielded_state, amount)?;

    // Get Merkle witness for each note
    let spends = spendable_notes
        .iter()
        .map(|note| {
            let merkle_path = shielded_state
                .commitment_tree
                .witness(note.position, 0)
                .map_err(|e| format!("Failed to get Merkle witness: {e}"))?
                .ok_or("No Merkle path available for note")?;
            Ok(SpendableNote {
                note: note.note,
                merkle_path,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let anchor = shielded_state
        .commitment_tree
        .anchor()
        .map_err(|e| format!("Failed to get tree anchor: {e}"))?;

    let change_addr = payment_address_to_orchard(&shielded_state.keys.default_address);

    let state_transition = build_unshield_transition(
        spends,
        to_platform_address,
        amount,
        &change_addr,
        &shielded_state.keys.fvk,
        &shielded_state.keys.ask,
        anchor,
        proving_key,
        [0u8; 36],
        sdk.version(),
    )
    .map_err(|e| format!("Failed to build unshield transition: {e}"))?;

    state_transition
        .broadcast(&sdk, None)
        .await
        .map_err(|e| format!("Failed to broadcast unshield transition: {e}"))?;

    Ok(())
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
fn payment_address_to_orchard(addr: &PaymentAddress) -> OrchardAddress {
    let raw = addr.to_raw_address_bytes();
    OrchardAddress::from_raw_bytes(&raw)
}
