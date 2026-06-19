//! Backend task: sign a message with a wallet-derived key at a given derivation path.
//! Fetches the seed JIT through the secret chokepoint; only the Base64 signature crosses back to the UI.

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::backend_task::wallet::dash_signed_message;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::dashcore::secp256k1::SecretKey;
use dash_sdk::dpp::identity::KeyType;
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use std::sync::Arc;

impl AppContext {
    /// Sign a message with a wallet-derived key at `derivation_path`.
    ///
    /// The HD seed is fetched just-in-time through the JIT chokepoint and
    /// borrowed only for the single derivation inside the closure; both the
    /// seed and the derived private key zeroize when the closure returns. Only
    /// the resulting public Base64 signature crosses back to the UI — no secret
    /// material leaves the backend. This is the seam the on-screen "sign
    /// message" feature uses for wallet-derived keys instead of deriving the key
    /// in the UI from the wallet's parked seed.
    pub(crate) async fn sign_message_with_key(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        derivation_path: DerivationPath,
        message: String,
        key_type: KeyType,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // Only ECDSA key types support message signing here; reject others
        // before touching the seed so no prompt fires for an unsupported key.
        if !matches!(key_type, KeyType::ECDSA_SECP256K1 | KeyType::ECDSA_HASH160) {
            return Err(TaskError::WalletMessageSignUnsupportedKeyType);
        }

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
        let signature = backend
            .secret_access()
            .with_secret(
                &crate::wallet_backend::SecretScope::HdSeed { seed_hash },
                |plaintext| {
                    let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;
                    let private_key = wallet
                        .private_key_at_derivation_path_with_seed(seed, &path_for_derive, network)
                        .map_err(|detail| {
                            tracing::warn!(error = %detail, "Sign-message key derivation failed");
                            TaskError::WalletMessageSigningFailed
                        })?;

                    let secret_key = SecretKey::from_byte_array(&private_key.inner.secret_bytes())
                        .map_err(|detail| {
                            tracing::warn!(error = %detail, "Sign-message secret key construction failed");
                            TaskError::WalletMessageSigningFailed
                        })?;
                    Ok(dash_signed_message(
                        message.as_str(),
                        &secret_key,
                        private_key.compressed,
                    ))
                },
            )
            .await?;

        Ok(BackendTaskSuccessResult::WalletMessageSigned {
            seed_hash,
            derivation_path,
            signature,
        })
    }
}

