use crate::backend_task::error::TaskError;
use crate::backend_task::identity::{IdentityTopUpInfo, TopUpIdentityFundingMethod};
use crate::backend_task::{BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use dash_sdk::Sdk;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::platform::Identifier;

/// How a top-up must be funded, decided by which wallet owns the identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TopUpRoute {
    /// The paying wallet owns the identity: fund from its HD slot, through the
    /// upstream orchestrator that requires the identity to be registered here.
    OwnIdentity,
    /// The identity belongs to another wallet: fund from the index-less
    /// account, since no slot in this wallet describes that identity.
    ForeignIdentity,
}

/// Resolve the funding route from the identity's recorded wallet link and the
/// paying wallet's seed hash.
///
/// The stored link — not `QualifiedIdentity::associated_wallets`, which
/// hydration fills with every loaded wallet — is what ownership means here. An
/// identity linked to no wallet keeps the existing fail-closed rejection: it
/// has no HD slot anywhere, and the callers pass a sentinel index for it.
/// Pure — no I/O — so it is unit-testable.
fn resolve_top_up_route(
    identity_id: Identifier,
    linked_wallet: Option<WalletSeedHash>,
    paying_wallet: &WalletSeedHash,
) -> Result<TopUpRoute, TaskError> {
    match linked_wallet {
        Some(owner) if owner == *paying_wallet => Ok(TopUpRoute::OwnIdentity),
        Some(_) => Ok(TopUpRoute::ForeignIdentity),
        None => Err(TaskError::IdentityNotWalletOwned { identity_id }),
    }
}

/// Validate a wallet-funded top-up's HD index against the identity's recorded
/// wallet position, fail-secure.
///
/// The op derives its asset-lock funding account from the wallet's HD tree at
/// `identity_index`, so that index must equal the identity's authoritative
/// `wallet_index`, and the identity must be wallet-owned (`Some`) at all. A
/// `None` wallet index means the identity is not owned by this wallet — there is
/// no HD slot to fund from, and the callers pass a sentinel index for it, which
/// must never reach the derivation. Pure — no I/O — so it is unit-testable.
fn validate_topup_index(
    identity_id: Identifier,
    wallet_index: Option<u32>,
    identity_index: u32,
) -> Result<(), TaskError> {
    match wallet_index {
        Some(wallet_index) if wallet_index == identity_index => Ok(()),
        Some(wallet_index) => Err(TaskError::IdentityIndexMismatch {
            identity_id,
            requested_index: identity_index,
            wallet_index,
        }),
        None => Err(TaskError::IdentityNotWalletOwned { identity_id }),
    }
}

impl AppContext {
    pub(super) async fn top_up_identity(
        &self,
        sdk: &Sdk,
        input: IdentityTopUpInfo,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let IdentityTopUpInfo {
            mut qualified_identity,
            wallet,
            identity_funding_method,
        } = input;

        let balance_before = qualified_identity.identity.balance();
        // This estimate is shown to the user and feeds the actual-fee
        // plausibility band, so it must track the active network fee multiplier —
        // use the context estimator rather than the hardcoded default.
        let fee_estimator = self.fee_estimator();
        let estimated_fee = fee_estimator.estimate_identity_topup();

        // Both wallet-funded top-up paths (fresh asset lock or resume from a
        // tracked asset lock) run end-to-end through the upstream
        // `IdentityWallet::top_up_identity_with_funding`. Upstream owns the
        // asset-lock build/broadcast, the IS→CL fallback, the
        // `TopUpIdentity` submission, and the asset-lock cleanup. DET only
        // mirrors the new balance into its local stores.
        let (funding, identity_index, top_up_index, amount_duffs_for_fee) =
            match identity_funding_method {
                TopUpIdentityFundingMethod::FundWithWallet(
                    amount,
                    identity_index,
                    top_up_index,
                ) => {
                    let funding =
                        platform_wallet::wallet::asset_lock::AssetLockFunding::FromWalletBalance {
                            amount_duffs: amount,
                            account_index: 0,
                        };
                    (funding, identity_index, top_up_index, Some(amount))
                }
                TopUpIdentityFundingMethod::UseAssetLock {
                    out_point,
                    identity_index,
                    top_up_index,
                } => {
                    let funding = platform_wallet::wallet::asset_lock::AssetLockFunding::FromExistingAssetLock {
                        out_point,
                        // Generic identity top-up resume, not the DashPay
                        // invitation-voucher reclaim flow, so it must never
                        // consume a bearer-voucher (invitation-typed) lock.
                        consume_invitation_voucher: false,
                    };
                    (funding, identity_index, top_up_index, None)
                }
            };

        let seed_hash = wallet.read().map_err(TaskError::from)?.seed_hash();
        let identity_id = qualified_identity.identity.id();
        let linked_wallet = self
            .stored_identity_wallet_link(&identity_id)?
            .map(|(owner, _)| owner);
        let new_balance = match resolve_top_up_route(identity_id, linked_wallet, &seed_hash)? {
            TopUpRoute::OwnIdentity => {
                // Fail-secure: a wallet-funded top-up of an own identity derives
                // its asset-lock account from this wallet's HD tree at the
                // identity's index, so the op index must equal the identity's
                // recorded `wallet_index`. Reject before any funds move.
                validate_topup_index(identity_id, qualified_identity.wallet_index, identity_index)?;
                self.wallet_backend()?
                    .top_up_identity(
                        &seed_hash,
                        &qualified_identity.identity,
                        funding,
                        identity_index,
                        None,
                    )
                    .await?
            }
            TopUpRoute::ForeignIdentity => {
                self.top_up_foreign_identity(sdk, &qualified_identity.identity, &seed_hash, funding)
                    .await?
            }
        };
        qualified_identity.identity.set_balance(new_balance);

        let actual_fee = match amount_duffs_for_fee {
            Some(amount) => {
                fee_estimator.resolve_identity_topup_actual_fee(amount, balance_before, new_balance)
            }
            None => estimated_fee,
        };
        tracing::info!(
            "Identity top-up complete: balance before {} credits, balance after {} credits, estimated fee {} credits, actual fee {} credits",
            balance_before,
            new_balance,
            estimated_fee,
            actual_fee,
        );

        if let Some(amount) = amount_duffs_for_fee {
            qualified_identity.top_ups.insert(top_up_index, amount);
            self.save_top_ups(
                &qualified_identity.identity.id(),
                &qualified_identity.top_ups,
            )?;
        }
        self.update_local_qualified_identity(&qualified_identity)?;

        let fee_result = FeeResult::new(estimated_fee, actual_fee);
        Ok(BackendTaskSuccessResult::ToppedUpIdentity(
            qualified_identity,
            fee_result,
        ))
    }

    /// Top up an identity that belongs to another wallet, paying from this one.
    /// Returns the identity's post-top-up balance (credits).
    ///
    /// The upstream orchestrator can only top up identities registered in the
    /// paying wallet's own manager, and registering one there files another
    /// wallet's identity — and its keys — under this wallet, state its next
    /// load cannot resolve. So this path funds an index-less asset lock and
    /// submits the transition through the SDK directly, exactly as the
    /// platform-address funding fallback does, leaving the paying wallet's
    /// identity state untouched. The credit output is derived in the paying
    /// wallet's own tree, so only its funds move.
    ///
    /// Two recovery steps the orchestrated path performs are unavailable here,
    /// because upstream keeps both `pub(crate)`: a Platform-rejected InstantSend
    /// proof is not retried under a ChainLock, and the spent lock is not marked
    /// consumed — it keeps its pre-consumption status in the funding list.
    async fn top_up_foreign_identity(
        &self,
        sdk: &Sdk,
        identity: &dash_sdk::platform::Identity,
        seed_hash: &WalletSeedHash,
        funding: platform_wallet::wallet::asset_lock::AssetLockFunding,
    ) -> Result<u64, TaskError> {
        use dash_sdk::platform::transition::top_up_identity::TopUpIdentity;
        use platform_wallet::AssetLockFundingType;
        use platform_wallet::wallet::asset_lock::AssetLockFunding;

        // TODO(upstream): restore the two recovery steps this path cannot run
        //   while `AssetLockManager::upgrade_to_chain_lock_proof` and
        //   `consume_asset_lock` stay `pub(crate)` — the IS→ChainLock retry on a
        //   rejected InstantSend proof, and marking the spent lock consumed so
        //   it leaves the resumable-funding list.
        let backend = self.wallet_backend()?;
        let (proof, credit_output_key) = match funding {
            AssetLockFunding::FromWalletBalance { amount_duffs, .. } => {
                let (proof, key, _txid) = backend
                    .create_asset_lock_proof(
                        seed_hash,
                        amount_duffs,
                        AssetLockFundingType::IdentityTopUpNotBound,
                        0,
                    )
                    .await?;
                (proof, key)
            }
            AssetLockFunding::FromExistingAssetLock { out_point, .. } => {
                backend
                    .resume_unbound_topup_asset_lock(seed_hash, out_point)
                    .await?
            }
        };

        identity
            .top_up_identity_with_private_key(sdk, proof, &credit_output_key, None)
            .await
            .map_err(|e| crate::wallet_backend::map_identity_top_up_sdk_error(identity.id(), e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wallet-owned identity whose op index matches its wallet index passes.
    #[test]
    fn validate_topup_index_accepts_matching_wallet_index() {
        let id = Identifier::random();
        assert!(validate_topup_index(id, Some(5), 5).is_ok());
    }

    /// A divergent op index is rejected with the indices captured as typed
    /// fields (not a string).
    #[test]
    fn validate_topup_index_rejects_mismatch_with_indices() {
        let id = Identifier::random();
        let err =
            validate_topup_index(id, Some(5), 3).expect_err("a divergent op index must reject");
        match err {
            TaskError::IdentityIndexMismatch {
                identity_id,
                requested_index,
                wallet_index,
            } => {
                assert_eq!(identity_id, id);
                assert_eq!(requested_index, 3);
                assert_eq!(wallet_index, 5);
            }
            other => panic!("expected IdentityIndexMismatch, got: {other:?}"),
        }
    }

    /// The paying wallet's own identity takes the orchestrated HD-slot route.
    #[test]
    fn resolve_top_up_route_sends_an_own_identity_through_the_hd_slot() {
        let paying: WalletSeedHash = [0x11u8; 32];
        assert_eq!(
            resolve_top_up_route(Identifier::random(), Some(paying), &paying)
                .expect("an own identity is routable"),
            TopUpRoute::OwnIdentity
        );
    }

    /// An identity linked to a different wallet must NOT take the orchestrated
    /// route: that route registers the identity under the payer, filing another
    /// wallet's identity — and its keys — where the payer's next load cannot
    /// resolve them, which fails the whole wallet load.
    #[test]
    fn resolve_top_up_route_sends_another_wallets_identity_through_the_foreign_path() {
        let owner: WalletSeedHash = [0x22u8; 32];
        let paying: WalletSeedHash = [0x33u8; 32];
        assert_eq!(
            resolve_top_up_route(Identifier::random(), Some(owner), &paying)
                .expect("another wallet's identity is routable"),
            TopUpRoute::ForeignIdentity
        );
    }

    /// An identity linked to no wallet keeps failing closed — it has no HD slot
    /// anywhere, and the callers pass a sentinel index for it.
    #[test]
    fn resolve_top_up_route_rejects_an_identity_linked_to_no_wallet() {
        let id = Identifier::random();
        let err = resolve_top_up_route(id, None, &[0x44u8; 32])
            .expect_err("an unlinked identity must reject");
        assert!(
            matches!(err, TaskError::IdentityNotWalletOwned { identity_id } if identity_id == id),
            "expected IdentityNotWalletOwned, got: {err:?}"
        );
    }

    /// A non-wallet-owned identity (`wallet_index == None`) fails closed even
    /// with the sentinel indices the UI (`u32::MAX >> 1`) and MCP (`0`) pass —
    /// the funds-safety hole this guard closes.
    #[test]
    fn validate_topup_index_rejects_non_wallet_owned() {
        let id = Identifier::random();
        for sentinel in [0u32, u32::MAX >> 1] {
            let err = validate_topup_index(id, None, sentinel)
                .expect_err("a non-wallet-owned identity must reject");
            assert!(
                matches!(err, TaskError::IdentityNotWalletOwned { identity_id } if identity_id == id),
                "expected IdentityNotWalletOwned, got: {err:?}"
            );
        }
    }
}
