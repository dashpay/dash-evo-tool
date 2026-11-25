//! The Identities hub hints that a locked wallet is not searched for
//! identities, offers to unlock it, and drops the hint once it is open.

use crate::support::{mount_app, with_isolated_data_dir};
use dash_evo_tool::model::secret::Secret;
use dash_evo_tool::model::wallet::Wallet;
use dash_evo_tool::ui::RootScreenType;
use egui_kittest::kittest::Queryable;
use std::sync::{Arc, RwLock};

const HINT: &str =
    "The wallet \"Savings\" is locked. Unlock it to look for identities created with it.";

fn protected_wallet(network: dash_sdk::dpp::dashcore::Network, seed: [u8; 64]) -> Wallet {
    Wallet::new_from_seed(
        seed,
        network,
        Some("Savings".to_string()),
        Some(&Secret::new("correct password")),
    )
    .expect("build protected wallet")
}

#[test]
fn locked_wallet_hint_offers_unlock_and_disappears_once_open() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let _guard = rt.enter();
        let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
        let app_context = harness.state().current_app_context().clone();
        let seed = dash_evo_tool::test_support::keys::seed();

        let mut locked = protected_wallet(app_context.network(), seed);
        locked.wallet_seed.close();
        let seed_hash = locked.seed_hash();
        app_context
            .wallet_context()
            .insert_test_wallet(seed_hash, Arc::new(RwLock::new(locked)));
        harness.run_steps(3);

        assert!(
            harness.query_by_label(HINT).is_some(),
            "a locked wallet must be named in the hub's hint"
        );
        harness
            .get_by_role_and_label(egui::accesskit::Role::Link, "Unlock")
            .click();
        harness.run_steps(3);
        assert!(
            harness
                .query_by_label("Enter password to unlock \"Savings\":")
                .is_some(),
            "the hint's link must open the unlock prompt for that wallet"
        );

        // An open protected wallet has nothing left to discover behind a password.
        app_context.wallet_context().insert_test_wallet(
            seed_hash,
            Arc::new(RwLock::new(protected_wallet(app_context.network(), seed))),
        );
        harness.run_steps(3);
        assert!(
            harness.query_by_label(HINT).is_none(),
            "the hint must disappear once no wallet is locked"
        );
    });
}
