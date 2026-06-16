//! Backend task: warm the identity-authentication public-key cache for one identity index.
//! Fetches the seed JIT through the secret chokepoint; only derived public keys are persisted — no secret leaves the backend.

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use crate::wallet_backend::SecretScope;
use std::sync::Arc;

impl AppContext {
    /// Warm the identity-authentication public-key cache for one identity index.
    ///
    /// The identity-key chooser runs in synchronous `ui()` and cannot `await`,
    /// so it reads the public keys it shows from the
    /// [`AuthPubkeyCache`](crate::model::wallet::auth_pubkey_cache::AuthPubkeyCache).
    /// On a cold cache this task fills it: the HD seed is fetched just-in-time
    /// through the JIT chokepoint, the first `key_count` auth keys are derived
    /// and persisted, and only a completion signal returns. The seed never
    /// crosses into the UI.
    ///
    /// Best-effort and idempotent: keys already cached are skipped; a single
    /// `with_secret` scope covers the whole range (one prompt for a protected
    /// wallet, though at the chooser the wallet is already open so the scope
    /// resolves from the session cache without prompting).
    pub(crate) async fn warm_identity_auth_pubkeys(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        identity_index: u32,
        key_count: u32,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let network = self.network;

        let wallet = {
            let wallets = self.wallets.read()?;
            wallets
                .get(&seed_hash)
                .cloned()
                .ok_or(TaskError::WalletNotFound)?
        };

        let backend = self.wallet_backend()?;
        let view = backend.auth_pubkey_cache();
        let mut cache = view.get(network, &seed_hash);

        let missing: Vec<u32> = (0..key_count)
            .filter(|&key_index| cache.get(network, identity_index, key_index).is_none())
            .collect();

        if missing.is_empty() {
            return Ok(BackendTaskSuccessResult::IdentityAuthPubkeysWarmed { identity_index });
        }

        backend
            .secret_access()
            .with_secret(&SecretScope::HdSeed { seed_hash }, |plaintext| {
                let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;
                let guard = wallet.read()?;
                let mut changed = false;
                for &key_index in &missing {
                    let public_key = guard
                        .identity_authentication_ecdsa_public_key_from_seed(
                            seed,
                            network,
                            identity_index,
                            key_index,
                        )
                        .map_err(|detail| {
                            tracing::warn!(error = %detail, "Identity-auth pubkey warm derivation failed");
                            TaskError::WalletKeyLookupFailed
                        })?;
                    changed |= cache.insert(network, identity_index, key_index, &public_key);
                }
                if changed {
                    view.put(network, &seed_hash, &cache)?;
                }
                Ok(())
            })
            .await?;

        Ok(BackendTaskSuccessResult::IdentityAuthPubkeysWarmed { identity_index })
    }
}
