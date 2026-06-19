//! kittest coverage for the secret-storage-seam interim at-rest disclosure
//! (Diziet §Item 1/2/3). Drives the public `MessageBanner` surface against the
//! exact copy the app emits at a migrating unlock, so a wording/type regression
//! fails here without a full `AppState`.

use dash_evo_tool::context::{
    INTERIM_AT_REST_DETAILS, single_key_migration_notice, wallet_migration_notice,
};
use dash_evo_tool::ui::MessageType;
use dash_evo_tool::ui::components::MessageBanner;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

/// QA-007 — Copy A (wallet) renders as a Warning banner with the wallet alias,
/// and the ⚠ icon is present (color is not the only indicator). Warning, not
/// Info, so it does not auto-dismiss on the short timer before it is read.
#[test]
fn qa_007_wallet_migration_notice_renders_as_warning() {
    let copy = wallet_migration_notice("paycheque");
    let copy_for_ui = copy.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 220.0))
        .build_ui(move |ui| {
            MessageBanner::set_global(ui.ctx(), &copy_for_ui, MessageType::Warning)
                .with_details(INTERIM_AT_REST_DETAILS);
            MessageBanner::show_global(ui);
        });
    harness.run();
    assert!(
        harness.query_by_label(&copy).is_some(),
        "Copy A must render verbatim",
    );
    assert!(copy.contains("paycheque"), "Copy A names the wallet alias",);
    // Warning glyph present.
    assert!(
        harness.query_by_label("\u{26A0}").is_some(),
        "Warning banner must show the ⚠ icon",
    );
}

/// QA-007 — Copy B (imported key) renders and names the key label.
#[test]
fn qa_007_single_key_migration_notice_renders() {
    let copy = single_key_migration_notice("savings");
    let copy_for_ui = copy.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 220.0))
        .build_ui(move |ui| {
            MessageBanner::set_global(ui.ctx(), &copy_for_ui, MessageType::Warning)
                .with_details(INTERIM_AT_REST_DETAILS);
            MessageBanner::show_global(ui);
        });
    harness.run();
    assert!(
        harness.query_by_label(&copy).is_some(),
        "Copy B must render verbatim",
    );
    assert!(copy.contains("savings"), "Copy B names the key label");
}

/// QA-007 — Copy A and Copy B MUST be distinct text, or `MessageBanner`'s
/// `set_global` text-dedup would collapse them when a wallet and an imported
/// key migrate in the same session.
#[test]
fn qa_007_wallet_and_single_key_copies_are_distinct() {
    let a = wallet_migration_notice("paycheque");
    let b = single_key_migration_notice("paycheque");
    assert_ne!(
        a, b,
        "Copy A and Copy B must differ so set_global keeps both"
    );

    // Both surface in one harness without collapsing.
    let (a_ui, b_ui) = (a.clone(), b.clone());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 320.0))
        .build_ui(move |ui| {
            MessageBanner::set_global(ui.ctx(), &a_ui, MessageType::Warning);
            MessageBanner::set_global(ui.ctx(), &b_ui, MessageType::Warning);
            MessageBanner::show_global(ui);
        });
    harness.run();
    assert!(
        harness.query_by_label(&a).is_some(),
        "wallet notice present"
    );
    assert!(harness.query_by_label(&b).is_some(), "key notice present");
}

/// QA-007 — the persona-facing copy stays jargon-free (no "AES", "vault",
/// "seam", "encryption", "0600"). The technical detail (Copy D) is opt-in and
/// likewise avoids raw internals.
#[test]
fn qa_007_disclosure_copy_is_jargon_free() {
    let banned = ["AES", "vault", "seam", "encryption", "0600", "AES-GCM"];
    for copy in [
        wallet_migration_notice("w"),
        single_key_migration_notice("k"),
        INTERIM_AT_REST_DETAILS.to_string(),
    ] {
        for word in banned {
            assert!(
                !copy.contains(word),
                "disclosure copy must avoid jargon {word:?}: {copy}",
            );
        }
    }
}
