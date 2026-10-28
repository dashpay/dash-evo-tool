//! DashPay contact QR codes (DIP-0015 auto-accept) through platform-wallet.
//!
//! platform-wallet builds the QR (`dash:?du=<username>&dapk=<key>`) and picks
//! its expiry; it asks the host for one thing only, the auto-accept private key
//! at `m/9'/coin'/16'/expiry'`. [`AutoAcceptKeyExport`] derives that key from
//! the wallet seed the secret seam lends for one operation, and refuses every
//! other path and every other crypto request.

use std::sync::Arc;

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, DerivationPath, ExtendedPubKey};
use dash_sdk::dpp::key_wallet::wallet::Wallet;
use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;
use dash_sdk::dpp::secp256k1::{PublicKey, SecretKey};
use platform_wallet::error::PlatformWalletError;
use platform_wallet::wallet::identity::crypto::auto_accept::{
    auto_accept_derivation_path, derive_auto_accept_private_key,
};
use platform_wallet::wallet::identity::network::{
    ContactCryptoProvider, ContactInfoOpened, ContactInfoSealed,
};
use zeroize::Zeroizing;

use crate::backend_task::dashpay::errors::DashPayError;
use crate::backend_task::error::TaskError;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::wallet_backend::{SecretScope, WalletBackend};

impl WalletBackend {
    /// Build a contact QR code for `identity`, valid for one hour.
    ///
    /// `username` is the identity's DPNS name to put in the code; pass an
    /// empty string to have platform-wallet look it up on Platform.
    ///
    /// # Errors
    ///
    /// The identity has no wallet in this app
    /// ([`DashPayError::ContactQrIdentityNotFromWallet`]) or its wallet is not
    /// loaded; the user cancels the wallet password prompt; or
    /// platform-wallet cannot build the code, typically because the identity
    /// has no username ([`DashPayError::ContactQrBuildFailed`]).
    pub(crate) async fn dashpay_build_contact_qr(
        &self,
        identity: &QualifiedIdentity,
        username: &str,
    ) -> Result<String, TaskError> {
        let owner = identity.identity.id();
        let seed_hash = identity
            .dashpay_wallet_seed_hash()
            .ok_or(DashPayError::ContactQrIdentityNotFromWallet)?;
        let wallet = self.resolve_wallet(&seed_hash).await?;
        let network = identity.network;
        let identity_wallet = wallet.identity();
        let dashpay = identity_wallet.dashpay();

        self.secret_access()
            .with_secret_session(&SecretScope::HdSeed { seed_hash }, async |session| {
                let plaintext = session.plaintext();
                let seed = plaintext
                    .expose_hd_seed()
                    .ok_or(DashPayError::ContactQrWalletSeedUnavailable)?;
                let provider = AutoAcceptKeyExport::new(seed, network);
                dashpay
                    .build_auto_accept_qr(&owner, username, &provider)
                    .await
                    .map_err(|source| {
                        DashPayError::ContactQrBuildFailed {
                            source: Arc::new(source),
                        }
                        .into()
                    })
            })
            .await
    }
}

/// Serves platform-wallet the DIP-0015 auto-accept key, and nothing else.
///
/// Holds a borrow of the seed for one operation only; it never copies the
/// seed out of that operation.
pub(crate) struct AutoAcceptKeyExport<'a> {
    seed: &'a [u8; 64],
    network: Network,
}

impl<'a> AutoAcceptKeyExport<'a> {
    pub(crate) fn new(seed: &'a [u8; 64], network: Network) -> Self {
        Self { seed, network }
    }

    /// The expiry index of `path` when it is exactly this network's
    /// auto-accept path `m/9'/coin'/16'/expiry'`, all hardened.
    #[expect(
        clippy::result_large_err,
        reason = "ContactCryptoProvider fixes PlatformWalletError as the error type"
    )]
    fn auto_accept_expiry(&self, path: &DerivationPath) -> Result<u32, PlatformWalletError> {
        let expected = auto_accept_derivation_path(self.network, 0)?;
        let (path, expected) = (path.as_ref(), expected.as_ref());
        match path {
            [feature @ .., ChildNumber::Hardened { index }]
                if path.len() == expected.len() && feature == &expected[..expected.len() - 1] =>
            {
                Ok(*index)
            }
            _ => Err(not_auto_accept()),
        }
    }
}

fn not_auto_accept() -> PlatformWalletError {
    PlatformWalletError::InvalidParameter(
        "this crypto provider serves the DashPay auto-accept key only".to_string(),
    )
}

#[async_trait::async_trait]
impl ContactCryptoProvider for AutoAcceptKeyExport<'_> {
    async fn receiving_xpub(
        &self,
        _path: &DerivationPath,
    ) -> Result<ExtendedPubKey, PlatformWalletError> {
        Err(not_auto_accept())
    }

    async fn ecdh_shared_secret(
        &self,
        _path: &DerivationPath,
        _peer: &PublicKey,
    ) -> Result<Zeroizing<[u8; 32]>, PlatformWalletError> {
        Err(not_auto_accept())
    }

    async fn export_auto_accept_private_key(
        &self,
        path: &DerivationPath,
    ) -> Result<SecretKey, PlatformWalletError> {
        let expiry = self.auto_accept_expiry(path)?;
        let wallet =
            Wallet::from_seed_bytes(*self.seed, self.network, WalletAccountCreationOptions::None)
                .map_err(|error| PlatformWalletError::KeyDerivation(error.to_string()))?;
        derive_auto_accept_private_key(&wallet, self.network, expiry)
    }

    async fn export_invitation_private_key(
        &self,
        _path: &DerivationPath,
    ) -> Result<SecretKey, PlatformWalletError> {
        Err(not_auto_accept())
    }

    async fn account_reference(
        &self,
        _path: &DerivationPath,
        _compact_xpub: &[u8],
        _account_index: u32,
        _version: u32,
    ) -> Result<u32, PlatformWalletError> {
        Err(not_auto_accept())
    }

    async fn unmask_account_reference(
        &self,
        _path: &DerivationPath,
        _compact_xpub: &[u8],
        _account_reference: u32,
    ) -> Result<(u32, u32), PlatformWalletError> {
        Err(not_auto_accept())
    }

    async fn contact_info_seal(
        &self,
        _root_path: &DerivationPath,
        _derivation_index: u32,
        _contact_id: &[u8; 32],
        _private_data_plaintext: &[u8],
        _private_data_iv: &[u8; 16],
    ) -> Result<ContactInfoSealed, PlatformWalletError> {
        Err(not_auto_accept())
    }

    async fn contact_info_open(
        &self,
        _root_path: &DerivationPath,
        _derivation_index: u32,
        _enc_to_user_id: &[u8; 32],
        _private_data_blob: &[u8],
    ) -> Result<ContactInfoOpened, PlatformWalletError> {
        Err(not_auto_accept())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::dashpay_contact_qr::ContactQrCode;
    use dash_sdk::platform::Identifier;
    use platform_wallet::wallet::identity::crypto::auto_accept::{
        AUTO_ACCEPT_TTL_SECS, verify_auto_accept_proof_with_pubkey,
    };
    use std::str::FromStr;

    /// Generated each run, so no seed bytes are committed.
    static SEED: std::sync::LazyLock<[u8; 64]> = std::sync::LazyLock::new(rand::random);
    const NETWORK: Network = Network::Testnet;
    const EXPIRY: u32 = 1_900_000_000;

    /// The key the wallet seed itself yields at `expiry`, derived by
    /// platform-wallet with no DET code in between.
    fn seed_key(seed: &[u8; 64], network: Network, expiry: u32) -> SecretKey {
        let wallet = Wallet::from_seed_bytes(*seed, network, WalletAccountCreationOptions::None)
            .expect("wallet");
        derive_auto_accept_private_key(&wallet, network, expiry).expect("derive")
    }

    #[tokio::test]
    async fn exports_the_key_the_wallet_seed_derives() {
        let provider = AutoAcceptKeyExport::new(&SEED, NETWORK);
        let path = auto_accept_derivation_path(NETWORK, EXPIRY).expect("path");
        let key = provider
            .export_auto_accept_private_key(&path)
            .await
            .expect("export");
        assert_eq!(key, seed_key(&SEED, NETWORK, EXPIRY));
    }

    /// Only this network's `m/9'/coin'/16'/expiry'` is served: no other
    /// feature, coin, depth or a non-hardened expiry.
    #[tokio::test]
    async fn refuses_every_path_but_the_auto_accept_one() {
        let provider = AutoAcceptKeyExport::new(&SEED, NETWORK);
        for path in [
            "m",
            "m/9'/1'/16'",
            "m/9'/1'/16'/5",
            "m/9'/1'/16'/5'/0'",
            "m/9'/1'/15'/5'",
            "m/9'/1'/5'/5'",
            "m/9'/5'/16'/5'",
            "m/44'/1'/16'/5'",
            "m/9/1'/16'/5'",
        ] {
            let path = DerivationPath::from_str(path).expect("path");
            assert!(
                provider
                    .export_auto_accept_private_key(&path)
                    .await
                    .is_err(),
                "{path}"
            );
        }
    }

    #[tokio::test]
    async fn refuses_every_other_crypto_request() {
        let provider = AutoAcceptKeyExport::new(&SEED, NETWORK);
        let path = auto_accept_derivation_path(NETWORK, EXPIRY).expect("path");
        assert!(provider.receiving_xpub(&path).await.is_err());
        let peer = PublicKey::from_secret_key(&seed_key(&SEED, NETWORK, EXPIRY));
        assert!(provider.ecdh_shared_secret(&path, &peer).await.is_err());
        assert!(provider.export_invitation_private_key(&path).await.is_err());
        assert!(provider.account_reference(&path, &[], 0, 0).await.is_err());
        assert!(
            provider
                .unmask_account_reference(&path, &[], 0)
                .await
                .is_err()
        );
        assert!(
            provider
                .contact_info_seal(&path, 0, &[0; 32], b"", &[0; 16])
                .await
                .is_err()
        );
        assert!(
            provider
                .contact_info_open(&path, 0, &[0; 32], b"")
                .await
                .is_err()
        );
    }

    /// The QR the backend builds carries the identity's username and the key
    /// its wallet seed derives, expires in one hour, and a scanner's proof
    /// made with it verifies against that seed's public key.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_qr_carries_the_wallet_seed_auto_accept_key() {
        use crate::model::wallet::Wallet as DetWallet;
        use crate::model::wallet::birth_height::WalletOrigin;
        use std::collections::BTreeMap;

        let dir = tempfile::tempdir().expect("temp dir");
        let context = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let backend = context.wallet_backend().expect("backend wired");
        let seed: [u8; 64] = rand::random();
        let wallet = DetWallet::new_from_seed(seed, NETWORK, Some("w".into()), None).expect("w");
        let (seed_hash, wallet) = context
            .register_wallet(wallet, &seed, WalletOrigin::Fresh)
            .expect("register wallet");
        backend
            .register_wallet_from_seed(&seed_hash, &seed, Some(0))
            .await
            .expect("register upstream");
        let owner = Identifier::from([0x11; 32]);
        let mut identity = crate::context::test_support::bare_user_identity(owner, NETWORK);
        identity.associated_wallets = BTreeMap::from([(seed_hash, wallet)]);

        let before = now_secs();
        let uri = backend
            .dashpay_build_contact_qr(&identity, "alice")
            .await
            .expect("build QR");
        let qr = ContactQrCode::parse(&uri).expect("parse");

        assert!(uri.starts_with("dash:?du=alice&dapk="), "{uri}");
        assert_eq!(qr.username(), "alice");
        let expiry = u64::from(qr.expires_at());
        let ttl = u64::from(AUTO_ACCEPT_TTL_SECS);
        assert!((before + ttl..=now_secs() + ttl).contains(&expiry));

        let scanner = Identifier::from([0x22; 32]);
        let proof = qr.sign_proof(&scanner, &owner, 3);
        let owner_key = PublicKey::from_secret_key(&seed_key(&seed, NETWORK, qr.expires_at()));
        assert!(verify_auto_accept_proof_with_pubkey(
            &owner_key, &proof, &scanner, &owner, 3
        ));
    }

    #[tokio::test]
    async fn an_identity_without_a_wallet_cannot_build_a_qr() {
        let dir = tempfile::tempdir().expect("temp dir");
        let context = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let backend = context.wallet_backend().expect("backend wired");
        let identity =
            crate::context::test_support::bare_user_identity(Identifier::from([5; 32]), NETWORK);

        let result = backend.dashpay_build_contact_qr(&identity, "alice").await;

        assert!(
            matches!(
                result,
                Err(TaskError::DashPay(
                    DashPayError::ContactQrIdentityNotFromWallet
                ))
            ),
            "{result:?}"
        );
    }

    fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_secs()
    }
}
