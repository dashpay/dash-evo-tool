use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::address_funds::AddressFundsFeeStrategy;
use dash_sdk::dpp::address_funds::AddressFundsFeeStrategyStep;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use dash_sdk::platform::transition::top_up_address::TopUpAddress;
use platform_wallet::AssetLockFundingType;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Output map for a Platform address top-up: each entry maps a recipient to an
/// explicit credit amount, or `None` to absorb the remainder after fees.
type FundingOutputs = BTreeMap<PlatformAddress, Option<u64>>;

/// Build the output map and fee strategy for the fee-from-wallet funding branch.
///
/// The `destination` receives exactly `amount` converted to credits, and a
/// separate `change_address` absorbs the asset-lock surplus minus the Platform
/// fee. The returned `ReduceOutput` step indexes the change output's position in
/// the (sorted) map, so the fee is always deducted from change, never from the
/// destination amount.
fn build_fee_from_wallet_outputs(
    amount: u64,
    destination: PlatformAddress,
    change_address: PlatformAddress,
) -> Result<(FundingOutputs, AddressFundsFeeStrategy), TaskError> {
    let amount_credits =
        amount
            .checked_mul(CREDITS_PER_DUFF)
            .ok_or(TaskError::CreditCalculationOverflow {
                amount,
                credits_per_duff: CREDITS_PER_DUFF,
            })?;

    let mut outputs: FundingOutputs = BTreeMap::new();
    outputs.insert(destination, Some(amount_credits));
    outputs.insert(change_address, None);

    let change_index = outputs.keys().position(|k| *k == change_address).ok_or(
        TaskError::ChangeAddressUnavailable {
            reason: "change address not found in outputs map",
        },
    )? as u16;

    Ok((
        outputs,
        vec![AddressFundsFeeStrategyStep::ReduceOutput(change_index)],
    ))
}

impl AppContext {
    /// Fund a Platform (DIP-17) address directly from wallet UTXOs.
    ///
    /// The asset-lock build/broadcast/track-to-proof step is owned by the
    /// upstream `AssetLockManager`; the Platform-side `TopUpAddress` state
    /// transition (DAPI/SDK) is retained DET orchestration.
    pub(crate) async fn fund_platform_address_from_wallet_utxos(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        amount: u64,
        destination: PlatformAddress,
        fee_deduct_from_output: bool,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // When fees are paid from the wallet (not the output), the asset lock
        // must be large enough to also cover the estimated Platform fee.
        let (asset_lock_amount, _allow_take_fee_from_amount) = if fee_deduct_from_output {
            (amount, true)
        } else {
            let estimated_platform_fee_duffs = self
                .fee_estimator()
                .estimate_address_funding_from_asset_lock_duffs(2);
            (amount.saturating_add(estimated_platform_fee_duffs), false)
        };

        let backend = self.wallet_backend()?;
        let (asset_lock_proof, asset_lock_private_key, _tx_id) = backend
            .create_asset_lock_proof(
                &seed_hash,
                asset_lock_amount,
                AssetLockFundingType::AssetLockAddressTopUp,
                0,
            )
            .await?;

        let wallet_arc = {
            let wallets = self.wallets.read()?;
            wallets
                .get(&seed_hash)
                .cloned()
                .ok_or(TaskError::WalletNotFound)?
        };
        let sdk = self.sdk.load().as_ref().clone();

        // Derive the change address, build the outputs, and sign — all inside
        // one held-seed scope so the operation prompts at most once and the
        // seed zeroizes on return. The change must be a watched DIP-17
        // platform-payment address (so its credits are synced and spendable),
        // not a BIP-44 change address — hence it is derived and registered via
        // the platform-payment path, then the signer index is rebuilt to cover
        // it.
        use crate::wallet_backend::{DetPlatformSigner, PlatformPathIndex};
        let network = self.network;
        let ctx = Arc::clone(self);
        backend
            .secret_access()
            .with_secret_session(
                &crate::wallet_backend::SecretScope::HdSeed { seed_hash },
                async |session| {
                    let plaintext = session.plaintext();
                    let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;

                    let (outputs, fee_strategy) =
                        if fee_deduct_from_output {
                            let mut outputs: FundingOutputs = BTreeMap::new();
                            outputs.insert(destination, None);
                            (outputs, vec![AddressFundsFeeStrategyStep::ReduceOutput(0)])
                        } else {
                            let change_core_addr = {
                                let mut wallet_w = wallet_arc.write()?;
                                wallet_w
                                    .generate_platform_receive_address_with_seed(
                                        seed,
                                        network,
                                        Some(&ctx),
                                    )
                                    .map_err(|_| TaskError::WalletPlatformReceiveAddressFailed)?
                            };
                            let change_address = PlatformAddress::try_from(change_core_addr)
                                .map_err(|e| TaskError::AddressConversionFailed {
                                    source: Box::new(e),
                                })?;
                            build_fee_from_wallet_outputs(amount, destination, change_address)?
                        };

                    let path_index = {
                        let wallet = wallet_arc.read()?;
                        PlatformPathIndex::from_wallet(&wallet, network)
                    };
                    let signer = DetPlatformSigner::from_held(seed, network, &path_index);
                    // A Platform submit failure here propagates via `?` below, so
                    // the flow never reports success on a failed top-up. What is
                    // missing is the upstream recovery pipeline: on submit failure
                    // the freshly-created asset lock is left tracked-but-unconsumed
                    // (resumable), and on success it is not marked `Consumed`.
                    //
                    // TODO(upstream-gated): route this through
                    // `platform_wallet::PlatformWallet::fund_from_asset_lock`,
                    // which runs resolve → `submit_with_cl_height_retry` →
                    // `consume_asset_lock`. That method is public on the public
                    // `PlatformWallet`, but DET reaches it only via
                    // `WalletBackend::resolve_wallet` (private, -> `Arc<PlatformWallet>`),
                    // and the route needs an external `Signer<PlatformAddress>` plus a
                    // `key_wallet::signer::Signer` and an `AssetLockFunding`. Wiring it
                    // is a funds-safety change gated on Smythe+Marvin review.
                    outputs
                        .top_up(
                            &sdk,
                            asset_lock_proof,
                            asset_lock_private_key,
                            fee_strategy,
                            &signer,
                            None,
                        )
                        .await
                        .map_err(TaskError::from)
                },
            )
            .await?;

        self.fetch_platform_address_balances(seed_hash).await?;

        Ok(BackendTaskSuccessResult::PlatformAddressFunded { seed_hash })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p2pkh(byte: u8) -> PlatformAddress {
        PlatformAddress::P2pkh([byte; 20])
    }

    /// The destination receives exactly `amount * CREDITS_PER_DUFF`, and the
    /// `ReduceOutput` index resolves to the change output — never the
    /// destination — regardless of how the addresses sort in the map.
    fn assert_outputs(destination: PlatformAddress, change: PlatformAddress) {
        let amount = 12_345u64;
        let (outputs, fee_strategy) =
            build_fee_from_wallet_outputs(amount, destination, change).expect("builds outputs");

        assert_eq!(
            outputs.get(&destination).copied(),
            Some(Some(amount * CREDITS_PER_DUFF)),
            "destination receives the exact credit amount",
        );
        assert_eq!(
            outputs.get(&change).copied(),
            Some(None),
            "change output absorbs the remainder",
        );

        assert_eq!(fee_strategy.len(), 1);
        let AddressFundsFeeStrategyStep::ReduceOutput(index) = fee_strategy[0] else {
            panic!("fee-from-wallet branch reduces an output");
        };
        let reduced = outputs
            .keys()
            .nth(index as usize)
            .expect("fee index points at an output");
        assert_eq!(
            *reduced, change,
            "the fee is deducted from the change output"
        );
    }

    #[test]
    fn fee_index_resolves_to_change_when_change_sorts_first() {
        // change (0x01) sorts before destination (0x02)
        assert_outputs(p2pkh(0x02), p2pkh(0x01));
    }

    #[test]
    fn fee_index_resolves_to_change_when_change_sorts_last() {
        // change (0x02) sorts after destination (0x01)
        assert_outputs(p2pkh(0x01), p2pkh(0x02));
    }

    #[test]
    fn amount_overflow_is_rejected() {
        let err = build_fee_from_wallet_outputs(u64::MAX, p2pkh(0x01), p2pkh(0x02))
            .expect_err("overflows");
        assert!(matches!(err, TaskError::CreditCalculationOverflow { .. }));
    }
}
