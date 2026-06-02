use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::wallet::{WalletAddressProvider, WalletSeedHash};
use dash_sdk::RequestSettings;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::platform::address_sync::AddressSyncConfig;
use dash_sdk::platform::address_sync::AddressSyncResult;
use std::sync::Arc;

impl AppContext {
    pub(crate) async fn fetch_platform_address_balances(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, crate::backend_task::error::TaskError> {
        tracing::info!("Platform address sync start");
        let start_time = std::time::Instant::now();

        let wallet_arc = {
            let wallets = self.wallets.read()?;
            wallets
                .get(&seed_hash)
                .cloned()
                .ok_or(crate::backend_task::error::TaskError::WalletNotFound)?
        };

        // Get last sync cursor from per-wallet k/v
        let (last_sync_timestamp, last_sync_height) =
            self.get_platform_sync_info(&seed_hash).unwrap_or((0, 0));

        // Create provider. Address derivation needs the DIP-17 account-level
        // xpub, which is derived once from the HD seed fetched just-in-time
        // through the chokepoint. The seed is borrowed for that single
        // derivation inside the closure and zeroizes on return — the provider
        // then derives every gap-limit child from the public xpub alone.
        let network = self.network;
        let backend = self.wallet_backend()?;
        let mut provider = backend
            .secret_access()
            .with_secret(
                &crate::wallet_backend::SecretScope::HdSeed { seed_hash },
                |plaintext| {
                    let seed = plaintext.expose_hd_seed().ok_or(
                        crate::backend_task::error::TaskError::ContactWalletSeedUnavailable,
                    )?;
                    let wallet = wallet_arc.read()?;
                    let provider = WalletAddressProvider::new(&wallet, network, seed).map_err(
                        |detail| {
                            crate::backend_task::error::TaskError::WalletAddressProviderSetupFailed {
                                detail,
                            }
                        },
                    )?;
                    Ok(provider.with_stored_state(&wallet, network, last_sync_height))
                },
            )
            .await?;

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
            "Sync complete: duration={:?}, found={}, absent={}, checkpoint={}, new_sync_height={}, new_sync_timestamp={}",
            start_time.elapsed(),
            result.found.len(),
            result.absent.len(),
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

        // Persist sync cursor to per-wallet k/v
        if let Err(e) = self.set_platform_sync_info(
            &seed_hash,
            result.new_sync_timestamp,
            result.new_sync_height,
        ) {
            tracing::warn!("Failed to save platform sync info: {}", e);
        }

        // Apply results to wallet and persist
        let balances = {
            let mut wallet = wallet_arc.write()?;

            // Update wallet with synced balances
            provider.apply_results_to_wallet(&mut wallet);

            // Persist platform-address balances to the per-wallet k/v.
            // T-W-01: the legacy `wallet_addresses` write that used to
            // sit alongside this loop is removed — no production read
            // path consumes it; the in-memory wallet maps plus the
            // platform-address-info k/v are the runtime source of truth.
            for (_index, (address, funds)) in provider.found_balances_with_indices() {
                if let Err(e) =
                    self.set_platform_address_info(&seed_hash, address, funds.balance, funds.nonce)
                {
                    tracing::warn!("Failed to persist Platform address info: {}", e);
                }
            }

            // Return the wallet's complete platform_address_info, not just
            // found_balances.  The SDK's incremental sync only reports addresses
            // whose balance changed; unchanged addresses are absent from
            // found_balances but still have valid nonces in the wallet.
            // Returning only found_balances would cause the UI to lose nonce
            // values for stable-balance addresses (issue #652).
            wallet
                .platform_address_info
                .iter()
                .filter_map(
                    |(addr, info)| match PlatformAddress::try_from(addr.clone()) {
                        Ok(pa) => Some((pa, (info.balance, info.nonce))),
                        Err(e) => {
                            tracing::warn!(
                                "Skipping platform address that could not be re-encoded: {e}"
                            );
                            None
                        }
                    },
                )
                .collect()
        };

        let addresses_with_balance = provider.found_balances().len();
        tracing::info!(
            "Platform address sync complete: total_duration={:?}, addresses_with_balance={}",
            start_time.elapsed(),
            addresses_with_balance
        );

        Ok(BackendTaskSuccessResult::PlatformAddressBalances {
            seed_hash,
            balances,
            network: self.network(),
        })
    }
}
