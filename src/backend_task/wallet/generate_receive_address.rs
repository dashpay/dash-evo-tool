use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use std::sync::Arc;

impl AppContext {
    /// Generate a fresh receive address via the wallet backend.
    pub(crate) async fn generate_receive_address(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // A seed hash that matches no wallet in the local store is a genuine
        // "not found". This is distinct from a known wallet whose backend is
        // still loading: the backend reports the latter as the transient,
        // retryable `WalletNotLoaded`. Resolving the existence question here,
        // where the DET-side wallet store lives, keeps that distinction honest
        // instead of collapsing both cases into `WalletNotLoaded`.
        if !self.wallets.read()?.contains_key(&seed_hash) {
            return Err(TaskError::WalletNotFound);
        }
        let backend = self.wallet_backend()?;
        let address = backend.next_receive_address(&seed_hash).await?;
        Ok(BackendTaskSuccessResult::GeneratedReceiveAddress { seed_hash, address })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_dir::ensure_env_file;
    use crate::context::connection_status::ConnectionStatus;
    use crate::database::test_helpers::create_database_at_path;
    use crate::utils::tasks::TaskManager;
    use dash_sdk::dpp::dashcore::Network;

    /// Regression for #860: a receive-address request for a seed hash that
    /// matches no locally-stored wallet must return `WalletNotFound`, NOT the
    /// transient `WalletNotLoaded`. The existence check runs before the wallet
    /// backend is consulted, so this holds even with no backend wired.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unknown_seed_hash_returns_wallet_not_found() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let data_dir = temp_dir.path().to_path_buf();
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
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .expect("offline testnet AppContext::new");

        // No wallets are loaded, so any seed hash is genuinely unknown.
        let unknown: WalletSeedHash = [0xAB; 32];
        let err = ctx
            .generate_receive_address(unknown)
            .await
            .expect_err("an unknown seed hash must fail, not succeed");
        assert!(
            matches!(err, TaskError::WalletNotFound),
            "expected WalletNotFound, got {err:?}"
        );
    }
}
