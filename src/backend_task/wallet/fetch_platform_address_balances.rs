use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::wallet::{
    DerivationPathHelpers, DerivationPathReference, DerivationPathType, WalletAddressProvider,
    WalletId,
};
use dash_sdk::RequestSettings;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use dash_sdk::platform::address_sync::AddressSyncConfig;
use dash_sdk::platform::address_sync::AddressSyncResult;
use std::sync::Arc;

impl AppContext {
    pub(crate) async fn fetch_platform_address_balances(
        self: &Arc<Self>,
        seed_hash: WalletId,
    ) -> Result<BackendTaskSuccessResult, crate::backend_task::error::TaskError> {
        tracing::info!("Platform address sync start");
        let start_time = std::time::Instant::now();

        // Validate via platform wallet bridge (establishes the new lookup path).
        // The platform wallet is not yet used for address derivation — the old
        // Wallet model's WalletAddressProvider handles that. This will be migrated
        // once PlatformAddressWallet provides equivalent functionality.
        let _platform_wallet = self.get_platform_wallet(&seed_hash);

        let wallet_arc = {
            let wallets = self.wallets.read()?;
            wallets
                .get(&seed_hash)
                .cloned()
                .ok_or(crate::backend_task::error::TaskError::WalletNotFound)?
        };

        // Get last sync timestamp from database
        let (last_sync_timestamp, last_sync_height) =
            self.db.get_platform_sync_info(&seed_hash).unwrap_or((0, 0));

        // Load stored platform address info from DB for incremental sync
        let stored_info = self
            .db
            .get_all_platform_address_info(&seed_hash, &self.network)
            .unwrap_or_default();

        // Create provider (requires wallet to be open for address derivation)
        let mut provider = {
            let wallet = wallet_arc.read()?;
            match WalletAddressProvider::new(&wallet, self.network) {
                Ok(provider) => {
                    provider.with_stored_state(&stored_info, self.network, last_sync_height)
                }
                Err(_) if !wallet.is_open() => {
                    return Err(crate::backend_task::error::TaskError::WalletLocked);
                }
                Err(e) => {
                    return Err(
                        crate::backend_task::error::TaskError::WalletAddressProviderSetupFailed {
                            detail: e,
                        },
                    );
                }
            }
        };

        // Sync using SDK's privacy-preserving method (handles both full and incremental)
        let sdk = self.sdk.load().as_ref().clone();

        let config = if sdk.network == Network::Regtest {
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

        let last_ts = if last_sync_timestamp > 0 {
            Some(last_sync_timestamp)
        } else {
            None
        };

        let result = match sdk
            .sync_address_balances(&mut provider, config, last_ts)
            .await
        {
            Ok(res) => res,
            // TODO: Replace with structural match when the SDK exposes a typed
            // variant for empty-tree proof responses.
            Err(e) if e.to_string().contains("empty tree") => {
                tracing::debug!(
                    "Platform address balance tree is empty. Returning empty sync result."
                );
                AddressSyncResult::default()
            }
            Err(e) => return Err(crate::backend_task::error::TaskError::from(e)),
        };

        tracing::info!(
            "Sync complete: duration={:?}, found={}, absent={}, highest_index={:?}, checkpoint={}, new_sync_height={}, new_sync_timestamp={}",
            start_time.elapsed(),
            result.found.len(),
            result.absent.len(),
            result.highest_found_index,
            result.checkpoint_height,
            result.new_sync_height,
            result.new_sync_timestamp,
        );

        // Log the found balances from provider
        for (addr, funds) in provider.found_balances() {
            use dash_sdk::dpp::address_funds::PlatformAddress;
            let platform_addr_str = PlatformAddress::try_from(addr.clone())
                .map(|p| p.to_bech32m_string(self.network))
                .unwrap_or_else(|_| addr.to_string());
            tracing::info!(
                "Sync found address: {} with balance: {}, nonce: {}",
                platform_addr_str,
                funds.balance,
                funds.nonce
            );
        }

        // Persist sync state
        if let Err(e) = self.db.set_platform_sync_info(
            &seed_hash,
            result.new_sync_timestamp,
            result.new_sync_height,
        ) {
            tracing::warn!("Failed to save platform sync info: {}", e);
        }

        // Persist addresses and balances to database
        for (index, (address, funds)) in provider.found_balances_with_indices() {
            // Persist the address to wallet_addresses table if not already there
            let derivation_path = DerivationPath::platform_payment_path(
                self.network,
                0, // account
                0, // key_class
                index,
            );
            if let Err(e) = self.db.add_address_if_not_exists(
                &seed_hash,
                address,
                &self.network,
                &derivation_path,
                DerivationPathReference::PlatformPayment,
                DerivationPathType::CLEAR_FUNDS,
                None,
            ) {
                tracing::warn!("Failed to persist Platform address: {}", e);
            }

            // Persist balance to platform_address_balances table
            if let Err(e) = self.db.set_platform_address_info(
                &seed_hash,
                address,
                funds.balance,
                funds.nonce,
                &self.network,
            ) {
                tracing::warn!("Failed to persist Platform address info: {}", e);
            }
        }

        // Return the complete platform_address_info from DB, not just
        // found_balances.  The SDK's incremental sync only reports addresses
        // whose balance changed; unchanged addresses are absent from
        // found_balances but still have valid nonces in the DB.
        // Returning only found_balances would cause the UI to lose nonce
        // values for stable-balance addresses (issue #652).
        let balances = self
            .db
            .get_all_platform_address_info(&seed_hash, &self.network)
            .unwrap_or_default()
            .into_iter()
            .map(|(addr, balance, nonce)| (addr, (balance, nonce)))
            .collect();

        let addresses_with_balance = provider.found_balances().len();
        tracing::info!(
            "Platform address sync complete: total_duration={:?}, addresses_with_balance={}",
            start_time.elapsed(),
            addresses_with_balance
        );

        Ok(BackendTaskSuccessResult::PlatformAddressBalances {
            seed_hash,
            balances,
        })
    }
}
