use super::AppContext;
use crate::backend_task::error::TaskError;
use dash_sdk::dpp::dashcore::transaction::special_transaction::TransactionPayload::AssetLockPayloadType;
use dash_sdk::dpp::dashcore::{Address, InstantLock, OutPoint, Transaction, TxOut};
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::InstantAssetLockProof;
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::chain::ChainAssetLockProof;
use dash_sdk::dpp::prelude::{AssetLockProof, CoreBlockHeight};

impl AppContext {
    /// Handle a finalized (InstantLock/ChainLock) transaction.
    ///
    /// Wallet-UTXO/balance bookkeeping is owned by the upstream wallet and the
    /// display-only `WalletSnapshot`. For asset-lock transactions this only
    /// resolves any `wait_for_asset_lock_proof` waiter blocked on the txid.
    /// The `Vec` return type is retained for existing call sites; it is
    /// always empty.
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

    /// Build the asset-lock proof for a finalized asset-lock transaction and
    /// hand it to any waiter parked on the txid. No DB or wallet state is
    /// written here — asset-lock ownership lives in the upstream
    /// `AssetLockManager`.
    pub(crate) fn received_asset_lock_finality(
        &self,
        tx: &Transaction,
        islock: Option<InstantLock>,
        chain_locked_height: Option<CoreBlockHeight>,
    ) -> Result<(), TaskError> {
        // Gate: only asset-lock transactions register finality here.
        let Some(AssetLockPayloadType(_)) = tx.special_transaction_payload.as_ref() else {
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

        // Resolve the asset-lock-proof waiter for this txid. Asset-lock
        // ownership and balances live in the upstream `AssetLockManager`;
        // DET only signals finality to any caller blocked in
        // `wait_for_asset_lock_proof`.
        let mut transactions = self.transactions_waiting_for_finality.lock()?;
        if let Some(asset_lock_proof) = transactions.get_mut(&tx.txid()) {
            *asset_lock_proof = proof;
        }

        Ok(())
    }
}

/// Asset-lock finality resolves the proof waiter without mutating any
/// `Wallet` state. Drives a chain-locked finality event through
/// `received_transaction_finality` and asserts the parked
/// `wait_for_asset_lock_proof` waiter is resolved while the legacy `utxos`
/// table receives zero writes. Wallet UTXO/balance bookkeeping lives in the
/// upstream wallet, not here.
#[cfg(test)]
mod path3_asset_lock_finality_no_wallet_mutation {
    use super::*;
    use crate::model::wallet::Wallet;
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
        // Force legacy wallet-family schema for tests — `initialize`
        // gates these out for truly-fresh installs post-T-DEV-01.
        db.create_tables(true).expect("create tables");
        db.set_default_version().expect("set version");

        let network = Network::Testnet;

        // A real HD wallet whose first known receive address funds the
        // asset-lock credit output.
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

        let app_kv = AppContext::open_app_kv(tmp.path()).expect("open app k/v");
        let secret_store = AppContext::open_secret_store(tmp.path()).expect("open secret store");
        let app_context = AppContext::new(
            tmp.path().to_path_buf(),
            network,
            db.clone(),
            Default::default(),
            Default::default(),
            egui::Context::default(),
            app_kv,
            secret_store,
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

        // Drive a chain-locked finality event.
        let out = app_context
            .received_transaction_finality(&tx, None, Some(900_001))
            .expect("finality processing");
        assert!(out.is_empty(), "slim path returns no wallet outpoints");

        let _ = (seed_hash, amount);

        // The proof waiter is resolved with a chain proof.
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

        // Core assertion: the finality path never writes the legacy `utxos`
        // table — wallet-UTXO bookkeeping lives in the upstream wallet.
        assert_eq!(
            utxos_row_count(&db),
            0,
            "finality path must not write the legacy utxos table"
        );
        assert!(
            db.get_utxos_by_address(&credit_addr.to_string(), &network.to_string())
                .expect("query credit-address utxos")
                .is_empty(),
            "no utxo persisted for the asset-lock credit address"
        );
    }
}
