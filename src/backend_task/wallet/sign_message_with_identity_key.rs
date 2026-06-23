//! Backend task: sign a message with a vault-backed identity key.
//! Fetches the raw key JIT through the chokepoint (`InVault` route); only the
//! Base64 signature crosses back to the UI.

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::backend_task::wallet::dash_signed_message;
use crate::context::AppContext;
use crate::model::qualified_identity::PrivateKeyTarget;
use dash_sdk::dpp::dashcore::secp256k1::SecretKey;
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

        let scope = crate::wallet_backend::SecretScope::IdentityKey {
            identity_id: identity_id.to_buffer(),
            target: target.clone(),
            key_id,
        };
        let backend = self.wallet_backend()?;
        let signature = backend
            .secret_access()
            .with_secret(&scope, |plaintext| {
                let key = plaintext
                    .expose_identity_key()
                    .ok_or(TaskError::IdentityKeyMissing)?;
                // Present-but-malformed key bytes are a decrypt/parse failure,
                // not a signing failure — same mapping as the display sibling.
                let secret_key = SecretKey::from_byte_array(key).map_err(|detail| {
                    tracing::warn!(error = %detail, "Identity-key sign secret construction failed");
                    TaskError::SecretDecryptFailed
                })?;
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
