//! Backend task: derive a vault-backed identity key for on-screen display.
//! Fetches the raw key JIT through the secret chokepoint (`InVault` route);
//! only the WIF crosses back to the UI.

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::PrivateKeyTarget;
use crate::model::secret::Secret;
use dash_sdk::dpp::dashcore::PrivateKey;
use dash_sdk::dpp::identity::KeyID;
use dash_sdk::platform::Identifier;
use std::sync::Arc;

impl AppContext {
    /// Derive an identity private key for on-screen display/export.
    ///
    /// The raw key is fetched just-in-time from the vault through the chokepoint
    /// (`SecretScope::IdentityKey`, prompt-free) and borrowed only inside the
    /// closure; it zeroizes when the closure returns. Only the WIF — wrapped in
    /// [`Secret`] — crosses back to the UI.
    pub(crate) async fn derive_identity_key_for_display(
        self: &Arc<Self>,
        identity_id: Identifier,
        target: PrivateKeyTarget,
        key_id: KeyID,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let network = self.network;
        let wif = self
            .with_identity_secret_key(identity_id, target.clone(), key_id, |secret_key| {
                let private_key = PrivateKey::new(secret_key, network);
                Ok(Secret::new(private_key.to_wif()))
            })
            .await?;

        Ok(BackendTaskSuccessResult::IdentityKeyForDisplay {
            identity_id,
            target,
            key_id,
            wif,
        })
    }
}
