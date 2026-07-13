//! Shared [`AppContext`] fixtures for unit tests in the `context` module tree.
//!
//! Follows the `kv_test_support` pattern: one fixture every test wires against
//! instead of hand-rolling a copy of the ~15-line construction dance.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicU8;

use dash_sdk::dpp::dashcore::Network;

use super::AppContext;
use crate::wallet_backend::DetKv;

/// Build a network-free [`AppContext`] backed by throwaway temp storage — enough
/// to exercise the settings read-modify-write and feature-gate paths.
pub(crate) fn test_app_context(dir: &Path) -> Arc<AppContext> {
    let app_kv = AppContext::open_app_kv(dir).expect("open app k/v");
    test_app_context_with_kv(dir, app_kv)
}

/// [`test_app_context`] with a caller-supplied app k/v store, so a test can
/// inject a fault-injecting backing store (see `kv_test_support::FailingKv`).
pub(crate) fn test_app_context_with_kv(dir: &Path, app_kv: Arc<DetKv>) -> Arc<AppContext> {
    crate::app_dir::ensure_env_file(dir);
    let db = Arc::new(crate::database::Database::new(dir.join("data.db")).expect("db"));
    db.create_tables(true).expect("create tables");
    db.set_default_version().expect("set version");
    let secret_store = AppContext::open_secret_store(dir).expect("open secret store");
    AppContext::new(
        dir.to_path_buf(),
        Network::Testnet,
        db,
        Default::default(),
        Default::default(),
        egui::Context::default(),
        app_kv,
        secret_store,
        Arc::new(AtomicU8::new(0)),
    )
    .expect("AppContext")
}
