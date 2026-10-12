use super::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use crate::model::wallet::WalletSeedHash;
use dash_sdk::Sdk;
use dash_sdk::platform::{Fetch, Identity};

impl AppContext {
    /// Load an identity by its DPNS name
    pub(super) async fn load_identity_by_dpns_name(
        &self,
        sdk: &Sdk,
        dpns_name: String,
        selected_wallet_seed_hash: Option<WalletSeedHash>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // The name's own record names its identity, which after a transfer is
        // not the document's owner.
        let identity_id = sdk
            .resolve_dpns_name(dpns_name.trim())
            .await
            .map_err(TaskError::from)?
            .ok_or(TaskError::IdentityNotFound)?;

        // Fetch the identity
        let identity = match Identity::fetch_by_identifier(sdk, identity_id).await {
            Ok(Some(identity)) => identity,
            Ok(None) => return Err(TaskError::IdentityNotFound),
            Err(e) => return Err(TaskError::from(e)),
        };

        let owned_dpns_names = self.fetch_owned_dpns_names(sdk, identity_id).await?;

        let wallets = self.wallet_context().wallets();

        // Try to derive keys from wallets if requested
        let mut encrypted_private_keys = std::collections::BTreeMap::new();

        if let Some((_, _, wallet_private_keys)) = self
            .match_user_identity_keys_with_wallet(&identity, &wallets, selected_wallet_seed_hash)
            .await?
        {
            encrypted_private_keys.extend(wallet_private_keys);
        }

        let qualified_identity = QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: encrypted_private_keys.into(),
            dpns_names: owned_dpns_names,
            associated_wallets: wallets
                .values()
                .map(|wallet| {
                    let w = wallet.read()?;
                    Ok::<_, TaskError>((w.seed_hash(), wallet.clone()))
                })
                .collect::<Result<_, _>>()?,
            secret_access: self.wallet_backend().ok().map(|b| b.secret_access()),
            wallet_index: None,
            top_ups: Default::default(),
            status: IdentityStatus::Active,
            network: self.network,
        };
        let wallet_info = qualified_identity
            .determine_wallet_info()
            .map_err(|e| TaskError::WalletInfoDeterminationFailed { detail: e })?;

        // Insert qualified identity into the database
        self.insert_local_qualified_identity(&qualified_identity, &wallet_info)?;

        Ok(BackendTaskSuccessResult::LoadedIdentity(qualified_identity))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::document::DocumentV0Getters;
    use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
    use dash_sdk::dpp::platform_value::Value;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::{Document, Identifier};
    use dash_sdk::query_types::Documents;

    fn documents(found: Document) -> Documents {
        Documents::from_iter([(found.id(), Some(found))])
    }

    /// The name's record, not its document owner, says whose identity to load;
    /// the network may hand that record back as plain bytes.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn loading_by_name_follows_the_identity_the_name_points_at() {
        let staged =
            crate::context::test_staging::stage_identity_with_vaulted_keys([0x11; 32], [0x22; 32])
                .await;
        let ctx = &staged.ctx;
        let target = Identifier::from([7; 32]);
        let name = crate::context::test_support::dpns_name_document(
            Identifier::from([0x5E; 32]),
            "Alice",
            Value::Bytes32(target.to_buffer()),
        );
        let mut sdk = crate::context::test_support::mock_sdk_for(ctx);
        sdk.mock()
            .expect_fetch_many::<Identifier, Document, _, Documents>(
                ctx.dpns_domain_query("a11ce"),
                Some(documents(name.clone())),
            )
            .await
            .expect("name lookup expectation");
        let identity =
            Identity::create_basic_identity(target, PlatformVersion::latest()).expect("identity");
        sdk.mock()
            .expect_fetch(target, Some(identity))
            .await
            .expect("identity expectation");
        sdk.mock()
            .expect_fetch_many::<Identifier, Document, _, Documents>(
                ctx.owned_dpns_names_query(target),
                Some(documents(name)),
            )
            .await
            .expect("owned names expectation");

        let loaded = ctx
            .load_identity_by_dpns_name(&sdk, " Alice.dash ".to_string(), None)
            .await
            .expect("the identity loads");

        let BackendTaskSuccessResult::LoadedIdentity(loaded) = loaded else {
            panic!("expected a loaded identity, got {loaded:?}");
        };
        assert_eq!(loaded.identity.id(), target);
        let names: Vec<&str> = loaded.dpns_names.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["Alice"]);
    }
}
