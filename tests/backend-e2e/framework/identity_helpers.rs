//! Helpers for constructing identity registration data in tests.

use dash_evo_tool::backend_task::identity::default_identity_key_specs;
use dash_evo_tool::backend_task::identity::{
    IdentityKeys, IdentityRegistrationInfo, KeyInput, RegisterIdentityFundingMethod,
};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::wallet::{Wallet, WalletSeedHash};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::KeyType;
use std::sync::{Arc, RwLock};

/// Build an `IdentityRegistrationInfo` for a wallet-funded identity.
///
/// Derives master key + additional keys from the wallet at identity_index 0.
/// Returns the registration info AND the raw master authentication private key
/// bytes (32 bytes). The key bytes must be captured here because the wallet
/// encrypts them after registration, making post-registration extraction
/// impossible.
///
/// # Panics
///
/// Panics if key derivation fails (programming error in test setup).
pub fn build_identity_registration(
    app_context: &Arc<AppContext>,
    wallet_arc: &Arc<RwLock<Wallet>>,
    wallet_seed_hash: WalletSeedHash,
) -> (IdentityRegistrationInfo, Vec<u8>) {
    let dashpay_contract_id = app_context.dashpay_contract_id();
    let key_specs = default_identity_key_specs(dashpay_contract_id);

    let identity_index: u32 = 0;
    let mut wallet = wallet_arc.write().expect("wallet lock");

    // Derive master key (identity authentication key at index 0)
    let (master_private_key, master_derivation_path) = wallet
        .identity_authentication_ecdsa_private_key(app_context, Network::Testnet, identity_index, 0)
        .expect("Failed to derive master private key");

    // Derive additional keys from specs
    let mut keys_input: Vec<KeyInput> = Vec::new();
    for (i, (key_type, purpose, security_level, contract_bounds)) in
        key_specs.into_iter().enumerate()
    {
        let key_index = (i + 1) as u32; // 0 is master
        let (private_key, derivation_path) = wallet
            .identity_authentication_ecdsa_private_key(
                app_context,
                Network::Testnet,
                identity_index,
                key_index,
            )
            .expect("Failed to derive key");
        keys_input.push((
            (private_key, derivation_path),
            key_type,
            purpose,
            security_level,
            contract_bounds,
        ));
    }

    drop(wallet);

    let master_key_bytes = master_private_key.inner.secret_bytes().to_vec();

    let reg_info = IdentityRegistrationInfo {
        alias_input: format!("e2e-test-{}", hex::encode(&wallet_seed_hash[..4])),
        keys: IdentityKeys::new(
            Some((master_private_key, master_derivation_path)),
            KeyType::ECDSA_HASH160,
            keys_input,
        ),
        wallet: wallet_arc.clone(),
        wallet_identity_index: identity_index,
        identity_funding_method: RegisterIdentityFundingMethod::FundWithWallet(
            // Asset lock amount in duffs. Platform credits ≈ duffs × 1000 minus fees.
            // 25M duffs → ~25B credits after fees.
            // Token contract registration: ~20B credits (base 10B + token 10B).
            // Identity registration: ~241M credits.
            // Remaining: ~5B for subsequent operations (top-up, transfer, etc.).
            25_000_000,
            identity_index,
        ),
    };

    (reg_info, master_key_bytes)
}

/// Get a receive address string from a wallet.
///
/// Extracts the `PlatformWallet` under a short sync lock, then drops the lock
/// before awaiting the async `next_receive_address` (which takes a tokio write
/// lock on `PlatformWalletInfo`). Holding `std::sync::RwLock` across `.await`
/// would deadlock with SPV block processing.
pub async fn get_receive_address(_app_context: &AppContext, wallet_arc: &Arc<RwLock<Wallet>>) -> String {
    let pw = {
        let wallet = wallet_arc.read().expect("wallet lock");
        wallet.platform_wallet.clone().expect("platform wallet must exist")
    };
    pw.core()
        .next_receive_address()
        .await
        .expect("Failed to get receive address")
        .to_string()
}
