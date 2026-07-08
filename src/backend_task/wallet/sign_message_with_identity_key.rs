//! Backend task: sign a message with a vault-backed identity key.
//! Fetches the raw key JIT through the chokepoint (`InVault` route); only the
//! Base64 signature crosses back to the UI.

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::backend_task::wallet::dash_signed_message;
use crate::context::AppContext;
use crate::model::qualified_identity::PrivateKeyTarget;
use dash_sdk::dpp::identity::{KeyID, KeyType};
use dash_sdk::platform::Identifier;
use std::sync::Arc;

impl AppContext {
    /// Sign a message with a vault-backed identity key.
    ///
    /// The raw key is fetched just-in-time through the chokepoint and borrowed
    /// only for the single sign inside the closure; it zeroizes on return. Only
    /// the public Base64 signature crosses back to the UI. Identity keys are
    /// compressed by convention, so the recoverable envelope uses `compressed`.
    pub(crate) async fn sign_message_with_identity_key(
        self: &Arc<Self>,
        identity_id: Identifier,
        target: PrivateKeyTarget,
        key_id: KeyID,
        message: String,
        key_type: KeyType,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // Reject non-ECDSA before touching the vault.
        if !matches!(key_type, KeyType::ECDSA_SECP256K1 | KeyType::ECDSA_HASH160) {
            return Err(TaskError::WalletMessageSignUnsupportedKeyType);
        }

        let signature = self
            .with_identity_secret_key(identity_id, target.clone(), key_id, |secret_key| {
                // Identity keys are compressed by convention.
                Ok(dash_signed_message(message.as_str(), &secret_key, true))
            })
            .await?;

        Ok(BackendTaskSuccessResult::IdentityMessageSigned {
            identity_id,
            target,
            key_id,
            signature,
        })
    }
}
