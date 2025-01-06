use std::sync::{Arc, RwLock};

use dash_sdk::{
    dpp::{
        data_contract::accessors::v0::DataContractV0Getters,
        identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0,
    },
    platform::IdentityPublicKey,
};

use crate::{
    context::AppContext,
    model::{
        qualified_identity::{
            encrypted_key_storage::PrivateKeyData, PrivateKeyTarget, QualifiedIdentity,
        },
        wallet::Wallet,
    },
};

pub mod add_existing_identity_screen;
pub mod add_new_identity_screen;
mod funding_common;
pub mod identities_screen;
pub mod keys;
pub mod register_dpns_name_screen;
pub mod top_up_identity_screen;
pub mod transfers;
pub mod withdraw_from_identity_screen;

pub fn get_selected_wallet(
    qualified_identity: &QualifiedIdentity,
    app_context: Option<&AppContext>, // Used for getting DPNS contract in DPNS screen
    selected_key: Option<&IdentityPublicKey>, // Used for all other screens
    error_message: &mut Option<String>,
) -> Option<Arc<RwLock<Wallet>>> {
    // If `app_context` is provided, use the DPNS-based approach
    let public_key = if let Some(context) = app_context {
        let dpns_contract = &context.dpns_contract;

        // Attempt to fetch the preorder document type
        let preorder_document_type = match dpns_contract.document_type_for_name("preorder") {
            Ok(doc_type) => doc_type,
            Err(e) => {
                *error_message = Some(format!("DPNS preorder document type not found: {}", e));
                return None;
            }
        };

        // Attempt to retrieve the public key from the identity
        match qualified_identity.document_signing_key(&preorder_document_type) {
            Some(key) => key,
            None => {
                *error_message = Some(
                    "Identity doesn't have an authentication key for signing document transitions"
                        .to_string(),
                );
                return None;
            }
        }
    } else {
        // Fallback: directly use the provided selected key
        match selected_key {
            Some(key) => key,
            None => {
                *error_message = Some("No key provided when getting selected wallet".to_string());
                return None;
            }
        }
    };

    // Once we have the public key (by either route), grab its private key data
    let key_lookup = (PrivateKeyTarget::PrivateKeyOnMainIdentity, public_key.id());
    if let Some((_, PrivateKeyData::AtWalletDerivationPath(wallet_derivation_path))) =
        qualified_identity
            .private_keys
            .private_keys
            .get(&key_lookup)
    {
        qualified_identity
            .associated_wallets
            .get(&wallet_derivation_path.wallet_seed_hash)
            .cloned()
    } else {
        None
    }
}
