//! DashPay contact QR codes (DIP-15 auto-accept) through upstream
//! platform-wallet.
//!
//! The owner's auto-accept key is derived from the identity's wallet seed at
//! `m/9'/coin'/16'/expiry'`. The seed is read only through the secret seam,
//! for one operation, and only [`AutoAcceptKeyExport`] sees it: a
//! [`ContactCryptoProvider`] that exports auto-accept keys and refuses every
//! other request.

use std::sync::Arc;

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, DerivationPath, ExtendedPubKey};
use dash_sdk::dpp::secp256k1::{PublicKey, SecretKey};
use platform_wallet::error::PlatformWalletError;
use platform_wallet::wallet::identity::crypto::auto_accept::auto_accept_derivation_path;
use platform_wallet::{ContactCryptoProvider, ContactInfoOpened, ContactInfoSealed};
use zeroize::Zeroizing;

use super::{SecretScope, WalletBackend};
use crate::backend_task::error::TaskError;
use crate::model::qualified_identity::QualifiedIdentity;

impl WalletBackend {
    /// The contact QR code of `identity` (`dash:?du=<username>&dapk=<key>`),
    /// built by upstream's `DashPayView::build_auto_accept_qr`: valid for
    /// upstream's auto-accept lifetime, naming the identity's first cached
    /// username (or one upstream finds on Platform).
    ///
    /// # Errors
    ///
    /// [`TaskError::ContactQrWalletMissing`] when the identity has no wallet
    /// on this device, the seam's error when the seed cannot be read, and
    /// [`TaskError::ContactQrBuildFailed`] when upstream cannot build the code.
    pub(crate) async fn dashpay_build_auto_accept_qr(
        &self,
        identity: &QualifiedIdentity,
    ) -> Result<String, TaskError> {
        let seed_hash = identity
            .dashpay_wallet_seed_hash()
            .ok_or(TaskError::ContactQrWalletMissing)?;
        let wallet = self.resolve_wallet(&seed_hash).await?;
        let owner = identity.identity.id();
        // Upstream resolves a name on Platform when given none.
        let username = identity
            .dpns_names
            .first()
            .map(|name| {
                let name = name.name.trim();
                name.strip_suffix(".dash").unwrap_or(name).to_string()
            })
            .unwrap_or_default();
        let network = identity.network;
        let identity_wallet = wallet.identity();
        let dashpay = identity_wallet.dashpay();

        self.secret_access()
            .with_secret_session(&SecretScope::HdSeed { seed_hash }, async |session| {
                let plaintext = session.plaintext();
                let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;
                let provider = AutoAcceptKeyExport::new(seed, network);
                dashpay
                    .build_auto_accept_qr(&owner, &username, &provider)
                    .await
                    .map_err(|source| TaskError::ContactQrBuildFailed {
                        source: Arc::new(source),
                    })
            })
            .await
    }
}

/// Exports the DIP-15 auto-accept private key of one wallet seed, and nothing
/// else: a request for any other path, or for another network's path, is
/// refused.
///
/// Borrows the seed for one secret session; it never copies it out.
pub(crate) struct AutoAcceptKeyExport<'a> {
    seed: &'a [u8; 64],
    network: Network,
}

impl<'a> AutoAcceptKeyExport<'a> {
    pub(crate) fn new(seed: &'a [u8; 64], network: Network) -> Self {
        Self { seed, network }
    }
}

/// The provider serves auto-accept key exports only.
fn not_auto_accept() -> PlatformWalletError {
    PlatformWalletError::InvalidParameter(
        "this crypto provider exports DashPay auto-accept keys only".to_string(),
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
        // Only `m/9'/coin'/16'/expiry'` of this network: the hardened leaf is
        // the expiry, and the whole path must be the one upstream builds for it.
        let expiry = match path.into_iter().last() {
            Some(ChildNumber::Hardened { index }) => *index,
            _ => return Err(not_auto_accept()),
        };
        if auto_accept_derivation_path(self.network, expiry)? != *path {
            return Err(not_auto_accept());
        }
        let extended = path
            .derive_priv_ecdsa_for_master_seed(self.seed, self.network)
            .map_err(|error| PlatformWalletError::KeyDerivation(error.to_string()))?;
        let secret = Zeroizing::new(*extended.private_key.to_bytes());
        SecretKey::from_secret_bytes(*secret)
            .map_err(|error| PlatformWalletError::KeyDerivation(error.to_string()))
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
    use crate::model::dashpay::ContactQr;
    use dash_sdk::dpp::key_wallet::wallet::Wallet;
    use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;
    use dash_sdk::platform::Identifier;
    use platform_wallet::wallet::identity::crypto::auto_accept::{
        derive_auto_accept_private_key, encode_auto_accept_key_blob, encode_dashpay_contact_uri,
        verify_auto_accept_proof,
    };

    const NETWORK: Network = Network::Testnet;
    const EXPIRY: u32 = 1_900_000_000;

    /// Regression: DET derived the auto-accept key from an identity
    /// ENCRYPTION key's private bytes, so no other DashPay client could
    /// verify its proofs. The exported key is the one upstream derives from
    /// the wallet seed.
    #[tokio::test]
    async fn the_exported_key_is_the_wallets_dip15_auto_accept_key() {
        let seed = Zeroizing::new(rand::random::<[u8; 64]>());
        let wallet = Wallet::from_seed_bytes(*seed, NETWORK, WalletAccountCreationOptions::None)
            .expect("a test wallet");
        let provider = AutoAcceptKeyExport::new(&seed, NETWORK);

        let path = auto_accept_derivation_path(NETWORK, EXPIRY).expect("a path");
        let exported = provider
            .export_auto_accept_private_key(&path)
            .await
            .expect("an auto-accept path");
        assert_eq!(
            exported,
            derive_auto_accept_private_key(&wallet, NETWORK, EXPIRY).expect("upstream key")
        );
    }

    /// Interop end to end: the code the owner's provider feeds upstream's
    /// QR codec is read by DET's scanner, whose proof upstream verifies
    /// against the owner's wallet.
    #[tokio::test]
    async fn a_scanned_code_yields_a_proof_the_owner_verifies() {
        let seed = Zeroizing::new(rand::random::<[u8; 64]>());
        let wallet = Wallet::from_seed_bytes(*seed, NETWORK, WalletAccountCreationOptions::None)
            .expect("a test wallet");
        let provider = AutoAcceptKeyExport::new(&seed, NETWORK);
        let key = provider
            .export_auto_accept_private_key(
                &auto_accept_derivation_path(NETWORK, EXPIRY).expect("a path"),
            )
            .await
            .expect("a key");
        let uri = encode_dashpay_contact_uri("alice", &encode_auto_accept_key_blob(&key, EXPIRY));

        let now = u64::from(EXPIRY) - 60;
        let (sender, owner) = (Identifier::new([1; 32]), Identifier::new([2; 32]));
        let proof = ContactQr::parse(&uri, now)
            .expect("valid")
            .sign_proof(&sender, &owner, 3, now)
            .expect("a proof");
        assert!(
            verify_auto_accept_proof(&wallet, NETWORK, &proof, &sender, &owner, 3)
                .expect("verifiable")
        );
    }

    /// Only an auto-accept path on the provider's network is exported; every
    /// other request is refused.
    #[tokio::test]
    async fn everything_but_an_auto_accept_key_is_refused() {
        let seed = Zeroizing::new(rand::random::<[u8; 64]>());
        let provider = AutoAcceptKeyExport::new(&seed, NETWORK);

        let mainnet = auto_accept_derivation_path(Network::Mainnet, EXPIRY).expect("a path");
        let identity_key: DerivationPath = vec![
            ChildNumber::from_hardened_idx(9).expect("valid"),
            ChildNumber::from_hardened_idx(1).expect("valid"),
            ChildNumber::from_hardened_idx(5).expect("valid"),
            ChildNumber::from_hardened_idx(0).expect("valid"),
        ]
        .into();
        let mut unhardened = auto_accept_derivation_path(NETWORK, EXPIRY)
            .expect("a path")
            .into_iter()
            .copied()
            .collect::<Vec<_>>();
        unhardened.pop();
        unhardened.push(ChildNumber::from_normal_idx(EXPIRY & 0x7FFF_FFFF).expect("valid"));
        for path in [mainnet, identity_key, DerivationPath::from(unhardened)] {
            assert!(
                provider
                    .export_auto_accept_private_key(&path)
                    .await
                    .is_err(),
                "{path}"
            );
        }

        let path = auto_accept_derivation_path(NETWORK, EXPIRY).expect("a path");
        let peer = PublicKey::from_secret_key(&SecretKey::from_secret_bytes([7; 32]).expect("key"));
        assert!(provider.receiving_xpub(&path).await.is_err());
        assert!(provider.ecdh_shared_secret(&path, &peer).await.is_err());
        assert!(provider.account_reference(&path, &[], 0, 0).await.is_err());
        assert!(
            provider
                .unmask_account_reference(&path, &[], 0)
                .await
                .is_err()
        );
        assert!(
            provider
                .contact_info_seal(&path, 0, &[0; 32], &[], &[0; 16])
                .await
                .is_err()
        );
        assert!(
            provider
                .contact_info_open(&path, 0, &[0; 32], &[])
                .await
                .is_err()
        );
    }
}
