use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::wallet::WalletId;
use dash_sdk::RequestSettings;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::platform::address_sync::AddressSyncConfig;
use std::sync::Arc;

impl AppContext {
    /// Sync platform address balances for the wallet.
    ///
    /// Delegates entirely to platform-wallet's
    /// [`PlatformAddressWallet::sync_balances`]. Address discovery,
    /// HD derivation, gap-limit extension, balance + nonce writes, and
    /// the incremental-sync watermark all flow through the
    /// [`PlatformAddressChangeSet`] → persister path. This task only
    /// reads back the persisted snapshot for the UI return value.
    pub(crate) async fn fetch_platform_address_balances(
        self: &Arc<Self>,
        wallet_id: WalletId,
    ) -> Result<BackendTaskSuccessResult, crate::backend_task::error::TaskError> {
        tracing::info!("Platform address sync start");
        let start_time = std::time::Instant::now();

        let platform_wallet = self
            .get_platform_wallet(&wallet_id)
            .ok_or(crate::backend_task::error::TaskError::WalletNotFound)?;

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

        let total_found: usize = per_account.values().map(|r| r.found.len()).sum();
        let total_absent: usize = per_account.values().map(|r| r.absent.len()).sum();

        // Read back the full snapshot from DB. The persister writes
        // balances + nonces for addresses that changed; unchanged
        // rows still have valid nonces. Returning only `found` would
        // cause the UI to lose nonce values for stable-balance
        // addresses (issue #652).
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
            "Platform address sync complete: duration={:?}, accounts={}, found={}, absent={}",
            start_time.elapsed(),
            per_account.len(),
            total_found,
            total_absent,
        );

        Ok(BackendTaskSuccessResult::PlatformAddressBalances {
            wallet_id,
            balances,
            network: self.network(),
        })
    }
}
