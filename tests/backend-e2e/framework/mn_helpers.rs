//! Helpers for loading masternode/evonode identity credentials from env vars.

use dash_evo_tool::backend_task::identity::IdentityInputToLoad;
use dash_evo_tool::model::qualified_identity::IdentityType;
use dash_evo_tool::model::secret::Secret;

/// Credentials for a registered testnet masternode/evonode, loaded from env vars.
pub struct MnCredentials {
    pub protx_hash: String,
    pub voting_key: Option<String>,
    pub owner_key: Option<String>,
    pub payout_key: Option<String>,
}

/// Read `E2E_MN_*` env vars. Returns `None` if `E2E_MN_PROTX_HASH` is not set.
pub fn load_mn_credentials() -> Option<MnCredentials> {
    let protx_hash = std::env::var("E2E_MN_PROTX_HASH").ok()?;
    if protx_hash.is_empty() {
        return None;
    }
    Some(MnCredentials {
        protx_hash,
        voting_key: non_empty_env("E2E_MN_VOTING_KEY"),
        owner_key: non_empty_env("E2E_MN_OWNER_KEY"),
        payout_key: non_empty_env("E2E_MN_PAYOUT_KEY"),
    })
}

/// Build an `IdentityInputToLoad` from masternode credentials.
pub fn build_mn_identity_input(
    creds: &MnCredentials,
    identity_type: IdentityType,
) -> IdentityInputToLoad {
    IdentityInputToLoad {
        identity_id_input: creds.protx_hash.clone(),
        identity_type,
        alias_input: String::new(),
        voting_private_key_input: Secret::new(creds.voting_key.as_deref().unwrap_or("")),
        owner_private_key_input: Secret::new(creds.owner_key.as_deref().unwrap_or("")),
        payout_address_private_key_input: Secret::new(creds.payout_key.as_deref().unwrap_or("")),
        keys_input: vec![],
        derive_keys_from_wallets: false,
        selected_wallet_id: None,
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}
