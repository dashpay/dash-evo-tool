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
//! wired into [`WalletBackend::sign_single_key`](super::WalletBackend::sign_single_key),
//! the JIT single-key signing chokepoint. That chokepoint has no production
//! caller yet — single-key *send* is still stubbed upstream (design §0.4) —
//! so the path is exercised through unit tests until that send flow is
//! un-gated.

use async_trait::async_trait;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::dashcore::secp256k1::{self, Message, PublicKey, Secp256k1, ecdsa};
use dash_sdk::dpp::key_wallet::bip32::{DerivationPath, ExtendedPrivKey, ExtendedPubKey};
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
            // An identity key is a raw secp256k1 secret, same shape as a
            // single key (no derivation tree) — `DetSigner` treats them
            // identically. Identity-platform signing normally goes straight
            // through the resolver, not here.
            SecretPlaintext::SingleKey(key) | SecretPlaintext::IdentityKey(key) => {
                HeldSecret::SingleKey(key)
            }
        };
        Self {
            secret,
            network,
            supported_methods: [SignerMethod::Digest],
        }
    }

    /// Derive the BIP-32 extended private key at `path` from the held HD
    /// seed. Errors with [`DetSignerError::WrongSecretKind`] if the held
    /// secret is a single key (no derivation tree). The returned
    /// `ExtendedPrivKey` zeroizes its secret scalar on drop.
    fn derive_xprv(&self, path: &DerivationPath) -> Result<ExtendedPrivKey, DetSignerError> {
        match &self.secret {
            HeldSecret::HdSeed(seed) => path
                .derive_priv_ecdsa_for_master_seed(seed.as_ref(), self.network)
                .map_err(DetSignerError::Derive),
            HeldSecret::SingleKey(_) => Err(DetSignerError::WrongSecretKind),
        }
    }

    /// Derive the secp256k1 secret at `path` from the held HD seed. Errors
    /// with [`DetSignerError::WrongSecretKind`] if the held secret is a
    /// single key (no derivation tree).
    fn derive_secret(&self, path: &DerivationPath) -> Result<secp256k1::SecretKey, DetSignerError> {
        Ok(self.derive_xprv(path)?.private_key)
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

    /// Derive the BIP-32 extended public key at `path` from the held HD seed
    /// (public point + chain code + parent fingerprint), letting callers
    /// non-hardened-derive descendants without a re-prompt. A single-key
    /// secret has no derivation tree, so it returns
    /// [`DetSignerError::WrongSecretKind`] rather than a leaf-only key — never
    /// a panic. The derived `ExtendedPrivKey` zeroizes on drop; the returned
    /// `ExtendedPubKey` carries only public material.
    async fn extended_public_key(
        &self,
        path: &DerivationPath,
    ) -> Result<ExtendedPubKey, Self::Error> {
        let xprv = self.derive_xprv(path)?;
        Ok(ExtendedPubKey::from_priv(&Secp256k1::signing_only(), &xprv))
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

    /// A pinned `(seed, path) → compressed pubkey` derivation vector for the
    /// HD parity test. A change to the BIP-32 derivation, the secp256k1
    /// primitive, or the seed handling would break this exact hex — catching a
    /// silent derivation regression that a self-consistent parity check alone
    /// could miss. Computed from [`PARITY_SEED`] at `m/44'/1'/0'/0/0` on
    /// Testnet (coin-type 1' = the network-independent BIP-44 testnet path).
    const PINNED_TESTNET_PUBKEY_HEX: &str =
        "03a9a554ad52bd61203ceee0c8ad65f60d019a1c43a73f657f1a080d9f321730f3";

    /// The deterministic seed both parity vectors derive from.
    const PARITY_SEED: [u8; 64] = [0x5A; 64];

    /// FUND-SAFETY PARITY: `DetSigner::sign_ecdsa` (the HD signer surface that
    /// authorises every payment / asset-lock / identity sign) must produce a
    /// byte-identical signature AND public key to an independent reference
    /// derivation (path → `derive_priv_ecdsa_for_master_seed` →
    /// `secp.sign_ecdsa`) for the same seed, path, and digest — on every
    /// network. A divergence here means wrong signatures and lost/failed funds.
    ///
    /// Mirrors the platform-signer parity test
    /// (`det_platform_signer::tests::platform_signer_parity_with_reference`):
    /// the reference recomputes the result without going through `DetSigner`,
    /// so the test proves genuine parity, not self-consistency. A pinned
    /// `(seed, path) → pubkey` vector additionally guards the derivation itself
    /// against a regression that would still be internally consistent.
    #[tokio::test]
    async fn hd_signer_parity_with_reference() {
        let path: DerivationPath = "m/44'/1'/0'/0/0".parse().unwrap();
        let sighash = [0x3Cu8; 32];

        for network in [Network::Testnet, Network::Mainnet] {
            // Independent reference: derive the key directly from the seed at
            // the path, then sign + recover the public key with the same
            // secp256k1 primitive `DetSigner` uses internally.
            let secp = Secp256k1::new();
            let reference_sk = path
                .derive_priv_ecdsa_for_master_seed(&PARITY_SEED, network)
                .expect("reference derive")
                .private_key;
            let reference_msg = Message::from_digest_slice(&sighash).expect("reference digest");
            let reference_sig = secp.sign_ecdsa(&reference_msg, &reference_sk);
            let reference_pk = PublicKey::from_secret_key(&secp, &reference_sk);

            // The JIT signer over the same seed, held open through the
            // chokepoint, must match byte-for-byte.
            let held = Zeroizing::new(PARITY_SEED);
            let signer = DetSigner::from_held(SecretPlaintext::HdSeed(&held), network);
            let (det_sig, det_pk) = signer.sign_ecdsa(&path, sighash).await.expect("det sign");

            assert_eq!(
                reference_sig, det_sig,
                "sign_ecdsa signature diverged from reference on {network:?}"
            );
            assert_eq!(
                reference_pk, det_pk,
                "sign_ecdsa public key diverged from reference on {network:?}"
            );

            // The derive-only surface must agree with the signing surface.
            let derive_only_pk = signer.public_key(&path).await.expect("public_key");
            assert_eq!(
                det_pk, derive_only_pk,
                "public_key disagreed with sign_ecdsa on {network:?}"
            );
        }
    }

    /// FUND-SAFETY PARITY: `DetSigner::extended_public_key` (the BIP-32 xpub
    /// export callers use to non-hardened-derive descendant payment addresses
    /// without a re-prompt) must reproduce, byte-for-byte, an independent
    /// reference derivation (seed → `derive_priv_ecdsa_for_master_seed` →
    /// `ExtendedPubKey::from_priv`) for the same seed, path, and network. A
    /// silent divergence in the chain code or parent fingerprint would derive
    /// the wrong descendants and misdirect funds. Its leaf point must also
    /// agree with `public_key` at the same path.
    #[tokio::test]
    async fn hd_signer_extended_public_key_parity_with_reference() {
        let path: DerivationPath = "m/44'/1'/0'/0/0".parse().unwrap();

        for network in [Network::Testnet, Network::Mainnet] {
            // Independent reference: derive the xprv straight from the seed at
            // the path, then convert to the xpub with the same primitive.
            let secp = Secp256k1::new();
            let reference_xprv = path
                .derive_priv_ecdsa_for_master_seed(&PARITY_SEED, network)
                .expect("reference derive");
            let reference_xpub = ExtendedPubKey::from_priv(&secp, &reference_xprv);

            let held = Zeroizing::new(PARITY_SEED);
            let signer = DetSigner::from_held(SecretPlaintext::HdSeed(&held), network);
            let det_xpub = signer
                .extended_public_key(&path)
                .await
                .expect("det extended_public_key");

            // Field-level checks first so a dropped BIP-32 metadatum fails with
            // a precise message before the full-struct assert catches the rest.
            assert_eq!(
                det_xpub.public_key, reference_xpub.public_key,
                "extended_public_key point diverged from reference on {network:?}"
            );
            assert_eq!(
                det_xpub.chain_code, reference_xpub.chain_code,
                "extended_public_key chain code diverged from reference on {network:?}"
            );
            assert_eq!(
                det_xpub, reference_xpub,
                "extended_public_key diverged from reference on {network:?}"
            );

            // The xpub's leaf point must agree with the compressed-point surface.
            let leaf = signer.public_key(&path).await.expect("public_key");
            assert_eq!(
                det_xpub.public_key, leaf,
                "extended_public_key point disagreed with public_key on {network:?}"
            );
        }
    }

    /// Pin the exact derived public key so a BIP-32 / coin-type / primitive
    /// regression is caught even if it stays internally consistent. Testnet
    /// path `m/44'/1'/0'/0/0` over [`PARITY_SEED`].
    #[test]
    fn hd_derivation_matches_pinned_vector() {
        let path: DerivationPath = "m/44'/1'/0'/0/0".parse().unwrap();
        let secp = Secp256k1::new();
        let sk = path
            .derive_priv_ecdsa_for_master_seed(&PARITY_SEED, Network::Testnet)
            .expect("derive")
            .private_key;
        let pk = PublicKey::from_secret_key(&secp, &sk);
        assert_eq!(
            hex::encode(pk.serialize()),
            PINNED_TESTNET_PUBKEY_HEX,
            "derived pubkey drifted from the pinned vector — a derivation regression"
        );
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
                    passphrase: Some(zeroize::Zeroizing::new(SENTINEL_PASSPHRASE.to_string())),
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
            // Extended-public-key export likewise has no derivation tree to
            // walk for a single key — it rejects rather than mis-deriving.
            let xpub_err = signer
                .extended_public_key(&path)
                .await
                .expect_err("wrong kind");
            assert!(matches!(xpub_err, DetSignerError::WrongSecretKind));
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
