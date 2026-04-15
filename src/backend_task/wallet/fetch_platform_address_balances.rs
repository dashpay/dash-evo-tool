use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::wallet::{
    DerivationPathHelpers, DerivationPathReference, DerivationPathType, WalletId,
};
use dash_sdk::RequestSettings;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use dash_sdk::platform::address_sync::AddressSyncConfig;
use std::sync::Arc;

impl AppContext {
    /// Sync platform address balances for the wallet.
    ///
    /// Delegates address discovery, derivation and balance writes to
    /// platform-wallet's [`PlatformAddressWallet::sync_balances`]. The
    /// per-address `(balance, nonce)` snapshots are persisted via the
    /// wallet's [`PlatformWalletPersistence`] implementation — evo-tool's
    /// `SqliteWalletPersister` writes them to the
    /// `platform_address_balances` table in a single transaction.
    ///
    /// This task is still responsible for the evo-tool-only rows that
    /// aren't part of the changeset:
    /// - `wallet_addresses` — HD derivation path metadata keyed by
    ///   derived address, needed so the UI can show which local wallet
    ///   owns each platform address.
    /// - `platform_sync_info` — the `(timestamp, height)` watermark used
    ///   to drive incremental sync on the next invocation.
    pub(crate) async fn fetch_platform_address_balances(
        self: &Arc<Self>,
        wallet_id: WalletId,
    ) -> Result<BackendTaskSuccessResult, crate::backend_task::error::TaskError> {
        tracing::info!("Platform address sync start");
        let start_time = std::time::Instant::now();

        let platform_wallet = self
            .get_platform_wallet(&wallet_id)
            .ok_or(crate::backend_task::error::TaskError::WalletNotFound)?;

        // Regtest needs `ban_failed_address = false` — transient DAPI
        // failures are expected in local setups.
        let config = if self.network == Network::Regtest {
            Some(AddressSyncConfig {
                request_settings: RequestSettings {
                    ban_failed_address: Some(false),
                    ..Default::default()
                },
                ..Default::default()
            })
        } else {
            None
        };

        let per_account = platform_wallet
            .platform()
            .sync_balances(config)
            .await
            .map_err(|e| crate::backend_task::error::TaskError::PlatformWallet {
                source: Box::new(e),
            })?;

        // Aggregate across accounts: evo-tool currently persists derivation
        // path metadata for account 0 only (the primary HD platform payment
        // account). Extending to multiple accounts requires schema work on
        // `wallet_addresses` which is out of scope here.
        let mut total_found = 0usize;
        let mut total_absent = 0usize;
        let mut max_new_sync_height = 0u64;
        let mut max_new_sync_timestamp = 0u64;

        for (account_index, result) in &per_account {
            total_found += result.found.len();
            total_absent += result.absent.len();
            max_new_sync_height = max_new_sync_height.max(result.new_sync_height);
            max_new_sync_timestamp = max_new_sync_timestamp.max(result.new_sync_timestamp);

            for ((index, address), funds) in &result.found {
                let derivation_path = DerivationPath::platform_payment_path(
                    self.network,
                    *account_index,
                    0, // key_class
                    *index,
                );
                let core_address = address.to_address_with_network(self.network);
                if let Err(e) = self.db.add_address_if_not_exists(
                    &wallet_id,
                    &core_address,
                    &self.network,
                    &derivation_path,
                    DerivationPathReference::PlatformPayment,
                    DerivationPathType::CLEAR_FUNDS,
                    None,
                ) {
                    tracing::warn!("Failed to persist Platform address metadata: {}", e);
                }
                tracing::info!(
                    "Sync found address: {} with balance: {}, nonce: {}",
                    address.to_bech32m_string(self.network),
                    funds.balance,
                    funds.nonce
                );
            }
        }

        // Persist the sync watermark — the max across accounts serves
        // as the "earliest point at which no account is ahead".
        if let Err(e) =
            self.db
                .set_platform_sync_info(&wallet_id, max_new_sync_timestamp, max_new_sync_height)
        {
            tracing::warn!("Failed to save platform sync info: {}", e);
        }

        // Return the complete platform_address_info from DB. The
        // persister writes balances + nonces for addresses that changed;
        // unchanged rows still have valid nonces. Returning only
        // `found` would cause the UI to lose nonce values for
        // stable-balance addresses (issue #652).
        let balances = self
            .db
            .get_all_platform_address_info(&wallet_id, &self.network)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(addr, balance, nonce)| {
                PlatformAddress::try_from(addr)
                    .ok()
                    .map(|pa| (pa, (balance, nonce)))
            })
            .collect();

        tracing::info!(
            "Platform address sync complete: total_duration={:?}, \
             accounts={}, found={}, absent={}, new_sync_height={}, new_sync_timestamp={}",
            start_time.elapsed(),
            per_account.len(),
            total_found,
            total_absent,
            max_new_sync_height,
            max_new_sync_timestamp,
        );

        Ok(BackendTaskSuccessResult::PlatformAddressBalances {
            wallet_id,
            balances,
            network: self.network(),
        })
    }
}
