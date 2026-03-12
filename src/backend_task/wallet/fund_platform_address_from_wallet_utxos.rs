use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
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
        seed_hash: WalletSeedHash,
        amount: u64,
        destination: PlatformAddress,
        fee_deduct_from_output: bool,
    ) -> Result<BackendTaskSuccessResult, crate::backend_task::error::TaskError> {
        use dash_sdk::dpp::address_funds::AddressFundsFeeStrategyStep;
        use dash_sdk::platform::transition::top_up_address::TopUpAddress;

        // When fee_deduct_from_output is false, we need to create a larger asset lock
        // that includes the estimated platform fee, so the recipient receives the exact amount.
        let (asset_lock_amount, allow_take_fee_from_amount) = if fee_deduct_from_output {
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

        // Step 1: Create the asset lock transaction (UTXOs are selected but NOT yet removed)
        let (asset_lock_transaction, asset_lock_private_key, _asset_lock_address, used_utxos) = {
            let wallet_arc = {
                let wallets = self
                    .wallets
                    .read()
                    .map_err(|_| crate::backend_task::error::TaskError::LockPoisoned {
                        resource: "wallets",
                    })?;
                wallets
                    .get(&seed_hash)
                    .cloned()
                    .ok_or(crate::backend_task::error::TaskError::WalletNotFound)?
            };

            let mut wallet = wallet_arc.write().map_err(|_| {
                crate::backend_task::error::TaskError::LockPoisoned { resource: "wallet" }
            })?;

            // Try to create the asset lock transaction, reload UTXOs if needed
            match wallet.generic_asset_lock_transaction(
                self,
                self.network,
                asset_lock_amount,
                allow_take_fee_from_amount,
            ) {
                Ok((tx, private_key, address, _change, utxos)) => (tx, private_key, address, utxos),
                Err(e) => {
                    // Reload UTXOs (RPC: fetches from Core; SPV: no-op).
                    // Only retry if something actually changed.
                    if !wallet.reload_utxos(self)? {
                        return Err(e.into());
                    }
                    let (tx, private_key, address, _change, utxos) = wallet
                        .generic_asset_lock_transaction(
                            self,
                            self.network,
                            asset_lock_amount,
                            allow_take_fee_from_amount,
                        )?;
                    (tx, private_key, address, utxos)
                }
            }
        };

        // Step 2–4: Store → broadcast → remove UTXOs (atomic pattern).
        let wallet_arc = {
            let wallets = self
                .wallets
                .read()
                .map_err(|_| crate::backend_task::error::TaskError::LockPoisoned {
                    resource: "wallets",
                })?;
            wallets
                .get(&seed_hash)
                .cloned()
                .ok_or(crate::backend_task::error::TaskError::WalletNotFound)?
        };

        let tx_id = self
            .broadcast_and_commit_asset_lock(
                &asset_lock_transaction,
                asset_lock_amount,
                &seed_hash,
                &wallet_arc,
                &used_utxos,
            )
            .await?;

        // Step 5: Wait for asset lock proof (InstantLock or ChainLock) via shared helper.
        // On timeout the helper cleans up the finality tracking entry.
        // Post-timeout recovery is mode-dependent:
        //   RPC  — fire-and-forget refresh_wallet_info to reconcile spent UTXOs
        //   SPV  — spent UTXOs are reconciled automatically on the next sync cycle
        let asset_lock_proof = match self.wait_for_asset_lock_proof(tx_id).await {
            Ok(proof) => proof,
            Err(timeout_err) => {
                use crate::spv::CoreBackendMode;

                match self.core_backend_mode() {
                    CoreBackendMode::Rpc => {
                        if let Some(wallet_arc) = self
                            .wallets
                            .read()
                            .ok()
                            .and_then(|w| w.get(&seed_hash).cloned())
                        {
                            let ctx = Arc::clone(self);
                            // Fire-and-forget — don't block the error return on refresh
                            tokio::task::spawn_blocking(move || {
                                if let Err(e) = ctx.refresh_wallet_info(wallet_arc) {
                                    tracing::warn!(
                                        "Failed to auto-refresh wallet after timeout: {}",
                                        e
                                    );
                                }
                            });
                        }
                    }
                    CoreBackendMode::Spv => {
                        tracing::warn!(
                            "Asset lock proof timed out in SPV mode (tx {}). \
                             Spent UTXOs will be reconciled automatically during \
                             the next SPV sync cycle when a new block arrives.",
                            tx_id
                        );
                    }
                }

                return Err(timeout_err);
            }
        };

        // Step 6: Get wallet, SDK, and derive a fresh change address if needed
        let (wallet, sdk, change_platform_address) = {
            let wallet_arc = {
                let wallets = self
                    .wallets
                    .read()
                    .map_err(|_| crate::backend_task::error::TaskError::LockPoisoned {
                        resource: "wallets",
                    })?;
                wallets
                    .get(&seed_hash)
                    .cloned()
                    .ok_or(crate::backend_task::error::TaskError::WalletNotFound)?
            };

            // Derive a fresh change address from the BIP44 internal (change) path
            // while we have write access (only needed when fees are NOT deducted
            // from the output). Using change_address() ensures proper BIP44
            // separation between receive and change addresses.
            let change_platform_address = if !fee_deduct_from_output {
                let mut wallet_w = wallet_arc.write().map_err(|_| {
                    crate::backend_task::error::TaskError::LockPoisoned { resource: "wallet" }
                })?;
                let addr = wallet_w.change_address(self.network, Some(self))?;
                Some(PlatformAddress::try_from(addr).map_err(|e| {
                    crate::backend_task::error::TaskError::Internal(format!(
                        "Failed to convert change address: {e}"
                    ))
                })?)
            } else {
                None
            };

            let wallet = wallet_arc
                .read()
                .map_err(|_| crate::backend_task::error::TaskError::LockPoisoned {
                    resource: "wallet",
                })?
                .clone();
            let sdk = self.sdk.load().as_ref().clone();
            (wallet, sdk, change_platform_address)
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
                crate::backend_task::error::TaskError::Internal(format!(
                    "Overflow converting {amount} duffs to credits (CREDITS_PER_DUFF = {CREDITS_PER_DUFF})"
                ))
            })?;

            if let Some(change_address) = change_platform_address {
                outputs.insert(destination, Some(amount_credits));
                outputs.insert(change_address, None); // Remainder recipient

                // Determine the BTreeMap index of the change address to target it
                // with the fee strategy (BTreeMap iterates in key order).
                let change_index = outputs
                    .keys()
                    .position(|k| *k == change_address)
                    .ok_or_else(|| {
                        crate::backend_task::error::TaskError::Internal(
                            "Change address not found in outputs map".to_string(),
                        )
                    })? as u16;
                vec![AddressFundsFeeStrategyStep::ReduceOutput(change_index)]
            } else {
                return Err(crate::backend_task::error::TaskError::Internal(
                    "Failed to derive a change address for platform funding".to_string(),
                ));
            }
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
            .map_err(crate::backend_task::error::TaskError::from)?;

        // Step 9: Refresh platform address balances
        self.fetch_platform_address_balances(seed_hash).await?;

        Ok(BackendTaskSuccessResult::PlatformAddressFunded { seed_hash })
    }
}
