//! Persisted-wallet load seam (G2).
//!
//! `PersistedWalletLoader` decouples "which wallets to register at startup"
//! from `WalletBackend`. Today the only impl is [`SeedReregistrationLoader`],
//! which re-derives each wallet from DET's retained encrypted-seed store.
//! When upstream `Wallet::from_persisted` + `persister.load()` populate
//! `ClientStartState.wallets`, an `UpstreamFromPersisted` impl drops in via a
//! one-line construction swap with zero blast radius (see
//! `docs/ai-design/2026-05-18-platform-wallet-migration/g2-mock-boundary.md`).

use std::sync::Arc;

use dash_sdk::dpp::dashcore::Network as CoreNetwork;
use dash_sdk::dpp::key_wallet::Network as WalletNetwork;
use zeroize::Zeroizing;

use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;

/// A DET-opaque descriptor of one wallet to register with the wallet backend.
///
/// Carries the in-memory seed (zeroized on drop — never persisted; the
/// secret boundary stays in DET's seed store) plus the identifying metadata
/// the backend needs. No `platform-wallet` type is exposed here.
pub struct WalletRegistration {
    /// DET seed hash — stable identifier across the seam.
    pub seed_hash: WalletSeedHash,
    /// 64-byte BIP-39 seed. Fed to upstream `create_wallet_from_seed_bytes`
    /// in memory; zeroized on drop.
    pub seed_bytes: Zeroizing<[u8; 64]>,
    /// Network the wallet belongs to.
    pub network: WalletNetwork,
    /// Optional user-facing alias.
    pub alias: Option<String>,
    /// Whether this is the user's primary wallet.
    pub is_main: bool,
}

impl std::fmt::Debug for WalletRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print seed bytes.
        f.debug_struct("WalletRegistration")
            .field("seed_hash", &hex::encode(self.seed_hash))
            .field("network", &self.network)
            .field("alias", &self.alias)
            .field("is_main", &self.is_main)
            .finish_non_exhaustive()
    }
}

/// Resolves the set of wallets to register with the backend at startup.
///
/// Object-safe so `WalletBackend` can hold `Arc<dyn PersistedWalletLoader>`
/// and swap impls without touching its own API (mirrors the single-key
/// `SingleKeyBackend` seam).
pub trait PersistedWalletLoader: Send + Sync {
    /// Return one [`WalletRegistration`] per wallet that should be registered.
    fn wallets_to_register(
        &self,
        ctx: &Arc<AppContext>,
    ) -> Result<Vec<WalletRegistration>, TaskError>;
}

fn core_to_wallet_network(network: CoreNetwork) -> WalletNetwork {
    match network {
        CoreNetwork::Mainnet => WalletNetwork::Mainnet,
        CoreNetwork::Testnet => WalletNetwork::Testnet,
        CoreNetwork::Devnet => WalletNetwork::Devnet,
        CoreNetwork::Regtest => WalletNetwork::Regtest,
    }
}

/// Re-registers each wallet from DET's retained encrypted-seed store.
///
/// This is the shipping G2 impl: it mocks the *seam*, not the *behavior*.
/// The behavior — re-derive from the seed the user already unlocks, then let
/// the upstream persister layer identity/DashPay/UTXO deltas and `SpvRuntime`
/// re-confirm on sync — is exactly the upstream-prescribed re-registration
/// path. Reads only already-open wallets; locked wallets surface the existing
/// typed seed-decrypt error path elsewhere and are skipped here.
pub struct SeedReregistrationLoader;

impl SeedReregistrationLoader {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SeedReregistrationLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl PersistedWalletLoader for SeedReregistrationLoader {
    fn wallets_to_register(
        &self,
        ctx: &Arc<AppContext>,
    ) -> Result<Vec<WalletRegistration>, TaskError> {
        let network = core_to_wallet_network(ctx.network);
        let wallets = ctx.wallets.read()?;

        let mut registrations = Vec::with_capacity(wallets.len());
        for (seed_hash, wallet_arc) in wallets.iter() {
            let guard = wallet_arc.read()?;
            if !guard.is_open() {
                // Locked wallet — the user unlocks it through the existing
                // password flow; it is registered on a later pass.
                tracing::debug!(
                    wallet = %hex::encode(seed_hash),
                    "Skipping locked wallet during persisted-wallet load"
                );
                continue;
            }
            let seed_bytes = match guard.seed_bytes() {
                Ok(bytes) => Zeroizing::new(*bytes),
                Err(_) => {
                    tracing::warn!(
                        wallet = %hex::encode(seed_hash),
                        "Open wallet has no accessible seed; skipping"
                    );
                    continue;
                }
            };
            registrations.push(WalletRegistration {
                seed_hash: *seed_hash,
                seed_bytes,
                network,
                alias: guard.alias.clone(),
                is_main: guard.is_main,
            });
        }

        Ok(registrations)
    }
}

// `UpstreamFromPersisted` is intentionally NOT implemented: no upstream
// `Wallet::from_persisted` / populated `ClientStartState.wallets` API exists
// yet. The trait slot above is the reserved swap point — when upstream lands
// it, add the impl and flip the one construction line in `WalletBackend::new`
// (see g2-mock-boundary.md §G2.4). Mirrors the single-key reserved-impl
// pattern; no placeholder type is created until there is a real API to back
// it.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wallet_registration_debug_redacts_seed() {
        let reg = WalletRegistration {
            seed_hash: [7u8; 32],
            seed_bytes: Zeroizing::new([0xABu8; 64]),
            network: WalletNetwork::Testnet,
            alias: Some("primary".to_string()),
            is_main: true,
        };
        let dbg = format!("{reg:?}");
        assert!(dbg.contains(&hex::encode([7u8; 32])));
        assert!(dbg.contains("primary"));
        // The 64-byte seed (0xAB repeated) must never appear.
        assert!(!dbg.contains("abab"));
        assert!(!dbg.contains("171717"));
    }

    #[test]
    fn core_to_wallet_network_maps_all_variants() {
        assert_eq!(
            core_to_wallet_network(CoreNetwork::Mainnet),
            WalletNetwork::Mainnet
        );
        assert_eq!(
            core_to_wallet_network(CoreNetwork::Testnet),
            WalletNetwork::Testnet
        );
        assert_eq!(
            core_to_wallet_network(CoreNetwork::Devnet),
            WalletNetwork::Devnet
        );
        assert_eq!(
            core_to_wallet_network(CoreNetwork::Regtest),
            WalletNetwork::Regtest
        );
    }

    /// Proves the G2.4 zero-blast swap: an alternate loader impl drops into
    /// `Arc<dyn PersistedWalletLoader>` with no other change. The behavioral
    /// "N seed-store wallets → N registrations" assertion needs a populated
    /// `AppContext` and lives in the P1 backend-e2e QA lane.
    #[test]
    fn swap_boundary_compiles_with_alternate_impl() {
        struct StubFromPersisted;
        impl PersistedWalletLoader for StubFromPersisted {
            fn wallets_to_register(
                &self,
                _ctx: &Arc<AppContext>,
            ) -> Result<Vec<WalletRegistration>, TaskError> {
                Ok(Vec::new())
            }
        }
        let _loader: Arc<dyn PersistedWalletLoader> = Arc::new(StubFromPersisted);
        let _default: Arc<dyn PersistedWalletLoader> = Arc::new(SeedReregistrationLoader::new());
    }
}
