//! Fail closed when Core's live funding set contradicts confirmed persisted spends.

use std::collections::BTreeSet;

use dash_sdk::dpp::dashcore::{OutPoint, Txid, hashes::Hash};
use platform_wallet::changeset::PlatformWalletPersistence;
use platform_wallet_storage::WalletStorageError;

use super::{WalletBackend, WalletSeedHash};
use crate::backend_task::error::{TaskError, WalletTransactionHistoryError};

impl WalletBackend {
    /// This is a wallet consistency check, not proof that unknown inputs are unspent.
    pub(super) async fn validate_core_spend_history(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<(), TaskError> {
        let wallet = self.resolve_wallet(seed_hash).await?;
        let wallet_id = wallet.wallet_id();
        let candidates: BTreeSet<OutPoint> = {
            let manager = wallet.wallet_manager().read().await;
            let info = manager
                .get_wallet_info(&wallet_id)
                .ok_or(TaskError::WalletStateInconsistent)?;
            info.core_wallet
                .accounts
                .all_funding_accounts()
                .into_iter()
                .flat_map(|account| account.utxos.keys().copied())
                .collect()
        };
        if candidates.is_empty() {
            return Ok(());
        }
        let backend = self.clone();
        tokio::task::spawn_blocking(move || backend.check_persisted_spends(wallet_id, &candidates))
            .await
            .map_err(|source| TaskError::WalletSpendHistoryWorker { source })?
    }

    fn check_persisted_spends(
        &self,
        wallet_id: [u8; 32],
        candidates: &BTreeSet<OutPoint>,
    ) -> Result<(), TaskError> {
        let sqlite_error = |source| TaskError::WalletSpendHistoryCheck {
            source: WalletTransactionHistoryError::Persistence {
                source: WalletStorageError::Sqlite(source).into(),
            },
        };
        let connection = rusqlite::Connection::open_with_flags(
            &self.inner.wallet_database_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(sqlite_error)?;
        let mut query = connection
            .prepare(
                "SELECT txid FROM core_transactions WHERE wallet_id = ?1 AND height IS NOT NULL",
            )
            .map_err(sqlite_error)?;
        let rows = query
            .query_map([wallet_id.as_slice()], |row| row.get::<_, Vec<u8>>(0))
            .map_err(sqlite_error)?;
        let mut txids = Vec::new();
        for row in rows {
            let bytes = row.map_err(sqlite_error)?;
            let txid = Txid::from_slice(&bytes).map_err(|source| {
                sqlite_error(rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Blob,
                    Box::new(source),
                ))
            })?;
            txids.push(txid);
        }
        drop(query);
        drop(connection);
        for txid in txids {
            let record = self
                .inner
                .wallet_persister
                .get_core_tx_record(wallet_id, &txid)
                .map_err(|source| TaskError::WalletSpendHistoryCheck {
                    source: WalletTransactionHistoryError::Persistence { source },
                })?
                .ok_or(TaskError::WalletSpendHistoryCheck {
                    source: WalletTransactionHistoryError::RecordMissing { txid },
                })?;
            if let Some(input) = record
                .transaction
                .input
                .iter()
                .find(|input| candidates.contains(&input.previous_output))
            {
                return Err(TaskError::WalletConfirmedInputConflict {
                    outpoint: input.previous_output,
                    confirmed_txid: txid,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::TaskResult, utils::egui_mpsc::SenderAsync};
    use dash_sdk::dpp::{
        dashcore::{BlockHash, Network, ScriptBuf, Transaction, TxIn, TxOut},
        key_wallet::{
            Utxo,
            account::{AccountType, StandardAccountType},
            managed_account::transaction_record::{TransactionDirection, TransactionRecord},
            transaction_checking::{
                BlockInfo, TransactionContext, transaction_router::TransactionType,
            },
        },
    };

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spend_history_blocks_creation_before_signing_and_survives_history_reset() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(tmp.path());
        let (sender, _receiver) = tokio::sync::mpsc::channel::<TaskResult>(16);
        ctx.ensure_wallet_backend(SenderAsync::new(sender, ctx.egui_ctx().clone()))
            .await
            .unwrap();
        let backend = ctx.wallet_backend().unwrap();
        let seed = [51; 64];
        let model = crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
            .unwrap();
        let seed_hash = model.seed_hash();
        backend
            .register_wallet_from_seed(&seed_hash, &seed, None)
            .await
            .unwrap();
        let wallet = backend.resolve_wallet(&seed_hash).await.unwrap();
        let wallet_id = wallet.wallet_id();
        let outpoint = OutPoint::new(Txid::from_byte_array([7; 32]), 1);
        let funding_address = {
            let mut wm = wallet.wallet_manager().write().await;
            let (w, info) = wm.get_wallet_mut_and_info_mut(&wallet_id).unwrap();
            let account = w.get_bip44_account(0).unwrap();
            let managed = info
                .core_wallet
                .accounts
                .standard_bip44_accounts
                .get_mut(&0)
                .unwrap();
            let address = managed
                .next_receive_address(Some(&account.account_xpub), true)
                .unwrap();
            let mut utxo = Utxo::new(
                outpoint,
                TxOut {
                    value: 100_000,
                    script_pubkey: address.script_pubkey(),
                },
                address.clone(),
                1,
                false,
            );
            utxo.is_confirmed = true;
            managed.utxos.insert(outpoint, utxo);
            address
        };
        let winner = TransactionRecord::new(
            Transaction {
                version: 1,
                lock_time: 0,
                input: vec![TxIn {
                    previous_output: outpoint,
                    ..Default::default()
                }],
                output: vec![TxOut {
                    value: 99_000,
                    script_pubkey: ScriptBuf::new(),
                }],
                special_transaction_payload: None,
            },
            AccountType::Standard {
                index: 0,
                standard_account_type: StandardAccountType::BIP44Account,
            },
            TransactionContext::InBlock(BlockInfo::new(
                10,
                BlockHash::from_byte_array([8; 32]),
                100,
            )),
            TransactionType::Standard,
            TransactionDirection::Outgoing,
            vec![],
            vec![],
            -100_000,
        );
        let blob = bincode::serde::encode_to_vec(&winner, bincode::config::standard()).unwrap();
        let conn = rusqlite::Connection::open(&backend.inner.wallet_database_path).unwrap();
        conn.execute("INSERT INTO core_transactions (wallet_id,txid,height,finalized,record_blob) VALUES (?1,?2,10,1,?3)",
            rusqlite::params![wallet_id.as_slice(), winner.txid.to_byte_array().as_slice(), &blob]).unwrap();
        backend
            .check_persisted_spends([42; 32], &BTreeSet::from([outpoint]))
            .unwrap();
        backend
            .check_persisted_spends(
                wallet_id,
                &BTreeSet::from([OutPoint::new(outpoint.txid, outpoint.vout + 1)]),
            )
            .unwrap();
        let check = backend
            .send_payment(&seed_hash, vec![(funding_address, 1000)])
            .await;
        assert!(
            matches!(&check, Err(TaskError::WalletConfirmedInputConflict { outpoint: actual, confirmed_txid }) if *actual == outpoint && *confirmed_txid == winner.txid),
            "Unexpected payment result: {check:?}"
        );
        backend.request_full_resync(seed_hash).await.unwrap();
        assert!(matches!(
            backend.validate_core_spend_history(&seed_hash).await,
            Err(TaskError::WalletConfirmedInputConflict { .. })
        ));
        let wm = wallet.wallet_manager().read().await;
        assert!(
            wm.get_wallet_info(&wallet_id)
                .unwrap()
                .core_wallet
                .accounts
                .standard_bip44_accounts[&0]
                .utxos
                .contains_key(&outpoint)
        );
        drop(wm);
        conn.execute(
            "UPDATE core_transactions SET height=NULL,finalized=0 WHERE wallet_id=?1",
            [wallet_id.as_slice()],
        )
        .unwrap();
        backend
            .validate_core_spend_history(&seed_hash)
            .await
            .unwrap();
        conn.execute(
            "UPDATE core_transactions SET height=10,record_blob=X'00' WHERE wallet_id=?1",
            [wallet_id.as_slice()],
        )
        .unwrap();
        assert!(matches!(
            backend.validate_core_spend_history(&seed_hash).await,
            Err(TaskError::WalletSpendHistoryCheck { .. })
        ));
    }
}
