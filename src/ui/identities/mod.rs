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
pub mod transfer_screen;
pub mod withdraw_screen;

/// Retrieves the appropriate wallet (if any) associated with the given identity.
///
/// # Description
///
/// This function tries to determine which wallet should be used, either via:
///
/// - The DPNS-based approach (if [`AppContext`] is provided), which looks up
///   the `preorder` document type in the DPNS contract and retrieves the
///   document-signing key from the given [`QualifiedIdentity`].
/// - The fallback approach (if `app_context` is `None`), which relies on a
///   directly provided key (`selected_key`).
///
/// # Parameters
///
/// - `qualified_identity`: A reference to the [`QualifiedIdentity`], which holds
///   the identity, keys, and associated wallets.
/// - `app_context`: Optional reference to the [`AppContext`] which contains the
///   DPNS contract. When present, DPNS logic is used to find the public key.
/// - `selected_key`: An optional reference to a chosen [`IdentityPublicKey`].
///   When `app_context` is not provided, this is required to get the wallet.
/// - `error_message`: A mutable optional string where any error message will
///   be written if the function fails to retrieve a wallet.
///
/// # Returns
///
/// Returns `Some(Arc<RwLock<Wallet>>)` if a matching wallet is found, or `None`
/// otherwise. If an error is encountered, an explanatory message is placed in
/// `error_message`.
///
/// # Errors
///
/// - If the DPNS document type can't be found or the identity is missing the
///   required DPNS signing key (when `app_context` is provided).
/// - If no `selected_key` is provided (when `app_context` is `None`).
/// - If the derived wallet derivation path is missing from the
///   [`QualifiedIdentity`].
pub fn get_selected_wallet(
    qualified_identity: &QualifiedIdentity,
    app_context: Option<&AppContext>, // Used for DPNS-based logic (the first scenario).
    selected_key: Option<&IdentityPublicKey>, // Used for direct-key logic (the fallback scenario).
    error_message: &mut Option<String>,
) -> Option<Arc<RwLock<Wallet>>> {
    // If `app_context` is provided, use the DPNS-based approach.
    let public_key = if let Some(context) = app_context {
        let dpns_contract = &context.dpns_contract;

        // Attempt to fetch the `preorder` document type from the DPNS contract.
        let preorder_document_type = match dpns_contract.document_type_for_name("preorder") {
            Ok(doc_type) => doc_type,
            Err(e) => {
                *error_message = Some(format!("DPNS preorder document type not found: {}", e));
                return None;
            }
        };

        // Attempt to retrieve the public key from the identity.
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
        // Fallback: directly use the provided selected key.
        match selected_key {
            Some(key) => key,
            None => {
                *error_message = Some("No key provided when getting selected wallet".to_string());
                return None;
            }
        }
    };

    // Once we have the public key (either from DPNS or directly), look up
    // the matching private key data in `qualified_identity`.
    let key_lookup = (PrivateKeyTarget::PrivateKeyOnMainIdentity, public_key.id());
    if let Some((_, PrivateKeyData::AtWalletDerivationPath(wallet_derivation_path))) =
        qualified_identity
            .private_keys
            .private_keys
            .get(&key_lookup)
    {
        // If found, return the associated wallet (cloned to preserve Arc).
        qualified_identity
            .associated_wallets
            .get(&wallet_derivation_path.wallet_seed_hash)
            .cloned()
    } else {
        None
    }
}
