//! Shared test harness for backend E2E tests.
//!
//! Provides [`BackendTestContext`] which constructs an [`AppContext`] headlessly
//! (no display, no GUI) suitable for testing backend flows against a live network.

use dash_evo_tool::app_dir::copy_env_file_if_not_exists;
use dash_evo_tool::context::AppContext;
use dash_evo_tool::context::connection_status::ConnectionStatus;
use dash_evo_tool::database::Database;
use dash_evo_tool::database::test_helpers::create_temp_database;
use dash_evo_tool::spv::{CoreBackendMode, SpvStatus};
use dash_evo_tool::utils::tasks::TaskManager;
use dash_sdk::dpp::dashcore::Network;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::timeout;

/// Headless [`AppContext`] wrapper for backend E2E tests.
///
/// On creation, copies `.env.example` (with real testnet DAPI addresses) to
/// the app config directory if absent, then constructs `AppContext` with an
/// in-memory database and a headless `egui::Context`.
///
/// Drop triggers a graceful SPV shutdown.
pub struct BackendTestContext {
    pub app_context: Arc<AppContext>,
    /// Kept alive so the temp database file is not deleted while tests run.
    _temp_dir: TempDir,
}

impl BackendTestContext {
    /// Create a new test context for the given network.
    ///
    /// # Panics
    ///
    /// Panics if `.env` config cannot be loaded or `AppContext` initialization
    /// fails (e.g., missing network config in `.env`).
    pub fn new(network: Network) -> Self {
        // Ensure .env.example is copied so Config::load() finds valid DAPI addresses.
        copy_env_file_if_not_exists();

        let (db, temp_dir) = create_temp_database().expect("Failed to create temp database");
        let db = Arc::new(db);
        let subtasks = Arc::new(TaskManager::new());
        let connection_status = Arc::new(ConnectionStatus::new());

        let app_context = AppContext::new(
            network,
            db,
            None,
            subtasks,
            connection_status,
            egui::Context::default(),
        )
        .expect("AppContext::new should succeed — check that .env has config for this network");

        Self {
            app_context,
            _temp_dir: temp_dir,
        }
    }

    /// Switch to SPV backend mode and start the SPV sync loop.
    pub fn start_spv(&self) -> Result<(), String> {
        self.app_context
            .set_core_backend_mode(CoreBackendMode::Spv);
        self.app_context.start_spv()
    }

    /// Wait until at least one SPV peer is connected, or timeout.
    ///
    /// Returns the [`SpvStatus`] on success, or an error on timeout.
    pub async fn wait_for_spv_peers(
        &self,
        wait_timeout: Duration,
    ) -> Result<SpvStatus, String> {
        let spv = self.app_context.spv_manager().clone();

        timeout(wait_timeout, async move {
            loop {
                let snapshot = spv.status_async().await;
                if snapshot.connected_peers > 0 {
                    return snapshot.status;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        })
        .await
        .map_err(|_| {
            format!(
                "Timed out after {:?} waiting for SPV peers",
                wait_timeout
            )
        })
    }
}

impl Drop for BackendTestContext {
    fn drop(&mut self) {
        // Graceful SPV shutdown
        self.app_context.spv_manager().stop();
        // TaskManager shutdown is handled by its own Drop impl
    }
}
