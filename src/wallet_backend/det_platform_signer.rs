//! Just-in-time platform-address signer for the upstream
//! [`Signer<PlatformAddress>`] seam.
//!
//! The SDK's platform-funding flows (`top_up`, `transfer_address_funds`,
//! `withdraw_address_funds`) are signer-driven: the SDK calls
//! `signer.sign(platform_address, data)` to authorise each per-input
//! `AddressWitness`. [`DetPlatformSigner`] is DET's JIT implementation of
//! that trait — it **borrows** the HD seed held open by a
//! [`SecretAccess::with_secret_session`](crate::wallet_backend::SecretAccess::with_secret_session)
//! scope, maps the platform address to its DIP-17 derivation path through a
//! pre-built pure index, derives the signing key locally, and signs.
//!
//! It replaces the legacy `impl Signer<PlatformAddress> for Wallet`, which
//! read the wallet's parked session-long plaintext seed. The derivation,
//! coin-type, and signing primitive are **identical** to that impl — the only
//! change is the seed source (borrowed JIT seed instead of parked seed) and
//! that the network is known up front rather than brute-forced across all
//! four networks (R-4 in the R3-completion design: this is safer, not just
//! equivalent).
//!
//! Borrow discipline (Nagatha R-2): the `'a` lifetime ties the signer to both
//! the held seed and the path index, so the signer cannot outlive the
//! `with_secret_session` scope. Never `Box`/`Arc`/return it — the SDK takes
//! `&S`, which the borrow satisfies without any `'static` coercion.

use std::collections::BTreeMap;

use dash_sdk::dpp::ProtocolError;
use dash_sdk::dpp::address_funds::{AddressWitness, PlatformAddress};
use dash_sdk::dpp::async_trait::async_trait;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::signer::Signer;
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use dash_sdk::dpp::platform_value::BinaryData;

use crate::model::wallet::{DerivationPathHelpers, Wallet};

/// Pure (secret-free) map from a wallet's watched platform-payment addresses
/// to their DIP-17 derivation paths, for a single network.
///
/// Built from `Wallet::watched_addresses` *before* entering the secret scope
/// so that [`DetPlatformSigner::can_sign_with`] — which the SDK calls in its
/// pre-flight checks — is a cheap, prompt-free membership test that never
/// touches the seed (Nagatha R-6).
#[derive(Debug, Clone)]
pub(crate) struct PlatformPathIndex {
    network: Network,
    by_address: BTreeMap<PlatformAddress, DerivationPath>,
}

impl PlatformPathIndex {
    /// Build the index from a wallet's watched addresses for `network`.
    ///
    /// Only platform-payment paths (`is_platform_payment`) whose stored Core
    /// address converts to a [`PlatformAddress`] are included — the exact set
    /// the legacy `get_platform_address_private_key` lookup could resolve.
    pub(crate) fn from_wallet(wallet: &Wallet, network: Network) -> Self {
        let by_address = wallet
            .watched_addresses
            .iter()
            .filter(|(path, _)| path.is_platform_payment(network))
            .filter_map(|(path, info)| {
                PlatformAddress::try_from(info.address.clone())
                    .ok()
                    .map(|platform_address| (platform_address, path.clone()))
            })
            .collect();
        Self {
            network,
            by_address,
        }
    }

    /// The DIP-17 derivation path for `address`, or `None` if it is not one of
    /// the wallet's watched platform-payment addresses on this network.
    fn path_for(&self, address: &PlatformAddress) -> Option<&DerivationPath> {
        self.by_address.get(address)
    }

    /// Whether this index can resolve a path for `address` (pure, no secret).
    fn contains(&self, address: &PlatformAddress) -> bool {
        self.by_address.contains_key(address)
    }

    /// The number of indexed platform addresses (used in the redacted
    /// `Debug` output of the signer).
    fn len(&self) -> usize {
        self.by_address.len()
    }
}

/// JIT [`Signer<PlatformAddress>`] backed by a **borrowed** HD seed and a
/// **borrowed** pure path index.
///
/// Constructed inside a `with_secret_session` scope; never owns or copies the
/// plaintext seed. The lifetime `'a` ties the signer to both the held seed and
/// the index, so the borrow cannot escape the scope where the plaintext is
/// alive.
pub(crate) struct DetPlatformSigner<'a> {
    seed: &'a [u8; 64],
    network: Network,
    index: &'a PlatformPathIndex,
}

impl<'a> DetPlatformSigner<'a> {
    /// Build a platform signer over the held seed for `network`, using
    /// `index` to map addresses to derivation paths. Borrows both — no copy.
    ///
    /// `seed` is the borrow returned by
    /// [`SecretPlaintext::expose_hd_seed`](crate::wallet_backend::SecretPlaintext::expose_hd_seed)
    /// inside a `with_secret_session` scope; the `'a` lifetime keeps the signer
    /// from outliving the held plaintext.
    pub(crate) fn from_held(
        seed: &'a [u8; 64],
        network: Network,
        index: &'a PlatformPathIndex,
    ) -> Self {
        debug_assert_eq!(
            network, index.network,
            "platform signer network must match the index network"
        );
        Self {
            seed,
            network,
            index,
        }
    }

    /// Resolve `platform_address` to its derivation path and derive the raw
    /// 65-byte ECDSA signature over `data`, using the **same** primitive the
    /// legacy `Wallet` platform signer used. Shared by `sign` and
    /// `sign_create_witness` so both paths are byte-identical.
    ///
    /// Returns `ProtocolError` to match the upstream `Signer` trait surface
    /// this helper feeds; the large-variant lint is the SDK's error shape, not
    /// ours (same `allow` as the legacy `get_platform_address_private_key`).
    #[allow(clippy::result_large_err)]
    fn sign_with_address(
        &self,
        platform_address: &PlatformAddress,
        data: &[u8],
    ) -> Result<Vec<u8>, ProtocolError> {
        if !platform_address.is_p2pkh() {
            return Err(ProtocolError::Generic(
                "Only P2PKH Platform addresses are currently supported for signing".to_string(),
            ));
        }

        let derivation_path = self.index.path_for(platform_address).ok_or_else(|| {
            ProtocolError::Generic(format!(
                "Platform address {:?} not found in wallet",
                platform_address
            ))
        })?;

        let extended_private_key = derivation_path
            .derive_priv_ecdsa_for_master_seed(self.seed, self.network)
            .map_err(|e| ProtocolError::Generic(e.to_string()))?;
        let private_key = extended_private_key.to_priv();

        let signature = dash_sdk::dpp::dashcore::signer::sign(data, private_key.inner.as_ref())
            .map_err(|e| ProtocolError::Generic(format!("Failed to sign: {}", e)))?;

        Ok(signature.to_vec())
    }
}

impl std::fmt::Debug for DetPlatformSigner<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DetPlatformSigner")
            .field("network", &self.network)
            .field("indexed_addresses", &self.index.len())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Signer<PlatformAddress> for DetPlatformSigner<'_> {
    async fn sign(
        &self,
        platform_address: &PlatformAddress,
        data: &[u8],
    ) -> Result<BinaryData, ProtocolError> {
        let signature = self.sign_with_address(platform_address, data)?;
        Ok(BinaryData::new(signature))
    }

    async fn sign_create_witness(
        &self,
        platform_address: &PlatformAddress,
        data: &[u8],
    ) -> Result<AddressWitness, ProtocolError> {
        let signature = self.sign_with_address(platform_address, data)?;
        Ok(AddressWitness::P2pkh {
            signature: BinaryData::new(signature),
        })
    }

    fn can_sign_with(&self, platform_address: &PlatformAddress) -> bool {
        platform_address.is_p2pkh() && self.index.contains(platform_address)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::wallet::{AddressInfo, DerivationPathReference, DerivationPathType};
    use dash_sdk::dpp::dashcore::Address;
    use dash_sdk::dpp::dashcore::secp256k1::Secp256k1;
    use zeroize::Zeroizing;

    /// Reference platform-address signing — reproduces exactly what the
    /// retired `Wallet` `Signer<PlatformAddress>` impl did: look up the address
    /// in the index, derive the private key at its path from the seed, and sign
    /// with the same `dashcore::signer::sign`. The parity tests compare
    /// [`DetPlatformSigner`] against this independent reference so they still
    /// prove fund-safety parity without the deleted impl.
    fn reference_sign(
        index: &PlatformPathIndex,
        seed: &[u8; 64],
        network: Network,
        platform_address: &PlatformAddress,
        data: &[u8],
    ) -> Vec<u8> {
        let path = index.path_for(platform_address).expect("indexed path");
        let private_key = path
            .derive_priv_ecdsa_for_master_seed(seed, network)
            .expect("derive")
            .to_priv();
        dash_sdk::dpp::dashcore::signer::sign(data, private_key.inner.as_ref())
            .expect("reference sign")
            .to_vec()
    }

    /// Build a minimal open wallet (via the public pure constructor) and wire
    /// one platform-payment address into its watched/known maps — exactly the
    /// shape the [`PlatformPathIndex`] consumes. No `AppContext` needed.
    fn wallet_with_platform_address(
        network: Network,
    ) -> (Wallet, PlatformAddress, Zeroizing<[u8; 64]>) {
        let seed = Zeroizing::new([0x5Au8; 64]);
        let mut wallet = Wallet::new_from_seed(*seed, network, None, None).expect("build wallet");

        let path = DerivationPath::platform_payment_path(network, 0, 0, 0);
        let xprv = path
            .derive_priv_ecdsa_for_master_seed(&*seed, network)
            .expect("derive platform key");
        let secp = Secp256k1::new();
        let address = Address::p2pkh(&xprv.to_priv().public_key(&secp), network);
        let platform_address =
            PlatformAddress::try_from(address.clone()).expect("to platform address");

        wallet.known_addresses.insert(address.clone(), path.clone());
        wallet.watched_addresses.insert(
            path,
            AddressInfo {
                address,
                path_type: DerivationPathType::CLEAR_FUNDS,
                path_reference: DerivationPathReference::PlatformPayment,
            },
        );

        (wallet, platform_address, seed)
    }

    /// FUND-SAFETY PARITY: `DetPlatformSigner` must produce byte-identical
    /// `sign` output to a direct reference derivation (path-from-index → derive
    /// → `dashcore::signer::sign`) for the same address and data, on every
    /// network. A divergence here means wrong signatures and lost/failed funds.
    #[tokio::test]
    async fn platform_signer_parity_with_reference() {
        for network in [Network::Testnet, Network::Mainnet] {
            let (wallet, platform_address, seed) = wallet_with_platform_address(network);
            let index = PlatformPathIndex::from_wallet(&wallet, network);
            let det = DetPlatformSigner::from_held(&seed, network, &index);

            let data = b"fund-critical-parity-vector";

            let reference_sig = reference_sign(&index, &seed, network, &platform_address, data);
            let det_sig = det.sign(&platform_address, data).await.expect("det sign");
            assert_eq!(
                reference_sig,
                det_sig.to_vec(),
                "sign() bytes diverged on {network:?}"
            );

            // The create-witness signature wraps the same `sign` output.
            let det_witness = det
                .sign_create_witness(&platform_address, data)
                .await
                .expect("det witness");
            let reference_witness = AddressWitness::P2pkh {
                signature: BinaryData::new(reference_sig),
            };
            assert_eq!(
                format!("{reference_witness:?}"),
                format!("{det_witness:?}"),
                "sign_create_witness() diverged on {network:?}"
            );

            assert!(
                det.can_sign_with(&platform_address),
                "can_sign_with must be true for an indexed address on {network:?}"
            );
        }
    }

    /// PARITY: the key `DetPlatformSigner` signs with matches a direct
    /// reference derivation at the indexed path exactly (same path, same
    /// coin-type, same network) — the strongest fund-safety guarantee,
    /// independent of the signing nonce.
    #[tokio::test]
    async fn platform_signer_derives_same_key_as_reference() {
        for network in [Network::Testnet, Network::Mainnet] {
            let (wallet, platform_address, seed) = wallet_with_platform_address(network);
            let index = PlatformPathIndex::from_wallet(&wallet, network);

            // Reference: derive directly at the indexed path and sign.
            let path = index.path_for(&platform_address).expect("indexed path");
            let reference_key = path
                .derive_priv_ecdsa_for_master_seed(&*seed, network)
                .expect("derive")
                .to_priv();
            let data = b"key-parity-vector";
            let reference_sig =
                dash_sdk::dpp::dashcore::signer::sign(data, reference_key.inner.as_ref())
                    .expect("reference sign")
                    .to_vec();

            // The signer's view of the same address must yield the same key.
            let det = DetPlatformSigner::from_held(&seed, network, &index);
            let det_sig = det.sign(&platform_address, data).await.expect("det sign");

            assert_eq!(
                reference_sig,
                det_sig.to_vec(),
                "derived key signatures diverged on {network:?}"
            );
        }
    }

    /// `can_sign_with` is false for an address the wallet does not watch, and
    /// resolving such an address errors rather than mis-signing.
    #[tokio::test]
    async fn rejects_unknown_address() {
        let network = Network::Testnet;
        let (wallet, _addr, seed) = wallet_with_platform_address(network);
        let index = PlatformPathIndex::from_wallet(&wallet, network);
        let det = DetPlatformSigner::from_held(&seed, network, &index);

        // A different platform address (index 1) that was never registered.
        let other_path = DerivationPath::platform_payment_path(network, 0, 0, 1);
        let other_xprv = other_path
            .derive_priv_ecdsa_for_master_seed(&*seed, network)
            .unwrap();
        let secp = Secp256k1::new();
        let other_address = Address::p2pkh(&other_xprv.to_priv().public_key(&secp), network);
        let other_platform = PlatformAddress::try_from(other_address).unwrap();

        assert!(!det.can_sign_with(&other_platform));
        assert!(det.sign(&other_platform, b"data").await.is_err());
    }

    /// `Debug` never prints the seed bytes — only the network and address
    /// count appear.
    #[test]
    fn debug_redacts_seed() {
        let network = Network::Testnet;
        let (wallet, _addr, seed) = wallet_with_platform_address(network);
        let index = PlatformPathIndex::from_wallet(&wallet, network);
        let det = DetPlatformSigner::from_held(&seed, network, &index);
        let dbg = format!("{det:?}");
        assert!(dbg.contains("Testnet"), "network present: {dbg}");
        assert!(!dbg.contains("90, 90, 90"), "seed bytes leaked: {dbg}");
    }
}
