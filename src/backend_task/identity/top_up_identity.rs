use crate::backend_task::error::TaskError;
use crate::backend_task::identity::{IdentityTopUpInfo, TopUpIdentityFundingMethod};
use crate::backend_task::{BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::platform::Identifier;

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
        // Fail-secure: a wallet-funded top-up derives its asset-lock account
        // from this wallet's HD tree at the identity's index, so the op index
        // must equal the identity's recorded `wallet_index` AND the identity
        // must be wallet-owned at all. Reject before any funds move — a foreign
        // identity has no HD slot here (the UI/MCP pass a sentinel index for
        // `None`, which must never reach the funding derivation). Verified:
        // `wallet_index == None` iff the identity is not wallet-owned, so every
        // valid target carries `Some(index)`.
        validate_topup_index(identity_id, qualified_identity.wallet_index, identity_index)?;
        let backend = self.wallet_backend()?;
        let new_balance = backend
            .top_up_identity(
                &seed_hash,
                &qualified_identity.identity,
                funding,
                identity_index,
                None,
            )
            .await?;
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
