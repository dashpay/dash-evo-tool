use std::sync::{Arc, RwLock};

use dash_sdk::{
    dpp::data_contract::accessors::v0::DataContractV0Getters, platform::IdentityPublicKey,
};

use crate::{
    context::AppContext,
    model::{qualified_identity::QualifiedIdentity, wallet::Wallet},
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

/// Confirmation shown once an identity is removed from this device.
pub const IDENTITY_REMOVED: &str = "The identity was removed from this device.";

/// Shown when the identity's removal succeeded — it is already gone from
/// every list — but a step that runs after delisting, including the one that
/// clears its private keys from this device's vault, did not finish.
///
/// Framed as a completed removal, since there is no retry control left for the
/// user to reach the identity with, and hedged on both things the
/// `cleanup_deferred` flag does not actually establish. It does not establish
/// that keys are present: the failing step may be a scope purge for a keyless
/// identity, whose manifest names no placements at all. It does not establish
/// that the next launch fixes it: the boot sweep is best-effort, skipped while
/// a storage update runs, and it retains the manifest whenever the purge or
/// vault delete fails again. So the message promises another automatic
/// attempt, never completion — and still names the one precaution the user can
/// act on now, because a warning that only hedges gives them nothing to do.
pub const IDENTITY_REMOVED_CLEANUP_PENDING: &str = "The identity was removed from this device, but its private keys may still be stored here. The app will try to clear them again the next time you open it. Until then, treat this device as if it still holds them.";

/// Shown when the identity was removed but the voter identity tied to it was
/// not. Naming the leftover matters: the user sees one entry disappear and one
/// stay, and this is what tells them the remaining entry is not a mistake.
pub const IDENTITY_REMOVED_VOTER_LEFT: &str = "The identity was removed, but its associated voter identity could not be removed. Retry after restarting the app.";

/// Shown when both leftover outcomes above apply at once: the associated
/// voter identity failed to remove *and* a post-delisting step for at least
/// one of the two identities this call touched did not finish. A
/// single-outcome banner would silently drop one of the two — the voter
/// identity looking like a clean failure with nothing else wrong, or the
/// possible key residue going unmentioned entirely — so this names both, under
/// the same hedge as [`IDENTITY_REMOVED_CLEANUP_PENDING`].
///
/// It carries that constant's precaution too. This is the worse of the two
/// outcomes, so it must not be the one that tells the user less: the same
/// uncertainty about key material applies, and the same thing can be done
/// about it now.
pub const IDENTITY_REMOVED_VOTER_LEFT_AND_CLEANUP_PENDING: &str = "The identity was removed, but its associated voter identity could not be removed — retry after restarting the app. Private keys for one or both of them may still be stored on this device. The app will try to clear them again the next time you open it. Until then, treat this device as if it still holds them.";

/// Shown when a removal is refused because the storage update is still running.
pub const IDENTITY_REMOVAL_BLOCKED_BY_STORAGE_UPDATE: &str =
    "The storage update is still running. Wait for it to finish before removing an identity.";

/// Which banner to show for a `BackendTaskSuccessResult::RemovedIdentities`
/// result, and how urgently. Shared by the Identity Hub and the legacy
/// identities screen so the two, which handle the same result type, don't
/// carry two independently-maintained copies of this 4-way decision — the
/// combined-flags case in particular is easy to get wrong by handling each
/// flag in isolation (see the `both` test case below).
pub fn removed_identities_banner(
    associated_cleanup_failed: bool,
    cleanup_deferred: bool,
) -> (&'static str, crate::ui::MessageType) {
    use crate::ui::MessageType;
    match (associated_cleanup_failed, cleanup_deferred) {
        (true, true) => (
            IDENTITY_REMOVED_VOTER_LEFT_AND_CLEANUP_PENDING,
            MessageType::Warning,
        ),
        (true, false) => (IDENTITY_REMOVED_VOTER_LEFT, MessageType::Warning),
        (false, true) => (IDENTITY_REMOVED_CLEANUP_PENDING, MessageType::Warning),
        (false, false) => (IDENTITY_REMOVED, MessageType::Success),
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
/// `Ok(None)` if no wallet is associated with the key, or `Err(String)` if
/// an error is encountered.
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
) -> Result<Option<Arc<RwLock<Wallet>>>, String> {
    // If `app_context` is provided, use the DPNS-based approach.
    let public_key = if let Some(context) = app_context {
        let dpns_contract = &context.dpns_contract;

        // Attempt to fetch the `preorder` document type from the DPNS contract.
        let preorder_document_type = dpns_contract
            .document_type_for_name("preorder")
            .map_err(|e| format!("DPNS preorder document type not found: {}", e))?;

        // Attempt to retrieve the public key from the identity.
        qualified_identity
            .document_signing_key(&preorder_document_type)
            .ok_or_else(|| {
                "Identity doesn't have an authentication key for signing document transitions"
                    .to_string()
            })?
    } else {
        // Fallback: directly use the provided selected key.
        selected_key.ok_or_else(|| "No key provided when getting selected wallet".to_string())?
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

    use super::*;
    use crate::ui::MessageType;

    /// The one case a naive if/else-if chain gets wrong: when the associated
    /// voter identity's cleanup failed *and* the primary or voter's own
    /// vault-key delete is separately still pending, both must surface —
    /// picking only the voter-failure message silently drops the still-live
    /// private key residue, and picking only the cleanup-pending message
    /// silently drops the unretryable voter failure.
    #[test]
    fn removed_identities_banner_names_both_outcomes_when_both_apply() {
        let (message, message_type) = removed_identities_banner(true, true);
        assert_eq!(message, IDENTITY_REMOVED_VOTER_LEFT_AND_CLEANUP_PENDING);
        // Case-insensitive: which clause opens a sentence is a copy decision,
        // and this assertion is about both outcomes being named, not casing.
        let message_lowercase = message.to_lowercase();
        assert!(
            message_lowercase.contains("voter identity")
                && message_lowercase.contains("private keys"),
            "the combined message must name both the voter failure and the key residue"
        );
        assert_eq!(message_type, MessageType::Warning);
    }

    /// `cleanup_deferred` records that a step failed *after* delisting. It does
    /// not establish that key material is present — `purge_identity_scope` can
    /// fail for a keyless identity whose manifest holds no placements — and it
    /// cannot promise the next launch fixes it, because the boot sweep is
    /// skipped while a storage update runs and retains the manifest if the
    /// purge or vault delete fails again. A banner that asserts either reads as
    /// a guarantee, and the one thing worse than warning a user about key
    /// residue is telling them it has been handled when it has not.
    #[test]
    fn cleanup_pending_banners_claim_no_more_than_the_flag_establishes() {
        for message in [
            IDENTITY_REMOVED_CLEANUP_PENDING,
            IDENTITY_REMOVED_VOTER_LEFT_AND_CLEANUP_PENDING,
        ] {
            assert!(
                !message.contains("are still stored"),
                "presence is not established, so the message must not assert it: {message}"
            );
            assert!(
                message.contains("may still be stored"),
                "the message must say the keys may be present, not that they are: {message}"
            );
            assert!(
                !message.contains("will be cleared"),
                "completion is not guaranteed, so the message must not promise it: {message}"
            );
            assert!(
                message.contains("try"),
                "the message must promise another attempt in place of the completion it \
                 cannot promise: {message}"
            );
        }
    }

    /// Honest uncertainty still has to leave the user something to do. A
    /// message that only hedges is its own failure — it reports a risk and
    /// hands over no way to act on it.
    #[test]
    fn cleanup_pending_banners_still_give_the_user_something_to_do() {
        for message in [
            IDENTITY_REMOVED_CLEANUP_PENDING,
            IDENTITY_REMOVED_VOTER_LEFT_AND_CLEANUP_PENDING,
        ] {
            assert!(
                message.contains("open"),
                "reopening the app is the action that triggers the next attempt: {message}"
            );
            assert!(
                message.contains("treat this device as if it still holds"),
                "the safe assumption under uncertainty is the precaution the user can take \
                 now, and the combined outcome is the worse one — it must not be the banner \
                 that says less: {message}"
            );
        }
    }

    #[test]
    fn removed_identities_banner_covers_every_single_flag_combination() {
        assert_eq!(
            removed_identities_banner(true, false),
            (IDENTITY_REMOVED_VOTER_LEFT, MessageType::Warning)
        );
        assert_eq!(
            removed_identities_banner(false, true),
            (IDENTITY_REMOVED_CLEANUP_PENDING, MessageType::Warning)
        );
        assert_eq!(
            removed_identities_banner(false, false),
            (IDENTITY_REMOVED, MessageType::Success)
        );
    }

    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
    use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::Identifier;

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
}
