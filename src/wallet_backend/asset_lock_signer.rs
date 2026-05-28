//! Asset-lock signer adapter for the upstream `key_wallet::signer::Signer` trait.
//!
//! Upstream `IdentityWallet::register_identity_with_funding` and
//! `top_up_identity_with_funding` are signer-driven: every secp256k1 sign of
//! a funding-input sighash and every credit-output public-key derivation goes
//! through the externally-injected signer. [`WalletAssetLockSigner`] is DET's
//! soft-wallet implementation — it owns a snapshot of the wallet seed and
//! derives + signs locally, with no host round-trip and no contention on the
//! upstream wallet manager lock.
//!
//! The seed snapshot is taken at signer construction (zeroized on drop) so
//! ECDSA derivation can run without acquiring `wallet_manager().write()`
//! across an `await` — which would deadlock against the wallet's own internal
//! locks during the asset-lock build path. The snapshot lifetime is one
//! identity operation, never persisted.
//!
//! M-DONT-LEAK-TYPES: this type and its error live entirely inside
//! `wallet_backend`; the upstream signer trait is the only seam that touches
//! `key_wallet::*` outside this module.

use async_trait::async_trait;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::dashcore::secp256k1::{self, Message, PublicKey, Secp256k1, ecdsa};
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use dash_sdk::dpp::key_wallet::signer::{Signer, SignerMethod};
use zeroize::Zeroizing;

/// Hash a 32-byte digest with the secp256k1 context, returning an
/// ECDSA-ready `Message`. Surfaces upstream digest-length errors as our own
/// typed signer error rather than panicking.
fn message_from_digest(digest: [u8; 32]) -> Result<Message, AssetLockSignerError> {
    Message::from_digest_slice(&digest).map_err(AssetLockSignerError::Sign)
}

/// Errors returned by [`WalletAssetLockSigner`]. Wired with `#[source]` so
/// callers can walk the cause chain; never carries user-facing prose (per
/// CLAUDE.md error-variant rules).
#[derive(Debug, thiserror::Error)]
pub enum AssetLockSignerError {
    /// Derivation from the seed snapshot failed.
    #[error("asset-lock signer key derivation failed")]
    Derive(#[source] dash_sdk::dpp::key_wallet::bip32::Error),
    /// secp256k1 sign / digest construction failed.
    #[error("asset-lock signer secp256k1 operation failed")]
    Sign(#[source] secp256k1::Error),
}

/// Soft-wallet [`Signer`] backed by a one-shot seed snapshot.
///
/// Constructed once per identity operation, dropped (and zeroized) when the
/// upstream call returns. Always supports [`SignerMethod::Digest`].
pub(crate) struct WalletAssetLockSigner {
    /// Snapshot of the 64-byte BIP-39 seed. Owned by the signer so derivation
    /// can run without contention on the upstream wallet manager lock.
    /// Zeroized on drop.
    seed_bytes: Zeroizing<[u8; 64]>,
    /// Network the wallet belongs to — needed for BIP-32 derivation.
    network: Network,
    /// Lazily-allocated `&'static`-shaped capability slice. `Signer` returns
    /// `&[SignerMethod]`, so we own the backing storage in the struct.
    supported_methods: [SignerMethod; 1],
}

impl WalletAssetLockSigner {
    pub(crate) fn new(seed_bytes: Zeroizing<[u8; 64]>, network: Network) -> Self {
        Self {
            seed_bytes,
            network,
            supported_methods: [SignerMethod::Digest],
        }
    }

    fn derive_secret(
        &self,
        path: &DerivationPath,
    ) -> Result<secp256k1::SecretKey, AssetLockSignerError> {
        let xprv = path
            .derive_priv_ecdsa_for_master_seed(self.seed_bytes.as_ref(), self.network)
            .map_err(AssetLockSignerError::Derive)?;
        Ok(xprv.private_key)
    }
}

impl std::fmt::Debug for WalletAssetLockSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WalletAssetLockSigner")
            .field("network", &self.network)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Signer for WalletAssetLockSigner {
    type Error = AssetLockSignerError;

    fn supported_methods(&self) -> &[SignerMethod] {
        &self.supported_methods
    }

    async fn sign_ecdsa(
        &self,
        path: &DerivationPath,
        sighash: [u8; 32],
    ) -> Result<(ecdsa::Signature, PublicKey), Self::Error> {
        let secret = self.derive_secret(path)?;
        let secp = Secp256k1::signing_only();
        let msg = message_from_digest(sighash)?;
        let signature = secp.sign_ecdsa(&msg, &secret);
        let public_key = PublicKey::from_secret_key(&secp, &secret);
        Ok((signature, public_key))
    }

    async fn public_key(&self, path: &DerivationPath) -> Result<PublicKey, Self::Error> {
        let secret = self.derive_secret(path)?;
        let secp = Secp256k1::signing_only();
        Ok(PublicKey::from_secret_key(&secp, &secret))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn supports_digest_method_only() {
        let signer = WalletAssetLockSigner::new(Zeroizing::new([0u8; 64]), Network::Testnet);
        let methods = signer.supported_methods();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0], SignerMethod::Digest);
    }

    #[tokio::test]
    async fn sign_ecdsa_returns_consistent_pubkey() {
        // Two signs at the same path return the same public key (the secret
        // is deterministic from the seed snapshot).
        let signer = WalletAssetLockSigner::new(Zeroizing::new([1u8; 64]), Network::Testnet);
        let path: DerivationPath = "m/44'/1'/0'/0/0".parse().unwrap();
        let sighash = [42u8; 32];
        let (_sig1, pk1) = signer.sign_ecdsa(&path, sighash).await.unwrap();
        let (_sig2, pk2) = signer.sign_ecdsa(&path, sighash).await.unwrap();
        assert_eq!(pk1, pk2);
        // Same path via `public_key` matches.
        let pk3 = signer.public_key(&path).await.unwrap();
        assert_eq!(pk1, pk3);
    }
}
