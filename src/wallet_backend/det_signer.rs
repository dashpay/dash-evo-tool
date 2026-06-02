//! Just-in-time soft-wallet signer for the upstream
//! [`key_wallet::signer::Signer`] seam.
//!
//! Upstream identity / payment / asset-lock flows are signer-driven: every
//! secp256k1 sign of a sighash and every public-key derivation goes through
//! an externally-injected [`Signer`]. [`DetSigner`] is DET's JIT
//! implementation — it **borrows** the plaintext secret held open by a
//! [`SecretAccess::with_secret_session`] scope and derives + signs locally,
//! with no host round-trip.
//!
//! The seed source is "borrow the held JIT secret" — never a by-value
//! `[u8; N]` snapshot (Smythe must-fix #2). The held secret zeroizes when
//! the `with_secret_session` scope ends, so the signer's borrow can never
//! outlive the plaintext.
//!
//! Two key sources:
//! - **HD seed** — the full [`Signer`] surface (BIP-32 derive at a path,
//!   then ECDSA). Used by identity / payment / asset-lock flows.
//! - **Single key** — a path-free raw ECDSA over the held 32 bytes via
//!   [`DetSigner::sign_single_key_ecdsa`]. Single keys carry no derivation
//!   tree, so the path-based `Signer` surface does not apply to them.
//!
//! M-DONT-LEAK-TYPES: this type and its error live inside `wallet_backend`;
//! the upstream signer trait is the only seam that touches `key_wallet::*`
//! outside the module.
//!
//! The HD `Signer` surface (`from_held` + `sign_ecdsa` / `public_key`) is
//! wired into every signer-driven HD flow (payment / asset-lock / identity).
//! The single-key raw-ECDSA helper [`DetSigner::sign_single_key_ecdsa`] is
//! built and unit-tested but has no live caller yet — single-key *send* is
//! still stubbed upstream (design §0.4) — so it carries a scoped
//! `expect(dead_code)` until that send path is un-gated.

use async_trait::async_trait;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::dashcore::secp256k1::{self, Message, PublicKey, Secp256k1, ecdsa};
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use dash_sdk::dpp::key_wallet::signer::{Signer, SignerMethod};
use zeroize::Zeroizing;

use crate::wallet_backend::secret_access::SecretPlaintext;

/// Errors returned by [`DetSigner`]. Wired with `#[source]` so callers can
/// walk the cause chain; never carries user-facing prose or any secret
/// (CLAUDE.md error-variant rules).
#[derive(Debug, thiserror::Error)]
pub enum DetSignerError {
    /// Derivation from the held seed failed.
    #[error("just-in-time signer key derivation failed")]
    Derive(#[source] dash_sdk::dpp::key_wallet::bip32::Error),
    /// secp256k1 sign / digest construction failed.
    #[error("just-in-time signer secp256k1 operation failed")]
    Sign(#[source] secp256k1::Error),
    /// The held secret is the wrong kind for the requested operation
    /// (e.g. a single-key plaintext asked for path-based HD signing).
    #[error("just-in-time signer received the wrong secret kind for this operation")]
    WrongSecretKind,
}

/// Hash a 32-byte digest into an ECDSA-ready [`Message`], surfacing the
/// upstream digest-length error as a typed signer error rather than
/// panicking.
fn message_from_digest(digest: [u8; 32]) -> Result<Message, DetSignerError> {
    Message::from_digest_slice(&digest).map_err(DetSignerError::Sign)
}

/// JIT [`Signer`] backed by a **borrowed** held secret.
///
/// Constructed from a [`SecretPlaintext`] borrow inside a
/// `with_secret_session` scope; never owns or copies the plaintext bytes.
/// The lifetime `'a` ties the signer to the held secret, so the borrow
/// cannot escape the scope where the plaintext is alive.
pub(crate) struct DetSigner<'a> {
    secret: HeldSecret<'a>,
    network: Network,
    /// `Signer::supported_methods` returns `&[SignerMethod]`; own the
    /// backing storage so the borrow is sound.
    supported_methods: [SignerMethod; 1],
}

/// The borrowed key material a [`DetSigner`] operates on.
enum HeldSecret<'a> {
    HdSeed(&'a Zeroizing<[u8; 64]>),
    SingleKey(&'a Zeroizing<[u8; 32]>),
}

impl<'a> DetSigner<'a> {
    /// Build a signer over the held plaintext for `network`. Borrows the
    /// secret — no copy.
    pub(crate) fn from_held(plaintext: SecretPlaintext<'a>, network: Network) -> Self {
        let secret = match plaintext {
            SecretPlaintext::HdSeed(seed) => HeldSecret::HdSeed(seed),
            SecretPlaintext::SingleKey(key) => HeldSecret::SingleKey(key),
        };
        Self {
            secret,
            network,
            supported_methods: [SignerMethod::Digest],
        }
    }

    /// Derive the secp256k1 secret at `path` from the held HD seed. Errors
    /// with [`DetSignerError::WrongSecretKind`] if the held secret is a
    /// single key (no derivation tree).
    fn derive_secret(&self, path: &DerivationPath) -> Result<secp256k1::SecretKey, DetSignerError> {
        match &self.secret {
            HeldSecret::HdSeed(seed) => {
                let xprv = path
                    .derive_priv_ecdsa_for_master_seed(seed.as_ref(), self.network)
                    .map_err(DetSignerError::Derive)?;
                Ok(xprv.private_key)
            }
            HeldSecret::SingleKey(_) => Err(DetSignerError::WrongSecretKind),
        }
    }

    /// Sign `msg` (a 32-byte digest) with the held **single key** directly,
    /// no derivation. Errors if the held secret is an HD seed. Used by the
    /// JIT single-key signing chokepoint
    /// ([`WalletBackend::sign_single_key`](super::WalletBackend::sign_single_key)).
    pub(crate) fn sign_single_key_ecdsa(
        &self,
        msg: &[u8; 32],
    ) -> Result<ecdsa::Signature, DetSignerError> {
        match &self.secret {
            HeldSecret::SingleKey(key) => {
                let sk =
                    secp256k1::SecretKey::from_byte_array(key).map_err(DetSignerError::Sign)?;
                let message = message_from_digest(*msg)?;
                Ok(Secp256k1::signing_only().sign_ecdsa(&message, &sk))
            }
            HeldSecret::HdSeed(_) => Err(DetSignerError::WrongSecretKind),
        }
    }
}

impl std::fmt::Debug for DetSigner<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.secret {
            HeldSecret::HdSeed(_) => "HdSeed",
            HeldSecret::SingleKey(_) => "SingleKey",
        };
        f.debug_struct("DetSigner")
            .field("secret_kind", &kind)
            .field("network", &self.network)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Signer for DetSigner<'_> {
    type Error = DetSignerError;

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
        Ok(PublicKey::from_secret_key(
            &Secp256k1::signing_only(),
            &secret,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::wallet::WalletSeedHash;
    use crate::model::wallet::encryption::encrypt_message;
    use crate::model::wallet::seed_envelope::StoredSeedEnvelope;
    use crate::wallet_backend::secret_access::SecretAccess;
    use crate::wallet_backend::secret_prompt::SecretScope;
    use crate::wallet_backend::secret_prompt::test_support::{ScriptedAnswer, TestPrompt};
    use crate::wallet_backend::single_key::open_secret_store;
    use crate::wallet_backend::wallet_seed_store::WalletSeedView;
    use std::sync::Arc;

    const SENTINEL_PASSPHRASE: &str = "correct-horse-battery-staple-SENTINEL";
    const SENTINEL_SEED: [u8; 64] = [0x5A; 64];

    fn store_with_protected_hd(
        dir: &std::path::Path,
        seed_hash: &WalletSeedHash,
    ) -> Arc<platform_wallet_storage::secrets::SecretStore> {
        let store = Arc::new(open_secret_store(&dir.join("v.pwsvault")).expect("vault"));
        let (encrypted_seed, salt, nonce) =
            encrypt_message(&SENTINEL_SEED, SENTINEL_PASSPHRASE).expect("enc");
        let envelope = StoredSeedEnvelope {
            encrypted_seed,
            salt,
            nonce,
            password_hint: None,
            uses_password: true,
            xpub_encoded: vec![0xCD; 78],
        };
        WalletSeedView::new(&store)
            .set(seed_hash, &envelope)
            .unwrap();
        store
    }

    /// A held HD seed produces a usable signer that derives + signs, and
    /// repeated signs at the same path return a stable public key (the
    /// secret is deterministic from the seed). Proves the JIT signer pulls
    /// the seed through `with_secret_session` without a re-prompt.
    #[tokio::test]
    async fn hd_signer_derives_and_signs_via_jit() {
        let dir = tempfile::tempdir().unwrap();
        let seed_hash: WalletSeedHash = [0x11; 32];
        let store = store_with_protected_hd(dir.path(), &seed_hash);
        let prompt = Arc::new(TestPrompt::new([ScriptedAnswer::once(SENTINEL_PASSPHRASE)]));
        let sa = SecretAccess::new(store, prompt.clone(), Network::Testnet);
        let scope = SecretScope::HdSeed { seed_hash };
        let path: DerivationPath = "m/44'/1'/0'/0/0".parse().unwrap();
        let sighash = [42u8; 32];

        let same_pubkey = sa
            .with_secret_session(&scope, async |session| {
                let signer = DetSigner::from_held(session.plaintext(), Network::Testnet);
                let (_s1, pk1) = signer.sign_ecdsa(&path, sighash).await.unwrap();
                let (_s2, pk2) = signer.sign_ecdsa(&path, sighash).await.unwrap();
                let pk3 = signer.public_key(&path).await.unwrap();
                Ok(pk1 == pk2 && pk1 == pk3)
            })
            .await
            .unwrap();
        assert!(same_pubkey, "two signs + a derive agree on the public key");
        assert_eq!(prompt.ask_count(), 1, "one prompt for the whole operation");
    }

    /// The held single-key plaintext signs raw ECDSA without derivation,
    /// and asking the HD `Signer` surface of a single-key-held signer
    /// returns the typed `WrongSecretKind` rather than mis-deriving.
    #[tokio::test]
    async fn single_key_signer_signs_raw_and_rejects_path() {
        use dash_sdk::dpp::dashcore::PrivateKey;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(open_secret_store(&dir.path().join("v.pwsvault")).expect("vault"));
        let index = std::sync::RwLock::new(std::collections::BTreeMap::new());
        let view = crate::wallet_backend::single_key::SingleKeyView::from_views(
            &store,
            &index,
            Network::Testnet,
            None,
        );
        let mut key_bytes = [0u8; 32];
        key_bytes[31] = 1;
        let sk = secp256k1::SecretKey::from_byte_array(&key_bytes).unwrap();
        let wif = PrivateKey::new(sk, Network::Testnet).to_wif();
        let imported = view
            .import_wif_with_passphrase(
                &wif,
                None,
                crate::wallet_backend::single_key::ImportPassphrase {
                    passphrase: Some(SENTINEL_PASSPHRASE.to_string()),
                    hint: None,
                },
            )
            .unwrap();

        let prompt = Arc::new(TestPrompt::new([ScriptedAnswer::once(SENTINEL_PASSPHRASE)]));
        let sa = SecretAccess::new(Arc::clone(&store), prompt, Network::Testnet);
        let scope = SecretScope::SingleKey {
            address: imported.address,
        };
        let path: DerivationPath = "m/44'/1'/0'/0/0".parse().unwrap();
        let msg = [7u8; 32];

        sa.with_secret_session(&scope, async |session| {
            let signer = DetSigner::from_held(session.plaintext(), Network::Testnet);
            // Raw single-key sign succeeds.
            signer.sign_single_key_ecdsa(&msg).expect("raw sign");
            // Path-based HD surface rejects a single-key secret.
            let err = signer.sign_ecdsa(&path, msg).await.expect_err("wrong kind");
            assert!(matches!(err, DetSignerError::WrongSecretKind));
            Ok(())
        })
        .await
        .unwrap();
    }

    /// `Debug` redacts the secret — only the kind tag and network appear,
    /// never the bytes.
    #[test]
    fn debug_redacts_held_secret() {
        let seed = Zeroizing::new([0x42u8; 64]);
        let signer = DetSigner::from_held(SecretPlaintext::HdSeed(&seed), Network::Testnet);
        let dbg = format!("{signer:?}");
        assert!(dbg.contains("HdSeed"), "kind tag present: {dbg}");
        assert!(!dbg.contains("42, 42, 42"), "seed bytes leaked: {dbg}");
        assert_eq!(signer.supported_methods(), &[SignerMethod::Digest]);
    }
}
