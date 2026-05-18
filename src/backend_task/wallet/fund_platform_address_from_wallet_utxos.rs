use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use crate::wallet_backend::AssetLockKind;
use dash_sdk::dpp::address_funds::AddressFundsFeeStrategyStep;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use dash_sdk::platform::transition::top_up_address::TopUpAddress;
use std::sync::Arc;

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
                AssetLockKind::PlatformAddressTopUp,
                0,
            )
            .await?;

        // Derive a fresh BIP-44 change address (only when fees are NOT
        // deducted from the output — it absorbs the fee-estimate surplus).
        let (wallet, sdk, change_platform_address) = {
            let wallet_arc = {
                let wallets = self.wallets.read()?;
                wallets
                    .get(&seed_hash)
                    .cloned()
                    .ok_or(TaskError::WalletNotFound)?
            };
            let change_platform_address = if !fee_deduct_from_output {
                let mut wallet_w = wallet_arc.write()?;
                let addr = wallet_w
                    .change_address(self.network, Some(self))
                    .map_err(|e| TaskError::WalletAddressDerivationFailed { detail: e })?;
                Some(PlatformAddress::try_from(addr).map_err(|e| {
                    TaskError::AddressConversionFailed {
                        source: Box::new(e),
                    }
                })?)
            } else {
                None
            };
            let wallet = wallet_arc.read()?.clone();
            let sdk = self.sdk.load().as_ref().clone();
            (wallet, sdk, change_platform_address)
        };

        let mut outputs = std::collections::BTreeMap::new();
        let fee_strategy = if fee_deduct_from_output {
            outputs.insert(destination, None);
            vec![AddressFundsFeeStrategyStep::ReduceOutput(0)]
        } else {
            let amount_credits = amount.checked_mul(CREDITS_PER_DUFF).ok_or({
                TaskError::CreditCalculationOverflow {
                    amount,
                    credits_per_duff: CREDITS_PER_DUFF,
                }
            })?;
            let change_address =
                change_platform_address.ok_or(TaskError::ChangeAddressUnavailable {
                    reason: "no change address was derived for platform funding",
                })?;
            outputs.insert(destination, Some(amount_credits));
            outputs.insert(change_address, None);
            let change_index = outputs.keys().position(|k| *k == change_address).ok_or(
                TaskError::ChangeAddressUnavailable {
                    reason: "change address not found in outputs map",
                },
            )? as u16;
            vec![AddressFundsFeeStrategyStep::ReduceOutput(change_index)]
        };

        outputs
            .top_up(
                &sdk,
                asset_lock_proof,
                asset_lock_private_key,
                fee_strategy,
                &wallet,
                None,
            )
            .await
            .map_err(TaskError::from)?;

        self.fetch_platform_address_balances(seed_hash).await?;

        Ok(BackendTaskSuccessResult::PlatformAddressFunded { seed_hash })
    }
}
