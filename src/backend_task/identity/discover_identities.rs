use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::identity_discovery::{
    DiscoverySummary, IDENTITY_GAP_LIMIT, IDENTITY_SCAN_HARD_CAP, should_continue_scan,
};
use crate::model::qualified_identity::DPNSNameInfo;
use crate::model::wallet::Wallet;
use crate::utils::egui_mpsc::SenderAsync;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use std::sync::{Arc, RwLock};

/// Number of authentication-key indices probed per identity index before
/// concluding no identity is registered there.
const AUTH_KEY_LOOKUP_WINDOW: u32 = 12;

/// Whether discovery is automatic, follows unlock, or is user-requested.
#[derive(Clone, Copy)]
pub(crate) enum IdentityDiscoveryMode {
    /// Automatic startup discovery; never prompts or restores unloaded identities.
    Background,
    /// Post-unlock discovery; may use the unlocked seed but never restores identities.
    WalletUnlock,
    /// User-started search; may deliberately restore an unloaded identity.
    ExplicitSearch,
}

impl IdentityDiscoveryMode {
    fn allow_prompt(self) -> bool {
        !matches!(self, Self::Background)
    }

    fn explicitly_reloads_forgotten(self) -> bool {
        matches!(self, Self::ExplicitSearch)
    }
}

impl AppContext {
    /// Discover and load identities derived from a wallet by checking the
    /// network, with a rolling gap-limited lookahead.
    ///
    /// The scan starts at index 0 and keeps probing while it is within
    /// [`IDENTITY_GAP_LIMIT`](crate::model::identity_discovery::IDENTITY_GAP_LIMIT)
    /// indices of the highest index that produced an identity, so each new
    /// discovery extends the window. `seed_from_index`, together with the
    /// wallet's already-known identity indices, seeds that window so a
    /// prior-session high index is never missed even if the early indices are
    /// empty.
    ///
    /// `mode` controls whether a cold secret-cache miss may prompt and whether
    /// the scan is an explicit user request that may restore an identity the
    /// user unloaded. Startup and wallet-unlock discovery always leave forgotten
    /// identities untouched.
    ///
    /// When `progress` is `Some`, a [`BackendTaskSuccessResult::Progress`] event
    /// is sent before each probed index.
    pub(crate) async fn discover_identities_gap_limited(
        self: &Arc<Self>,
        wallet: &Arc<RwLock<Wallet>>,
        seed_from_index: u32,
        mode: IdentityDiscoveryMode,
        progress: Option<&SenderAsync<TaskResult>>,
    ) -> Result<DiscoverySummary, TaskError> {
        use dash_sdk::platform::Fetch;
        use dash_sdk::platform::types::identity::NonUniquePublicKeyHashQuery;

        let sdk = self.sdk.load().as_ref().clone();
        let scan_network = self.network;
        let seed_hash = wallet.read()?.seed_hash();

        // Seed the rolling window from the explicit seed index and from any
        // identity already known to this wallet, so a high prior-session index
        // keeps the scan open long enough to re-reach it.
        let highest_known_index = {
            let guard = wallet.read()?;
            guard.identities.keys().copied().max()
        };
        let seed_window = match highest_known_index {
            Some(known) => Some(known.max(seed_from_index)),
            None if seed_from_index > 0 => Some(seed_from_index),
            None => None,
        };

        tracing::info!(
            seed = %hex::encode(seed_hash),
            seed_window = ?seed_window,
            allow_prompt = mode.allow_prompt(),
            "Starting gap-limited identity discovery for wallet"
        );

        let mut summary = DiscoverySummary::default();
        let mut highest_found = seed_window;
        let mut current_index = 0u32;

        while should_continue_scan(current_index, highest_found) {
            if let Some(sender) = progress {
                let next = current_index.saturating_add(1);
                // Soft total: the current rolling-window upper bound (it grows
                // as identities are found), clamped to the hard cap. A rolling
                // scan has no fixed end, so this is a best-effort denominator.
                let soft_total = highest_found
                    .map_or(IDENTITY_GAP_LIMIT, |h| h.saturating_add(IDENTITY_GAP_LIMIT))
                    .saturating_add(1)
                    .min(IDENTITY_SCAN_HARD_CAP);
                sender
                    .send(TaskResult::unattributed_success(
                        BackendTaskSuccessResult::Progress {
                            message: format!(
                                "Searching wallet identity index {next} of about {soft_total}."
                            ),
                            current: next,
                            total: soft_total,
                        },
                    ))
                    .await
                    .map_err(|_| TaskError::InternalSendError)?;
            }

            let mut fetched_identity = None;
            let mut matched_key_index = None;

            for key_index in 0..AUTH_KEY_LOOKUP_WINDOW {
                let public_key = match self
                    .resolve_identity_auth_pubkey(
                        wallet,
                        mode.allow_prompt(),
                        current_index,
                        key_index,
                    )
                    .await
                {
                    Ok(key) => key,
                    // A locked, protected wallet in the no-prompt path: skip the
                    // whole wallet — every later index needs the same seed.
                    Err(TaskError::AuthKeyUnlockRequired) => {
                        tracing::debug!(
                            seed = %hex::encode(seed_hash),
                            "Skipping locked wallet during background identity discovery"
                        );
                        return Ok(summary);
                    }
                    Err(e) => {
                        tracing::debug!(
                            error = %e,
                            current_index,
                            key_index,
                            "Could not derive auth key during discovery"
                        );
                        continue;
                    }
                };

                let key_hash = public_key.pubkey_hash().into();
                let query = NonUniquePublicKeyHashQuery {
                    key_hash,
                    after: None,
                };

                match dash_sdk::platform::Identity::fetch(&sdk, query).await {
                    Ok(Some(identity)) => {
                        fetched_identity = Some(identity);
                        matched_key_index = Some(key_index);
                        break;
                    }
                    Ok(None) => continue,
                    Err(e) => {
                        tracing::debug!(
                            error = %e,
                            current_index,
                            key_index,
                            "Error querying identity during discovery"
                        );
                        continue;
                    }
                }
            }

            if let Some(identity) = fetched_identity {
                let identity_id = identity.id();
                tracing::info!(
                    identity_id = %identity_id,
                    current_index,
                    key_index = ?matched_key_index,
                    "Discovered identity from wallet"
                );

                summary.found = summary.found.saturating_add(1);
                highest_found = Some(highest_found.map_or(current_index, |h| h.max(current_index)));

                match self
                    .upsert_discovered_identity(
                        &sdk,
                        identity,
                        wallet,
                        scan_network,
                        mode,
                        current_index,
                    )
                    .await
                {
                    Ok(true) => summary.stored = summary.stored.saturating_add(1),
                    Ok(false) => {}
                    Err(e) => tracing::warn!(
                        identity_id = %identity_id,
                        error = %e,
                        "Failed to store discovered identity"
                    ),
                }
            }

            current_index = current_index.saturating_add(1);
        }

        tracing::info!(
            seed = %hex::encode(seed_hash),
            found = summary.found,
            stored = summary.stored,
            "Gap-limited identity discovery complete"
        );

        Ok(summary)
    }

    /// Discover and load identities derived from a wallet on wallet unlock.
    ///
    /// Thin wrapper over [`Self::discover_identities_gap_limited`] that prompts
    /// for the seed if needed (the unlock gesture already implies user consent)
    /// and seeds the rolling window from `max_identity_index`.
    pub(crate) async fn discover_identities_from_wallet(
        self: &Arc<Self>,
        wallet: &Arc<RwLock<Wallet>>,
        max_identity_index: u32,
    ) -> Result<(), TaskError> {
        self.discover_identities_gap_limited(
            wallet,
            max_identity_index,
            IdentityDiscoveryMode::WalletUnlock,
            None,
        )
        .await?;
        Ok(())
    }

    /// Fetch, build, and store one discovered identity, preserving DET-only
    /// metadata (the user alias) from any existing stored record.
    ///
    /// The freshly-built [`QualifiedIdentity`](crate::model::qualified_identity::QualifiedIdentity)
    /// carries `alias: None`; before storing, the existing record's alias is
    /// copied onto it so a re-discovery refreshes keys and DPNS names without
    /// wiping a user-assigned alias. Wallet association and top-up history are
    /// preserved by [`AppContext::update_local_qualified_identity`] /
    /// the separate top-up KV key, respectively.
    ///
    /// `scan_network` is the network the scan started on. If the active network
    /// changed mid-scan, the store is skipped — defense-in-depth so an in-flight
    /// pass never writes a discovered identity under the wrong network scope.
    async fn upsert_discovered_identity(
        self: &Arc<Self>,
        sdk: &dash_sdk::Sdk,
        identity: dash_sdk::platform::Identity,
        wallet: &Arc<RwLock<Wallet>>,
        scan_network: dash_sdk::dpp::dashcore::Network,
        mode: IdentityDiscoveryMode,
        identity_index: u32,
    ) -> Result<bool, TaskError> {
        if self.network != scan_network {
            tracing::debug!("Network changed mid-scan; skipping store of discovered identity");
            return Ok(false);
        }

        let identity_id = identity.id();
        let seed_hash = wallet.read()?.seed_hash();

        let qualified_identity = self
            .build_qualified_identity_from_wallet(
                sdk,
                identity,
                wallet,
                mode.allow_prompt(),
                identity_index,
            )
            .await?;

        if !self.persist_discovered_identity(
            qualified_identity.clone(),
            seed_hash,
            identity_index,
            mode.explicitly_reloads_forgotten(),
        )? {
            tracing::debug!(
                identity_id = %identity_id,
                "Skipped a discovered identity that the user unloaded"
            );
            return Ok(false);
        }

        if let Ok(mut wallet_guard) = wallet.write() {
            wallet_guard
                .identities
                .insert(identity_index, qualified_identity.identity.clone());
        }
        tracing::info!(
            identity_id = %identity_id,
            "Successfully loaded discovered identity"
        );
        Ok(true)
    }

    /// Persist one discovery result unless automatic discovery must leave it unloaded.
    pub(crate) fn persist_discovered_identity(
        &self,
        mut qualified_identity: crate::model::qualified_identity::QualifiedIdentity,
        seed_hash: crate::model::wallet::WalletSeedHash,
        identity_index: u32,
        explicit_reload: bool,
    ) -> Result<bool, TaskError> {
        let identity_id = qualified_identity.identity.id();
        let load_guard = self.begin_identity_load(identity_id, None)?;
        if self.is_identity_forgotten(&identity_id)? && !explicit_reload {
            load_guard.loaded();
            return Ok(false);
        }

        match self.get_identity_by_id(&identity_id)? {
            Some(existing) => {
                // Carry DET-only metadata onto the refreshed identity, then
                // update in place — `update_local_qualified_identity` keeps the
                // stored wallet association, and top-ups live under a separate
                // KV key untouched by this write.
                qualified_identity.alias = existing.alias;
                self.update_local_qualified_identity(&qualified_identity)?;
            }
            None => {
                self.insert_local_qualified_identity(
                    &qualified_identity,
                    &Some((seed_hash, identity_index)),
                )?;
            }
        }
        if explicit_reload {
            self.finish_identity_load_after_persist(&identity_id, load_guard);
        } else {
            load_guard.loaded();
        }
        Ok(true)
    }

    /// Build a QualifiedIdentity from a fetched Identity with wallet key derivation paths.
    /// This matches identity public keys to wallet-derived keys and fetches DPNS names.
    async fn build_qualified_identity_from_wallet(
        &self,
        sdk: &dash_sdk::Sdk,
        identity: dash_sdk::platform::Identity,
        wallet: &Arc<RwLock<Wallet>>,
        allow_prompt: bool,
        identity_index: u32,
    ) -> Result<crate::model::qualified_identity::QualifiedIdentity, TaskError> {
        use crate::model::qualified_identity::encrypted_key_storage::{
            PrivateKeyData, WalletDerivationPath,
        };
        use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
        use crate::model::qualified_identity::{
            IdentityStatus, IdentityType, PrivateKeyTarget, QualifiedIdentity,
        };
        use dash_sdk::dpp::identity::KeyType;
        use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
        use dash_sdk::dpp::key_wallet::bip32::{DerivationPath, KeyDerivationType};

        let seed_hash = wallet.read()?.seed_hash();

        // Get the highest key ID in the identity to know how many keys to derive
        let highest_key_id = identity.public_keys().keys().max().copied().unwrap_or(0);
        let derive_up_to = highest_key_id.saturating_add(6); // Add buffer for future keys

        // Derive authentication keys from wallet and build lookup maps,
        // cache-first (one JIT scope on a cold cache).
        let (public_key_to_index, public_key_hash_to_index) = self
            .resolve_identity_auth_pubkeys_data_map(
                wallet,
                false,
                allow_prompt,
                identity_index,
                0..derive_up_to.saturating_add(1),
            )
            .await?;

        // Match identity keys with wallet derivation paths
        let private_keys_map: std::collections::BTreeMap<_, _> = identity
            .public_keys()
            .iter()
            .filter_map(|(key_id, identity_key)| {
                // Try to match by full public key or by hash
                let matched_index = match identity_key.key_type() {
                    KeyType::ECDSA_SECP256K1 => public_key_to_index
                        .get(identity_key.data().as_slice())
                        .copied(),
                    KeyType::ECDSA_HASH160 => {
                        let hash: [u8; 20] = identity_key.data().as_slice().try_into().ok()?;
                        public_key_hash_to_index.get(&hash).copied()
                    }
                    _ => None,
                }?;

                let derivation_path = DerivationPath::identity_authentication_path(
                    self.network,
                    KeyDerivationType::ECDSA,
                    identity_index,
                    matched_index,
                );

                let wallet_derivation_path = WalletDerivationPath {
                    wallet_seed_hash: seed_hash,
                    derivation_path,
                };

                Some((
                    (PrivateKeyTarget::PrivateKeyOnMainIdentity, *key_id),
                    (
                        QualifiedIdentityPublicKey::from_identity_public_key_in_wallet(
                            identity_key.clone(),
                            Some(wallet_derivation_path.clone()),
                        ),
                        PrivateKeyData::AtWalletDerivationPath(wallet_derivation_path),
                    ),
                ))
            })
            .collect();

        // Fetch DPNS names for this identity
        let dpns_names = {
            use dash_sdk::dpp::document::DocumentV0Getters;
            use dash_sdk::dpp::platform_value::Value;
            use dash_sdk::drive::query::{SelectProjection, WhereClause, WhereOperator};
            use dash_sdk::platform::{Document, DocumentQuery, FetchMany};

            let query = DocumentQuery {
                select: SelectProjection::documents(),
                data_contract: self.dpns_contract.clone(),
                document_type_name: "domain".to_string(),
                where_clauses: vec![WhereClause {
                    field: "records.identity".to_string(),
                    operator: WhereOperator::Equal,
                    value: Value::Identifier(identity.id().into()),
                }],
                group_by: Vec::new(),
                having: Vec::new(),
                order_by_clauses: vec![],
                limit: 100,
                start: None,
            };

            match Document::fetch_many(sdk, query).await {
                Ok(document_map) => document_map
                    .values()
                    .filter_map(|maybe_doc| {
                        maybe_doc.as_ref().and_then(|doc| {
                            let name = doc
                                .get("label")
                                .map(|label| label.to_str().unwrap_or_default());
                            let acquired_at = doc
                                .created_at()
                                .into_iter()
                                .chain(doc.transferred_at())
                                .max();

                            match (name, acquired_at) {
                                (Some(name), Some(acquired_at)) => Some(DPNSNameInfo {
                                    name: name.to_string(),
                                    acquired_at,
                                }),
                                _ => None,
                            }
                        })
                    })
                    .collect::<Vec<DPNSNameInfo>>(),
                Err(e) => {
                    tracing::warn!("Failed to fetch DPNS names for identity: {}", e);
                    Vec::new()
                }
            }
        };

        // Build the qualified identity
        let mut associated_wallets = std::collections::BTreeMap::new();
        associated_wallets.insert(seed_hash, Arc::clone(wallet));

        Ok(QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: private_keys_map.into(),
            dpns_names,
            associated_wallets,
            secret_access: self.wallet_backend().ok().map(|b| b.secret_access()),
            wallet_index: Some(identity_index),
            top_ups: Default::default(),
            status: IdentityStatus::Unknown,
            network: self.network,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::TaskResult;
    use crate::context::identity_load_registry::IdentityLoadPhase;
    use crate::context::test_support::test_app_context;
    use crate::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
    use crate::utils::egui_mpsc::SenderAsync;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::{Identifier, Identity};
    use std::collections::BTreeMap;

    fn wallet_derived_identity(
        id: Identifier,
        wallet: &Arc<RwLock<Wallet>>,
        identity_index: u32,
    ) -> QualifiedIdentity {
        let wallet_seed_hash = wallet.read().expect("read wallet").seed_hash();
        QualifiedIdentity {
            identity: Identity::create_basic_identity(id, PlatformVersion::latest())
                .expect("create identity"),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: Default::default(),
            dpns_names: Vec::new(),
            associated_wallets: BTreeMap::from([(wallet_seed_hash, Arc::clone(wallet))]),
            secret_access: None,
            wallet_index: Some(identity_index),
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn forgotten_identity_is_not_resurrected_by_discovery_and_explicit_load_clears_marker() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(temp_dir.path());
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");
        let backend = ctx.wallet_backend().expect("wallet backend");
        let wallet = Arc::new(RwLock::new(
            Wallet::new_from_seed([0x61; 64], Network::Testnet, None, None).expect("build wallet"),
        ));
        let wallet_seed_hash = wallet.read().expect("read wallet").seed_hash();
        ctx.wallets()
            .write()
            .expect("write wallets")
            .insert(wallet_seed_hash, Arc::clone(&wallet));
        let identity_id = Identifier::from([0x62; 32]);
        let identity = wallet_derived_identity(identity_id, &wallet, 4);
        ctx.insert_local_qualified_identity(&identity, &Some((wallet_seed_hash, 4)))
            .expect("insert wallet-derived identity");

        ctx.unload_identity(identity_id)
            .expect("unload wallet-derived identity");
        assert!(
            ctx.is_identity_forgotten(&identity_id)
                .expect("read forgotten marker")
        );

        let stored = ctx
            .persist_discovered_identity(identity.clone(), wallet_seed_hash, 4, false)
            .expect("simulate discovery persistence");
        assert!(!stored, "discovery must skip a forgotten identity");
        assert_eq!(
            ctx.latest_identity_load_phase(&identity_id),
            Some(IdentityLoadPhase::Loaded),
            "intentionally skipping a forgotten identity is a successful load no-op"
        );
        assert!(
            ctx.get_identity_by_id(&identity_id)
                .expect("read identity")
                .is_none(),
            "discovery must not resurrect an unloaded identity"
        );

        assert!(
            ctx.persist_discovered_identity(identity.clone(), wallet_seed_hash, 4, true)
                .expect("simulate explicit wallet load"),
            "an explicit load must restore the identity"
        );
        assert!(
            !ctx.is_identity_forgotten(&identity_id)
                .expect("read cleared marker")
        );

        ctx.delete_local_qualified_identity(&identity_id)
            .expect("remove identity without recording another unload");
        assert!(
            ctx.persist_discovered_identity(identity.clone(), wallet_seed_hash, 4, false)
                .expect("simulate discovery after explicit reload"),
            "discovery must work normally after the explicit load clears the marker"
        );
        assert!(
            ctx.get_identity_by_id(&identity_id)
                .expect("read rediscovered identity")
                .is_some()
        );

        ctx.db()
            .record_forgotten_identity(Network::Testnet, &identity_id)
            .expect("record forgotten marker");
        ctx.db()
            .execute(
                "CREATE TRIGGER fail_discovery_marker_cleanup
                 BEFORE DELETE ON forgotten_identities
                 BEGIN
                     SELECT RAISE(FAIL, 'injected discovery marker cleanup failure');
                 END;",
                [],
            )
            .expect("install marker cleanup failure trigger");
        assert!(
            ctx.persist_discovered_identity(identity, wallet_seed_hash, 4, true)
                .expect("persist despite marker cleanup failure"),
            "durable persistence remains successful when marker cleanup fails"
        );
        assert_eq!(
            ctx.latest_identity_load_phase(&identity_id),
            Some(IdentityLoadPhase::Loaded)
        );
        assert!(
            ctx.is_identity_forgotten(&identity_id)
                .expect("read retained marker"),
            "the injected cleanup fault must leave the marker in place"
        );

        backend.shutdown().await;
    }
}
