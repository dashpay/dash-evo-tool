use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use std::sync::Arc;

impl AppContext {
    /// Persist a new alias for an HD wallet to the wallet-meta sidecar.
    ///
    /// The existing metadata is read through the FALLIBLE
    /// [`WalletMetaView::try_get`](crate::wallet_backend::WalletMetaView::try_get)
    /// path: a storage/read failure aborts the rename instead of defaulting and
    /// clobbering the other sidecar fields (`is_main` / `core_wallet_name` /
    /// xpub / password fields) on the follow-up write. Only a genuinely absent
    /// row seeds a fresh entry, carrying the wallet's xpub so the cold-boot
    /// picker can still render the wallet without unlocking the seed.
    ///
    /// # Errors
    ///
    /// - [`TaskError::WalletNotFound`] when `seed_hash` matches no loaded wallet.
    /// - [`TaskError::KvSidecarStorage`] when the sidecar cannot be read or written.
    /// - [`TaskError::InvalidWalletAliasLength`] when `alias` exceeds the limit.
    pub(crate) fn rename_hd_wallet(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        alias: String,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let rename_lock = self.hd_wallet_rename_lock(seed_hash);
        let _rename_guard = rename_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // Retaining the map guard makes existence and persistence one operation
        // relative to wallet removal. The inner wallet guard is disk-I/O-free.
        let wallets = self.wallets.read()?;
        let wallet = wallets.get(&seed_hash).ok_or(TaskError::WalletNotFound)?;
        let xpub_encoded = wallet
            .read()?
            .master_bip44_ecdsa_extended_public_key
            .encode()
            .to_vec();

        let backend = self.wallet_backend()?;
        let meta_view = backend.wallet_meta();
        // Fallible read: a storage/read failure aborts here instead of
        // defaulting and clobbering the row's other fields on the write below.
        // Only a genuinely absent row (`Ok(None)`) seeds a fresh default.
        let mut meta = meta_view
            .try_get(self.network, &seed_hash)?
            .unwrap_or_default();
        meta.alias = alias.clone();
        if meta.xpub_encoded.is_empty() {
            meta.xpub_encoded = xpub_encoded;
        }
        meta_view.set(self.network, &seed_hash, &meta)?;

        Ok(BackendTaskSuccessResult::WalletAliasRenamed { seed_hash, alias })
    }

    /// Persist a new alias for an imported single-key wallet to the single-key
    /// sidecar, delegating to the typed
    /// [`SingleKeyView::set_alias`](crate::wallet_backend::single_key::SingleKeyView::set_alias)
    /// chokepoint (which validates the alias and refreshes the in-memory index).
    ///
    /// # Errors
    ///
    /// - [`TaskError::ImportedKeyNotFound`] when `address` was never imported.
    /// - [`TaskError::SingleKeyMetaStorage`] when the sidecar cannot be written.
    /// - [`TaskError::InvalidWalletAliasLength`] when `alias` exceeds the limit.
    pub(crate) fn rename_single_key_wallet(
        self: &Arc<Self>,
        address: String,
        alias: String,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let backend = self.wallet_backend()?;
        backend
            .single_key()
            .set_alias(&address, Some(alias.clone()))?;
        Ok(BackendTaskSuccessResult::SingleKeyAliasRenamed { address, alias })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::TaskResult;
    use crate::app_dir::ensure_env_file;
    use crate::context::connection_status::ConnectionStatus;
    use crate::database::test_helpers::create_database_at_path;
    use crate::model::user_role::UserRoleCell;
    use crate::model::wallet::Wallet;
    use crate::model::wallet::birth_height::WalletOrigin;
    use crate::model::wallet::meta::WalletMeta;
    use crate::utils::egui_mpsc::SenderAsync;
    use crate::utils::tasks::TaskManager;
    use crate::wallet_backend::kv_test_support::InMemoryKv;
    use crate::wallet_backend::{DetKv, DetScope};
    use dash_sdk::dpp::dashcore::secp256k1::SecretKey;
    use dash_sdk::dpp::dashcore::{Network, PrivateKey};
    use platform_wallet_storage::{KvError, KvStore, ObjectId};
    use std::sync::{Condvar, Mutex};
    use std::time::Duration;
    use tempfile::TempDir;
    use tokio::sync::mpsc::Receiver;

    /// An offline testnet context with one registered HD wallet whose backend
    /// is wired (so `wallet_meta()` and `single_key()` are usable). Registration
    /// writes an initial wallet-meta row. The receiver and temp dir must outlive
    /// the context.
    struct Fixture {
        ctx: Arc<AppContext>,
        seed_hash: WalletSeedHash,
        _rx: Receiver<TaskResult>,
        _dir: TempDir,
    }

    async fn fixture_with_app_kv(app_kv: Arc<DetKv>) -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        fixture_from_parts(dir, app_kv).await
    }

    async fn fixture_from_parts(dir: TempDir, app_kv: Arc<DetKv>) -> Fixture {
        let data_dir = dir.path().to_path_buf();
        ensure_env_file(&data_dir);
        let db = Arc::new(create_database_at_path(&data_dir.join("data.db")).expect("db"));
        let secret_store = AppContext::open_secret_store(&data_dir).expect("secret store");
        let ctx = AppContext::new(
            data_dir,
            Network::Testnet,
            db,
            Arc::new(TaskManager::new()),
            Arc::new(ConnectionStatus::new()),
            egui::Context::default(),
            app_kv,
            secret_store,
            UserRoleCell::default(),
        )
        .expect("offline testnet AppContext");

        let (tx, rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());

        let seed = [0x5Au8; 64];
        let wallet =
            Wallet::new_from_seed(seed, Network::Testnet, None, None).expect("build wallet");
        let seed_hash = wallet.seed_hash();
        ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh)
            .expect("register wallet");
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");

        Fixture {
            ctx,
            seed_hash,
            _rx: rx,
            _dir: dir,
        }
    }

    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let app_kv = AppContext::open_app_kv(dir.path()).expect("app kv");
        fixture_from_parts(dir, app_kv).await
    }

    #[derive(Default)]
    struct ReadGateState {
        armed: bool,
        intercepted: bool,
        released: bool,
    }

    #[derive(Default)]
    struct FirstWalletMetaReadGate {
        inner: InMemoryKv,
        state: Mutex<ReadGateState>,
        changed: Condvar,
    }

    impl FirstWalletMetaReadGate {
        fn arm(&self) {
            let mut state = self.state.lock().expect("gate state");
            *state = ReadGateState {
                armed: true,
                ..Default::default()
            };
        }

        fn wait_until_intercepted(&self) {
            let state = self.state.lock().expect("gate state");
            let (state, timeout) = self
                .changed
                .wait_timeout_while(state, Duration::from_secs(5), |state| !state.intercepted)
                .expect("gate wait");
            assert!(
                !timeout.timed_out() && state.intercepted,
                "rename read gate"
            );
        }

        fn release(&self) {
            let mut state = self.state.lock().expect("gate state");
            state.released = true;
            self.changed.notify_all();
        }
    }

    impl KvStore for FirstWalletMetaReadGate {
        fn get(&self, scope: &ObjectId, key: &str) -> Result<Option<Vec<u8>>, KvError> {
            let value = self.inner.get(scope, key)?;
            let mut state = self.state.lock().expect("gate state");
            if state.armed && !state.intercepted && key.contains(":wallet_meta:") {
                state.intercepted = true;
                self.changed.notify_all();
                state = self
                    .changed
                    .wait_while(state, |state| !state.released)
                    .expect("gate release");
            }
            drop(state);
            Ok(value)
        }

        fn put(&self, scope: &ObjectId, key: &str, value: &[u8]) -> Result<(), KvError> {
            self.inner.put(scope, key, value)
        }

        fn delete(&self, scope: &ObjectId, key: &str) -> Result<(), KvError> {
            self.inner.delete(scope, key)
        }

        fn list_keys(
            &self,
            scope: &ObjectId,
            prefix: Option<&str>,
        ) -> Result<Vec<String>, KvError> {
            self.inner.list_keys(scope, prefix)
        }
    }

    /// A bare `u8` whose bincode string-length varint runs past the end of the
    /// blob — unreadable as either the current or the legacy `WalletMeta` shape,
    /// so it forces a sidecar READ failure rather than a decode-to-default.
    const UNREADABLE_META_SENTINEL: u8 = 2;

    /// The headline regression: a FAILED metadata read must surface as an error
    /// and leave the stored blob untouched, never silently default-and-overwrite
    /// (which would drop `is_main` / `core_wallet_name` / xpub / password fields).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rename_hd_read_failure_surfaces_and_does_not_clobber() {
        let f = fixture().await;
        let key = crate::wallet_backend::wallet_meta::key_for(f.ctx.network, &f.seed_hash);
        // Overwrite the registration-written meta with an unreadable blob.
        f.ctx
            .app_kv()
            .put(DetScope::Global, &key, &UNREADABLE_META_SENTINEL)
            .expect("plant unreadable blob");

        let err = f
            .ctx
            .rename_hd_wallet(f.seed_hash, "renamed".into())
            .expect_err("a failed metadata read must surface, not silently overwrite");
        assert!(
            matches!(
                err,
                TaskError::KvSidecarStorage {
                    sidecar: "wallet_meta",
                    ..
                }
            ),
            "got {err:?}"
        );

        // The unreadable blob is untouched — the rename aborted before writing.
        let raw: Option<u8> = f
            .ctx
            .app_kv()
            .get(DetScope::Global, &key)
            .expect("raw read");
        assert_eq!(
            raw,
            Some(UNREADABLE_META_SENTINEL),
            "the unreadable blob must not be overwritten by a defaulted meta"
        );
    }

    /// Renaming preserves every non-alias field of an existing meta row.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rename_hd_preserves_other_meta_fields() {
        let f = fixture().await;
        let backend = f.ctx.wallet_backend().expect("backend");
        let seeded = WalletMeta {
            alias: "old".into(),
            is_main: true,
            core_wallet_name: Some("local-dashd".into()),
            xpub_encoded: vec![0xAB; 78],
            uses_password: true,
            password_hint: Some("granny's birthday".into()),
        };
        backend
            .wallet_meta()
            .set(f.ctx.network, &f.seed_hash, &seeded)
            .expect("seed meta");

        let result = f
            .ctx
            .rename_hd_wallet(f.seed_hash, "renamed".into())
            .expect("rename");
        assert!(
            matches!(
                &result,
                BackendTaskSuccessResult::WalletAliasRenamed { seed_hash, alias }
                    if *seed_hash == f.seed_hash && alias == "renamed"
            ),
            "got {result:?}"
        );

        let after = backend
            .wallet_meta()
            .get(f.ctx.network, &f.seed_hash)
            .expect("meta present");
        assert_eq!(after.alias, "renamed", "alias updated");
        assert!(after.is_main, "is_main preserved");
        assert_eq!(
            after.core_wallet_name.as_deref(),
            Some("local-dashd"),
            "core wallet name preserved"
        );
        assert_eq!(after.xpub_encoded, vec![0xAB; 78], "xpub preserved");
        assert!(after.uses_password, "uses_password preserved");
        assert_eq!(
            after.password_hint.as_deref(),
            Some("granny's birthday"),
            "password hint preserved"
        );
    }

    /// A genuinely absent meta row is seeded fresh with the alias and the
    /// wallet's xpub (so the cold-boot picker renders without the seed).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rename_hd_seeds_fresh_meta_when_absent() {
        let f = fixture().await;
        let backend = f.ctx.wallet_backend().expect("backend");
        backend
            .wallet_meta()
            .delete(f.ctx.network, &f.seed_hash)
            .expect("delete registration meta");

        f.ctx
            .rename_hd_wallet(f.seed_hash, "fresh".into())
            .expect("rename");

        let after = backend
            .wallet_meta()
            .get(f.ctx.network, &f.seed_hash)
            .expect("meta present after rename");
        assert_eq!(after.alias, "fresh");
        let expected_xpub = f
            .ctx
            .wallet_arc(&f.seed_hash)
            .expect("wallet")
            .read()
            .expect("read")
            .master_bip44_ecdsa_extended_public_key
            .encode()
            .to_vec();
        assert_eq!(
            after.xpub_encoded, expected_xpub,
            "a fresh meta seeds the wallet xpub"
        );
        assert!(!after.is_main, "fresh meta is not main");
        assert!(
            after.core_wallet_name.is_none(),
            "fresh meta has no core link"
        );
    }

    /// Renaming an unknown seed hash is a genuine `WalletNotFound`, not a
    /// backend transient.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rename_hd_unknown_seed_hash_returns_wallet_not_found() {
        let f = fixture().await;
        let unknown: WalletSeedHash = [0xAB; 32];
        let err = f
            .ctx
            .rename_hd_wallet(unknown, "x".into())
            .expect_err("an unknown wallet must fail");
        assert!(matches!(err, TaskError::WalletNotFound), "got {err:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn overlapping_hd_renames_serialize_so_later_invocation_wins() {
        let store = Arc::new(FirstWalletMetaReadGate::default());
        let f = fixture_with_app_kv(Arc::new(DetKv::from_store(store.clone()))).await;
        store.arm();

        let first_ctx = f.ctx.clone();
        let seed_hash = f.seed_hash;
        let first =
            std::thread::spawn(move || first_ctx.rename_hd_wallet(seed_hash, "first".into()));
        store.wait_until_intercepted();

        let later_ctx = f.ctx.clone();
        let (later_tx, later_rx) = std::sync::mpsc::channel();
        let later = std::thread::spawn(move || {
            let result = later_ctx.rename_hd_wallet(seed_hash, "later".into());
            later_tx.send(result).expect("send later result");
        });

        let later_while_first_blocked = later_rx.recv_timeout(Duration::from_secs(1)).ok();
        store.release();
        first
            .join()
            .expect("first rename thread")
            .expect("first rename");
        let later_result = match later_while_first_blocked {
            Some(result) => result,
            None => later_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("later rename completion"),
        };
        later_result.expect("later rename");
        later.join().expect("later rename thread");

        let alias = f
            .ctx
            .wallet_backend()
            .expect("backend")
            .wallet_meta()
            .try_get(f.ctx.network, &seed_hash)
            .expect("read final meta")
            .expect("final meta")
            .alias;
        assert_eq!(
            alias, "later",
            "the later invocation must be the final persisted alias"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remove_wallet_racing_rename_does_not_resurrect_wallet_meta() {
        let store = Arc::new(FirstWalletMetaReadGate::default());
        let f = fixture_with_app_kv(Arc::new(DetKv::from_store(store.clone()))).await;
        store.arm();

        let rename_ctx = f.ctx.clone();
        let seed_hash = f.seed_hash;
        let rename = std::thread::spawn(move || {
            rename_ctx.rename_hd_wallet(seed_hash, "rename in flight".into())
        });
        store.wait_until_intercepted();

        let remove_ctx = f.ctx.clone();
        let runtime_handle = tokio::runtime::Handle::current();
        let (remove_tx, remove_rx) = std::sync::mpsc::channel();
        let remove = std::thread::spawn(move || {
            let _runtime_guard = runtime_handle.enter();
            let result = remove_ctx.remove_wallet(&seed_hash);
            remove_tx.send(result).expect("send removal result");
        });

        let removal_while_rename_blocked = remove_rx.recv_timeout(Duration::from_secs(1)).ok();
        store.release();
        rename
            .join()
            .expect("rename thread")
            .expect("rename completion");
        let removal_result = match removal_while_rename_blocked {
            Some(result) => result,
            None => remove_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("removal completion"),
        };
        removal_result.expect("remove wallet");
        remove.join().expect("remove thread");

        let meta = f
            .ctx
            .wallet_backend()
            .expect("backend")
            .wallet_meta()
            .try_get(f.ctx.network, &seed_hash)
            .expect("read final meta");
        assert!(
            meta.is_none(),
            "a completed removal must leave wallet metadata deleted"
        );
    }

    /// The single-key rename persists the new alias through the typed chokepoint
    /// and returns the `SingleKeyAliasRenamed` result carrying it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rename_single_key_persists_and_returns_typed_result() {
        let f = fixture().await;
        let backend = f.ctx.wallet_backend().expect("backend");
        // Mint a deterministic throwaway key rather than committing a WIF.
        let sk = SecretKey::from_byte_array(&[0x11u8; 32]).expect("valid scalar");
        let wif = PrivateKey::new(sk, Network::Testnet).to_wif();
        let imported = backend
            .single_key()
            .import_wif(&wif, Some("old name".into()))
            .expect("import");
        let address = imported.address.clone();

        let result = f
            .ctx
            .rename_single_key_wallet(address.clone(), "new name".into())
            .expect("rename");
        assert!(
            matches!(
                &result,
                BackendTaskSuccessResult::SingleKeyAliasRenamed { address: a, alias }
                    if *a == address && alias == "new name"
            ),
            "got {result:?}"
        );

        let listed = backend.single_key().list();
        let entry = listed
            .iter()
            .find(|e| e.address == address)
            .expect("imported key present");
        assert_eq!(entry.alias.as_deref(), Some("new name"), "alias persisted");
    }

    /// Renaming an address that was never imported surfaces the typed
    /// `ImportedKeyNotFound`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rename_single_key_unknown_address_is_typed_not_found() {
        let f = fixture().await;
        let err = f
            .ctx
            .rename_single_key_wallet("yNeverImported".into(), "x".into())
            .expect_err("an unknown address must fail");
        assert!(matches!(err, TaskError::ImportedKeyNotFound), "got {err:?}");
    }
}
