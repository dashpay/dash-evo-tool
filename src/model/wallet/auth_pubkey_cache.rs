//! Memoised identity-authentication ECDSA public keys (D4b).
//!
//! The identity-authentication derivation path is hardened at every
//! level, including the leaf `key_index'`, so its public keys cannot be
//! derived from a stored account xpub (BIP-32 forbids public-from-public
//! derivation across a hardened boundary). The seed is mandatory for the
//! first derivation of every key. To keep the *steady-state* read of
//! these keys seed-free, the already-derived public keys are memoised
//! here, keyed by `(network, identity_index, key_index)`.
//!
//! The cache stores **public material only** — a leak of these bytes is
//! not a secret leak (the keys are derivable and end up on-chain). It is
//! persisted in the DET KV sidecar under `DetScope::Wallet(seed_hash)`;
//! an absent key reads as [`AuthPubkeyCache::default`] (cold), and a cold
//! miss self-heals into a warm hit via one just-in-time seed derivation.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::dashcore::PublicKey;
use serde::{Deserialize, Serialize};

/// Serde-stable network tag for cache coordinates.
///
/// Persisted as part of the bincode key, so the discriminants are pinned
/// — `dashcore::Network` is not serialised directly because its repr is
/// not guaranteed stable across upstream versions. Keying on network
/// keeps mainnet/testnet/devnet/regtest entries distinct under one blob,
/// matching the per-network coin-type baked into the derivation path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum NetworkTag {
    /// Dash main network.
    Mainnet = 0,
    /// Dash test network.
    Testnet = 1,
    /// A development network.
    Devnet = 2,
    /// A local regression-test network.
    Regtest = 3,
}

impl From<Network> for NetworkTag {
    fn from(network: Network) -> Self {
        match network {
            Network::Mainnet => NetworkTag::Mainnet,
            Network::Testnet => NetworkTag::Testnet,
            Network::Devnet => NetworkTag::Devnet,
            Network::Regtest => NetworkTag::Regtest,
        }
    }
}

/// Coordinate uniquely identifying one cached identity-auth public key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AuthKeyCoord {
    /// Network the key belongs to — partitions cross-network entries.
    pub network: NetworkTag,
    /// Zero-based identity index within the wallet.
    pub identity_index: u32,
    /// Zero-based authentication key index within the identity.
    pub key_index: u32,
}

impl AuthKeyCoord {
    /// Build a coordinate from a `dashcore::Network` and the two indices.
    pub fn new(network: Network, identity_index: u32, key_index: u32) -> Self {
        Self {
            network: network.into(),
            identity_index,
            key_index,
        }
    }
}

/// Cached identity-authentication ECDSA public keys for one HD wallet.
///
/// Public material only. Values are 33-byte compressed `secp256k1`
/// public keys — there is no chain code to keep, because no further
/// public derivation is possible past the hardened leaf. The two readers
/// reconstruct both the serialized form and the hash160 from these
/// bytes. Uses a [`BTreeMap`] for deterministic bincode output, matching
/// the project's persisted-shape discipline (`WalletMeta`,
/// `AppSettings`).
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthPubkeyCache {
    /// `(network, identity_index, key_index)` -> 33-byte compressed
    /// pubkey. Stored as `Vec<u8>` (always exactly 33 bytes on write)
    /// because serde does not derive `Deserialize` for `[u8; 33]`; the
    /// length is re-validated on read via `PublicKey::from_slice`.
    pub entries: BTreeMap<AuthKeyCoord, Vec<u8>>,
}

impl AuthPubkeyCache {
    /// Look up a cached public key by coordinate.
    ///
    /// Returns `None` on a miss or when the stored bytes fail to parse as
    /// a compressed public key (a corrupt entry degrades to a cold miss,
    /// which the caller self-heals by re-deriving).
    pub fn get(&self, network: Network, identity_index: u32, key_index: u32) -> Option<PublicKey> {
        let coord = AuthKeyCoord::new(network, identity_index, key_index);
        let bytes = self.entries.get(&coord)?;
        PublicKey::from_slice(bytes).ok()
    }

    /// Insert (or overwrite) a derived public key at the given coordinate.
    ///
    /// Returns `true` when the entry was newly added or changed, `false`
    /// when an identical value was already present — lets warm/cold-fill
    /// callers skip a redundant persist.
    pub fn insert(
        &mut self,
        network: Network,
        identity_index: u32,
        key_index: u32,
        public_key: &PublicKey,
    ) -> bool {
        let coord = AuthKeyCoord::new(network, identity_index, key_index);
        let bytes = public_key.inner.serialize().to_vec();
        match self.entries.entry(coord) {
            Entry::Occupied(e) if *e.get() == bytes => false,
            Entry::Occupied(mut e) => {
                e.insert(bytes);
                true
            }
            Entry::Vacant(e) => {
                e.insert(bytes);
                true
            }
        }
    }

    /// Number of cached entries across all networks.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache holds no entries (a fresh / cold cache).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Terse `Debug` — entry count only, never the keys. Matches the
/// `WalletMeta` / `StoredSeedEnvelope` house style (public keys need no
/// redaction, but the blob is not log-useful expanded).
impl std::fmt::Debug for AuthPubkeyCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthPubkeyCache")
            .field("entries", &self.entries.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::dashcore::secp256k1::{
        PublicKey as Secp256k1PublicKey, Secp256k1, SecretKey,
    };

    /// Deterministic test public key from a one-byte seed.
    fn pubkey(seed: u8) -> PublicKey {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[seed.max(1); 32]).expect("valid secret key");
        let inner = Secp256k1PublicKey::from_secret_key(&secp, &sk);
        PublicKey::new(inner)
    }

    /// AUTH-CACHE-001 — a fresh cache is cold: no entries, every lookup
    /// misses. This is the "absent key reads as Default" contract.
    #[test]
    fn default_is_cold() {
        let cache = AuthPubkeyCache::default();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert!(cache.get(Network::Testnet, 0, 0).is_none());
    }

    /// AUTH-CACHE-002 — an inserted key round-trips through `get` and the
    /// returned `PublicKey` is byte-identical to the one inserted.
    #[test]
    fn insert_then_get_round_trips() {
        let mut cache = AuthPubkeyCache::default();
        let pk = pubkey(7);
        assert!(cache.insert(Network::Mainnet, 1, 2, &pk));
        let got = cache.get(Network::Mainnet, 1, 2).expect("hit");
        assert_eq!(got, pk);
        assert_eq!(got.inner.serialize(), pk.inner.serialize());
    }

    /// AUTH-CACHE-003 — coordinates are network-keyed: the same indices
    /// on a different network do not alias (SEC-001 class — cross-network
    /// key reuse — is structurally excluded).
    #[test]
    fn coordinates_are_network_keyed() {
        let mut cache = AuthPubkeyCache::default();
        let mainnet_pk = pubkey(11);
        let testnet_pk = pubkey(22);
        cache.insert(Network::Mainnet, 0, 0, &mainnet_pk);
        cache.insert(Network::Testnet, 0, 0, &testnet_pk);
        assert_eq!(cache.get(Network::Mainnet, 0, 0), Some(mainnet_pk));
        assert_eq!(cache.get(Network::Testnet, 0, 0), Some(testnet_pk));
        assert_ne!(
            cache.get(Network::Mainnet, 0, 0),
            cache.get(Network::Testnet, 0, 0)
        );
    }

    /// AUTH-CACHE-004 — `insert` reports change: a new coordinate and a
    /// value change both return `true`; re-inserting the same value
    /// returns `false` so callers can skip a redundant persist.
    #[test]
    fn insert_reports_change() {
        let mut cache = AuthPubkeyCache::default();
        let first = pubkey(3);
        let second = pubkey(4);
        assert!(cache.insert(Network::Devnet, 5, 6, &first), "new entry");
        assert!(
            !cache.insert(Network::Devnet, 5, 6, &first),
            "identical re-insert is a no-op"
        );
        assert!(
            cache.insert(Network::Devnet, 5, 6, &second),
            "value change is a write"
        );
        assert_eq!(cache.get(Network::Devnet, 5, 6), Some(second));
    }

    /// AUTH-CACHE-005 — the persisted shape round-trips through bincode
    /// with `BTreeMap` determinism, the same coverage `WalletMeta` and
    /// `AppSettings` carry. Adding a field that breaks decoding surfaces
    /// here.
    #[test]
    fn cache_round_trips_through_bincode() {
        let mut original = AuthPubkeyCache::default();
        original.insert(Network::Mainnet, 0, 0, &pubkey(1));
        original.insert(Network::Testnet, 3, 9, &pubkey(2));
        let bytes =
            bincode::serde::encode_to_vec(&original, bincode::config::standard()).expect("encode");
        let (decoded, _): (AuthPubkeyCache, _) =
            bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).expect("decode");
        assert_eq!(decoded, original);
        // Determinism: re-encoding the decoded value yields the same bytes.
        let bytes2 =
            bincode::serde::encode_to_vec(&decoded, bincode::config::standard()).expect("encode");
        assert_eq!(bytes, bytes2);
    }

    /// AUTH-CACHE-006 — the network tag discriminants are pinned. A
    /// reorder or renumber would silently invalidate every persisted
    /// blob, so lock the wire values explicitly.
    #[test]
    fn network_tag_discriminants_are_pinned() {
        assert_eq!(NetworkTag::Mainnet as u8, 0);
        assert_eq!(NetworkTag::Testnet as u8, 1);
        assert_eq!(NetworkTag::Devnet as u8, 2);
        assert_eq!(NetworkTag::Regtest as u8, 3);
    }

    /// AUTH-CACHE-007 — `Debug` reveals only the entry count, never the
    /// key bytes (house style; keeps logs terse).
    #[test]
    fn debug_is_terse() {
        let mut cache = AuthPubkeyCache::default();
        cache.insert(Network::Mainnet, 0, 0, &pubkey(1));
        let rendered = format!("{cache:?}");
        assert!(rendered.contains("entries: 1"), "got: {rendered}");
    }
}
