use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use crate::spv::CoreBackendMode;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use dash_sdk::dpp::prelude::AssetLockProof;
use std::sync::Arc;
use std::time::Duration;

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
    ) -> Result<BackendTaskSuccessResult, String> {
        use dash_sdk::dashcore_rpc::RpcApi;
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

        // Step 1: Create the asset lock transaction
        let (asset_lock_transaction, asset_lock_private_key, _asset_lock_address, used_utxos) = {
            let wallet_arc = {
                let wallets = self.wallets.read().unwrap();
                wallets
                    .get(&seed_hash)
                    .cloned()
                    .ok_or_else(|| "Wallet not found".to_string())?
            };

            let mut wallet = wallet_arc.write().map_err(|e| e.to_string())?;

            // Try to create the asset lock transaction, reload UTXOs if needed
            match wallet.generic_asset_lock_transaction(
                self.network,
                asset_lock_amount,
                allow_take_fee_from_amount,
                Some(self),
            ) {
                Ok((tx, private_key, address, _change, utxos)) => (tx, private_key, address, utxos),
                Err(_) => {
                    // Reload UTXOs and try again
                    wallet
                        .reload_utxos(
                            &self
                                .core_client
                                .read()
                                .expect("Core client lock was poisoned"),
                            self.network,
                            Some(self),
                        )
                        .map_err(|e| e.to_string())?;

                    let (tx, private_key, address, _change, utxos) = wallet
                        .generic_asset_lock_transaction(
                            self.network,
                            asset_lock_amount,
                            allow_take_fee_from_amount,
                            Some(self),
                        )?;
                    (tx, private_key, address, utxos)
                }
            }
        };

        let tx_id = asset_lock_transaction.txid();

        // Step 2: Register this transaction as waiting for finality
        {
            let mut proofs = self.transactions_waiting_for_finality.lock().unwrap();
            proofs.insert(tx_id, None);
        }

        // Step 3: Broadcast the transaction
        self.core_client
            .read()
            .expect("Core client lock was poisoned")
            .send_raw_transaction(&asset_lock_transaction)
            .map_err(|e| format!("Failed to broadcast asset lock transaction: {}", e))?;

        // Step 4: Remove used UTXOs from wallet
        {
            let wallet_arc = {
                let wallets = self.wallets.read().unwrap();
                wallets
                    .get(&seed_hash)
                    .cloned()
                    .ok_or_else(|| "Wallet not found".to_string())?
            };

            let mut wallet = wallet_arc.write().map_err(|e| e.to_string())?;
            wallet.utxos.retain(|_, utxo_map| {
                utxo_map.retain(|outpoint, _| !used_utxos.contains_key(outpoint));
                !utxo_map.is_empty()
            });

            for utxo in used_utxos.keys() {
                self.db
                    .drop_utxo(utxo, &self.network.to_string())
                    .map_err(|e| e.to_string())?;
            }

            wallet.recalculate_affected_address_balances(&used_utxos, self)?;
        }

        // Step 5: Wait for asset lock proof (InstantLock or ChainLock) with timeout
        let asset_lock_proof: AssetLockProof;
        let timeout = tokio::time::sleep(Duration::from_secs(300)); // 5 minute timeout
        tokio::pin!(timeout);

        loop {
            tokio::select! {
                _ = &mut timeout => {
                    // Best-effort cleanup: use try_lock to avoid blocking the
                    // async runtime if another thread holds the mutex.
                    if let Ok(mut proofs) = self.transactions_waiting_for_finality.try_lock() {
                        proofs.remove(&tx_id);
                    }

                    // Auto-refresh wallet UTXOs in RPC mode so the broadcast tx's
                    // spent inputs are reconciled (the tx was already broadcast and
                    // may confirm later). SPV handles its own reconciliation.
                    if self.core_backend_mode() == CoreBackendMode::Rpc
                        && let Some(wallet_arc) = self.wallets.read().ok()
                            .and_then(|w| w.get(&seed_hash).cloned())
                    {
                        let ctx = Arc::clone(self);
                        // Fire-and-forget — don't block the error return on refresh
                        tokio::task::spawn_blocking(move || {
                            if let Err(e) = ctx.refresh_wallet_info(wallet_arc) {
                                tracing::warn!("Failed to auto-refresh wallet after timeout: {}", e);
                            }
                        });
                    }

                    return Err("Timeout waiting for asset lock proof — no InstantLock or ChainLock received within 5 minutes".to_string());
                }
                _ = tokio::time::sleep(Duration::from_millis(200)) => {
                    // Brief lock to check for proof — acquired and released quickly
                    // so contention is minimal.
                    let proofs = self.transactions_waiting_for_finality.lock().unwrap();
                    if let Some(Some(proof)) = proofs.get(&tx_id) {
                        asset_lock_proof = proof.clone();
                        break;
                    }
                }
            }
        }

        // Step 6: Clean up the finality tracking
        {
            let mut proofs = self.transactions_waiting_for_finality.lock().unwrap();
            proofs.remove(&tx_id);
        }

        // Step 7: Get wallet, SDK, and derive a fresh change address if needed
        let (wallet, sdk, change_platform_address) = {
            let wallet_arc = {
                let wallets = self.wallets.read().unwrap();
                wallets
                    .get(&seed_hash)
                    .cloned()
                    .ok_or_else(|| "Wallet not found".to_string())?
            };

            // Derive a fresh change address from the BIP44 internal (change) path
            // while we have write access (only needed when fees are NOT deducted
            // from the output). Using change_address() ensures proper BIP44
            // separation between receive and change addresses.
            let change_platform_address = if !fee_deduct_from_output {
                let mut wallet_w = wallet_arc.write().map_err(|e| e.to_string())?;
                let addr = wallet_w.change_address(self.network, Some(self))?;
                Some(
                    PlatformAddress::try_from(addr)
                        .map_err(|e| format!("Failed to convert change address: {}", e))?,
                )
            } else {
                None
            };

            let wallet = wallet_arc.read().map_err(|e| e.to_string())?.clone();
            let sdk = self.sdk.read().map_err(|e| e.to_string())?.clone();
            (wallet, sdk, change_platform_address)
        };

        // Step 8: Fund the destination platform address
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
                format!(
                    "Overflow converting {amount} duffs to credits (CREDITS_PER_DUFF = {CREDITS_PER_DUFF})"
                )
            })?;

            if let Some(change_address) = change_platform_address {
                outputs.insert(destination, Some(amount_credits));
                outputs.insert(change_address, None); // Remainder recipient

                // Determine the BTreeMap index of the change address to target it
                // with the fee strategy (BTreeMap iterates in key order).
                let change_index = outputs
                    .keys()
                    .position(|k| *k == change_address)
                    .ok_or("Change address not found in outputs map")?
                    as u16;
                vec![AddressFundsFeeStrategyStep::ReduceOutput(change_index)]
            } else {
                return Err("Failed to derive a change address for platform funding".to_string());
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
            .map_err(|e| format!("Failed to fund platform address: {}", e))?;

        // Step 9: Refresh platform address balances
        self.fetch_platform_address_balances(seed_hash).await?;

        Ok(BackendTaskSuccessResult::PlatformAddressFunded { seed_hash })
    }
}
