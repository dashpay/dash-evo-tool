use crate::backend_task::error::{
    TaskError, ensure_existing_funding_covers_network_fee, ensure_funding_covers_network_fee,
};
use crate::backend_task::identity::{IdentityRegistrationInfo, RegisterIdentityFundingMethod};
use crate::backend_task::{BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::model::fee_estimation::identity_create_min_funding_duffs;
use crate::model::identity_balance::BalanceProvenance;
use crate::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use crate::model::request_type::RequestType;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::prelude::AddressNonce;
use dash_sdk::platform::{FetchMany, Identity};
use dash_sdk::query_types::AddressInfo;
use std::collections::BTreeMap;

/// Whether `qi` still carries the all-zeros placeholder id assigned before
/// the upstream wallet-backend path learns the real identity id.
///
/// A placeholder identity must never be persisted: keyed by the all-zeros
/// id it pollutes the identity store and enumeration index with a phantom
/// entry that every subsequent failure overwrites.
fn is_placeholder_identity(qi: &QualifiedIdentity) -> bool {
    qi.identity.id() == dash_sdk::platform::Identifier::default()
}

impl AppContext {
    pub(super) async fn register_identity(
        &self,
        input: IdentityRegistrationInfo,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let IdentityRegistrationInfo {
            keys,
            wallet,
            wallet_identity_index,
            identity_funding_method,
        } = input;

        let sdk = self.sdk.load().as_ref().clone();

        let public_keys = keys.to_public_keys_map(self.platform_version())?;
        let key_count = public_keys.len();
        let estimated_fee = self.fee_estimator().estimate_identity_create(key_count);

        let wallet_seed_hash = { wallet.read().map_err(TaskError::from)?.seed_hash() };

        // Wallet-funded paths (fresh asset lock or resume from a tracked
        // asset lock) are handled end-to-end by the upstream
        // `IdentityWallet`. It builds (or resumes) the asset lock,
        // broadcasts, waits for IS/CL, submits the identity-create state
        // transition with the upstream IS→CL fallback, and cleans up the
        // tracked asset lock on success.
        match identity_funding_method {
            RegisterIdentityFundingMethod::FundWithWallet(amount_duffs, identity_index) => {
                // Before any wallet work: the network refuses a funding below
                // its fee only after the funds have left the wallet.
                ensure_funding_covers_network_fee(
                    amount_duffs,
                    identity_create_min_funding_duffs(key_count, sdk.version()),
                )?;
                let funding =
                    platform_wallet::wallet::asset_lock::AssetLockFunding::FromWalletBalance {
                        amount_duffs,
                        account_index: 0,
                    };
                self.register_identity_via_wallet_backend(
                    funding,
                    identity_index,
                    wallet_identity_index,
                    public_keys,
                    keys,
                    wallet,
                    wallet_seed_hash,
                    estimated_fee,
                )
                .await
            }
            RegisterIdentityFundingMethod::UseAssetLock {
                out_point,
                identity_index,
            } => {
                // Before any signing or network work.
                let backend = self.wallet_backend()?;
                ensure_existing_funding_covers_network_fee(
                    &backend.list_tracked_asset_locks(&wallet_seed_hash).await?,
                    &out_point,
                    identity_create_min_funding_duffs(key_count, sdk.version()),
                )?;
                let funding =
                    platform_wallet::wallet::asset_lock::AssetLockFunding::FromExistingAssetLock {
                        out_point,
                        // Generic identity-registration resume, not the DashPay
                        // invitation-voucher reclaim flow, so it must never
                        // consume a bearer-voucher (invitation-typed) lock.
                        consume_invitation_voucher: false,
                    };
                self.register_identity_via_wallet_backend(
                    funding,
                    identity_index,
                    wallet_identity_index,
                    public_keys,
                    keys,
                    wallet,
                    wallet_seed_hash,
                    estimated_fee,
                )
                .await
            }
            RegisterIdentityFundingMethod::FundWithPlatformAddresses {
                inputs,
                wallet_seed_hash,
            } => {
                // Fetch fresh nonces from platform to ensure we have current values
                let addresses_to_fetch: std::collections::BTreeSet<PlatformAddress> =
                    inputs.keys().cloned().collect();

                let fetched_address_infos =
                    AddressInfo::fetch_many(&sdk, addresses_to_fetch.clone())
                        .await
                        .map_err(|e| TaskError::PlatformFetchError {
                            source: Box::new(e),
                        })?;

                let inputs_with_nonces = inputs_with_next_nonces(inputs, &fetched_address_infos);

                self.register_identity_from_platform_addresses(
                    keys,
                    wallet,
                    wallet_identity_index,
                    inputs_with_nonces,
                    wallet_seed_hash,
                )
                .await
            }
        }
    }

    /// Drive identity registration through the upstream signer-driven
    /// orchestrator. Upstream owns asset-lock build/broadcast, IS→CL
    /// fallback, the actual submit, and the tracked-asset-lock cleanup —
    /// DET stays out of that loop and only updates its own local mirror.
    #[allow(clippy::too_many_arguments)]
    async fn register_identity_via_wallet_backend(
        &self,
        funding: platform_wallet::wallet::asset_lock::AssetLockFunding,
        identity_index: u32,
        wallet_identity_index: u32,
        public_keys: BTreeMap<
            dash_sdk::dpp::identity::KeyID,
            dash_sdk::dpp::identity::IdentityPublicKey,
        >,
        keys: super::IdentityKeySpecs,
        wallet: std::sync::Arc<std::sync::RwLock<super::Wallet>>,
        wallet_seed_hash: super::WalletSeedHash,
        estimated_fee: u64,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let backend = self.wallet_backend()?;

        // Build a placeholder identity to seed the local QualifiedIdentity
        // bookkeeping; the upstream call returns the authoritative Identity
        // and we replace it on success.
        let placeholder_id = dash_sdk::platform::Identifier::default();
        let placeholder_identity = Identity::new_with_id_and_keys(
            placeholder_id,
            public_keys.clone(),
            self.platform_version(),
        )
        .map_err(|e| TaskError::IdentityCreationError {
            source: Box::new(e),
        })?;
        let private_keys = keys
            .to_key_storage(wallet_seed_hash, self.platform_version())
            .map_err(|source| TaskError::IdentityCreationError { source })?;

        let mut qualified_identity = QualifiedIdentity {
            identity: placeholder_identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys,
            dpns_names: vec![],
            associated_wallets: BTreeMap::from([(wallet_seed_hash, wallet.clone())]),
            secret_access: self.wallet_backend().ok().map(|b| b.secret_access()),
            wallet_index: Some(wallet_identity_index),
            top_ups: Default::default(),
            status: IdentityStatus::PendingCreation,
            network: self.network,
        };

        let registered_identity = backend
            .register_identity(
                &wallet_seed_hash,
                identity_index,
                funding,
                public_keys,
                &qualified_identity,
                None,
            )
            .await
            .inspect_err(|_| {
                // On this path the real identity id is only known once upstream
                // succeeds; the local mirror still carries the all-zeros
                // placeholder. Persisting it would key a bogus zero-id record in
                // the identity store and enumeration index, surfacing a phantom
                // identity that every later failure overwrites. There is no real
                // id to anchor a FailedCreation marker to, so skip the insert —
                // the upstream discovery loop owns the asset-lock bookkeeping.
                if is_placeholder_identity(&qualified_identity) {
                    return;
                }
                qualified_identity
                    .status
                    .update(IdentityStatus::FailedCreation);
                let _ = self.insert_local_qualified_identity(
                    &qualified_identity,
                    &Some((wallet_seed_hash, wallet_identity_index)),
                );
            })?;

        qualified_identity.identity = registered_identity.clone();
        qualified_identity.status = IdentityStatus::Unknown; // force refresh

        self.insert_local_qualified_identity(
            &qualified_identity,
            &Some((wallet_seed_hash, wallet_identity_index)),
        )?;
        {
            let mut wallet_w = wallet.write().map_err(TaskError::from)?;
            wallet_w
                .identities
                .insert(wallet_identity_index, registered_identity);
        }
        // The upstream identity discovery loop owns the asset-lock → identity
        // mapping on the new path; the DET-side `asset_lock_to_identity_id`
        // table is only consulted on the legacy staged-asset-lock recovery
        // path, so no mirror write is needed here.

        let fee_result = FeeResult::estimated_only(estimated_fee);
        Ok(BackendTaskSuccessResult::RegisteredIdentity(
            qualified_identity,
            fee_result,
        ))
    }

    /// Register a new identity funded by Platform addresses.
    ///
    /// `inputs` is a map of Platform addresses to (nonce, credits) tuples. Nonces must be incremented by 1
    /// from the current nonce of the address.
    async fn register_identity_from_platform_addresses(
        &self,
        keys: super::IdentityKeySpecs,
        wallet: std::sync::Arc<std::sync::RwLock<super::Wallet>>,
        wallet_identity_index: u32,
        inputs: BTreeMap<
            dash_sdk::dpp::address_funds::PlatformAddress,
            (AddressNonce, dash_sdk::dpp::fee::Credits),
        >,
        wallet_seed_hash: super::WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        use dash_sdk::platform::transition::put_identity::PutIdentity;

        let sdk = self.sdk.load().as_ref().clone();

        let public_keys = keys.to_public_keys_map(sdk.version())?;

        // Calculate fee estimate for identity creation from platform addresses
        let key_count = public_keys.len();
        let input_count = inputs.len();
        let estimated_fee = self
            .fee_estimator()
            .estimate_identity_create_from_addresses(input_count, false, key_count);

        // Clone the wallet for the pure address→path index (needed across the
        // async boundary). The signing key never lives in this snapshot — it is
        // derived JIT from the borrowed HD seed inside the secret scope below.
        let wallet_snapshot = { wallet.read().map_err(TaskError::from)?.clone() };

        let identity = Identity::new_with_input_addresses_and_keys(
            &inputs,
            public_keys.clone(),
            sdk.version(),
        )
        .map_err(|e| TaskError::IdentityCreationError {
            source: Box::new(e),
        })?;

        let wallet_seed_hash_actual = { wallet.read().map_err(TaskError::from)?.seed_hash() };
        let private_keys = keys
            .to_key_storage(wallet_seed_hash_actual, sdk.version())
            .map_err(|source| TaskError::IdentityCreationError { source })?;
        let mut qualified_identity = QualifiedIdentity {
            identity: identity.clone(),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys,
            dpns_names: vec![],
            associated_wallets: BTreeMap::from([(wallet_seed_hash_actual, wallet.clone())]),
            secret_access: self.wallet_backend().ok().map(|b| b.secret_access()),
            wallet_index: Some(wallet_identity_index),
            top_ups: Default::default(),
            status: IdentityStatus::PendingCreation,
            network: self.network,
        };

        // Sign each funding input through a JIT platform signer that borrows the
        // HD seed only for the duration of the SDK call. The pure path index is
        // built before the secret scope; the seed zeroizes on return and never
        // enters this layer by value.
        use crate::wallet_backend::{DetPlatformSigner, PlatformPathIndex, SecretScope};
        let network = self.network;
        let path_index = PlatformPathIndex::from_wallet(&wallet_snapshot, network);
        let backend = self.wallet_backend()?;

        // Send to Platform using address funding and wait for response
        let put_result = backend
            .secret_access()
            .with_secret_session(
                &SecretScope::HdSeed {
                    seed_hash: wallet_seed_hash,
                },
                async |session| {
                    let plaintext = session.plaintext();
                    let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;
                    let signer = DetPlatformSigner::from_held(seed, network, &path_index);
                    Ok(identity
                        .put_with_address_funding(
                            &sdk,
                            inputs,
                            None,
                            &qualified_identity,
                            &signer,
                            None,
                        )
                        .await)
                },
            )
            .await?;

        match put_result {
            Ok((updated_identity, address_infos, height)) => {
                qualified_identity.identity = updated_identity;
                qualified_identity.status = IdentityStatus::Unknown; // Force refresh

                // Show the spent balances in platform-wallet, DET's view and
                // the wallet total at once, pinned at the proof's height.
                self.record_platform_address_spend_proof(
                    wallet_seed_hash,
                    &address_infos,
                    height,
                    "identity registration",
                )
                .await;

                // The creation proof's height orders this first balance, so a
                // later fetch from a lagging node cannot replace it.
                self.insert_local_qualified_identity_observed(
                    &mut qualified_identity,
                    &Some((wallet_seed_hash, wallet_identity_index)),
                    BalanceProvenance::ObservedAt(height),
                )?;

                {
                    let mut wallet_guard = wallet.write().map_err(TaskError::from)?;
                    wallet_guard
                        .identities
                        .insert(wallet_identity_index, qualified_identity.identity.clone());
                }

                let fee_result = FeeResult::estimated_only(estimated_fee);
                Ok(BackendTaskSuccessResult::RegisteredIdentity(
                    qualified_identity,
                    fee_result,
                ))
            }
            Err(e) => {
                // Log proof errors and convert via log_drive_proof_error for consistent handling
                let task_error =
                    self.log_drive_proof_error(e, RequestType::BroadcastStateTransition);

                qualified_identity
                    .status
                    .update(IdentityStatus::FailedCreation);

                self.insert_local_qualified_identity(
                    &qualified_identity,
                    &Some((wallet_seed_hash, wallet_identity_index)),
                )?;

                Err(task_error)
            }
        }
    }
}

/// Pair each input with the nonce its spend must carry: one above the nonce
/// Platform just proved for it. A `None` entry is a proven absence, so the
/// address has never spent (nonce 0); a cached nonce could only be stale.
fn inputs_with_next_nonces(
    inputs: BTreeMap<PlatformAddress, Credits>,
    fetched: &dash_sdk::query_types::AddressInfos,
) -> BTreeMap<PlatformAddress, (AddressNonce, Credits)> {
    inputs
        .into_iter()
        .map(|(addr, credits)| {
            let nonce = fetched
                .get(&addr)
                .and_then(|opt| opt.as_ref())
                .map_or(0, |info| info.nonce);
            (addr, (nonce.saturating_add(1), credits))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dash_spv::Network;

    /// Platform proved one input absent and the other at nonce 4. The absent
    /// one is spent with nonce 1 and the other with 5.
    #[test]
    fn inputs_take_the_next_nonce_platform_proved() {
        let absent = PlatformAddress::P2pkh([1; 20]);
        let known = PlatformAddress::P2pkh([2; 20]);
        let fetched: dash_sdk::query_types::AddressInfos = [
            (absent, None),
            (
                known,
                Some(AddressInfo {
                    address: known,
                    nonce: 4,
                    balance: 900,
                }),
            ),
        ]
        .into_iter()
        .collect();
        let inputs = BTreeMap::from([(absent, 100), (known, 200)]);

        let signed = inputs_with_next_nonces(inputs, &fetched);

        assert_eq!(
            signed,
            BTreeMap::from([(absent, (1, 100)), (known, (5, 200))])
        );
    }
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::Identifier;

    fn qualified_identity_with_id(id: Identifier) -> QualifiedIdentity {
        let identity =
            Identity::create_basic_identity(id, PlatformVersion::latest()).expect("basic identity");
        QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: Default::default(),
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: Default::default(),
            status: IdentityStatus::PendingCreation,
            network: Network::Testnet,
        }
    }

    /// A failed wallet-backend registration must not persist its all-zeros
    /// placeholder identity. The guard keys on the placeholder id; this pins
    /// that an unresolved (default-id) identity is recognised as a placeholder
    /// and a real-id identity is not.
    #[test]
    fn placeholder_identity_is_not_persistable() {
        let placeholder = qualified_identity_with_id(Identifier::default());
        assert!(
            is_placeholder_identity(&placeholder),
            "all-zeros id must be treated as a placeholder and skipped on the failure path"
        );

        let mut real_id_bytes = [0u8; 32];
        real_id_bytes[0] = 7;
        real_id_bytes[31] = 9;
        let real = qualified_identity_with_id(Identifier::from(real_id_bytes));
        assert!(
            !is_placeholder_identity(&real),
            "a real identity id must be persisted, not skipped"
        );
    }
}
