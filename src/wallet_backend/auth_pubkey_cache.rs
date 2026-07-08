//! DET-side identity-authentication public-key cache view (D4b).
//!
//! [`AuthPubkeyCacheView`] is the only doorway DET code uses to read or
//! write the memoised identity-authentication ECDSA public keys for an HD
//! wallet (see [`AuthPubkeyCache`]). The view borrows a shared [`DetKv`]
//! handle pointing at `det-app.sqlite` and stores one whole-blob entry
//! per wallet under a colon-prefixed, network-scoped key:
//!
//! ```text
//! <network>:auth_pubkeys:<seed_hash_base58>
//! ```
//!
//! Unlike [`WalletMetaView`](crate::wallet_backend::WalletMetaView) — which
//! must use the global scope because it renders the picker before any
//! wallet is registered — this cache is only ever read or written from
//! paths that already resolved the wallet's seed (identity load/discover,
//! bootstrap). The wallet therefore exists, so the entry is scoped to
//! [`DetScope::Wallet`] and cascades when the wallet is removed.
//!
//! The cache is an optimisation: a missing or unreadable blob degrades to
//! a cold [`AuthPubkeyCache::default`] (empty map), and the read path
//! self-heals via one just-in-time seed derivation that repopulates it.
//! Correctness never depends on the cache being present.

use std::sync::Arc;

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::dashcore::base58;

use crate::backend_task::error::TaskError;
use crate::model::wallet::WalletSeedHash;
use crate::model::wallet::auth_pubkey_cache::AuthPubkeyCache;
use crate::wallet_backend::kv::KvAdapterError;
use crate::wallet_backend::{DetKv, DetScope};

/// Colon-separated namespace for the per-wallet auth-pubkey blob. The
/// full key is `<network>:auth_pubkeys:<seed_hash_base58>`.
pub(crate) const KEY_INFIX: &str = ":auth_pubkeys:";

/// Build the canonical k/v key for a wallet's auth-pubkey cache blob.
pub(crate) fn key_for(network: Network, seed_hash: &WalletSeedHash) -> String {
    let net = network_prefix(network);
    let hash = base58::encode_slice(seed_hash);
    format!("{net}{KEY_INFIX}{hash}")
}

/// Cross-network prefix `<network>:` used by every entry key. Matches the
/// vocabulary of [`WalletMetaView`](crate::wallet_backend::WalletMetaView)
/// and `resolve_spv_storage_dir`.
fn network_prefix(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "mainnet",
        Network::Testnet => "testnet",
        Network::Devnet => "devnet",
        Network::Regtest => "regtest",
    }
}

/// View borrowing a shared [`DetKv`] handle. Cheap to construct, so
/// callers build one per operation rather than threading it.
pub struct AuthPubkeyCacheView<'a> {
    kv: &'a Arc<DetKv>,
}

impl<'a> AuthPubkeyCacheView<'a> {
    /// Borrow a [`DetKv`] handle as a typed auth-pubkey-cache view.
    pub fn new(kv: &'a Arc<DetKv>) -> Self {
        Self { kv }
    }

    /// Load the cache for one wallet.
    ///
    /// Returns an empty (cold) [`AuthPubkeyCache`] when the key is absent
    /// or the blob fails to decode (logged) — the read path self-heals,
    /// so a corrupt blob must never block identity load/discovery.
    pub fn get(&self, network: Network, seed_hash: &WalletSeedHash) -> AuthPubkeyCache {
        let key = key_for(network, seed_hash);
        match self
            .kv
            .get::<AuthPubkeyCache>(DetScope::Wallet(seed_hash), &key)
        {
            Ok(Some(cache)) => cache,
            Ok(None) => AuthPubkeyCache::default(),
            Err(e) => {
                tracing::warn!(
                    target = "wallet_backend::auth_pubkey_cache",
                    key = %key,
                    error = ?e,
                    "Failed to read auth-pubkey cache; treating as cold",
                );
                AuthPubkeyCache::default()
            }
        }
    }

    /// Whole-blob upsert of the cache for one wallet. The map is tiny
    /// (a handful of identities x a few keys) and writes only happen on
    /// cold-cache first-touch, so a whole-blob write matches the
    /// `WalletMeta` discipline — no need for row-granular storage.
    pub fn put(
        &self,
        network: Network,
        seed_hash: &WalletSeedHash,
        cache: &AuthPubkeyCache,
    ) -> Result<(), TaskError> {
        let key = key_for(network, seed_hash);
        self.kv
            .put(DetScope::Wallet(seed_hash), &key, cache)
            .map_err(map_kv_error_to_task_error)
    }

    /// Delete the cache for one wallet. Idempotent — a missing key
    /// returns `Ok(())`. The wallet-scope cascade is the real deletion
    /// mechanism in production; this direct delete exists for tests.
    #[cfg(test)]
    pub fn delete(&self, network: Network, seed_hash: &WalletSeedHash) -> Result<(), TaskError> {
        let key = key_for(network, seed_hash);
        self.kv
            .delete(DetScope::Wallet(seed_hash), &key)
            .map_err(map_kv_error_to_task_error)
    }
}

/// Auth-pubkey-cache adapter errors funnel into the dedicated
/// [`TaskError::KvSidecarStorage`] envelope.
fn map_kv_error_to_task_error(e: KvAdapterError) -> TaskError {
    TaskError::KvSidecarStorage {
        sidecar: "auth_pubkey_cache",
        source: Box::new(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use dash_sdk::dpp::dashcore::PublicKey;
    use dash_sdk::dpp::dashcore::secp256k1::{
        PublicKey as Secp256k1PublicKey, Secp256k1, SecretKey,
    };

    use crate::wallet_backend::kv_test_support::InMemoryKv;

    fn kv() -> Arc<DetKv> {
        Arc::new(DetKv::from_store(Arc::new(InMemoryKv::default())))
    }

    fn pubkey(seed: u8) -> PublicKey {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[seed.max(1); 32]).expect("valid secret key");
        PublicKey::new(Secp256k1PublicKey::from_secret_key(&secp, &sk))
    }

    /// AUTH-CACHE-VIEW-001 — a written cache round-trips through `get`
    /// for the same wallet.
    #[test]
    fn put_then_get_round_trips() {
        let kv = kv();
        let view = AuthPubkeyCacheView::new(&kv);
        let seed: WalletSeedHash = [0x11; 32];
        let mut cache = AuthPubkeyCache::default();
        cache.insert(Network::Testnet, 0, 0, &pubkey(5));
        cache.insert(Network::Testnet, 1, 3, &pubkey(6));
        view.put(Network::Testnet, &seed, &cache).expect("put");
        let got = view.get(Network::Testnet, &seed);
        assert_eq!(got, cache);
    }

    /// AUTH-CACHE-VIEW-002 — `get` on an absent key returns a cold
    /// (empty) cache rather than erroring; this is the read-path
    /// graceful-degradation contract.
    #[test]
    fn get_missing_returns_cold_default() {
        let kv = kv();
        let view = AuthPubkeyCacheView::new(&kv);
        let seed: WalletSeedHash = [0x66; 32];
        let got = view.get(Network::Devnet, &seed);
        assert!(got.is_empty());
        assert_eq!(got, AuthPubkeyCache::default());
    }

    /// AUTH-CACHE-VIEW-003 — entries do not leak across networks: the
    /// same seed hash on two networks yields two independent blobs.
    #[test]
    fn get_partitions_by_network() {
        let kv = kv();
        let view = AuthPubkeyCacheView::new(&kv);
        let seed: WalletSeedHash = [0x33; 32];
        let mut mainnet = AuthPubkeyCache::default();
        mainnet.insert(Network::Mainnet, 0, 0, &pubkey(11));
        let mut testnet = AuthPubkeyCache::default();
        testnet.insert(Network::Testnet, 0, 0, &pubkey(22));
        view.put(Network::Mainnet, &seed, &mainnet).unwrap();
        view.put(Network::Testnet, &seed, &testnet).unwrap();
        assert_eq!(view.get(Network::Mainnet, &seed), mainnet);
        assert_eq!(view.get(Network::Testnet, &seed), testnet);
    }

    /// AUTH-CACHE-VIEW-004 — `put` is an upsert; a second write replaces
    /// the prior blob.
    #[test]
    fn put_upserts() {
        let kv = kv();
        let view = AuthPubkeyCacheView::new(&kv);
        let seed: WalletSeedHash = [0x22; 32];
        let mut first = AuthPubkeyCache::default();
        first.insert(Network::Mainnet, 0, 0, &pubkey(1));
        view.put(Network::Mainnet, &seed, &first).unwrap();
        let mut second = AuthPubkeyCache::default();
        second.insert(Network::Mainnet, 0, 0, &pubkey(2));
        view.put(Network::Mainnet, &seed, &second).unwrap();
        assert_eq!(view.get(Network::Mainnet, &seed), second);
    }

    /// AUTH-CACHE-VIEW-005 — `delete` is idempotent and removes the blob.
    #[test]
    fn delete_is_idempotent() {
        let kv = kv();
        let view = AuthPubkeyCacheView::new(&kv);
        let seed: WalletSeedHash = [0x55; 32];
        view.delete(Network::Testnet, &seed).expect("delete absent");
        let mut cache = AuthPubkeyCache::default();
        cache.insert(Network::Testnet, 0, 0, &pubkey(9));
        view.put(Network::Testnet, &seed, &cache).unwrap();
        view.delete(Network::Testnet, &seed).expect("first delete");
        view.delete(Network::Testnet, &seed).expect("second delete");
        assert!(view.get(Network::Testnet, &seed).is_empty());
    }

    /// AUTH-CACHE-VIEW-006 — the canonical key shape uses the
    /// `<network>:auth_pubkeys:<base58>` layout. Locks the shape so a
    /// future change needs an explicit migration.
    #[test]
    fn key_for_uses_base58_seed_hash() {
        let seed: WalletSeedHash = [0xAB; 32];
        let key = key_for(Network::Mainnet, &seed);
        assert!(key.starts_with("mainnet:auth_pubkeys:"));
        let suffix = key.trim_start_matches("mainnet:auth_pubkeys:");
        let decoded = base58::decode(suffix).expect("base58 decodes");
        assert_eq!(decoded.as_slice(), seed.as_slice());
    }

    /// AUTH-CACHE-VIEW-007 — the cache is a *new* KV key, not a schema
    /// change: adding it must not bump the DET KV `SCHEMA_VERSION` byte
    /// (an additive new key is forward/backward tolerant). This is the
    /// load-bearing "no DivergentVersion / no schema bump" guard from the
    /// D4b design — a future maintainer who bumps the version trips here.
    #[test]
    fn does_not_bump_kv_schema_version() {
        assert_eq!(
            crate::wallet_backend::KV_SCHEMA_VERSION,
            1,
            "adding the auth-pubkey cache key must not bump the KV schema version"
        );
    }
}
