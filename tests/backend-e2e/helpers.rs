//! Shared test harness for backend E2E tests.
//!
//! Provides [`BackendTestContext`] which constructs an [`AppContext`] headlessly
//! (no display, no GUI) suitable for testing backend flows against a live network.
//!
//! All data (database, SPV chain state, config) is isolated in a temp directory
//! via `DASH_EVO_DATA_DIR` so tests never touch the real user data directory.

use dash_evo_tool::app_dir::copy_env_file_if_not_exists;
use dash_evo_tool::context::AppContext;
use dash_evo_tool::context::connection_status::ConnectionStatus;
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
/// On creation, redirects all app data to a temp directory via
/// `DASH_EVO_DATA_DIR`, copies `.env.example` there, then constructs
/// `AppContext` with an isolated database and a headless `egui::Context`.
///
/// Drop triggers a graceful SPV shutdown.
pub struct BackendTestContext {
    pub app_context: Arc<AppContext>,
    /// Kept alive so the temp directory (and its database/SPV data) is not
    /// deleted while tests run.
    _data_dir: TempDir,
}

impl BackendTestContext {
    /// Create a new test context for the given network.
    ///
    /// All data is isolated: the database, SPV chain state, and `.env` config
    /// live in a temp directory that is cleaned up when this context is dropped.
    ///
    /// # Panics
    ///
    /// Panics if the temp directory cannot be created, `.env` config cannot be
    /// loaded, or `AppContext` initialization fails.
    pub fn new(network: Network) -> Self {
        // Create an isolated data directory for this test run.
        let data_dir = tempfile::tempdir().expect("Failed to create temp data directory");

        // Point all app_dir functions at the temp directory.
        // SAFETY: This test runs single-threaded at this point (before spawning
        // tokio workers). No other thread reads this env var concurrently.
        unsafe { std::env::set_var("DASH_EVO_DATA_DIR", data_dir.path()) };

        // Create the directory structure and copy bundled .env.example so
        // Config::load() finds valid DAPI addresses.
        std::fs::create_dir_all(data_dir.path()).expect("Failed to create data dir");
        copy_env_file_if_not_exists();

        let (db, _db_temp) = create_temp_database().expect("Failed to create temp database");
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
            _data_dir: data_dir,
        }
    }

    /// Switch to SPV backend mode and start the SPV sync loop.
    pub fn start_spv(&self) -> Result<(), String> {
        self.app_context.set_core_backend_mode(CoreBackendMode::Spv);
        self.app_context.start_spv()
    }

    /// Wait until at least one SPV peer is connected, or timeout.
    ///
    /// Returns the [`SpvStatus`] on success, or an error on timeout.
    pub async fn wait_for_spv_peers(&self, wait_timeout: Duration) -> Result<SpvStatus, String> {
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
        .map_err(|_| format!("Timed out after {:?} waiting for SPV peers", wait_timeout))
    }
}

impl Drop for BackendTestContext {
    fn drop(&mut self) {
        // Graceful SPV shutdown
        self.app_context.spv_manager().stop();
        // Clean up the env var override so other tests (if any) don't inherit it.
        // SAFETY: Drop runs after all async tasks are shut down, so no
        // concurrent readers of this env var.
        unsafe { std::env::remove_var("DASH_EVO_DATA_DIR") };
        // TaskManager shutdown is handled by its own Drop impl.
        // _data_dir is dropped here, removing all temp files.
    }
}
