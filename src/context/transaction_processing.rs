use super::AppContext;
use crate::backend_task::error::TaskError;
use dash_sdk::Sdk;
use dash_sdk::dpp::dashcore::transaction::special_transaction::TransactionPayload::AssetLockPayloadType;
use dash_sdk::dpp::dashcore::{Address, InstantLock, OutPoint, Transaction, TxOut, Txid};
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::InstantAssetLockProof;
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::chain::ChainAssetLockProof;
use dash_sdk::dpp::prelude::{AssetLockProof, CoreBlockHeight};

impl AppContext {
    /// Handle a finalized (InstantLock/ChainLock) transaction.
    ///
    /// Wallet-UTXO/balance bookkeeping is owned by the upstream wallet and the
    /// display-only `WalletSnapshot`. This path only registers asset-lock
    /// finality (`store_asset_lock_transaction` + `unused_asset_locks`) for the
    /// ZMQ-fed asset-lock consumer. The `Vec` return type is retained for the
    /// ZMQ call sites; it is always empty.
    pub(crate) fn received_transaction_finality(
        &self,
        tx: &Transaction,
        islock: Option<InstantLock>,
        chain_locked_height: Option<CoreBlockHeight>,
    ) -> Result<Vec<(OutPoint, TxOut, Address)>, TaskError> {
        if matches!(
            tx.special_transaction_payload,
            Some(AssetLockPayloadType(_))
        ) {
            self.received_asset_lock_finality(tx, islock, chain_locked_height)?;
        }
        Ok(Vec::new())
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

        {
            let mut transactions = self.transactions_waiting_for_finality.lock()?;

            if let Some(asset_lock_proof) = transactions.get_mut(&tx.txid()) {
                *asset_lock_proof = proof.clone();
            }
        }

        // Identify the wallet associated with the transaction
        let wallets = self.wallets.read()?;
        for wallet_arc in wallets.values() {
            let mut wallet = wallet_arc.write()?;

            // Check if any of the addresses in the transaction outputs match the wallet's known addresses
            let matches_wallet = payload.credit_outputs.iter().any(|tx_out| {
                if let Ok(output_addr) = Address::from_script(&tx_out.script_pubkey, self.network) {
                    wallet.known_addresses.contains_key(&output_addr)
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

                let first = payload
                    .credit_outputs
                    .first()
                    .ok_or(TaskError::AssetLockNoCreditOutputs)?;

                let address = Address::from_script(&first.script_pubkey, self.network)
                    .map_err(|e| TaskError::AssetLockAddressDerivationFailed { source: e })?;

                // Add the asset lock to the wallet's unused_asset_locks
                wallet
                    .unused_asset_locks
                    .push((tx.clone(), address, amount, islock, proof));

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

/// QA-003 (release-blocking) — Path-3 asset-lock finality without any
/// `Wallet` mutation (I5).
///
/// P4a.5 slimmed `received_transaction_finality` to asset-lock-finality-only:
/// the `Wallet.utxos` / `address_balances` / legacy-`utxos`-table write
/// branches are deleted (the `Wallet` struct no longer even has `utxos` /
/// `address_balances` fields). The asset-lock detection + registration
/// branch (`store_asset_lock_transaction` + the finality-wait channel +
/// `unused_asset_locks`) is RETAINED. This lane drives a ZMQ-style
/// (chain-locked) finality event through `received_transaction_finality`
/// and proves the asset lock is detected and the finality-wait channel
/// resolves, while the legacy `utxos` table receives ZERO writes.
#[cfg(test)]
mod path3_asset_lock_finality_no_wallet_mutation {
    use super::*;
    use crate::model::wallet::Wallet;
    use dash_sdk::dpp::dashcore::hashes::Hash;
    use dash_sdk::dpp::dashcore::transaction::special_transaction::TransactionPayload;
    use dash_sdk::dpp::dashcore::transaction::special_transaction::asset_lock::AssetLockPayload;
    use dash_sdk::dpp::dashcore::{Network, Transaction, TxOut};
    use std::sync::Arc;

    fn ensure_test_env(data_dir: &std::path::Path) {
        crate::app_dir::ensure_env_file(data_dir);
    }

    fn utxos_row_count(db: &crate::database::Database) -> i64 {
        let conn = db.shared_connection();
        let conn = conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM utxos", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn chain_locked_asset_lock_finality_registers_without_touching_utxos() {
        let tmp = tempfile::tempdir().expect("tempdir");
        ensure_test_env(tmp.path());
        let db_file = tmp.path().join("data.db");
        let db = Arc::new(crate::database::Database::new(&db_file).expect("db"));
        db.initialize(&db_file).expect("init");

        let network = Network::Testnet;

        // A real HD wallet; its first known receive address will be the
        // asset-lock credit output so `received_asset_lock_finality` matches
        // it. Persist a legacy `wallet` row so the FK on
        // `asset_lock_transaction.wallet` is satisfied.
        let wallet =
            Wallet::new_from_seed([42u8; 64], network, Some("p3".into()), None).expect("wallet");
        let seed_hash = wallet.seed_hash();
        let credit_addr = wallet
            .known_addresses
            .keys()
            .next()
            .expect("wallet has a first receive address")
            .clone();
        {
            let epk = wallet.master_bip44_ecdsa_extended_public_key.encode();
            db.execute(
                "INSERT INTO wallet (seed_hash, encrypted_seed, salt, nonce, \
                 master_ecdsa_bip44_account_0_epk, alias, is_main, uses_password, \
                 network) VALUES (?1, ?2, ?3, ?4, ?5, 'p3', 1, 0, ?6)",
                rusqlite::params![
                    seed_hash.as_slice(),
                    vec![0u8; 64],
                    vec![0u8; 16],
                    vec![0u8; 12],
                    epk.to_vec(),
                    network.to_string(),
                ],
            )
            .expect("seed legacy wallet row");
        }

        let app_context = AppContext::new(
            tmp.path().to_path_buf(),
            network,
            db.clone(),
            None,
            Default::default(),
            Default::default(),
            egui::Context::default(),
        )
        .expect("AppContext");

        // Register the wallet in the context so the finality path can match
        // the credit output to a wallet.
        app_context
            .wallets
            .write()
            .unwrap()
            .insert(seed_hash, Arc::new(std::sync::RwLock::new(wallet)));

        // An asset-lock tx whose single credit output pays the wallet's
        // first known address.
        let amount = 250_000u64;
        let tx = Transaction {
            version: 3,
            lock_time: 0,
            input: vec![],
            output: vec![],
            special_transaction_payload: Some(TransactionPayload::AssetLockPayloadType(
                AssetLockPayload {
                    version: 1,
                    credit_outputs: vec![TxOut {
                        value: amount,
                        script_pubkey: credit_addr.script_pubkey(),
                    }],
                },
            )),
        };
        let txid = tx.txid();

        // The waiter (e.g. `wait_for_asset_lock_proof`) registered its txid
        // before broadcast; finality must resolve it.
        app_context
            .transactions_waiting_for_finality
            .lock()
            .unwrap()
            .insert(txid, None);

        assert_eq!(utxos_row_count(&db), 0, "precondition: utxos empty");

        // Drive the ZMQ-style chain-locked finality event.
        let out = app_context
            .received_transaction_finality(&tx, None, Some(900_001))
            .expect("finality processing");
        assert!(out.is_empty(), "slim path returns no wallet outpoints");

        // Asset-lock DETECTED + registered (durable record).
        let stored = db
            .get_asset_lock_transaction(&txid.to_byte_array())
            .expect("query asset lock")
            .expect("asset lock must be stored on finality");
        assert_eq!(stored.1, amount, "stored amount matches credit output");
        assert_eq!(stored.3, seed_hash, "stored against the owning wallet");

        // Finality-wait channel RESOLVED with a chain proof.
        let proof = app_context
            .transactions_waiting_for_finality
            .lock()
            .unwrap()
            .get(&txid)
            .cloned()
            .flatten();
        assert!(
            proof.is_some(),
            "wait_for_asset_lock_proof's channel must be resolved on finality"
        );

        // Retained registration branch populated the wallet's
        // `unused_asset_locks` (the asset-lock consumer's source).
        {
            let wallets = app_context.wallets.read().unwrap();
            let w = wallets.get(&seed_hash).unwrap().read().unwrap();
            assert_eq!(
                w.unused_asset_locks.len(),
                1,
                "asset lock registered into unused_asset_locks"
            );
        }

        // I5 core assertion: NO write to the legacy `utxos` table — the
        // slim path never touches wallet-UTXO bookkeeping. (`Wallet.utxos`
        // / `address_balances` no longer exist as fields, so the only
        // observable legacy-UTXO surface is this table.)
        assert_eq!(
            utxos_row_count(&db),
            0,
            "Path-3 slim must not write the legacy utxos table"
        );
        assert!(
            db.get_utxos_by_address(&credit_addr.to_string(), &network.to_string())
                .expect("query credit-address utxos")
                .is_empty(),
            "no utxo persisted for the asset-lock credit address"
        );
    }
}
