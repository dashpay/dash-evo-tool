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
        // Existence + xpub: a seed hash matching no locally-stored wallet is a
        // genuine `WalletNotFound`, resolved here where the DET-side wallet
        // store lives rather than collapsed into a backend transient.
        let xpub_encoded = self
            .wallet_arc(&seed_hash)?
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
    use crate::wallet_backend::DetScope;
    use dash_sdk::dpp::dashcore::secp256k1::SecretKey;
    use dash_sdk::dpp::dashcore::{Network, PrivateKey};
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

    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let data_dir = dir.path().to_path_buf();
        ensure_env_file(&data_dir);
        let db = Arc::new(create_database_at_path(&data_dir.join("data.db")).expect("db"));
        let app_kv = AppContext::open_app_kv(&data_dir).expect("app kv");
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
