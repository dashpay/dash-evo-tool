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

use std::sync::{Arc, Mutex, PoisonError};

use dash_sdk::dpp::dashcore::Network;

use crate::backend_task::error::TaskError;
use crate::model::wallet::WalletSeedHash;
use crate::model::wallet::auth_pubkey_cache::AuthPubkeyCache;
use crate::wallet_backend::DetKv;
use crate::wallet_backend::kv::{KvAdapterError, map_kv_storage_error};
#[cfg(test)]
use crate::wallet_backend::sidecar::sidecar_key;
use crate::wallet_backend::sidecar::{SidecarScope, SidecarValue, SidecarView};
use crate::wallet_backend::wallet_context::WalletContext;

/// Colon-separated namespace for the per-wallet auth-pubkey blob. The
/// full key is `<network>:auth_pubkeys:<seed_hash_base58>`.
pub(crate) const KEY_INFIX: &str = ":auth_pubkeys:";

/// Build the canonical k/v key for a wallet's auth-pubkey cache blob. The
/// generic view builds keys itself; this mirror exists for key-shape tests.
#[cfg(test)]
pub(crate) fn key_for(network: Network, seed_hash: &WalletSeedHash) -> String {
    sidecar_key(network, KEY_INFIX, seed_hash)
}

impl SidecarValue for AuthPubkeyCache {}

/// Typed auth-pubkey-cache sidecar (D4b). A thin wrapper over the generic
/// [`SidecarView`]. Unlike the metadata sidecars it is
/// [`SidecarScope::WalletById`]-scoped: it is only ever touched from paths that
/// already resolved the wallet's seed, so the entry cascades on wallet removal.
/// The cache is an optimisation — a missing or unreadable blob degrades to a
/// cold [`AuthPubkeyCache::default`] that the read path self-heals.
///
/// Every write is a read-modify-write of the whole per-wallet blob, so writers
/// go through [`Self::update`], which serialises them on `write_lock`.
///
/// Wallet removal pairs with the same lock: the wallet leaves the loaded
/// [`WalletContext`] first, then [`Self::delete`] clears its entry under the
/// lock, and [`Self::update`] writes only while the wallet is still loaded. A
/// writer that finished deriving after the removal therefore cannot recreate
/// an entry the wallet-scope cascade would never clean up.
pub struct AuthPubkeyCacheView<'a> {
    sidecar: SidecarView<'a, AuthPubkeyCache>,
    /// Serialises read-modify-writes of the blobs this view writes. Owned by
    /// the wallet backend, whose views are the only writers of its network's
    /// entries.
    write_lock: &'a Mutex<()>,
    /// Loaded wallets; writes for a wallet not in it are dropped. `None` only
    /// in unit tests of the storage behaviour itself.
    loaded_wallets: Option<&'a WalletContext>,
}

impl<'a> AuthPubkeyCacheView<'a> {
    /// Borrow a [`DetKv`] handle as a typed auth-pubkey-cache view whose
    /// writes serialise on `write_lock` and only land for wallets loaded in
    /// `loaded_wallets`.
    pub fn new(
        kv: &'a Arc<DetKv>,
        write_lock: &'a Mutex<()>,
        loaded_wallets: &'a WalletContext,
    ) -> Self {
        Self::with_wallet_gate(kv, write_lock, Some(loaded_wallets))
    }

    /// A view that writes regardless of which wallets are loaded. Tests of
    /// the storage behaviour only.
    #[cfg(test)]
    pub(crate) fn without_wallet_gate(kv: &'a Arc<DetKv>, write_lock: &'a Mutex<()>) -> Self {
        Self::with_wallet_gate(kv, write_lock, None)
    }

    fn with_wallet_gate(
        kv: &'a Arc<DetKv>,
        write_lock: &'a Mutex<()>,
        loaded_wallets: Option<&'a WalletContext>,
    ) -> Self {
        Self {
            sidecar: SidecarView::new(
                kv,
                KEY_INFIX,
                SidecarScope::WalletById,
                map_kv_error_to_task_error,
            ),
            write_lock,
            loaded_wallets,
        }
    }

    /// Load the cache for one wallet.
    ///
    /// Returns an empty (cold) [`AuthPubkeyCache`] when the key is absent
    /// or the blob fails to decode (logged) — the read path self-heals,
    /// so a corrupt blob must never block identity load/discovery.
    pub fn get(&self, network: Network, seed_hash: &WalletSeedHash) -> AuthPubkeyCache {
        self.sidecar.get(network, seed_hash).unwrap_or_default()
    }

    /// Apply `edit` to the freshly read cache for one wallet and write the
    /// blob back if it changed; returns `edit`'s result.
    ///
    /// The read, `edit` and write run under the view's write lock, so a
    /// writer holding an older snapshot can never overwrite a newer entry
    /// (for example undo a repaired entry). The lock is a `std` mutex held
    /// only for this synchronous call — never across an `.await` — and
    /// `edit` must only touch the cache it is given: taking another lock or
    /// prompting inside it could deadlock.
    ///
    /// When the wallet is no longer loaded (it was removed while the caller
    /// was deriving), `edit` still runs but nothing is written, so a late
    /// writer cannot leave an orphaned entry behind.
    ///
    /// The map is tiny (a handful of identities x a few keys), so a
    /// whole-blob write matches the `WalletMeta` discipline — no need for
    /// row-granular storage.
    pub fn update<R>(
        &self,
        network: Network,
        seed_hash: &WalletSeedHash,
        edit: impl FnOnce(&mut AuthPubkeyCache) -> R,
    ) -> Result<R, TaskError> {
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut cache = self.get(network, seed_hash);
        let before = cache.clone();
        let result = edit(&mut cache);
        if cache == before {
            return Ok(result);
        }
        // Checked under the write lock: removal drops the wallet from the
        // loaded set before `delete` takes this lock, so either this write
        // lands first and is deleted, or the wallet is already gone here.
        if self
            .loaded_wallets
            .is_some_and(|wallets| !wallets.contains_hd(seed_hash))
        {
            tracing::debug!(
                wallet = %hex::encode(seed_hash),
                "Skipping auth-pubkey cache write for a wallet that is no longer loaded"
            );
            return Ok(result);
        }
        self.sidecar.set(network, seed_hash, &cache)?;
        Ok(result)
    }

    /// Whole-blob overwrite of the cache for one wallet, bypassing
    /// [`Self::update`]'s merge. Test seeding only.
    #[cfg(test)]
    pub fn put(
        &self,
        network: Network,
        seed_hash: &WalletSeedHash,
        cache: &AuthPubkeyCache,
    ) -> Result<(), TaskError> {
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.sidecar.set(network, seed_hash, cache)
    }

    /// Delete the cache for one wallet, serialised with [`Self::update`].
    /// Idempotent — a missing key returns `Ok(())`. Called on wallet removal
    /// after the wallet left the loaded set; the wallet-scope cascade only
    /// runs once the upstream wallet row is gone, which may be later or never.
    pub fn delete(&self, network: Network, seed_hash: &WalletSeedHash) -> Result<(), TaskError> {
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.sidecar.delete(network, seed_hash)
    }
}

/// Auth-pubkey-cache adapter errors funnel into the dedicated
/// [`TaskError::KvSidecarStorage`] envelope.
fn map_kv_error_to_task_error(e: KvAdapterError) -> TaskError {
    map_kv_storage_error(e, |source| TaskError::KvSidecarStorage {
        sidecar: "auth_pubkey_cache",
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use dash_sdk::dpp::dashcore::PublicKey;
    use dash_sdk::dpp::dashcore::base58;
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
        let lock = Mutex::new(());
        let view = AuthPubkeyCacheView::without_wallet_gate(&kv, &lock);
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
        let lock = Mutex::new(());
        let view = AuthPubkeyCacheView::without_wallet_gate(&kv, &lock);
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
        let lock = Mutex::new(());
        let view = AuthPubkeyCacheView::without_wallet_gate(&kv, &lock);
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
        let lock = Mutex::new(());
        let view = AuthPubkeyCacheView::without_wallet_gate(&kv, &lock);
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
        let lock = Mutex::new(());
        let view = AuthPubkeyCacheView::without_wallet_gate(&kv, &lock);
        let seed: WalletSeedHash = [0x55; 32];
        view.delete(Network::Testnet, &seed).expect("delete absent");
        let mut cache = AuthPubkeyCache::default();
        cache.insert(Network::Testnet, 0, 0, &pubkey(9));
        view.put(Network::Testnet, &seed, &cache).unwrap();
        view.delete(Network::Testnet, &seed).expect("first delete");
        view.delete(Network::Testnet, &seed).expect("second delete");
        assert!(view.get(Network::Testnet, &seed).is_empty());
    }

    /// AUTH-CACHE-VIEW-008 (SEC-106) — `update` edits the freshest blob, so a
    /// writer that read its snapshot before a repair cannot write the repaired
    /// entry back to its old value.
    #[test]
    fn update_merges_into_the_fresh_blob_instead_of_a_stale_snapshot() {
        let kv = kv();
        let lock = Mutex::new(());
        let view = AuthPubkeyCacheView::without_wallet_gate(&kv, &lock);
        let seed: WalletSeedHash = [0x77; 32];
        let (bad, genuine, other) = (pubkey(1), pubkey(2), pubkey(3));
        let mut poisoned = AuthPubkeyCache::default();
        poisoned.insert(Network::Testnet, 0, 1, &bad);
        view.put(Network::Testnet, &seed, &poisoned).unwrap();

        // A warm reads its snapshot (still poisoned) and decides what is missing.
        let stale = view.get(Network::Testnet, &seed);
        let missing: Vec<u32> = (0..3)
            .filter(|&i| stale.get(Network::Testnet, 0, i).is_none())
            .collect();
        // The repair lands in between.
        let previous = view
            .update(Network::Testnet, &seed, |cache| {
                let previous = cache.get(Network::Testnet, 0, 1);
                cache.insert(Network::Testnet, 0, 1, &genuine);
                previous
            })
            .unwrap();
        assert_eq!(previous, Some(bad));
        // The warm then writes only what it derived.
        let changed = view
            .update(Network::Testnet, &seed, |cache| {
                missing.iter().fold(false, |changed, &i| {
                    cache.insert(Network::Testnet, 0, i, &other) | changed
                })
            })
            .unwrap();
        assert!(changed);

        let got = view.get(Network::Testnet, &seed);
        assert_eq!(got.get(Network::Testnet, 0, 1), Some(genuine));
        assert_eq!(got.get(Network::Testnet, 0, 0), Some(other));
        assert_eq!(got.get(Network::Testnet, 0, 2), Some(other));
    }

    /// AUTH-CACHE-VIEW-009 (SEC-106) — concurrent writers through views
    /// sharing one write lock never lose each other's entries.
    #[test]
    fn concurrent_updates_keep_every_entry() {
        const THREADS: u32 = 8;
        const PER_THREAD: u32 = 25;
        let kv = kv();
        let lock = Mutex::new(());
        let seed: WalletSeedHash = [0x88; 32];
        let key = pubkey(4);
        std::thread::scope(|scope| {
            for thread in 0..THREADS {
                let (kv, lock) = (&kv, &lock);
                scope.spawn(move || {
                    let view = AuthPubkeyCacheView::without_wallet_gate(kv, lock);
                    for i in 0..PER_THREAD {
                        view.update(Network::Testnet, &seed, |cache| {
                            cache.insert(Network::Testnet, thread, i, &key)
                        })
                        .unwrap();
                    }
                });
            }
        });
        let view = AuthPubkeyCacheView::without_wallet_gate(&kv, &lock);
        let got = view.get(Network::Testnet, &seed);
        for thread in 0..THREADS {
            for i in 0..PER_THREAD {
                assert_eq!(
                    got.get(Network::Testnet, thread, i),
                    Some(key),
                    "entry ({thread}, {i}) was lost",
                );
            }
        }
    }

    /// AUTH-CACHE-VIEW-010 — a warm that finishes after its wallet was
    /// removed cannot recreate the entry the removal deleted.
    #[test]
    fn late_write_after_wallet_removal_leaves_no_entry() {
        use crate::model::wallet::Wallet;
        use std::sync::RwLock;

        let kv = kv();
        let lock = Mutex::new(());
        let wallets = WalletContext::default();
        let wallet = Wallet::new_from_seed([7; 64], Network::Testnet, None, None).unwrap();
        let seed = wallet.seed_hash();
        wallets.insert_test_wallet(seed, Arc::new(RwLock::new(wallet)));
        let view = AuthPubkeyCacheView::new(&kv, &lock, &wallets);

        view.update(Network::Testnet, &seed, |cache| {
            cache.insert(Network::Testnet, 0, 0, &pubkey(1))
        })
        .unwrap();
        assert!(
            view.get(Network::Testnet, &seed)
                .get(Network::Testnet, 0, 0)
                .is_some(),
            "a loaded wallet's entry is written"
        );

        // Removal: the wallet leaves the loaded set, then its entry is deleted.
        wallets.remove_wallet(&seed).unwrap();
        view.delete(Network::Testnet, &seed).unwrap();

        // The in-flight warm completes afterwards.
        view.update(Network::Testnet, &seed, |cache| {
            cache.insert(Network::Testnet, 0, 1, &pubkey(2))
        })
        .unwrap();
        assert_eq!(
            view.get(Network::Testnet, &seed),
            AuthPubkeyCache::default(),
            "a removed wallet's cache must stay deleted"
        );
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
