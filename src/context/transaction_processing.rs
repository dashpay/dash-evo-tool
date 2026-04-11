use super::AppContext;
use crate::backend_task::error::TaskError;
use dash_sdk::Sdk;
use dash_sdk::dpp::dashcore::transaction::special_transaction::TransactionPayload::AssetLockPayloadType;
use dash_sdk::dpp::dashcore::{Address, InstantLock, OutPoint, Transaction, TxOut, Txid};
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::InstantAssetLockProof;
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::chain::ChainAssetLockProof;
use dash_sdk::dpp::prelude::{AssetLockProof, CoreBlockHeight};

impl AppContext {
    pub(crate) fn received_transaction_finality(
        &self,
        tx: &Transaction,
        islock: Option<InstantLock>,
        chain_locked_height: Option<CoreBlockHeight>,
    ) -> Result<Vec<(OutPoint, TxOut, Address)>, TaskError> {
        // Initialize a vector to collect wallet outpoints
        let mut wallet_outpoints = Vec::new();

        // Identify the wallets associated with the transaction
        let wallets = self.wallets.read()?;
        for wallet_arc in wallets.values() {
            let wallet = wallet_arc.write()?;
            for (vout, tx_out) in tx.output.iter().enumerate() {
                let address = if let Ok(output_addr) =
                    Address::from_script(&tx_out.script_pubkey, self.network)
                {
                    if wallet.has_address(&output_addr) {
                        output_addr
                    } else {
                        continue;
                    }
                } else {
                    continue;
                };
                // UTXOs and address balances are persisted via the changeset
                // path (SPV adapter stages them, auto-flushed via
                // FlushStrategy::Immediate). Only in-memory wallet state and
                // app-level metadata (DashPay contacts) are updated here.

                // Collect the outpoint (UTXOs tracked by ManagedWalletInfo)
                let out_point = OutPoint::new(tx.txid(), vout as u32);
                wallet_outpoints.push((out_point, tx_out.clone(), address.clone()));

                // Check if this is a DashPay contact payment by asking
                // THIS wallet's platform-wallet whether the address
                // belongs to one of its `DashpayReceivingFunds`
                // accounts (Phase 9b-4). We use the platform wallet on
                // the already-held write guard directly — going
                // through `app_context.wallets` here would deadlock
                // trying to re-acquire a read guard on the wallet we
                // already hold the writer on.
                //
                // key-wallet's block processing already advances the
                // `DashpayReceivingFunds` account's address pool
                // `highest_used` when the tx output matches — no
                // separate "bump contact highest receive index" call
                // is needed (Phase 9b-3 rollback).
                if let Some(pw) = wallet.platform_wallet.as_ref() {
                    let dashpay_match = match pw
                        .dashpay()
                        .try_match_incoming_dashpay_address(&address)
                    {
                        Ok(m) => m,
                        Err(()) => {
                            tracing::debug!(
                                %address,
                                "DashPay address match skipped: wallet busy. \
                                 Will be picked up on a future tx or refresh."
                            );
                            None
                        }
                    };
                    if let Some(m) = dashpay_match {
                        let owner_id = m.user_identity_id;
                        let contact_id = m.friend_identity_id;
                        let address_index = m.address_index;

                        // Record the received payment via the platform
                        // wallet — persister catches the changeset and
                        // writes to `dashpay_payments` on flush
                        // (Phase 9b-2).
                        crate::backend_task::dashpay::platform_wallet_cache::cache_payment_with_pw_blocking(
                            pw,
                            &owner_id,
                            tx.txid().to_string(),
                            platform_wallet::wallet::dashpay::PaymentEntry::new_received(
                                contact_id,
                                tx_out.value,
                                None,
                            ),
                        );

                        tracing::info!(
                            "DashPay payment received: {} duffs from contact {} to address {} (index {})",
                            tx_out.value,
                            contact_id.to_string(
                                dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58
                            ),
                            address,
                            address_index
                        );
                    }
                }
            }
        }
        if matches!(
            tx.special_transaction_payload,
            Some(AssetLockPayloadType(_))
        ) {
            self.received_asset_lock_finality(tx, islock, chain_locked_height)?;
        }
        Ok(wallet_outpoints)
    }

    /// Store the asset lock transaction in the database and update the wallet.
    pub(crate) fn received_asset_lock_finality(
        &self,
        tx: &Transaction,
        islock: Option<InstantLock>,
        chain_locked_height: Option<CoreBlockHeight>,
    ) -> Result<(), TaskError> {
        // Extract the asset lock payload from the transaction
        let Some(AssetLockPayloadType(payload)) = tx.special_transaction_payload.as_ref() else {
            return Ok(());
        };

        let proof = if let Some(islock) = islock.as_ref() {
            // Deserialize the InstantLock
            Some(AssetLockProof::Instant(InstantAssetLockProof::new(
                islock.clone(),
                tx.clone(),
                0,
            )))
        } else {
            chain_locked_height.map(|chain_locked_height| {
                AssetLockProof::Chain(ChainAssetLockProof {
                    core_chain_locked_height: chain_locked_height,
                    out_point: OutPoint::new(tx.txid(), 0),
                })
            })
        };

        // Identify the wallet associated with the transaction
        let wallets = self.wallets.read()?;
        for wallet_arc in wallets.values() {
            let wallet = wallet_arc.read()?;

            // Check if any of the addresses in the transaction outputs match the wallet's known addresses
            let matches_wallet = payload.credit_outputs.iter().any(|tx_out| {
                if let Ok(output_addr) = Address::from_script(&tx_out.script_pubkey, self.network) {
                    wallet.has_address(&output_addr)
                } else {
                    false
                }
            });

            if matches_wallet {
                // Calculate the total amount from the credit outputs
                let amount: u64 = payload
                    .credit_outputs
                    .iter()
                    .map(|tx_out| tx_out.value)
                    .sum();

                // Store the asset lock transaction in the database
                self.db.store_asset_lock_transaction(
                    tx,
                    amount,
                    islock.as_ref(),
                    &wallet.seed_hash(),
                    self.network,
                )?;

                // Register with PlatformWallet's AssetLockManager
                if let Some(pw) = wallet.platform_wallet.as_ref() {
                    pw.asset_locks().recover_asset_lock_blocking(
                        tx.clone(),
                        amount,
                        0,
                        platform_wallet::AssetLockFundingType::IdentityRegistration,
                        0,
                        dash_sdk::dpp::dashcore::OutPoint::new(tx.txid(), 0),
                        proof.clone(),
                    );
                }

                break; // Exit the loop after updating the relevant wallet
            }
        }

        Ok(())
    }
}

pub(crate) struct DapiTransactionInfo {
    pub is_chain_locked: bool,
    pub height: u32,
    pub confirmations: u32,
}

/// Query transaction info from DAPI. Works in both SPV and RPC modes
/// since DAPI (platform gRPC) is always available via the SDK.
pub(crate) async fn get_transaction_info(
    sdk: &Sdk,
    tx_id: &Txid,
) -> Result<DapiTransactionInfo, TaskError> {
    use dash_sdk::dapi_client::{DapiRequestExecutor, IntoInner, RequestSettings};
    use dash_sdk::dapi_grpc::core::v0::GetTransactionRequest;

    let response = sdk
        .execute(
            GetTransactionRequest {
                id: tx_id.to_string(),
            },
            RequestSettings::default(),
        )
        .await
        .into_inner()
        .map_err(|e| TaskError::PlatformFetchError {
            source: Box::new(dash_sdk::Error::DapiClientError(e)),
        })?;

    Ok(DapiTransactionInfo {
        is_chain_locked: response.is_chain_locked,
        height: response.height,
        confirmations: response.confirmations,
    })
}
