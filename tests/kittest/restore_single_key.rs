//! Kittest coverage for the T-SK-03 "Restore a protected imported key"
//! dialog. Drives [`RestoreSingleKeyDialog`] directly through
//! `show_in_ui` so the harness needs no wallets-screen instance.
//!
//! The restore *logic* round-trip (decrypt → re-import → gate
//! recognition) is covered by the integration tests in
//! `src/context/wallet_lifecycle.rs`; this exercises the rendered UI.

use crate::support::with_isolated_data_dir;
use dash_evo_tool::backend_task::migration::single_key_restore::PendingProtectedRestore;
use dash_evo_tool::ui::wallets::restore_single_key::RestoreSingleKeyDialog;
use dash_sdk::dpp::dashcore::Network;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn targeted_dialog() -> RestoreSingleKeyDialog {
    let mut dialog = RestoreSingleKeyDialog::new();
    dialog.set_target(PendingProtectedRestore {
        address: "yProtectedRestoreAddr".into(),
        alias: Some("savings".into()),
        network: Network::Testnet,
    });
    dialog
}

/// A targeted dialog renders the protected key's address, its alias, the
/// old-password prompt, and the Restore action so the user can complete
/// the flow.
#[test]
fn targeted_dialog_renders_address_and_old_password_prompt() {
    with_isolated_data_dir(|| {
        let mut dialog = targeted_dialog();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(640.0, 480.0))
            .build_ui(move |ui| {
                dialog.show_in_ui(ui);
            });
        harness.run();

        assert!(
            harness
                .query_by_label_contains("yProtectedRestoreAddr")
                .is_some(),
            "the protected address must be shown so the user can identify the key"
        );
        assert!(
            harness.query_by_label_contains("savings").is_some(),
            "the key's nickname should be shown"
        );
        assert!(
            harness.query_by_label("Old password").is_some(),
            "the old-password prompt must be present"
        );
        assert!(
            harness.query_by_label("Restore key").is_some(),
            "the Restore action must be present"
        );
    });
}

/// The "Protect with a new passphrase" toggle is on by default and
/// reveals the new-passphrase fields, but a user can complete a restore
/// without them (the toggle drives optional re-protection).
#[test]
fn new_passphrase_fields_render_when_protect_again_enabled() {
    with_isolated_data_dir(|| {
        let mut dialog = targeted_dialog();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(640.0, 480.0))
            .build_ui(move |ui| {
                dialog.show_in_ui(ui);
            });
        harness.run();

        assert!(
            harness
                .query_by_label("Protect with a new passphrase")
                .is_some(),
            "the re-protection toggle must be present"
        );
        assert!(
            harness.query_by_label("New passphrase").is_some(),
            "new-passphrase field should render while re-protection is on"
        );
    });
}
