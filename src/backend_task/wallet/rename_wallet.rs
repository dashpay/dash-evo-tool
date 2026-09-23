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
    /// `alias` is the raw user input. It is resolved through
    /// the shared wallet context: a blank alias resets the wallet
    /// to the smallest unused "Wallet N", and a name another HD wallet already
    /// uses is rejected. The returned result carries the alias actually saved.
    ///
    /// # Errors
    ///
    /// - [`TaskError::WalletNotFound`] when `seed_hash` matches no loaded wallet.
    /// - [`TaskError::KvSidecarStorage`] when the sidecar cannot be read or written.
    /// - [`TaskError::InvalidWalletAliasLength`] when the cleaned alias exceeds the limit.
    /// - [`TaskError::WalletAliasAlreadyUsed`] when another HD wallet uses the alias.
    pub(crate) fn rename_hd_wallet(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        alias: String,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let alias = self
            .wallet_context()
            .rename_hd(seed_hash, &alias, |xpub_encoded, alias| {
                self.wallet_backend()?;
                let kv = self.app_kv();
                let meta_view = crate::wallet_backend::WalletMetaView::new(&kv);
                let mut meta = meta_view
                    .try_get(self.network, &seed_hash)?
                    .unwrap_or_default();
                meta.alias = alias.to_owned();
                if meta.xpub_encoded.is_empty() {
                    meta.xpub_encoded = xpub_encoded;
                }
                meta_view.set(self.network, &seed_hash, &meta)?;
                Ok(meta)
            })?;

        Ok(BackendTaskSuccessResult::WalletAliasRenamed { seed_hash, alias })
    }

    /// Persist a new alias for an imported single-key wallet to the single-key
    /// sidecar, delegating to the typed
    /// [`SingleKeyView::set_alias`](crate::wallet_backend::single_key::SingleKeyView::set_alias)
    /// chokepoint (which resolves the alias and refreshes the in-memory index).
    /// A blank alias resets the key to the smallest unused "Key N"; the result
    /// carries the alias actually saved.
    ///
    /// # Errors
    ///
    /// - [`TaskError::ImportedKeyNotFound`] when `address` was never imported.
    /// - [`TaskError::SingleKeyMetaStorage`] when the sidecar cannot be written.
    /// - [`TaskError::InvalidWalletAliasLength`] when the cleaned alias exceeds the limit.
    /// - [`TaskError::WalletAliasAlreadyUsed`] when another imported key uses the alias.
    pub(crate) fn rename_single_key_wallet(
        self: &Arc<Self>,
        address: String,
        alias: String,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let backend = self.wallet_backend()?;
        let alias = backend.single_key().set_alias(&address, &alias)?;
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
        fixture_from_parts(dir, app_kv, None).await
    }

    async fn fixture_from_parts(
        dir: TempDir,
        app_kv: Arc<DetKv>,
        prompt: Option<Arc<dyn crate::wallet_backend::SecretPrompt>>,
    ) -> Fixture {
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
        if let Some(prompt) = prompt {
            ctx.install_secret_prompt(prompt);
        }

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
        fixture_from_parts(dir, app_kv, None).await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn legacy_envelope_hint_survives_hydration_and_rename() {
        use crate::model::wallet::meta::WalletMetaV1;
        use crate::model::wallet::seed_envelope::StoredSeedEnvelope;
        use crate::wallet_backend::SecretScope;
        use crate::wallet_backend::secret_prompt::test_support::{ScriptedAnswer, TestPrompt};
        use crate::wallet_backend::wallet_seed_store::WalletSeedView;

        let dir = tempfile::tempdir().unwrap();
        let app_kv = AppContext::open_app_kv(dir.path()).unwrap();
        let prompt = Arc::new(TestPrompt::new([
            ScriptedAnswer::Cancel,
            ScriptedAnswer::Cancel,
            ScriptedAnswer::Cancel,
        ]));
        let f = fixture_from_parts(dir, app_kv, Some(prompt.clone())).await;
        let backend = f.ctx.wallet_backend().expect("backend");
        let meta = backend
            .wallet_meta()
            .get(f.ctx.network, &f.seed_hash)
            .unwrap();
        let key = crate::wallet_backend::wallet_meta::key_for(f.ctx.network, &f.seed_hash);
        f.ctx
            .app_kv()
            .put(
                DetScope::Global,
                &key,
                &WalletMetaV1 {
                    alias: "Legacy".into(),
                    xpub_encoded: meta.xpub_encoded.clone(),
                    ..Default::default()
                },
            )
            .unwrap();
        let seeds = WalletSeedView::new(backend.secret_store());
        seeds.delete_raw(&f.seed_hash).unwrap();
        seeds
            .set(
                &f.seed_hash,
                &StoredSeedEnvelope {
                    encrypted_seed: zeroize::Zeroizing::new(vec![0; 80]),
                    salt: vec![0; 16],
                    nonce: vec![0; 12],
                    password_hint: Some("Legacy hint".into()),
                    uses_password: true,
                    xpub_encoded: meta.xpub_encoded,
                },
            )
            .unwrap();
        f.ctx.wallet_context().clear();
        backend.hydrate_context_wallets(&f.ctx).unwrap();
        let scope = SecretScope::HdSeed {
            seed_hash: f.seed_hash,
        };
        for label in ["Legacy", "Renamed", "After unlock"] {
            if label == "Renamed" {
                f.ctx.rename_hd_wallet(f.seed_hash, label.into()).unwrap();
                backend.hydrate_context_wallets(&f.ctx).unwrap();
            }
            if label == "After unlock" {
                seeds
                    .set_protected(
                        &f.seed_hash,
                        &[0x5A; 64],
                        &platform_wallet_storage::secrets::SecretString::new("test-only-password"),
                    )
                    .unwrap();
                seeds.delete(&f.seed_hash).unwrap();
                f.ctx.rename_hd_wallet(f.seed_hash, label.into()).unwrap();
                backend.hydrate_context_wallets(&f.ctx).unwrap();
            }
            assert!(
                backend
                    .secret_access()
                    .with_secret(&scope, |_| Ok(()))
                    .await
                    .is_err()
            );
            let request = prompt.requests().pop().expect("password prompt");
            assert_eq!(request.display_label, label);
            assert_eq!(request.hint.as_deref(), Some("Legacy hint"));
        }
    }

    #[derive(Default)]
    struct ReadGateState {
        armed: bool,
        intercept_write: bool,
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

        fn arm_write(&self) {
            self.arm();
            self.state.lock().unwrap().intercept_write = true;
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
            if state.armed
                && !state.intercept_write
                && !state.intercepted
                && key.contains(":wallet_meta:")
            {
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
            let mut state = self.state.lock().unwrap();
            if state.armed
                && state.intercept_write
                && !state.intercepted
                && key.contains(":wallet_meta:")
            {
                state.intercepted = true;
                self.changed.notify_all();
                state = self
                    .changed
                    .wait_while(state, |state| !state.released)
                    .unwrap();
            }
            drop(state);
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

    /// Register a second HD wallet named `alias` in the fixture context.
    fn register_other_wallet(f: &Fixture, alias: &str) -> WalletSeedHash {
        let seed = [0x6Bu8; 64];
        let wallet = Wallet::new_from_seed(seed, Network::Testnet, Some(alias.into()), None)
            .expect("build other wallet");
        f.ctx
            .register_wallet(wallet, &seed, WalletOrigin::Fresh)
            .expect("register other wallet")
            .0
    }

    fn in_memory_alias(f: &Fixture) -> Option<String> {
        f.ctx.wallet_context().hd_alias(&f.seed_hash)
    }

    fn persisted_alias(f: &Fixture) -> String {
        f.ctx
            .wallet_backend()
            .expect("backend")
            .wallet_meta()
            .get(f.ctx.network, &f.seed_hash)
            .expect("meta present")
            .alias
    }

    /// Renaming to a blank name is supported: it resets the wallet to the
    /// smallest unused default name, in memory and on disk.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rename_hd_blank_alias_resets_to_default_name() {
        let f = fixture().await;
        f.ctx
            .rename_hd_wallet(f.seed_hash, "Custom".into())
            .expect("rename to a custom name");
        register_other_wallet(&f, "Wallet 1");

        let result = f
            .ctx
            .rename_hd_wallet(f.seed_hash, " \u{200B} ".into())
            .expect("a blank rename resets to the default name");

        assert!(
            matches!(
                &result,
                BackendTaskSuccessResult::WalletAliasRenamed { alias, .. } if alias == "Wallet 2"
            ),
            "got {result:?}"
        );
        assert_eq!(persisted_alias(&f), "Wallet 2");
        assert_eq!(in_memory_alias(&f).as_deref(), Some("Wallet 2"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rename_hd_cleans_alias_and_updates_memory() {
        let f = fixture().await;

        let result = f
            .ctx
            .rename_hd_wallet(f.seed_hash, "  \u{202E}Spending ".into())
            .expect("rename");

        assert!(
            matches!(
                &result,
                BackendTaskSuccessResult::WalletAliasRenamed { alias, .. } if alias == "Spending"
            ),
            "got {result:?}"
        );
        assert_eq!(persisted_alias(&f), "Spending");
        assert_eq!(in_memory_alias(&f).as_deref(), Some("Spending"));
    }

    /// A wallet never collides with itself.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rename_hd_to_its_own_alias_succeeds() {
        let f = fixture().await;
        f.ctx
            .rename_hd_wallet(f.seed_hash, "Savings".into())
            .expect("first rename");

        f.ctx
            .rename_hd_wallet(f.seed_hash, "Savings".into())
            .expect("renaming to the current name succeeds");

        assert_eq!(persisted_alias(&f), "Savings");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rename_hd_to_another_wallets_alias_is_rejected_without_writing() {
        let f = fixture().await;
        f.ctx
            .rename_hd_wallet(f.seed_hash, "Spending".into())
            .expect("first rename");
        register_other_wallet(&f, "Savings");

        let err = f
            .ctx
            .rename_hd_wallet(f.seed_hash, "Savings".into())
            .expect_err("the name belongs to another wallet");

        assert!(
            matches!(err, TaskError::WalletAliasAlreadyUsed { .. }),
            "got {err:?}"
        );
        assert_eq!(persisted_alias(&f), "Spending");
        assert_eq!(in_memory_alias(&f).as_deref(), Some("Spending"));
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
    async fn concurrent_hd_registrations_assign_distinct_default_names() {
        let store = Arc::new(FirstWalletMetaReadGate::default());
        let f = fixture_with_app_kv(Arc::new(DetKv::from_store(store.clone()))).await;
        store.arm_write();
        let register = |byte| {
            let ctx = f.ctx.clone();
            let runtime = tokio::runtime::Handle::current();
            std::thread::spawn(move || {
                let _runtime_guard = runtime.enter();
                let seed = [byte; 64];
                let wallet = Wallet::new_from_seed(seed, Network::Testnet, None, None).unwrap();
                ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh)
            })
        };
        let first = register(0x61);
        store.wait_until_intercepted();
        let second = register(0x62);
        std::thread::sleep(Duration::from_millis(200));
        store.release();
        let first = first.join().unwrap().unwrap();
        let second = second.join().unwrap().unwrap();
        let mut aliases = [first, second].map(|(hash, _wallet)| {
            let alias = f.ctx.wallet_context().hd_alias(&hash).unwrap();
            assert_eq!(
                f.ctx
                    .wallet_backend()
                    .unwrap()
                    .wallet_meta()
                    .get(f.ctx.network, &hash)
                    .unwrap()
                    .alias,
                alias
            );
            alias
        });
        aliases.sort();
        assert_eq!(aliases, ["Wallet 2", "Wallet 3"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn registration_racing_hd_rename_rejects_collision_without_seed_write() {
        use crate::wallet_backend::wallet_seed_store::WalletSeedView;

        let store = Arc::new(FirstWalletMetaReadGate::default());
        let f = fixture_with_app_kv(Arc::new(DetKv::from_store(store.clone()))).await;
        store.arm();
        let ctx = f.ctx.clone();
        let hash = f.seed_hash;
        let rename = std::thread::spawn(move || ctx.rename_hd_wallet(hash, "Savings".into()));
        store.wait_until_intercepted();
        let seed = [0x63; 64];
        let wallet =
            Wallet::new_from_seed(seed, Network::Testnet, Some("Savings".into()), None).unwrap();
        let new_hash = wallet.seed_hash();
        let ctx = f.ctx.clone();
        let runtime = tokio::runtime::Handle::current();
        let (tx, rx) = std::sync::mpsc::channel();
        let registration = std::thread::spawn(move || {
            let _runtime_guard = runtime.enter();
            tx.send(ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh))
                .unwrap();
        });
        let early = rx.recv_timeout(Duration::from_millis(200)).ok();
        store.release();
        rename.join().unwrap().unwrap();
        let result = early.unwrap_or_else(|| rx.recv_timeout(Duration::from_secs(5)).unwrap());
        registration.join().unwrap();
        assert!(matches!(
            result,
            Err(TaskError::WalletAliasAlreadyUsed { .. })
        ));
        let secrets = f.ctx.secret_store();
        let seeds = WalletSeedView::new(&secrets);
        assert!(seeds.get_raw(&new_hash).unwrap().is_none());
        assert!(seeds.get(&new_hash).unwrap().is_none());
        assert!(!f.ctx.wallet_context().wallets().contains_key(&new_hash));
        assert!(
            f.ctx
                .wallet_backend()
                .unwrap()
                .wallet_meta()
                .get(f.ctx.network, &new_hash)
                .is_none()
        );
        assert_eq!(persisted_alias(&f), "Savings");
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
        let (imported, display_wallet) = f
            .ctx
            .import_single_key_wif(
                &wif,
                crate::model::wallet::alias::AliasSource::UserEntered("old name".into()),
                Default::default(),
            )
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

        assert_eq!(
            f.ctx
                .wallet_context()
                .single_alias(&display_wallet.read().unwrap().address.to_string())
                .as_deref(),
            Some("new name")
        );

        let listed = backend.single_key().list();
        let entry = listed
            .iter()
            .find(|e| e.address == address)
            .expect("imported key present");
        assert_eq!(entry.alias.as_deref(), Some("new name"), "alias persisted");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn single_key_reimport_publishes_metadata_without_mutating_retained_handles() {
        use crate::model::wallet::alias::AliasSource;
        let f = fixture().await;
        let backend = f.ctx.wallet_backend().expect("backend");
        let key = SecretKey::from_byte_array(&[0x33; 32]).expect("valid scalar");
        let wif = PrivateKey::new(key, Network::Testnet).to_wif();
        let (first, retained) = f
            .ctx
            .import_single_key_wif(
                &wif,
                AliasSource::UserEntered("First".into()),
                Default::default(),
            )
            .unwrap();
        let retained_read = retained.read().unwrap();
        let (second, _) = f
            .ctx
            .import_single_key_wif(
                &wif,
                AliasSource::UserEntered("Second".into()),
                Default::default(),
            )
            .unwrap();
        assert_eq!(first.address, second.address);
        assert_eq!(
            f.ctx
                .wallet_context()
                .single_alias(&retained_read.address.to_string())
                .as_deref(),
            Some("Second")
        );
        assert_eq!(
            backend.single_key().list()[0].alias.as_deref(),
            Some("Second")
        );
        assert_eq!(f.ctx.wallet_context().single_key_wallets().len(), 1);
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

    /// Renaming a key to a blank name resets it to the smallest unused
    /// "Key N" and the result carries that name.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rename_single_key_blank_alias_resets_to_default_name() {
        let f = fixture().await;
        let backend = f.ctx.wallet_backend().expect("backend");
        let sk = SecretKey::from_byte_array(&[0x22u8; 32]).expect("valid scalar");
        let wif = PrivateKey::new(sk, Network::Testnet).to_wif();
        let address = backend
            .single_key()
            .import_wif(
                &wif,
                crate::model::wallet::alias::AliasSource::UserEntered("Custom".into()),
            )
            .expect("import")
            .address;

        let result = f
            .ctx
            .rename_single_key_wallet(address.clone(), String::new())
            .expect("a blank rename resets to the default name");

        assert!(
            matches!(
                &result,
                BackendTaskSuccessResult::SingleKeyAliasRenamed { alias, .. } if alias == "Key 1"
            ),
            "got {result:?}"
        );
        let listed = backend.single_key().list();
        let entry = listed
            .iter()
            .find(|e| e.address == address)
            .expect("imported key present");
        assert_eq!(entry.alias.as_deref(), Some("Key 1"));
    }
}
