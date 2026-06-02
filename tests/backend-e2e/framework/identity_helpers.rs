//! Helpers for constructing identity registration data in tests.

use dash_evo_tool::backend_task::identity::{
    IdentityRegistrationInfo, build_identity_registration as build_identity_registration_inner,
};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::wallet::{Wallet, WalletSeedHash};
use dash_sdk::dpp::dashcore::Network;
use std::sync::{Arc, RwLock};

/// Asset lock amount in duffs for the e2e registration fixture. Platform
/// credits ≈ duffs × 1000 minus fees. 25M duffs → ~25B credits after fees,
/// enough for identity registration plus subsequent operations.
const E2E_FUNDING_DUFFS: u64 = 25_000_000;

/// Build an `IdentityRegistrationInfo` for a wallet-funded identity at index 0.
///
/// Derives the **public** master key + additional keys via the production
/// builder, which resolves the wallet's HD seed once through the JIT chokepoint.
/// The private keys are never materialized here — signing derives them
/// just-in-time at registration — so no raw key bytes are returned.
///
/// # Panics
///
/// Panics if the seed cannot be resolved or key derivation fails (test setup
/// error).
pub async fn build_identity_registration(
    app_context: &Arc<AppContext>,
    wallet_arc: &Arc<RwLock<Wallet>>,
    wallet_seed_hash: WalletSeedHash,
) -> IdentityRegistrationInfo {
    let identity_index: u32 = 0;
    let mut reg_info = build_identity_registration_inner(
        app_context,
        wallet_arc,
        identity_index,
        E2E_FUNDING_DUFFS,
    )
    .await
    .expect("Failed to build identity registration");
    reg_info.alias_input = format!("e2e-test-{}", hex::encode(&wallet_seed_hash[..4]));
    reg_info
}

/// Get a receive address string from a wallet.
pub fn get_receive_address(app_context: &AppContext, wallet_arc: &Arc<RwLock<Wallet>>) -> String {
    let mut wallet = wallet_arc.write().expect("wallet lock");
    wallet
        .receive_address(Network::Testnet, false, Some(app_context))
        .expect("Failed to get receive address")
        .to_string()
}
