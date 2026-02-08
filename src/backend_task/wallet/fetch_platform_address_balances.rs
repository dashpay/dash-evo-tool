use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::wallet::PlatformSyncMode;
use crate::context::AppContext;
use crate::lock_helper::RwLockExt;
use crate::model::wallet::{
    DerivationPathHelpers, DerivationPathReference, DerivationPathType, Wallet,
    WalletAddressProvider, WalletSeedHash,
};
use dash_sdk::RequestSettings;
use dash_sdk::Sdk;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use dash_sdk::platform::address_sync::AddressSyncConfig;
use dash_sdk::platform::address_sync::AddressSyncResult;
use std::sync::{Arc, RwLock};

impl AppContext {
    pub(crate) async fn fetch_platform_address_balances(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        sync_mode: PlatformSyncMode,
    ) -> Result<BackendTaskSuccessResult, String> {
        // 6 days and 20 hours in seconds (to be safe before 7 days)
        const FULL_SYNC_INTERVAL_SECS: u64 = 6 * 24 * 60 * 60 + 20 * 60 * 60; // 590400 seconds

        tracing::info!("Platform address sync start (mode: {:?})", sync_mode);
        let start_time = std::time::Instant::now();

        let wallet_arc = {
            let wallets = self.wallets.read_or_recover();
            wallets
                .get(&seed_hash)
                .cloned()
                .ok_or_else(|| "Wallet not found".to_string())?
        };

        // Check last full sync time and terminal block from database
        let (last_full_sync, stored_checkpoint, last_terminal_block) = self
            .db
            .get_platform_sync_info(&seed_hash)
            .unwrap_or((0, 0, 0));

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Determine if we need a full sync based on mode
        let needs_full_sync = match sync_mode {
            PlatformSyncMode::ForceFull => true,
            PlatformSyncMode::TerminalOnly => {
                if stored_checkpoint == 0 {
                    return Err(
                        "Terminal-only sync requested but no checkpoint exists. Run a full sync first."
                            .to_string(),
                    );
                }
                false
            }
            PlatformSyncMode::Auto => {
                last_full_sync == 0
                    || stored_checkpoint == 0
                    || now.saturating_sub(last_full_sync) >= FULL_SYNC_INTERVAL_SECS
            }
        };

        // Create provider (requires wallet to be open for address derivation)
        let mut provider = {
            let wallet = wallet_arc.read().map_err(|e| e.to_string())?;
            match WalletAddressProvider::new(&wallet, self.network) {
                Ok(provider) => provider,
                Err(_) if !wallet.is_open() => {
                    return Err("Wallet is locked. Please unlock it first to refresh.".to_string());
                }
                Err(e) => return Err(e),
            }
        };

        // Sync using SDK's privacy-preserving method
        let sdk = {
            let guard = self.sdk.read().map_err(|e| e.to_string())?;
            guard.clone()
        };

        let (_checkpoint_height, highest_block_processed) = if needs_full_sync {
            tracing::info!(
                "Performing full platform address sync (last sync: {} seconds ago)",
                now.saturating_sub(last_full_sync)
            );

            // trunk state query is failing if tree is empty with internal error
            // this happens when we don't have any balances yet
            // this case most often happens for local network
            // so we do not ban addresses in case of failure
            // and return empty `AddressSyncResult`
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

            // Perform the base sync
            let base_start = std::time::Instant::now();
            let result = match sdk
                .sync_address_balances(&mut provider, config.clone())
                .await
            {
                Ok(res) => res,
                Err(e) if e.to_string().contains("empty tree") => {
                    tracing::debug!(
                        "Platform address balance tree is empty. Returning empty sync result."
                    );
                    AddressSyncResult::default()
                }
                Err(e) => return Err(format!("Failed to sync Platform addresses: {}", e)),
            };
            let base_duration = base_start.elapsed();

            tracing::info!(
                "Base sync complete: duration={:?}, found={}, absent={}, checkpoint={}",
                base_duration,
                result.found.len(),
                result.absent.len(),
                result.checkpoint_height
            );

            // Apply terminal updates and capture the highest block processed
            let terminal_start_height = result.checkpoint_height.max(last_terminal_block);
            let terminal_sync_start = std::time::Instant::now();
            let highest_block_processed = self
                .apply_recent_balance_changes(
                    &sdk,
                    &wallet_arc,
                    &mut provider,
                    terminal_start_height,
                )
                .await?;
            let terminal_sync_duration = terminal_sync_start.elapsed();
            tracing::info!(
                "Terminal balance updates complete: duration={:?}, start_height={}, end_height={}",
                terminal_sync_duration,
                terminal_start_height,
                highest_block_processed
            );

            tracing::info!(
                "Full sync complete: duration={:?}, found={}, absent={}, highest_index={:?}, checkpoint_height={}",
                start_time.elapsed(),
                result.found.len(),
                result.absent.len(),
                result.highest_found_index,
                result.checkpoint_height
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

            // Save the new full sync timestamp and checkpoint
            if let Err(e) =
                self.db
                    .set_platform_sync_info(&seed_hash, now, result.checkpoint_height)
            {
                tracing::warn!("Failed to save platform sync info: {}", e);
            }

            (result.checkpoint_height, highest_block_processed)
        } else {
            let terminal_only_start = std::time::Instant::now();
            tracing::info!(
                "Performing terminal-only platform address sync (last full sync: {} seconds ago, checkpoint={}, last_terminal_block={})",
                now.saturating_sub(last_full_sync),
                stored_checkpoint,
                last_terminal_block
            );

            // Pre-populate provider with LAST SYNCED balances (not current balances)
            // This prevents double-counting when proof-verified updates happened after last sync
            let mut pre_populated_count = 0;
            {
                let wallet = wallet_arc.read().map_err(|e| e.to_string())?;
                for (core_addr, platform_addr) in wallet.platform_addresses(self.network) {
                    if let Some(info) = wallet.get_platform_address_info(&core_addr) {
                        // Only pre-populate if we have a last_full_sync_balance
                        // (meaning this address was found in a previous full sync)
                        // This prevents double-counting AddToCredits after app restart
                        if let Some(full_sync_balance) = info.last_full_sync_balance {
                            let lookup_addr = platform_addr.to_address_with_network(self.network);
                            provider.update_balance(&lookup_addr, full_sync_balance);
                            pre_populated_count += 1;
                            tracing::debug!(
                                "Pre-populated balance for {}: {} (from last full sync)",
                                platform_addr.to_bech32m_string(self.network),
                                full_sync_balance
                            );
                        } else {
                            tracing::debug!(
                                "Skipping pre-population for {} (no last_full_sync_balance, needs full sync)",
                                platform_addr.to_bech32m_string(self.network)
                            );
                        }
                    }
                }
            }
            tracing::info!(
                "Terminal-only sync setup complete: duration={:?}, pre_populated={} addresses",
                terminal_only_start.elapsed(),
                pre_populated_count
            );

            // For terminal-only sync, fetch recent balance changes
            // Use the higher of checkpoint_height or last_terminal_block to avoid
            // re-applying changes we've already processed.
            let terminal_start_height = stored_checkpoint.max(last_terminal_block);
            let terminal_sync_start = std::time::Instant::now();
            let highest_block_processed = self
                .apply_recent_balance_changes(
                    &sdk,
                    &wallet_arc,
                    &mut provider,
                    terminal_start_height,
                )
                .await?;
            let terminal_sync_duration = terminal_sync_start.elapsed();
            tracing::info!(
                "Terminal balance updates complete: duration={:?}, start_height={}, end_height={}",
                terminal_sync_duration,
                terminal_start_height,
                highest_block_processed
            );

            (stored_checkpoint, highest_block_processed)
        };

        // Save the highest block we've processed to avoid re-applying the same changes
        if highest_block_processed > last_terminal_block
            && let Err(e) = self
                .db
                .set_last_terminal_block(&seed_hash, highest_block_processed)
        {
            tracing::warn!("Failed to save last terminal block: {}", e);
        }

        // Apply results to wallet and persist
        let balances = {
            let mut wallet = wallet_arc.write().map_err(|e| e.to_string())?;

            // Update wallet with synced balances (also updates last_full_sync_balance for next sync)
            provider.apply_results_to_wallet(&mut wallet);

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
                // Use the nonce from AddressFunds which comes directly from SDK sync
                // This is a sync operation, so update last_full_sync_balance
                if let Err(e) = self.db.set_platform_address_info(
                    &seed_hash,
                    address,
                    funds.balance,
                    funds.nonce,
                    &self.network,
                    true, // Sync operation - update last_full_sync_balance
                ) {
                    tracing::warn!("Failed to persist Platform address info: {}", e);
                }
            }

            // Return balances for result (use nonce from AddressFunds)
            provider
                .found_balances()
                .iter()
                .map(|(addr, funds)| (addr.clone(), (funds.balance, funds.nonce)))
                .collect()
        };

        let addresses_with_balance = provider.found_balances().len();
        let total_duration = start_time.elapsed();
        tracing::info!(
            "Platform address sync complete: total_duration={:?}, mode={:?}, addresses_with_balance={}",
            total_duration,
            sync_mode,
            addresses_with_balance
        );

        Ok(BackendTaskSuccessResult::PlatformAddressBalances {
            seed_hash,
            balances,
        })
    }

    /// Apply recent balance changes (terminal updates) to catch changes after a starting block.
    ///
    /// The trunk/branch sync provides balances as of a checkpoint (every ~10 minutes).
    /// This function fetches balance changes since the starting block to provide
    /// more up-to-date balances.
    ///
    /// Two queries are performed in sequence:
    /// 1. RecentCompactedAddressBalanceChanges - merged changes for ranges of blocks
    /// 2. RecentAddressBalanceChanges - individual per-block changes for most recent blocks
    ///
    /// Returns the highest block height processed, or an error if network requests failed.
    async fn apply_recent_balance_changes(
        &self,
        sdk: &Sdk,
        wallet_arc: &Arc<RwLock<Wallet>>,
        provider: &mut WalletAddressProvider,
        start_height: u64,
    ) -> Result<u64, String> {
        use dash_sdk::dpp::address_funds::PlatformAddress;
        use dash_sdk::dpp::balances::credits::{BlockAwareCreditOperation, CreditOperation};
        use dash_sdk::platform::{
            Fetch, RecentAddressBalanceChangesQuery, RecentCompactedAddressBalanceChangesQuery,
        };
        use dash_sdk::query_types::{
            RecentAddressBalanceChanges, RecentCompactedAddressBalanceChanges,
        };

        // The trunk/branch sync provides balances as of the checkpoint height.
        // We query for compacted changes starting from that start height,
        // then query recent non-compacted changes starting from where compacted ends.

        // Query from start_height + 1 because start_height was already processed
        // in the previous sync (last_terminal_block is the highest block we've seen)
        let query_from_height = start_height.saturating_add(1);

        tracing::debug!(
            "Fetching terminal balance updates from height {} (start_height={})",
            query_from_height,
            start_height
        );

        // Get the wallet's platform addresses to filter relevant changes
        let wallet_platform_addresses: std::collections::HashSet<PlatformAddress> = {
            let wallet = match wallet_arc.read() {
                Ok(w) => w,
                Err(e) => return Err(format!("Failed to read wallet: {}", e)),
            };
            wallet
                .platform_addresses(self.network)
                .into_iter()
                .map(|(_, platform_addr)| platform_addr)
                .collect()
        };

        let mut updates_applied = 0;
        let mut highest_block_seen = start_height;

        // Step 1: Fetch compacted balance changes (merged changes for ranges of blocks)
        // Start from query_from_height (start_height + 1) to get changes since the last sync
        let compacted_fetch_start = std::time::Instant::now();
        let compacted_query = RecentCompactedAddressBalanceChangesQuery::new(query_from_height);
        let compacted_result = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            RecentCompactedAddressBalanceChanges::fetch(sdk, compacted_query),
        )
        .await;
        let compacted_duration = compacted_fetch_start.elapsed();
        tracing::info!(
            "Compacted balance changes fetch: duration={:?}, from_height={}",
            compacted_duration,
            query_from_height
        );
        let compacted_result = match compacted_result {
            Ok(result) => result,
            Err(_) => {
                return Err("Compacted balance changes fetch timed out after 30s".to_string());
            }
        };
        let compacted_changes = match compacted_result {
            Ok(Some(changes)) => Some(changes),
            Ok(None) => None,
            Err(e) => {
                return Err(format!("Failed to fetch compacted balance changes: {}", e));
            }
        };
        if let Some(compacted_changes) = compacted_changes {
            for block_changes in compacted_changes.into_inner() {
                // Track the highest block height we've processed
                if block_changes.end_block_height > highest_block_seen {
                    highest_block_seen = block_changes.end_block_height;
                }

                for (platform_addr, credit_op) in block_changes.changes {
                    if wallet_platform_addresses.contains(&platform_addr) {
                        let core_addr = platform_addr.to_address_with_network(self.network);
                        let current_balance = provider
                            .found_balances()
                            .get(&core_addr)
                            .map(|funds| funds.balance)
                            .unwrap_or(0);

                        let new_balance = match credit_op {
                            BlockAwareCreditOperation::SetCredits(credits) => {
                                tracing::debug!(
                                    "Compacted SetCredits: {} = {}",
                                    platform_addr.to_bech32m_string(self.network),
                                    credits
                                );
                                credits
                            }
                            BlockAwareCreditOperation::AddToCreditsOperations(operations) => {
                                // Only apply credits from blocks at or after our query height
                                // (since we query from start_height + 1, all results should be valid)
                                let total_to_add: u64 = operations
                                    .iter()
                                    .filter(|(height, _)| **height >= query_from_height)
                                    .map(|(_, credits)| *credits)
                                    .sum();
                                tracing::debug!(
                                    "Compacted AddToCredits: {} current={} + add={} = {}",
                                    platform_addr.to_bech32m_string(self.network),
                                    current_balance,
                                    total_to_add,
                                    current_balance.saturating_add(total_to_add)
                                );
                                current_balance.saturating_add(total_to_add)
                            }
                        };

                        if new_balance != current_balance {
                            provider.update_balance(&core_addr, new_balance);
                            let addr_str = platform_addr.to_bech32m_string(self.network);
                            tracing::info!(
                                "Compacted update: {} balance {} -> {}",
                                addr_str,
                                current_balance,
                                new_balance
                            );
                            updates_applied += 1;
                        }
                    }
                }
            }
        }

        // Step 2: Fetch non-compacted balance changes (individual per-block changes)
        // Use the highest block height from compacted changes + 1 as the start
        let recent_fetch_start = std::time::Instant::now();
        let recent_query = RecentAddressBalanceChangesQuery::new(highest_block_seen + 1);
        let recent_result = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            RecentAddressBalanceChanges::fetch(sdk, recent_query),
        )
        .await;
        let recent_duration = recent_fetch_start.elapsed();
        tracing::info!(
            "Recent balance changes fetch: duration={:?}, from_height={}",
            recent_duration,
            highest_block_seen + 1
        );
        let recent_result = match recent_result {
            Ok(result) => result,
            Err(_) => {
                return Err("Recent balance changes fetch timed out after 30s".to_string());
            }
        };
        let recent_changes = match recent_result {
            Ok(Some(changes)) => Some(changes),
            Ok(None) => None,
            Err(e) => {
                return Err(format!("Failed to fetch recent balance changes: {}", e));
            }
        };
        if let Some(recent_changes) = recent_changes {
            for block_changes in recent_changes.into_inner() {
                // Track the block height from non-compacted changes
                if block_changes.block_height > highest_block_seen {
                    highest_block_seen = block_changes.block_height;
                }

                for (platform_addr, credit_op) in block_changes.changes {
                    if wallet_platform_addresses.contains(&platform_addr) {
                        let core_addr = platform_addr.to_address_with_network(self.network);
                        let current_balance = provider
                            .found_balances()
                            .get(&core_addr)
                            .map(|funds| funds.balance)
                            .unwrap_or(0);

                        let new_balance = match credit_op {
                            CreditOperation::SetCredits(credits) => {
                                tracing::debug!(
                                    "Recent SetCredits: {} = {}",
                                    platform_addr.to_bech32m_string(self.network),
                                    credits
                                );
                                credits
                            }
                            CreditOperation::AddToCredits(credits) => {
                                tracing::debug!(
                                    "Recent AddToCredits: {} current={} + add={} = {}",
                                    platform_addr.to_bech32m_string(self.network),
                                    current_balance,
                                    credits,
                                    current_balance.saturating_add(credits)
                                );
                                current_balance.saturating_add(credits)
                            }
                        };

                        if new_balance != current_balance {
                            provider.update_balance(&core_addr, new_balance);
                            let addr_str = platform_addr.to_bech32m_string(self.network);
                            tracing::info!(
                                "Recent update: {} balance {} -> {}",
                                addr_str,
                                current_balance,
                                new_balance
                            );
                            updates_applied += 1;
                        }
                    }
                }
            }
        }

        if updates_applied > 0 {
            tracing::info!(
                "Applied {} terminal balance updates from recent blocks (up to block {})",
                updates_applied,
                highest_block_seen
            );
        }

        Ok(highest_block_seen)
    }
}
