use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::WalletId;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use std::sync::Arc;

impl AppContext {
    /// Fund a platform address directly from wallet UTXOs.
    /// Creates an asset lock, broadcasts it, waits for confirmation, then funds the destination.
    ///
    /// If `fee_deduct_from_output` is true, fees are deducted from the amount (recipient receives less).
    /// If `fee_deduct_from_output` is false, fees are paid from extra wallet balance (recipient receives exact amount).
    pub(crate) async fn fund_platform_address_from_wallet_utxos(
        self: &Arc<Self>,
        seed_hash: WalletId,
        amount: u64,
        destination: PlatformAddress,
        fee_deduct_from_output: bool,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        use dash_sdk::dpp::address_funds::AddressFundsFeeStrategyStep;
        use dash_sdk::platform::transition::top_up_address::TopUpAddress;

        // When fee_deduct_from_output is false, we need to create a larger asset lock
        // that includes the estimated platform fee, so the recipient receives the exact amount.
        let (asset_lock_amount, _allow_take_fee_from_amount) = if fee_deduct_from_output {
            // Fees deducted from output: use the requested amount, allow core fee to be taken from it
            (amount, true)
        } else {
            // Fees paid from wallet: add estimated platform fee to asset lock amount.
            // We use 2 outputs: the destination (explicit amount) and a change address
            // (remainder recipient that absorbs the fee).
            let estimated_platform_fee_duffs = self
                .fee_estimator()
                .estimate_address_funding_from_asset_lock_duffs(2);
            let asset_lock_amount = amount.saturating_add(estimated_platform_fee_duffs);
            (asset_lock_amount, false)
        };

        // Build, broadcast, and wait for finality proof in a single call.
        // AssetLockManager handles the full lifecycle: UTXO selection, TX
        // construction, broadcast, and IS-lock / ChainLock proof wait.
        let platform_wallet = self.require_platform_wallet(&seed_hash)?;

        let (asset_lock_proof, asset_lock_private_key, _out_point) = platform_wallet
            .asset_locks()
            .create_funded_asset_lock_proof(
                asset_lock_amount,
                0,
                platform_wallet::AssetLockFundingType::IdentityRegistration,
                0,
            )
            .await
            .map_err(|e| TaskError::AssetLockTransactionBuildFailed {
                detail: e.to_string(),
            })?;

        // Step 6: Get platform wallet, SDK, and derive a fresh change address if needed
        let (platform_wallet, sdk, change_platform_address) = {
            let wallet_arc = {
                let wallets = self.wallets.read()?;
                wallets
                    .get(&seed_hash)
                    .cloned()
                    .ok_or(TaskError::WalletNotFound)?
            };

            // Derive a fresh change address from the BIP44 internal (change) path
            // while we have write access (only needed when fees are NOT deducted
            // from the output). Using change_address() ensures proper BIP44
            // separation between receive and change addresses.
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

            let wallet_guard = wallet_arc.read()?;
            let platform_wallet = wallet_guard
                .platform_wallet
                .as_ref()
                .cloned()
                .ok_or(crate::backend_task::error::TaskError::WalletLocked)?;
            drop(wallet_guard);
            let sdk = self.sdk.load().as_ref().clone();
            (platform_wallet, sdk, change_platform_address)
        };

        // Step 7: Fund the destination platform address
        let mut outputs = std::collections::BTreeMap::new();

        let fee_strategy = if fee_deduct_from_output {
            // Fee deducted from output: destination is the remainder recipient (gets
            // asset lock value minus fee). ReduceOutput(0) tells Platform to deduct
            // the fee from the single output.
            outputs.insert(destination, None);
            vec![AddressFundsFeeStrategyStep::ReduceOutput(0)]
        } else {
            // Fee NOT deducted from output: destination receives the exact requested
            // amount. We use a fresh wallet-controlled change address to absorb the
            // fee estimate surplus, keeping it spendable.
            let amount_credits = amount.checked_mul(CREDITS_PER_DUFF).ok_or_else(|| {
                TaskError::CreditCalculationOverflow {
                    amount,
                    credits_per_duff: CREDITS_PER_DUFF,
                }
            })?;

            if let Some(change_address) = change_platform_address {
                outputs.insert(destination, Some(amount_credits));
                outputs.insert(change_address, None); // Remainder recipient

                // Determine the BTreeMap index of the change address to target it
                // with the fee strategy (BTreeMap iterates in key order).
                let change_index = outputs
                    .keys()
                    .position(|k| *k == change_address)
                    .ok_or_else(|| TaskError::ChangeAddressUnavailable {
                        reason: "change address not found in outputs map",
                    })? as u16;
                vec![AddressFundsFeeStrategyStep::ReduceOutput(change_index)]
            } else {
                return Err(TaskError::ChangeAddressUnavailable {
                    reason: "no change address was derived for platform funding",
                });
            }
        };

        outputs
            .top_up(
                &sdk,
                asset_lock_proof,
                asset_lock_private_key,
                fee_strategy,
                platform_wallet.platform(),
                None,
            )
            .await
            .map_err(TaskError::from)?;

        // Step 9: Refresh platform address balances
        self.fetch_platform_address_balances(seed_hash).await?;

        Ok(BackendTaskSuccessResult::PlatformAddressFunded { seed_hash })
    }
}
