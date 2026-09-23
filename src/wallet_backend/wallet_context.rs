//! Shared wallet membership and committed display/prompt metadata.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use super::PromptMeta;
use super::poison::{read_recover, write_recover};
use crate::backend_task::error::TaskError;
use crate::model::single_key::ImportedKey;
use crate::model::wallet::alias::{
    AliasSource, DefaultAliasKind, dedupe_preserved_alias, ensure_alias_unique, next_default_alias,
    resolve_alias,
};
use crate::model::wallet::meta::WalletMeta;
use crate::model::wallet::single_key::{SingleKeyHash, SingleKeyWallet};
use crate::model::wallet::{Wallet, WalletSeedHash};

type HdWallets = BTreeMap<WalletSeedHash, Arc<RwLock<Wallet>>>;
type SingleKeyWallets = BTreeMap<SingleKeyHash, Arc<RwLock<SingleKeyWallet>>>;

/// Per-network wallet membership and authoritative live metadata.
///
/// Readers receive owned snapshots; persistence never holds the snapshot lock.
///
/// # Lock order
///
/// `writer` mutex → `state` RwLock → an inner `Wallet`/`SingleKeyWallet`
/// handle lock. Mutators run their persistence callback under `writer`, and
/// `rename_hd` reads the inner wallet while holding it. Therefore:
///
/// - never call a mutator (or a backend-bound metadata view such as
///   `WalletBackend::wallet_meta()`) while holding an inner wallet guard;
/// - never re-enter this context's writer from a persistence callback: use
///   raw storage adapters (e.g. `WalletMetaView::new`) inside callbacks.
///   Re-entry on the same thread panics rather than deadlocking.
#[derive(Default)]
pub struct WalletContext {
    writer: Mutex<()>,
    state: RwLock<WalletState>,
}

impl std::fmt::Debug for WalletContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WalletContext").finish_non_exhaustive()
    }
}

#[derive(Default)]
struct WalletState {
    wallets: HdWallets,
    single_wallets: SingleKeyWallets,
    hd: BTreeMap<WalletSeedHash, WalletMeta>,
    single: BTreeMap<String, ImportedKey>,
    single_membership: BTreeMap<String, SingleKeyHash>,
}

impl WalletState {
    /// Bind `address` to `hash`, evicting the handle it replaces: protecting a
    /// re-imported key changes its hash, and removal only evicts the bound one.
    fn set_single_membership(&mut self, address: String, hash: SingleKeyHash) {
        if let Some(old) = self.single_membership.insert(address, hash)
            && old != hash
        {
            self.single_wallets.remove(&old);
        }
    }
}

/// Metadata and reconstructed runtime wallets loaded in one hydration operation.
#[derive(Default)]
pub(crate) struct WalletHydration {
    pub hd: Vec<(WalletSeedHash, WalletMeta)>,
    pub wallets: Vec<(WalletSeedHash, Wallet)>,
    pub single: Vec<ImportedKey>,
    pub single_wallets: Vec<(SingleKeyHash, SingleKeyWallet)>,
}

thread_local! {
    /// Addresses of the [`WalletContext`]s whose writer this thread holds.
    static HELD_WRITERS: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Writer-mutex guard that records ownership so re-entry panics instead of deadlocking.
struct WriterGuard<'a> {
    _lock: MutexGuard<'a, ()>,
    owner: usize,
}

impl Drop for WriterGuard<'_> {
    fn drop(&mut self) {
        // Runs before `_lock` is released, on the thread that took it.
        HELD_WRITERS.with_borrow_mut(|held| {
            if let Some(pos) = held.iter().rposition(|&owner| owner == self.owner) {
                held.swap_remove(pos);
            }
        });
    }
}

impl WalletContext {
    /// Take the writer mutex, panicking if this thread already holds it.
    ///
    /// # Panics
    ///
    /// When a persistence callback re-enters this context (for example through
    /// `WalletBackend::wallet_meta()`): `std::sync::Mutex` is not re-entrant,
    /// so the alternative is a silent self-deadlock. This is a programming bug
    /// (M-PANIC-ON-BUG); callbacks must use raw storage adapters instead.
    fn lock_writer(&self) -> WriterGuard<'_> {
        let owner = std::ptr::from_ref(self) as usize;
        let reentered = HELD_WRITERS.with_borrow(|held| held.contains(&owner));
        assert!(
            !reentered,
            "WalletContext writer re-entered from a persistence callback; \
             use a raw storage adapter (e.g. WalletMetaView::new) inside callbacks"
        );
        let lock = self
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        HELD_WRITERS.with_borrow_mut(|held| held.push(owner));
        WriterGuard { _lock: lock, owner }
    }

    /// Snapshot of loaded HD wallet handles for iterate-and-release callers;
    /// prefer the targeted readers for lookups and counts. Aliases are read
    /// through [`Self::hd_alias`].
    pub fn wallets(&self) -> HdWallets {
        read_recover(&self.state).wallets.clone()
    }

    /// Snapshot of loaded imported-key handles for iterate-and-release callers;
    /// prefer the targeted readers for lookups and counts. Aliases are read
    /// through [`Self::single_alias`].
    pub fn single_key_wallets(&self) -> SingleKeyWallets {
        read_recover(&self.state).single_wallets.clone()
    }

    /// Whether an HD wallet with this seed hash is loaded.
    pub fn contains_hd(&self, seed: &WalletSeedHash) -> bool {
        read_recover(&self.state).wallets.contains_key(seed)
    }

    /// Whether an imported key with this key hash is loaded.
    pub fn contains_single(&self, hash: &SingleKeyHash) -> bool {
        read_recover(&self.state).single_wallets.contains_key(hash)
    }

    /// Number of loaded HD wallets.
    pub fn hd_count(&self) -> usize {
        read_recover(&self.state).wallets.len()
    }

    /// Whether any HD wallet is loaded.
    pub fn has_hd_wallets(&self) -> bool {
        !read_recover(&self.state).wallets.is_empty()
    }

    /// Whether any imported key is loaded.
    pub fn has_single_key_wallets(&self) -> bool {
        !read_recover(&self.state).single_wallets.is_empty()
    }

    /// Whether any HD wallet or imported key is loaded.
    pub fn has_any_wallet(&self) -> bool {
        let state = read_recover(&self.state);
        !state.wallets.is_empty() || !state.single_wallets.is_empty()
    }

    /// Loaded HD wallet handle for `seed`, if any.
    pub fn hd_wallet(&self, seed: &WalletSeedHash) -> Option<Arc<RwLock<Wallet>>> {
        read_recover(&self.state).wallets.get(seed).cloned()
    }

    /// Loaded imported-key handle for `hash`, if any.
    pub fn single_key_wallet(&self, hash: &SingleKeyHash) -> Option<Arc<RwLock<SingleKeyWallet>>> {
        read_recover(&self.state).single_wallets.get(hash).cloned()
    }

    /// HD wallet handle with the lowest seed hash, if any.
    pub fn first_hd(&self) -> Option<Arc<RwLock<Wallet>>> {
        read_recover(&self.state).wallets.values().next().cloned()
    }

    /// Imported-key handle with the lowest key hash, if any.
    pub fn first_single(&self) -> Option<Arc<RwLock<SingleKeyWallet>>> {
        read_recover(&self.state)
            .single_wallets
            .values()
            .next()
            .cloned()
    }

    /// Resolve a loaded wallet without exposing registry mutation.
    pub fn wallet(&self, seed: &WalletSeedHash) -> Result<Arc<RwLock<Wallet>>, TaskError> {
        self.hd_wallet(seed).ok_or(TaskError::WalletNotFound)
    }

    /// Current committed HD metadata, including the password-prompt label.
    pub fn hd_metadata(&self, seed: &WalletSeedHash) -> Option<WalletMeta> {
        read_recover(&self.state).hd.get(seed).cloned()
    }

    /// Current committed name, with unnamed legacy wallets preserved.
    pub fn hd_alias(&self, seed: &WalletSeedHash) -> Option<String> {
        self.hd_metadata(seed)
            .and_then(|m| (!m.alias.is_empty()).then_some(m.alias))
    }

    /// Current HD password-prompt copy from the same metadata used by displays.
    pub fn hd_prompt(&self, seed: &WalletSeedHash) -> PromptMeta {
        self.hd_metadata(seed)
            .map(|m| PromptMeta {
                alias: (!m.alias.is_empty()).then_some(m.alias),
                password_hint: m.password_hint,
            })
            .unwrap_or_default()
    }

    /// Current imported-key metadata snapshot.
    pub fn single_key(&self, address: &str) -> Option<ImportedKey> {
        read_recover(&self.state).single.get(address).cloned()
    }

    /// Current imported-key name.
    pub fn single_alias(&self, address: &str) -> Option<String> {
        self.single_key(address).and_then(|m| m.alias)
    }

    /// Imported-key metadata ordered by address.
    pub fn imported_keys(&self) -> Vec<ImportedKey> {
        read_recover(&self.state).single.values().cloned().collect()
    }

    // Compatibility reads can re-store an older record format.
    pub(crate) fn read_metadata<T>(&self, read: impl FnOnce() -> T) -> T {
        let _writer = self.lock_writer();
        read()
    }

    pub(crate) fn save_hd_metadata(
        &self,
        seed: WalletSeedHash,
        meta: WalletMeta,
        persist: impl FnOnce() -> Result<(), TaskError>,
    ) -> Result<(), TaskError> {
        let _writer = self.lock_writer();
        persist()?;
        write_recover(&self.state).hd.insert(seed, meta);
        Ok(())
    }

    pub(crate) fn delete_hd_metadata(
        &self,
        seed: &WalletSeedHash,
        persist: impl FnOnce() -> Result<(), TaskError>,
    ) -> Result<(), TaskError> {
        let _writer = self.lock_writer();
        persist()?;
        let mut state = write_recover(&self.state);
        state.hd.remove(seed);
        Ok(())
    }

    pub(crate) fn register_hd(
        &self,
        mut wallet: Wallet,
        persist: impl FnOnce(&Wallet) -> Result<WalletMeta, TaskError>,
    ) -> Result<Arc<RwLock<Wallet>>, TaskError> {
        let _writer = self.lock_writer();
        let seed = wallet.seed_hash();
        let state = read_recover(&self.state);
        if state.wallets.contains_key(&seed) || state.hd.contains_key(&seed) {
            return Err(TaskError::WalletAlreadyImported);
        }
        let alias = resolve_hd(
            &state,
            wallet.initial_alias.as_deref().unwrap_or_default(),
            &seed,
        )?;
        drop(state);
        wallet.initial_alias = Some(alias);
        let meta = persist(&wallet)?;
        let wallet = Arc::new(RwLock::new(wallet));
        let mut state = write_recover(&self.state);
        state.hd.insert(seed, meta);
        state.wallets.insert(seed, wallet.clone());
        Ok(wallet)
    }

    pub(crate) fn rename_hd(
        &self,
        seed: WalletSeedHash,
        raw: &str,
        persist: impl FnOnce(Vec<u8>, &str) -> Result<WalletMeta, TaskError>,
    ) -> Result<String, TaskError> {
        let _writer = self.lock_writer();
        let state = read_recover(&self.state);
        let wallet = state
            .wallets
            .get(&seed)
            .cloned()
            .ok_or(TaskError::WalletNotFound)?;
        let alias = resolve_hd(&state, raw, &seed)?;
        drop(state);
        let xpub_encoded = wallet
            .read()?
            .master_bip44_ecdsa_extended_public_key
            .encode()
            .to_vec();
        let meta = persist(xpub_encoded, &alias)?;
        write_recover(&self.state).hd.insert(seed, meta);
        Ok(alias)
    }

    pub(crate) fn import_single_key(
        &self,
        address: &str,
        source: AliasSource,
        persist: impl FnOnce(Option<String>) -> Result<(ImportedKey, SingleKeyWallet), TaskError>,
    ) -> Result<(ImportedKey, Arc<RwLock<SingleKeyWallet>>), TaskError> {
        let _writer = self.lock_writer();
        let state = read_recover(&self.state);
        let taken: Vec<&str> = state
            .single
            .iter()
            .filter(|(key, _)| key.as_str() != address)
            .filter_map(|(_, m)| m.alias.as_deref())
            .collect();
        let alias = match source {
            AliasSource::UserEntered(raw) => {
                Some(resolve_name(&raw, DefaultAliasKind::SingleKey, &taken)?)
            }
            AliasSource::Preserved(alias) => {
                alias.map(|a| dedupe_preserved_alias(a, taken.iter().copied()))
            }
        };
        drop(state);
        let (meta, wallet) = persist(alias)?;
        let mut state = write_recover(&self.state);
        let hash = wallet.key_hash();
        let wallet = Arc::new(RwLock::new(wallet));
        state.set_single_membership(address.to_owned(), hash);
        state.single_wallets.insert(hash, wallet.clone());
        state.single.insert(address.to_owned(), meta.clone());
        Ok((meta, wallet))
    }

    pub(crate) fn rename_single_key(
        &self,
        address: &str,
        raw: &str,
        persist: impl FnOnce(&ImportedKey) -> Result<(), TaskError>,
    ) -> Result<String, TaskError> {
        let _writer = self.lock_writer();
        let state = read_recover(&self.state);
        let mut meta = state
            .single
            .get(address)
            .cloned()
            .ok_or(TaskError::ImportedKeyNotFound)?;
        let taken: Vec<&str> = state
            .single
            .iter()
            .filter(|(key, _)| key.as_str() != address)
            .filter_map(|(_, m)| m.alias.as_deref())
            .collect();
        let alias = resolve_name(raw, DefaultAliasKind::SingleKey, &taken)?;
        drop(state);
        meta.alias = Some(alias.clone());
        persist(&meta)?;
        write_recover(&self.state)
            .single
            .insert(address.to_owned(), meta);
        Ok(alias)
    }

    pub(crate) fn remove_single_key(
        &self,
        address: &str,
        persist: impl FnOnce() -> Result<(), TaskError>,
    ) -> Result<(), TaskError> {
        let _writer = self.lock_writer();
        persist()?;
        let mut state = write_recover(&self.state);
        state.single.remove(address);
        if let Some(hash) = state.single_membership.remove(address) {
            state.single_wallets.remove(&hash);
        }
        Ok(())
    }

    pub(crate) fn hydrate(
        &self,
        load: impl FnOnce() -> Result<WalletHydration, TaskError>,
    ) -> Result<(), TaskError> {
        let _writer = self.lock_writer();
        let loaded = load()?;
        let mut state = write_recover(&self.state);
        for (seed, meta) in loaded.hd {
            state.hd.insert(seed, meta);
        }
        for meta in loaded.single {
            state.single.insert(meta.address.clone(), meta);
        }
        for (seed, wallet) in loaded.wallets {
            state
                .wallets
                .entry(seed)
                .or_insert_with(|| Arc::new(RwLock::new(wallet)));
        }
        for (hash, wallet) in loaded.single_wallets {
            state.set_single_membership(wallet.address.to_string(), hash);
            state
                .single_wallets
                .entry(hash)
                .or_insert_with(|| Arc::new(RwLock::new(wallet)));
        }
        Ok(())
    }

    pub(crate) fn remove_wallet(&self, seed: &WalletSeedHash) -> Result<(), TaskError> {
        let _writer = self.lock_writer();
        let mut state = write_recover(&self.state);
        state
            .wallets
            .remove(seed)
            .ok_or(TaskError::WalletNotFound)?;
        state.hd.remove(seed);
        Ok(())
    }

    pub(crate) fn clear(&self) {
        let _writer = self.lock_writer();
        *write_recover(&self.state) = WalletState::default();
    }

    /// Seed a runtime fixture without exposing the registry's write lock.
    #[cfg(any(test, feature = "testing"))]
    pub fn insert_test_wallet(&self, seed: WalletSeedHash, wallet: Arc<RwLock<Wallet>>) {
        let meta = {
            let w = read_recover(&wallet);
            WalletMeta {
                alias: w.initial_alias.clone().unwrap_or_default(),
                password_hint: w.password_hint().clone(),
                uses_password: w.uses_password,
                ..Default::default()
            }
        };
        let _writer = self.lock_writer();
        let mut state = write_recover(&self.state);
        state.hd.insert(seed, meta);
        state.wallets.insert(seed, wallet);
    }

    /// Seed an imported-key fixture through the same metadata owner as its display.
    #[cfg(any(test, feature = "testing"))]
    pub fn insert_test_single_key(
        &self,
        network: dash_sdk::dpp::dashcore::Network,
        hash: SingleKeyHash,
        wallet: Arc<RwLock<SingleKeyWallet>>,
    ) {
        let meta = {
            let w = read_recover(&wallet);
            ImportedKey {
                address: w.address.to_string(),
                alias: w.initial_alias.clone(),
                network,
                has_passphrase: w.uses_password,
                passphrase_hint: None,
                public_key_bytes: w.public_key.inner.serialize().to_vec(),
            }
        };
        let _writer = self.lock_writer();
        let mut state = write_recover(&self.state);
        state.single_membership.insert(meta.address.clone(), hash);
        state.single.insert(meta.address.clone(), meta);
        state.single_wallets.insert(hash, wallet);
    }

    /// Simulate a successful HD backend rename in UI-only tests.
    #[cfg(any(test, feature = "testing"))]
    pub fn rename_test_hd(&self, seed: WalletSeedHash, alias: &str) -> Result<String, TaskError> {
        self.rename_hd(seed, alias, |_, alias| {
            let mut meta = self.hd_metadata(&seed).unwrap_or_default();
            meta.alias = alias.to_owned();
            Ok(meta)
        })
    }

    /// Simulate a successful imported-key backend rename in UI-only tests.
    #[cfg(any(test, feature = "testing"))]
    pub fn rename_test_single(&self, address: &str, alias: &str) -> Result<String, TaskError> {
        self.rename_single_key(address, alias, |_| Ok(()))
    }
}

fn resolve_name(raw: &str, kind: DefaultAliasKind, taken: &[&str]) -> Result<String, TaskError> {
    let alias = resolve_alias(raw, || next_default_alias(kind, taken.iter().copied()))?;
    ensure_alias_unique(&alias, taken.iter().copied())?;
    Ok(alias)
}

/// Only LOADED wallets reserve a name. A metadata row without a loaded wallet
/// (e.g. its seed envelope is gone) is invisible in the UI, so letting it
/// block a name — or a default "Wallet N" slot — would reject names the user
/// cannot see being used.
fn resolve_hd(state: &WalletState, raw: &str, seed: &WalletSeedHash) -> Result<String, TaskError> {
    let taken: Vec<&str> = state
        .hd
        .iter()
        .filter(|(key, _)| *key != seed && state.wallets.contains_key(*key))
        .map(|(_, m)| m.alias.as_str())
        .collect();
    resolve_name(raw, DefaultAliasKind::HdWallet, &taken)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    #[should_panic(expected = "WalletContext writer re-entered")]
    fn mutator_reentered_from_persist_callback_panics_instead_of_deadlocking() {
        let context = WalletContext::default();
        let _ = context.save_hd_metadata([4; 32], WalletMeta::default(), || {
            context.delete_hd_metadata(&[4; 32], || Ok(()))
        });
    }

    #[test]
    #[should_panic(expected = "WalletContext writer re-entered")]
    fn metadata_read_reentered_from_persist_callback_panics_instead_of_deadlocking() {
        let context = WalletContext::default();
        let _ = context.save_hd_metadata([5; 32], WalletMeta::default(), || {
            context.read_metadata(|| Ok(()))
        });
    }

    #[test]
    fn writer_is_reusable_after_a_callback_error_and_on_other_contexts() {
        let context = WalletContext::default();
        let other = WalletContext::default();
        let result = context.save_hd_metadata([6; 32], WalletMeta::default(), || {
            // A distinct context's writer is independent, so nesting is allowed.
            other.save_hd_metadata([6; 32], WalletMeta::default(), || Ok(()))?;
            Err(TaskError::WalletNotFound)
        });
        assert!(result.is_err());
        assert!(other.hd_metadata(&[6; 32]).is_some());
        context
            .save_hd_metadata([6; 32], WalletMeta::default(), || Ok(()))
            .expect("the writer is released after a failed callback");
    }

    #[test]
    fn writer_guard_is_cleared_when_a_callback_unwinds() {
        let context = WalletContext::default();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = context.save_hd_metadata([7; 32], WalletMeta::default(), || {
                panic!("callback failure")
            });
        }));
        assert!(unwound.is_err());
        // The poisoned writer is recovered and the held marker was dropped,
        // so the same thread can write again without a false re-entry panic.
        context
            .save_hd_metadata([7; 32], WalletMeta::default(), || Ok(()))
            .expect("writer usable after an unwinding callback");
    }

    #[test]
    fn targeted_readers_track_hd_membership() {
        let context = WalletContext::default();
        assert!(!context.has_any_wallet() && !context.has_hd_wallets());
        assert!(!context.has_single_key_wallets());
        assert_eq!(context.hd_count(), 0);
        assert!(context.first_hd().is_none() && context.first_single().is_none());
        assert!(!context.contains_single(&[9; 32]));
        assert!(context.single_key_wallet(&[9; 32]).is_none());

        let wallet = Wallet::new_from_seed(
            rand::random(),
            dash_sdk::dpp::dashcore::Network::Testnet,
            Some("Readers".into()),
            None,
        )
        .unwrap();
        let seed = wallet.seed_hash();
        let handle = context
            .register_hd(wallet, |_| Ok(WalletMeta::default()))
            .unwrap();
        assert!(context.has_any_wallet() && context.has_hd_wallets());
        assert!(context.contains_hd(&seed));
        assert_eq!(context.hd_count(), 1);
        assert!(Arc::ptr_eq(&context.hd_wallet(&seed).unwrap(), &handle));
        assert!(Arc::ptr_eq(&context.first_hd().unwrap(), &handle));
        assert!(!context.has_single_key_wallets());

        context.remove_wallet(&seed).unwrap();
        assert!(!context.contains_hd(&seed) && context.hd_wallet(&seed).is_none());
        assert!(!context.has_any_wallet());
    }

    #[test]
    fn removed_wallet_can_be_reimported_and_its_alias_reused() {
        let context = WalletContext::default();
        let make_wallet = |seed| {
            Wallet::new_from_seed(
                seed,
                dash_sdk::dpp::dashcore::Network::Testnet,
                Some("Savings".into()),
                None,
            )
            .unwrap()
        };
        let persist = |wallet: &Wallet| {
            Ok(WalletMeta {
                alias: wallet.initial_alias.clone().unwrap(),
                ..Default::default()
            })
        };
        let seed = rand::random();
        let original = make_wallet(seed);
        let hash = original.seed_hash();
        context.register_hd(original, persist).unwrap();
        context.remove_wallet(&hash).unwrap();
        assert!(context.hd_prompt(&hash).alias.is_none());
        let reimported = context.register_hd(make_wallet(seed), persist).unwrap();
        assert_eq!(context.hd_alias(&hash).as_deref(), Some("Savings"));
        assert_eq!(reimported.read().unwrap().seed_hash(), hash);
        context.remove_wallet(&hash).unwrap();
        let replacement = make_wallet(rand::random());
        let replacement_hash = replacement.seed_hash();
        context.register_hd(replacement, persist).unwrap();
        assert_eq!(
            context.hd_alias(&replacement_hash).as_deref(),
            Some("Savings")
        );
    }

    /// A metadata row with no loaded wallet (e.g. its seed envelope is gone)
    /// is invisible in the UI, so it must not reserve a name the user can
    /// never see — only loaded wallets do.
    #[test]
    fn orphan_metadata_row_does_not_reserve_its_alias() {
        let context = WalletContext::default();
        for (seed, alias) in [([1; 32], "Savings"), ([2; 32], "Wallet 1")] {
            let meta = WalletMeta {
                alias: alias.into(),
                ..Default::default()
            };
            context.save_hd_metadata(seed, meta, || Ok(())).unwrap();
        }
        let persist = |wallet: &Wallet| {
            Ok(WalletMeta {
                alias: wallet.initial_alias.clone().unwrap(),
                ..Default::default()
            })
        };
        let register = |alias: &str| {
            let wallet = Wallet::new_from_seed(
                rand::random(),
                dash_sdk::dpp::dashcore::Network::Testnet,
                Some(alias.into()),
                None,
            )
            .unwrap();
            let hash = wallet.seed_hash();
            context
                .register_hd(wallet, persist)
                .expect("an orphan row's name is free");
            context.hd_alias(&hash)
        };

        assert_eq!(register("Savings").as_deref(), Some("Savings"));
        assert_eq!(register("").as_deref(), Some("Wallet 1"));
        assert!(
            matches!(
                context.register_hd(
                    Wallet::new_from_seed(
                        rand::random(),
                        dash_sdk::dpp::dashcore::Network::Testnet,
                        Some("Savings".into()),
                        None,
                    )
                    .unwrap(),
                    persist,
                ),
                Err(TaskError::WalletAliasAlreadyUsed { .. })
            ),
            "a loaded wallet still reserves its name"
        );
    }

    #[test]
    fn failed_metadata_write_keeps_committed_alias() {
        let context = WalletContext::default();
        let seed = [1; 32];
        let original = WalletMeta {
            alias: "Saved".into(),
            ..Default::default()
        };
        context.save_hd_metadata(seed, original, || Ok(())).unwrap();
        let next = WalletMeta {
            alias: "Not saved".into(),
            ..Default::default()
        };
        assert!(
            context
                .save_hd_metadata(seed, next, || Err(TaskError::WalletNotFound))
                .is_err()
        );
        assert_eq!(context.hd_alias(&seed).as_deref(), Some("Saved"));
        assert_eq!(context.hd_prompt(&seed).alias.as_deref(), Some("Saved"));
    }

    #[test]
    fn metadata_readers_remain_available_during_persistence() {
        let context = Arc::new(WalletContext::default());
        let seed = [2; 32];
        context
            .save_hd_metadata(
                seed,
                WalletMeta {
                    alias: "Before".into(),
                    ..Default::default()
                },
                || Ok(()),
            )
            .unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let writer_context = context.clone();
        let writer = std::thread::spawn(move || {
            writer_context.save_hd_metadata(
                seed,
                WalletMeta {
                    alias: "After".into(),
                    ..Default::default()
                },
                || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(())
                },
            )
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let (read_tx, read_rx) = mpsc::channel();
        let reader_context = context.clone();
        let reader = std::thread::spawn(move || {
            read_tx
                .send((
                    reader_context.hd_alias(&seed),
                    reader_context.hd_prompt(&seed).alias,
                ))
                .unwrap()
        });
        let snapshot = read_rx.recv_timeout(Duration::from_secs(1));
        release_tx.send(()).unwrap();
        writer.join().unwrap().unwrap();
        reader.join().unwrap();
        assert_eq!(
            snapshot.unwrap(),
            (Some("Before".into()), Some("Before".into()))
        );
        assert_eq!(context.hd_alias(&seed).as_deref(), Some("After"));
        assert_eq!(context.hd_prompt(&seed).alias.as_deref(), Some("After"));
    }

    #[test]
    fn hydration_cannot_publish_over_a_later_metadata_write() {
        let context = Arc::new(WalletContext::default());
        let seed = [3; 32];
        let (loaded_tx, loaded_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let hydrator_context = context.clone();
        let hydrator = std::thread::spawn(move || {
            hydrator_context.hydrate(|| {
                let old = WalletMeta {
                    alias: "Stored".into(),
                    ..Default::default()
                };
                loaded_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(WalletHydration {
                    hd: vec![(seed, old)],
                    ..Default::default()
                })
            })
        });
        loaded_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let writer_context = context.clone();
        let (done_tx, done_rx) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            writer_context
                .save_hd_metadata(
                    seed,
                    WalletMeta {
                        alias: "Renamed".into(),
                        ..Default::default()
                    },
                    || Ok(()),
                )
                .unwrap();
            done_tx.send(()).unwrap();
        });
        let completed_early = done_rx.recv_timeout(Duration::from_millis(100)).is_ok();
        release_tx.send(()).unwrap();
        hydrator.join().unwrap().unwrap();
        writer.join().unwrap();
        assert!(
            !completed_early,
            "hydration must order publication with writers"
        );
        assert_eq!(context.hd_alias(&seed).as_deref(), Some("Renamed"));
        assert_eq!(context.hd_prompt(&seed).alias.as_deref(), Some("Renamed"));
    }

    /// Same key, different protection: the two runtime handles hash differently.
    fn single_key_pair() -> (ImportedKey, SingleKeyWallet, SingleKeyWallet) {
        let network = dash_sdk::dpp::dashcore::Network::Testnet;
        let open = SingleKeyWallet::new([7; 32], network, None, None).unwrap();
        let mut protected = SingleKeyWallet::new([7; 32], network, None, None).unwrap();
        protected.key_hash = [8; 32];
        let meta = ImportedKey {
            address: open.address.to_string(),
            alias: None,
            network,
            has_passphrase: false,
            passphrase_hint: None,
            public_key_bytes: open.public_key.inner.serialize().to_vec(),
        };
        (meta, open, protected)
    }

    #[test]
    fn reimport_with_a_new_key_hash_leaves_no_stale_handle_after_removal() {
        let context = WalletContext::default();
        let (meta, open, protected) = single_key_pair();
        let (address, old_hash) = (meta.address.clone(), open.key_hash());
        for wallet in [open, protected] {
            let meta = meta.clone();
            context
                .import_single_key(&address, AliasSource::Preserved(None), |_| {
                    Ok((meta, wallet))
                })
                .unwrap();
        }
        assert!(!context.contains_single(&old_hash));
        assert_eq!(context.single_key_wallets().len(), 1);
        context.remove_single_key(&address, || Ok(())).unwrap();
        assert!(!context.has_single_key_wallets());
    }

    #[test]
    fn hydration_with_a_new_key_hash_leaves_no_stale_handle_after_removal() {
        let context = WalletContext::default();
        let (meta, open, protected) = single_key_pair();
        let (address, old_hash) = (meta.address.clone(), open.key_hash());
        for wallet in [open, protected] {
            let meta = meta.clone();
            context
                .hydrate(|| {
                    Ok(WalletHydration {
                        single: vec![meta],
                        single_wallets: vec![(wallet.key_hash(), wallet)],
                        ..Default::default()
                    })
                })
                .unwrap();
        }
        assert!(!context.contains_single(&old_hash));
        assert_eq!(context.single_key_wallets().len(), 1);
        context.remove_single_key(&address, || Ok(())).unwrap();
        assert!(!context.has_single_key_wallets());
    }

    #[test]
    fn retained_wallet_and_metadata_snapshots_cannot_change_live_alias() {
        let context = WalletContext::default();
        let wallet = Wallet::new_from_seed(
            rand::random(),
            dash_sdk::dpp::dashcore::Network::Testnet,
            Some("First".into()),
            None,
        )
        .unwrap();
        let seed = wallet.seed_hash();
        let wallet = context
            .register_hd(wallet, |wallet| {
                Ok(WalletMeta {
                    alias: wallet.initial_alias.clone().unwrap(),
                    ..Default::default()
                })
            })
            .unwrap();
        context
            .rename_hd(seed, "Second", |_, alias| {
                Ok(WalletMeta {
                    alias: alias.to_owned(),
                    ..Default::default()
                })
            })
            .unwrap();
        wallet.write().unwrap().initial_alias = Some("Stale handle".into());
        context.hd_metadata(&seed).unwrap().alias = "Edited snapshot".into();
        assert_eq!(context.hd_alias(&seed).as_deref(), Some("Second"));
        assert_eq!(context.hd_prompt(&seed).alias.as_deref(), Some("Second"));
    }
}
