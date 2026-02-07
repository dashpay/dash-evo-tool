use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::wallet::PlatformSyncMode;
use crate::context::AppContext;
use crate::model::fee_estimation::PlatformFeeEstimator;
use crate::model::wallet::WalletSeedHash;
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
            let estimated_platform_fee_duffs =
                PlatformFeeEstimator::new().estimate_address_funding_from_asset_lock_duffs(2);
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

            // Update address_balances for affected addresses
            let affected_addresses: std::collections::BTreeSet<_> =
                used_utxos.values().map(|(_, addr)| addr.clone()).collect();
            for address in affected_addresses {
                // Recalculate balance from remaining UTXOs for this address
                let new_balance = wallet
                    .utxos
                    .get(&address)
                    .map(|utxo_map| utxo_map.values().map(|tx_out| tx_out.value).sum())
                    .unwrap_or(0);
                let _ = wallet.update_address_balance(&address, new_balance, self);
            }
        }

        // Step 5: Wait for asset lock proof (InstantLock or ChainLock)
        let asset_lock_proof: AssetLockProof;
        loop {
            {
                let proofs = self.transactions_waiting_for_finality.lock().unwrap();
                if let Some(Some(proof)) = proofs.get(&tx_id) {
                    asset_lock_proof = proof.clone();
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        // Step 6: Clean up the finality tracking
        {
            let mut proofs = self.transactions_waiting_for_finality.lock().unwrap();
            proofs.remove(&tx_id);
        }

        // Step 7: Get wallet and SDK for the platform funding operation
        let (wallet, sdk) = {
            let wallet_arc = {
                let wallets = self.wallets.read().unwrap();
                wallets
                    .get(&seed_hash)
                    .cloned()
                    .ok_or_else(|| "Wallet not found".to_string())?
            };
            let wallet = wallet_arc.read().map_err(|e| e.to_string())?.clone();
            let sdk = self.sdk.read().map_err(|e| e.to_string())?.clone();
            (wallet, sdk)
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
            // amount. We use a separate "change" address derived from the asset lock
            // key as the remainder recipient (absorbs the fee estimate surplus).
            // The fee is deducted from the change output, not the destination.
            let amount_credits = amount.saturating_mul(CREDITS_PER_DUFF);
            let change_address = PlatformAddress::from(&asset_lock_private_key);

            if change_address == destination {
                // Extremely unlikely: random key collides with destination address.
                // Fall back to single-output mode with remainder.
                outputs.insert(destination, None);
                vec![AddressFundsFeeStrategyStep::ReduceOutput(0)]
            } else {
                outputs.insert(destination, Some(amount_credits));
                outputs.insert(change_address, None); // Remainder recipient

                // Determine the BTreeMap index of the change address to target it
                // with the fee strategy (BTreeMap iterates in key order).
                let change_index = outputs
                    .keys()
                    .position(|k| *k == change_address)
                    .unwrap_or(0) as u16;
                vec![AddressFundsFeeStrategyStep::ReduceOutput(change_index)]
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
        self.fetch_platform_address_balances(seed_hash, PlatformSyncMode::Auto)
            .await?;

        Ok(BackendTaskSuccessResult::PlatformAddressFunded { seed_hash })
    }
}
