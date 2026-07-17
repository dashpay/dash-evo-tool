use std::error::Error as StdError;
use std::fmt;
use std::sync::{Arc, RwLock};

use dash_sdk::{
    dpp::{
        data_contract::accessors::v0::DataContractV0Getters,
        data_contract::errors::DataContractError,
    },
    platform::IdentityPublicKey,
};

use crate::{
    context::AppContext,
    model::{qualified_identity::QualifiedIdentity, wallet::Wallet},
    ui::{MessageType, components::MessageBanner},
};

pub mod add_existing_identity_screen;
pub mod add_new_identity_screen;
pub mod funding_common;
pub mod identities_screen;
pub mod keys;
pub mod register_dpns_name_screen;
pub mod top_up_identity_screen;
pub mod transfer_screen;
pub mod withdraw_screen;

#[derive(Debug)]
pub enum SelectedWalletError {
    DpnsPreorderDocumentTypeNotFound {
        source: DataContractError,
    },
    MissingDocumentSigningKey {
        identity_label: Option<String>,
        identity_id: String,
    },
    NoKeyProvided,
}

impl fmt::Display for SelectedWalletError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DpnsPreorderDocumentTypeNotFound { .. } => {
                write!(f, "DPNS preorder document type not found")
            }
            Self::MissingDocumentSigningKey {
                identity_label,
                identity_id,
            } => {
                if let Some(identity_label) = identity_label {
                    write!(
                        f,
                        "Identity {identity_label} ({identity_id}) cannot sign this action \
                         because it has no high-security signing key. Add a high-security \
                         signing key to this identity, or choose a different identity."
                    )
                } else {
                    write!(
                        f,
                        "Identity {identity_id} cannot sign this action because it has no \
                         high-security signing key. Add a high-security signing key to this \
                         identity, or choose a different identity."
                    )
                }
            }
            Self::NoKeyProvided => write!(f, "No key provided when getting selected wallet"),
        }
    }
}

impl StdError for SelectedWalletError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::DpnsPreorderDocumentTypeNotFound { source } => Some(source),
            Self::MissingDocumentSigningKey { .. } | Self::NoKeyProvided => None,
        }
    }
}

/// Returns `true` if the given error is the expected "identity has no
/// document-signing key" error from [`get_selected_wallet`].
pub fn is_missing_document_signing_key_error(error: &SelectedWalletError) -> bool {
    matches!(error, SelectedWalletError::MissingDocumentSigningKey { .. })
}

/// Applies the auto-selection policy for screen construction:
/// suppress the expected missing-document-signing-key error for
/// auto-selected identities, but surface every other failure.
pub fn auto_selected_wallet_or_banner(
    ctx: &egui::Context,
    result: Result<Option<Arc<RwLock<Wallet>>>, SelectedWalletError>,
) -> Option<Arc<RwLock<Wallet>>> {
    match result {
        Ok(wallet) => wallet,
        Err(error) if is_missing_document_signing_key_error(&error) => None,
        Err(error) => {
            MessageBanner::set_global(ctx, &error, MessageType::Error);
            None
        }
    }
}

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
///
/// # Returns
///
/// Returns `Ok(Some(Arc<RwLock<Wallet>>))` if a matching wallet is found,
/// `Ok(None)` if no wallet is associated with the key, or
/// `Err(SelectedWalletError)` if an error is encountered.
///
/// # Errors
///
/// - If the DPNS document type can't be found or the identity is missing the
///   required DPNS signing key (when `app_context` is provided).
/// - If no `selected_key` is provided (when `app_context` is `None`).
pub fn get_selected_wallet(
    qualified_identity: &QualifiedIdentity,
    app_context: Option<&AppContext>,
    selected_key: Option<&IdentityPublicKey>,
) -> Result<Option<Arc<RwLock<Wallet>>>, SelectedWalletError> {
    // If `app_context` is provided, use the DPNS-based approach.
    let public_key = if let Some(context) = app_context {
        let dpns_contract = &context.dpns_contract;

        // Attempt to fetch the `preorder` document type from the DPNS contract.
        let preorder_document_type = dpns_contract
            .document_type_for_name("preorder")
            .map_err(|source| SelectedWalletError::DpnsPreorderDocumentTypeNotFound { source })?;

        // Attempt to retrieve the public key from the identity.
        qualified_identity
            .document_signing_key(&preorder_document_type)
            .ok_or_else(|| {
                use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
                use dash_sdk::dpp::platform_value::string_encoding::Encoding;
                let identity_label = qualified_identity.alias.as_deref().or_else(|| {
                    qualified_identity
                        .dpns_names
                        .first()
                        .map(|n| n.name.as_str())
                });
                SelectedWalletError::MissingDocumentSigningKey {
                    identity_label: identity_label.map(str::to_owned),
                    identity_id: qualified_identity.identity.id().to_string(Encoding::Base58),
                }
            })?
    } else {
        // Fallback: directly use the provided selected key.
        selected_key.ok_or(SelectedWalletError::NoKeyProvided)?
    };

    // Once we have the public key (either from DPNS or directly), ask which
    // wallet derives it — under any placement, since a key filed under two
    // stores need not be wallet-derived under the first one probed.
    match qualified_identity
        .private_keys
        .wallet_derived_at(public_key)
    {
        Some(wallet_derivation_path) => Ok(qualified_identity
            .associated_wallets
            .get(&wallet_derivation_path.wallet_seed_hash)
            .cloned()),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
    use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::Identifier;

    use super::*;
    use crate::model::qualified_identity::encrypted_key_storage::{
        KeyStorage, PrivateKeyData, WalletDerivationPath,
    };
    use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
    use crate::model::qualified_identity::{
        IdentityStatus, IdentityType, PrivateKeyTarget, QualifiedIdentity,
    };

    /// An identity publishing `key`, holding `private_keys`, linked to
    /// `wallets` — the three things a test here varies; every other field is
    /// an inert default.
    fn identity_with(
        key: &IdentityPublicKey,
        private_keys: KeyStorage,
        wallets: BTreeMap<crate::model::wallet::WalletSeedHash, Arc<RwLock<Wallet>>>,
    ) -> QualifiedIdentity {
        QualifiedIdentity {
            identity: Identity::new_with_id_and_keys(
                Identifier::from([1u8; 32]),
                BTreeMap::from([(key.id(), key.clone())]),
                PlatformVersion::latest(),
            )
            .expect("identity"),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::Masternode,
            alias: None,
            private_keys,
            dpns_names: vec![],
            associated_wallets: wallets,
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        }
    }

    /// A key filed under two placements, wallet-derived only under the second.
    /// Taking whichever placement is probed first answers "no wallet" — the
    /// same answer as an identity with no wallet at all — and the screen then
    /// offers no unlock for a wallet it needs.
    #[test]
    fn a_wallet_is_found_under_a_later_placement_too() {
        let seed_hash = [0x66; 32];
        let wallet = Wallet::new_from_seed([0x11; 64], Network::Testnet, None, None)
            .expect("build a test wallet");
        let key = IdentityPublicKey::random_key(0, Some(1), PlatformVersion::latest());

        let mut private_keys = KeyStorage::default();
        private_keys.insert_at(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, key.id()),
            (
                QualifiedIdentityPublicKey::from(key.clone()),
                PrivateKeyData::Clear([0x22; 32]),
            ),
        );
        private_keys.insert_at(
            (PrivateKeyTarget::PrivateKeyOnVoterIdentity, key.id()),
            (
                QualifiedIdentityPublicKey::from(key.clone()),
                PrivateKeyData::AtWalletDerivationPath(WalletDerivationPath {
                    wallet_seed_hash: seed_hash,
                    derivation_path: DerivationPath::from(vec![]),
                }),
            ),
        );

        let qualified_identity = identity_with(
            &key,
            private_keys,
            BTreeMap::from([(seed_hash, Arc::new(RwLock::new(wallet)))]),
        );

        let selected = get_selected_wallet(&qualified_identity, None, Some(&key))
            .expect("a key given directly needs no DPNS contract");
        assert!(
            selected.is_some(),
            "the wallet deriving this key must be found whichever placement names it"
        );
    }

    fn missing_document_signing_key_error(identity_label: Option<&str>) -> SelectedWalletError {
        SelectedWalletError::MissingDocumentSigningKey {
            identity_label: identity_label.map(str::to_owned),
            identity_id: "TestIdentityId".to_string(),
        }
    }

    #[test]
    fn missing_document_signing_key_display_includes_label_when_present() {
        let error = missing_document_signing_key_error(Some("Test Identity"));

        assert_eq!(
            error.to_string(),
            "Identity Test Identity (TestIdentityId) cannot sign this action because it has no \
             high-security signing key. Add a high-security signing key to this identity, or \
             choose a different identity."
        );
    }

    #[test]
    fn missing_document_signing_key_display_uses_identity_id_without_label() {
        let error = missing_document_signing_key_error(None);

        assert_eq!(
            error.to_string(),
            "Identity TestIdentityId cannot sign this action because it has no high-security \
             signing key. Add a high-security signing key to this identity, or choose a \
             different identity."
        );
    }

    #[test]
    fn auto_selection_policy_suppresses_missing_document_signing_key() {
        let ctx = egui::Context::default();

        let wallet =
            auto_selected_wallet_or_banner(&ctx, Err(missing_document_signing_key_error(None)));

        assert!(wallet.is_none());
        assert!(
            !MessageBanner::has_global(&ctx),
            "automatic selection must suppress the typed missing-document-signing-key banner"
        );
    }

    #[test]
    fn auto_selection_policy_surfaces_unrelated_selected_wallet_error() {
        let ctx = egui::Context::default();

        let wallet = auto_selected_wallet_or_banner(&ctx, Err(SelectedWalletError::NoKeyProvided));

        assert!(wallet.is_none());
        assert!(
            MessageBanner::has_global(&ctx),
            "automatic selection must only suppress MissingDocumentSigningKey"
        );
    }
}
