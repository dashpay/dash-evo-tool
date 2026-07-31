//! Shared [`AppContext`] fixtures for unit tests in the `context` module tree.
//!
//! Follows the `kv_test_support` pattern: one fixture every test wires against
//! instead of hand-rolling a copy of the ~15-line construction dance.

use std::path::Path;
use std::sync::Arc;

use dash_sdk::dpp::dashcore::Network;

use super::AppContext;
use crate::model::user_role::UserRoleCell;
use crate::wallet_backend::DetKv;

/// Build a network-free [`AppContext`] backed by throwaway temp storage — enough
/// to exercise the settings read-modify-write and feature-gate paths.
pub(crate) fn test_app_context(dir: &Path) -> Arc<AppContext> {
    let app_kv = AppContext::open_app_kv(dir).expect("open app k/v");
    test_app_context_with_kv_and_network(dir, app_kv, Network::Testnet)
}

/// Build a throwaway [`AppContext`] for a specific network.
pub(crate) fn test_app_context_for_network(dir: &Path, network: Network) -> Arc<AppContext> {
    let app_kv = AppContext::open_app_kv(dir).expect("open app k/v");
    test_app_context_with_kv_and_network(dir, app_kv, network)
}

/// [`test_app_context`] with a caller-supplied app k/v store, so a test can
/// inject a fault-injecting backing store (see `kv_test_support::FailingKv`).
pub(crate) fn test_app_context_with_kv(dir: &Path, app_kv: Arc<DetKv>) -> Arc<AppContext> {
    test_app_context_with_kv_and_network(dir, app_kv, Network::Testnet)
}

fn test_app_context_with_kv_and_network(
    dir: &Path,
    app_kv: Arc<DetKv>,
    network: Network,
) -> Arc<AppContext> {
    crate::app_dir::ensure_env_file(dir);
    let db = Arc::new(crate::database::Database::new(dir.join("data.db")).expect("db"));
    db.create_tables(true).expect("create tables");
    db.set_default_version().expect("set version");
    let secret_store = AppContext::open_secret_store(dir).expect("open secret store");
    AppContext::new(
        dir.to_path_buf(),
        network,
        db,
        Default::default(),
        Default::default(),
        egui::Context::default(),
        app_kv,
        secret_store,
        UserRoleCell::default(),
    )
    .expect("AppContext")
}
