use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::secret::Secret;
use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use std::sync::Arc;

impl AppContext {
    /// Derive a private key for on-screen display/export.
    ///
    /// The HD seed is fetched just-in-time through the JIT chokepoint and
    /// borrowed only for the single derivation inside the closure; it zeroizes
    /// when the closure returns. Only the resulting WIF — wrapped in
    /// [`Secret`] — crosses back to the UI. This is the seam the sync UI key
    /// viewers use instead of reading the wallet's parked seed.
    pub(crate) async fn derive_key_for_display(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        derivation_path: DerivationPath,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let wallet = {
            let wallet_arc = {
                let wallets = self.wallets.read()?;
                wallets
                    .get(&seed_hash)
                    .cloned()
                    .ok_or(TaskError::WalletNotFound)?
            };
            wallet_arc.read()?.clone()
        };

        let network = self.network;
        let path_for_derive = derivation_path.clone();
        let backend = self.wallet_backend()?;
        let wif = backend
            .secret_access()
            .with_secret(
                &crate::wallet_backend::SecretScope::HdSeed { seed_hash },
                |plaintext| {
                    let seed = plaintext
                        .expose_hd_seed()
                        .ok_or(TaskError::ContactWalletSeedUnavailable)?;
                    let private_key = wallet
                        .private_key_at_derivation_path_with_seed(seed, &path_for_derive, network)
                        .map_err(|detail| {
                            tracing::warn!(error = %detail, "Key-for-display derivation failed");
                            TaskError::WalletKeyLookupFailed
                        })?;
                    Ok(Secret::new(private_key.to_wif()))
                },
            )
            .await?;

        Ok(BackendTaskSuccessResult::WalletKeyForDisplay {
            seed_hash,
            derivation_path,
            wif,
        })
    }
}
