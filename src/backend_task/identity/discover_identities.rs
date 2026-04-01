use crate::context::AppContext;
use crate::model::qualified_identity::DPNSNameInfo;
use crate::model::wallet::Wallet;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use std::sync::{Arc, RwLock};

impl AppContext {
    /// Discover and load identities derived from a wallet by checking the network.
    /// This is called automatically on wallet unlock to find any identities that
    /// were registered using keys from the wallet.
    ///
    /// When a platform-wallet is available for this wallet, delegates the
    /// gap-limit scan + DPNS lookup to `IdentityWallet::sync()` and converts
    /// the results into `QualifiedIdentity`. Falls back to the legacy scan
    /// when the platform-wallet is not registered.
    pub(crate) async fn discover_identities_from_wallet(
        self: &Arc<Self>,
        wallet: &Arc<RwLock<Wallet>>,
        max_identity_index: u32,
    ) -> Result<(), String> {
        let seed_hash = wallet.read().map_err(|e| e.to_string())?.seed_hash();

        // Try to delegate to platform-wallet's sync() when available.
        if let Some(platform_wallet) = self.get_platform_wallet(&seed_hash) {
            return self
                .discover_identities_via_platform_wallet(wallet, &platform_wallet, &seed_hash)
                .await;
        }

        // Fallback: legacy scan when platform-wallet is not available.
        self.discover_identities_legacy(wallet, max_identity_index, &seed_hash)
            .await
    }

    /// Delegate identity discovery to the platform-wallet's `IdentityWallet::sync()`.
    ///
    /// This calls the platform-wallet's gap-limit scanner (12 key indices per
    /// identity index, DPNS lookup, key storage) and then converts the
    /// discovered `ManagedIdentity` entries into evo-tool's `QualifiedIdentity`.
    async fn discover_identities_via_platform_wallet(
        self: &Arc<Self>,
        wallet: &Arc<RwLock<Wallet>>,
        platform_wallet: &crate::platform_wallet_bridge::PlatformWallet,
        seed_hash: &[u8; 32],
    ) -> Result<(), String> {
        use crate::model::qualified_identity::encrypted_key_storage::{
            PrivateKeyData, WalletDerivationPath,
        };
        use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
        use crate::model::qualified_identity::{
            IdentityStatus, IdentityType, PrivateKeyTarget, QualifiedIdentity,
        };

        tracing::info!(
            seed = %hex::encode(seed_hash),
            "Starting identity discovery via platform-wallet sync"
        );

        let identity_wallet = platform_wallet.identity();

        // Run the platform-wallet gap-limit scan + DPNS lookup.
        let discovered = identity_wallet.sync().await.map_err(|e| {
            format!("Platform wallet identity sync failed: {}", e)
        })?;

        if discovered.is_empty() {
            tracing::info!(
                seed = %hex::encode(seed_hash),
                "Platform-wallet sync found no new identities"
            );
            return Ok(());
        }

        tracing::info!(
            seed = %hex::encode(seed_hash),
            count = discovered.len(),
            "Platform-wallet sync discovered identities, converting to QualifiedIdentity"
        );

        // Read back the managed identity data from the identity manager.
        let manager = identity_wallet.identity_manager().await;

        let mut found_count = 0;
        for identity in &discovered {
            let identity_id = identity.id();

            // Check if we already have this identity stored in the evo-tool DB.
            let already_exists = {
                let wallets = self.wallets.read().map_err(|e| e.to_string())?;
                let existing = self.db.get_identity_by_id(&identity_id, self, &wallets);
                existing.is_ok() && existing.unwrap().is_some()
            };

            if already_exists {
                tracing::info!(
                    identity_id = %identity_id,
                    "Identity already loaded, skipping"
                );
                continue;
            }

            let managed = match manager.managed_identity(&identity_id) {
                Some(m) => m,
                None => {
                    tracing::warn!(
                        identity_id = %identity_id,
                        "Identity discovered but not found in identity manager"
                    );
                    continue;
                }
            };

            let identity_index = managed.identity_index;

            // Convert key storage from platform-wallet types to evo-tool types.
            let private_keys_map: std::collections::BTreeMap<_, _> = managed
                .key_storage
                .iter()
                .map(|(key_id, (pub_key, pk_data))| {
                    let (evo_pk_data, wallet_path) = match pk_data {
                        platform_wallet::PrivateKeyData::AtWalletDerivationPath {
                            wallet_seed_hash,
                            derivation_path,
                        } => {
                            let wallet_derivation_path = WalletDerivationPath {
                                wallet_seed_hash: *wallet_seed_hash,
                                derivation_path: derivation_path.clone(),
                            };
                            (
                                PrivateKeyData::AtWalletDerivationPath(
                                    wallet_derivation_path.clone(),
                                ),
                                Some(wallet_derivation_path),
                            )
                        }
                        platform_wallet::PrivateKeyData::Clear(key_bytes) => {
                            (PrivateKeyData::Clear(**key_bytes), None)
                        }
                    };

                    let qualified_pub_key =
                        QualifiedIdentityPublicKey::from_identity_public_key_in_wallet(
                            pub_key.clone(),
                            wallet_path,
                        );

                    (
                        (PrivateKeyTarget::PrivateKeyOnMainIdentity, *key_id),
                        (qualified_pub_key, evo_pk_data),
                    )
                })
                .collect();

            // Convert DPNS names.
            let dpns_names: Vec<DPNSNameInfo> = managed
                .dpns_names
                .iter()
                .map(|n| DPNSNameInfo {
                    name: n.label.clone(),
                    acquired_at: n.acquired_at.unwrap_or(0),
                })
                .collect();

            // Build QualifiedIdentity.
            let mut associated_wallets = std::collections::BTreeMap::new();
            associated_wallets.insert(*seed_hash, Arc::clone(wallet));

            let qualified_identity = QualifiedIdentity {
                identity: identity.clone(),
                associated_voter_identity: None,
                associated_operator_identity: None,
                associated_owner_key_id: None,
                identity_type: IdentityType::User,
                alias: None,
                private_keys: private_keys_map.into(),
                dpns_names,
                associated_wallets,
                wallet_index: Some(identity_index),
                top_ups: Default::default(),
                status: IdentityStatus::Unknown,
                network: self.network,
            };

            // Store the identity in the evo-tool DB.
            if let Err(e) = self.insert_local_qualified_identity(
                &qualified_identity,
                &Some((*seed_hash, identity_index)),
            ) {
                tracing::warn!(
                    identity_id = %identity_id,
                    error = %e,
                    "Failed to store discovered identity"
                );
            } else {
                // Add to wallet's identities map.
                if let Ok(mut wallet_guard) = wallet.write() {
                    wallet_guard
                        .identities
                        .insert(identity_index, qualified_identity.identity.clone());
                }
                found_count += 1;
                tracing::info!(
                    identity_id = %identity_id,
                    "Successfully loaded discovered identity via platform-wallet"
                );
            }
        }

        tracing::info!(
            seed = %hex::encode(seed_hash),
            found_count,
            "Identity discovery via platform-wallet complete"
        );

        Ok(())
    }

    /// Legacy identity discovery that queries Platform directly without the
    /// platform-wallet library. Used as a fallback when the platform-wallet
    /// is not available for a given wallet.
    async fn discover_identities_legacy(
        self: &Arc<Self>,
        wallet: &Arc<RwLock<Wallet>>,
        max_identity_index: u32,
        seed_hash: &[u8; 32],
    ) -> Result<(), String> {
        use dash_sdk::platform::Fetch;
        use dash_sdk::platform::types::identity::NonUniquePublicKeyHashQuery;

        const AUTH_KEY_LOOKUP_WINDOW: u32 = 12;

        let sdk = self.sdk.load().as_ref().clone();

        tracing::info!(
            seed = %hex::encode(seed_hash),
            "Starting legacy identity discovery for wallet (checking indices 0..{})",
            max_identity_index
        );

        let mut found_count = 0;

        for identity_index in 0..=max_identity_index {
            // Try to find an identity at this index by checking authentication keys
            let mut fetched_identity = None;
            let mut matched_key_index = None;

            for key_index in 0..AUTH_KEY_LOOKUP_WINDOW {
                let public_key = {
                    let wallet_guard = wallet.read().map_err(|e| e.to_string())?;
                    match wallet_guard.identity_authentication_ecdsa_public_key(
                        self.network,
                        identity_index,
                        key_index,
                    ) {
                        Ok(key) => key,
                        Err(e) => {
                            tracing::debug!(
                                "Could not derive key at index {}/{}: {}",
                                identity_index,
                                key_index,
                                e
                            );
                            continue;
                        }
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
                            "Error querying identity at index {}/{}: {}",
                            identity_index,
                            key_index,
                            e
                        );
                        continue;
                    }
                }
            }

            // If we found an identity, process and store it
            if let Some(identity) = fetched_identity {
                let identity_id = identity.id();
                tracing::info!(
                    identity_id = %identity_id,
                    identity_index,
                    key_index = ?matched_key_index,
                    "Discovered identity from wallet"
                );

                // Check if we already have this identity stored
                let already_exists = {
                    let wallets = self.wallets.read().map_err(|e| e.to_string())?;
                    let existing = self.db.get_identity_by_id(&identity_id, self, &wallets);
                    existing.is_ok() && existing.unwrap().is_some()
                };

                if already_exists {
                    tracing::info!(
                        identity_id = %identity_id,
                        "Identity already loaded, skipping"
                    );
                    continue;
                }

                // Build qualified identity with wallet key derivation paths
                match self
                    .build_qualified_identity_from_wallet(&sdk, identity, wallet, identity_index)
                    .await
                {
                    Ok(qualified_identity) => {
                        // Store the identity
                        if let Err(e) = self.insert_local_qualified_identity(
                            &qualified_identity,
                            &Some((*seed_hash, identity_index)),
                        ) {
                            tracing::warn!(
                                identity_id = %identity_id,
                                error = %e,
                                "Failed to store discovered identity"
                            );
                        } else {
                            // Add to wallet's identities map
                            if let Ok(mut wallet_guard) = wallet.write() {
                                wallet_guard
                                    .identities
                                    .insert(identity_index, qualified_identity.identity.clone());
                            }
                            found_count += 1;
                            tracing::info!(
                                identity_id = %identity_id,
                                "Successfully loaded discovered identity"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            identity_id = %identity_id,
                            error = %e,
                            "Failed to build qualified identity"
                        );
                    }
                }
            }
        }

        tracing::info!(
            seed = %hex::encode(seed_hash),
            found_count,
            "Legacy identity discovery complete"
        );

        Ok(())
    }

    /// Build a QualifiedIdentity from a fetched Identity with wallet key derivation paths.
    /// This matches identity public keys to wallet-derived keys and fetches DPNS names.
    async fn build_qualified_identity_from_wallet(
        &self,
        sdk: &dash_sdk::Sdk,
        identity: dash_sdk::platform::Identity,
        wallet: &Arc<RwLock<Wallet>>,
        identity_index: u32,
    ) -> Result<crate::model::qualified_identity::QualifiedIdentity, String> {
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

        let seed_hash = wallet.read().map_err(|e| e.to_string())?.seed_hash();

        // Get the highest key ID in the identity to know how many keys to derive
        let highest_key_id = identity.public_keys().keys().max().copied().unwrap_or(0);
        let derive_up_to = highest_key_id.saturating_add(6); // Add buffer for future keys

        // Derive authentication keys from wallet and build lookup maps
        let mut public_key_to_index: std::collections::BTreeMap<Vec<u8>, u32> =
            std::collections::BTreeMap::new();
        let mut public_key_hash_to_index: std::collections::BTreeMap<[u8; 20], u32> =
            std::collections::BTreeMap::new();

        {
            let wallet_guard = wallet.read().map_err(|e| e.to_string())?;
            for key_index in 0..=derive_up_to {
                if let Ok(public_key) = wallet_guard.identity_authentication_ecdsa_public_key(
                    self.network,
                    identity_index,
                    key_index,
                ) {
                    public_key_to_index.insert(public_key.to_bytes().to_vec(), key_index);
                    public_key_hash_to_index.insert(public_key.pubkey_hash().into(), key_index);
                }
            }
        }

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
            use dash_sdk::drive::query::{WhereClause, WhereOperator};
            use dash_sdk::platform::{Document, DocumentQuery, FetchMany};

            let query = DocumentQuery {
                data_contract: self.dpns_contract.clone(),
                document_type_name: "domain".to_string(),
                where_clauses: vec![WhereClause {
                    field: "records.identity".to_string(),
                    operator: WhereOperator::Equal,
                    value: Value::Identifier(identity.id().into()),
                }],
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
            wallet_index: Some(identity_index),
            top_ups: Default::default(),
            status: IdentityStatus::Unknown,
            network: self.network,
        })
    }
}
