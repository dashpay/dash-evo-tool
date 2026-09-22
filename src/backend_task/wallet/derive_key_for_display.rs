//! Backend task: derive a wallet HD key for on-screen display or export.
//! Fetches the seed JIT through the secret chokepoint; only the WIF crosses back to the UI.

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
        let wif = self
            .with_wallet_derived_key(
                seed_hash,
                &derivation_path,
                TaskError::WalletKeyLookupFailed,
                |private_key| Ok(Secret::new(private_key.to_wif())),
            )
            .await?;

        Ok(BackendTaskSuccessResult::WalletKeyForDisplay {
            seed_hash,
            derivation_path,
            wif,
        })
    }
}
