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

        let wallet_arc = self.wallet_arc(&seed_hash)?;

        // Create provider. Address derivation needs the DIP-17 account-level
        // xpub, which is derived once from the HD seed fetched just-in-time
        // through the chokepoint. The seed is borrowed for that single
        // derivation inside the closure and zeroizes on return — the provider
        // then derives every gap-limit child from the public xpub alone.
        //
        // This explicit refresh does a full tree scan: with no DET-side cursor
        // it always re-derives from scratch and returns every funded address.
        // Steady-state freshness comes from the coordinator's background pass.
        let network = self.network;
        let backend = self.wallet_backend()?;
        let mut provider = backend
            .secret_access()
            .with_secret(
                &crate::wallet_backend::SecretScope::HdSeed { seed_hash },
                |plaintext| {
                    let seed = plaintext
                        .expose_hd_seed()
                        .ok_or(crate::backend_task::error::TaskError::WalletLocked)?;
                    // Backfill the platform-payment account xpub while the seed
                    // is borrowed, so later seedless coordinator pushes can
                    // reconcile addresses for signing. No-op once cached.
                    wallet_arc
                        .write()?
                        .ensure_platform_payment_account_xpub(seed, network);
                    let wallet = wallet_arc.read()?;
                    WalletAddressProvider::new(&wallet, network, seed).map_err(|source| {
                        crate::backend_task::error::TaskError::WalletAddressProviderSetupFailed {
                            source,
                        }
                    })
                },
            )
            .await?;

        // Sync using SDK's privacy-preserving method (handles both full and incremental)
        let sdk = backend.sdk().clone();

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

        let result = match sdk.sync_address_balances(&mut provider, config, None).await {
            Ok(res) => res,
            // A never-funded wallet has no platform-balance tree to prove
            // against, so the proof layer reports an empty tree. That is the
            // expected first-sync state, not a failure — treat it as an empty
            // result. Matched structurally against the typed proof variants
            // (see `is_empty_tree_proof`); the leaf marker is the only string
            // the upstream proof error exposes for this case.
            Err(e) if crate::backend_task::error::is_empty_tree_proof(&e) => {
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

        // Per-address balances/nonces are intentionally not logged: the summary
        // line above carries the aggregate counts, and per-address financial
        // detail does not belong in plaintext logs at the default level.

        // Apply results to the in-memory wallet. Persistence is the upstream
        // coordinator's job: it owns the `platform_addresses` rows and re-pushes
        // them on its next pass, so DET keeps no parallel at-rest copy.
        let balances = {
            let mut wallet = wallet_arc.write()?;

            provider.apply_results_to_wallet(&mut wallet);

            // Return the wallet's complete platform_address_info, not just
            // found_balances, so a wallet whose balances were already current
            // keeps its full per-address set with valid nonces (issue #652).
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
