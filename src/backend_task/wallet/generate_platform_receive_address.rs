//! Backend task: generate a fresh Platform (DIP-17/18) receive address for a wallet.
//! Fetches the seed JIT through the secret chokepoint; only the Bech32m address crosses back to the UI.

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::address_funds::PlatformAddress;
use std::sync::Arc;

impl AppContext {
    /// Generate a fresh Platform (DIP-17/18) receive address for a wallet.
    ///
    /// The HD seed is fetched just-in-time through the JIT chokepoint and
    /// borrowed only for the single derivation inside the closure; it zeroizes
    /// when the closure returns. The new address is derived, registered on the
    /// in-memory wallet, and only the Bech32m-encoded address string crosses
    /// back to the UI — the seed never leaves the backend. This is the seam the
    /// sync receive-address UI uses instead of reading the wallet's parked seed.
    pub(crate) async fn generate_platform_receive_address(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let wallet_arc = {
            let wallets = self.wallets.read()?;
            wallets
                .get(&seed_hash)
                .cloned()
                .ok_or(TaskError::WalletNotFound)?
        };

        let network = self.network;
        let ctx = Arc::clone(self);
        let backend = self.wallet_backend()?;
        let address = backend
            .secret_access()
            .with_secret(
                &crate::wallet_backend::SecretScope::HdSeed { seed_hash },
                |plaintext| {
                    let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;
                    let mut guard = wallet_arc.write()?;
                    let address = guard
                        .generate_platform_receive_address_with_seed(seed, network, Some(&ctx))
                        .map_err(|detail| {
                            tracing::warn!(error = %detail, "Platform receive-address derivation failed");
                            TaskError::WalletPlatformReceiveAddressFailed
                        })?;
                    let platform_address = PlatformAddress::try_from(address).map_err(|detail| {
                        tracing::warn!(error = %detail, "Derived address is not a valid Platform address");
                        TaskError::WalletPlatformReceiveAddressFailed
                    })?;
                    Ok(platform_address.to_bech32m_string(network))
                },
            )
            .await?;

        Ok(BackendTaskSuccessResult::GeneratedPlatformReceiveAddress { seed_hash, address })
    }
}
