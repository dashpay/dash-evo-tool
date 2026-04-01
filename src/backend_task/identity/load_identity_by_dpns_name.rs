use super::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::{
    DPNSNameInfo, IdentityStatus, IdentityType, QualifiedIdentity,
};
use crate::model::wallet::WalletSeedHash;
use dash_sdk::Sdk;
use dash_sdk::platform::{Fetch, Identity};

impl AppContext {
    /// Load an identity by its DPNS name.
    ///
    /// Uses the SDK's `resolve_dpns_name()` for name resolution (replacing the
    /// manual DPNS document query), then fetches the identity and all its DPNS
    /// names, and builds a `QualifiedIdentity` with optional wallet key
    /// matching.
    pub(super) async fn load_identity_by_dpns_name(
        &self,
        sdk: &Sdk,
        dpns_name: String,
        selected_wallet_seed_hash: Option<WalletSeedHash>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // Step 1: Resolve the DPNS name to an identity ID using the SDK.
        //
        // The SDK's resolve_dpns_name() handles homograph-safe normalization,
        // parent domain matching, and normalizedLabel lookup internally.
        let identity_id = sdk
            .resolve_dpns_name(&dpns_name)
            .await
            .map_err(|e| TaskError::DpnsFetchError {
                source: Box::new(e),
            })?
            .ok_or(TaskError::IdentityNotFound)?;

        // Step 2: Also notify the platform-wallet (if available) so it can add
        // the identity to its watched_identities collection.
        //
        // We pick any available platform wallet since they all share the same
        // SDK and the watched_identities store is per-wallet.
        if let Some(platform_wallet) = self.first_available_platform_wallet()
            && let Err(e) = platform_wallet
                .identity()
                .load_identity_by_dpns_name(&dpns_name)
                .await
        {
            tracing::debug!(
                "Platform-wallet load_identity_by_dpns_name failed (non-fatal): {}",
                e
            );
        }

        // Step 3: Fetch the identity from Platform.
        let identity = match Identity::fetch_by_identifier(sdk, identity_id).await {
            Ok(Some(identity)) => identity,
            Ok(None) => return Err(TaskError::IdentityNotFound),
            Err(e) => return Err(TaskError::from(e)),
        };

        // Extract a display label from the input name.
        let label = dpns_name
            .strip_suffix(".dash")
            .unwrap_or(&dpns_name)
            .to_string();

        // Step 4: Fetch all DPNS names owned by this identity.
        let owned_dpns_names = sdk
            .get_dpns_usernames_by_identity(identity_id, None)
            .await
            .map(|dpns_usernames| {
                dpns_usernames
                    .into_iter()
                    .map(|u| DPNSNameInfo {
                        name: u.label,
                        acquired_at: 0,
                    })
                    .collect::<Vec<DPNSNameInfo>>()
            })
            .map_err(TaskError::from)?;

        let wallets = self.wallets.read().map_err(TaskError::from)?.clone();

        // Step 5: Try to derive keys from wallets if requested.
        let mut encrypted_private_keys = std::collections::BTreeMap::new();

        if let Some((_, _, wallet_private_keys)) = self.match_user_identity_keys_with_wallet(
            &identity,
            &wallets,
            selected_wallet_seed_hash,
        )? {
            encrypted_private_keys.extend(wallet_private_keys);
        }

        let qualified_identity = QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: Some(format!("{}.dash", label)),
            private_keys: encrypted_private_keys.into(),
            dpns_names: owned_dpns_names,
            associated_wallets: wallets
                .values()
                .map(|wallet| {
                    let w = wallet.read()?;
                    Ok::<_, TaskError>((w.seed_hash(), wallet.clone()))
                })
                .collect::<Result<_, _>>()?,
            wallet_index: None,
            top_ups: Default::default(),
            status: IdentityStatus::Active,
            network: self.network,
        };
        let wallet_info = qualified_identity
            .determine_wallet_info()
            .map_err(|e| TaskError::WalletInfoDeterminationFailed { detail: e })?;

        // Insert qualified identity into the database.
        self.insert_local_qualified_identity(&qualified_identity, &wallet_info)?;

        Ok(BackendTaskSuccessResult::LoadedIdentity(qualified_identity))
    }
}
