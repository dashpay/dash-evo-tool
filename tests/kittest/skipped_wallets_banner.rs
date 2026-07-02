//! kittest coverage for the persisted-wallet load-skip banner seam.
//!
//! `WalletBackend::register_persisted_wallets` raises the banner via
//! `MessageBanner::set_global(ctx.egui_ctx(), skipped_wallets_banner_text(n),
//! MessageType::Warning)`. The async/`AppContext` wrapper can't be booted in a
//! unit test, so these drive the exact copy + banner type the app emits
//! through the public `MessageBanner` surface — the same seam the app loop
//! uses — so a regression in the copy or the Warning type fails here.

use dash_evo_tool::ui::MessageType;
use dash_evo_tool::ui::components::MessageBanner;
use dash_evo_tool::wallet_backend::skipped_wallets_banner_text;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

/// A non-empty skip count renders a Warning-typed banner carrying the
/// singular copy verbatim, alongside the ⚠ icon and a dismiss control.
#[test]
fn skipped_banner_renders_warning_with_singular_copy() {
    let label = skipped_wallets_banner_text(1).expect("one skip yields banner text");

    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 200.0))
        .build_ui(move |ui| {
            MessageBanner::set_global(ui.ctx(), &label, MessageType::Warning);
            MessageBanner::show_global(ui);
        });
    harness.run();

    assert!(
        harness
            .query_by_label(
                "1 saved wallet couldn't be opened. Re-add it from its recovery phrase to restore it."
            )
            .is_some(),
        "skip banner must render the singular copy verbatim",
    );
    // Warning icon (color is not the only signal) + dismiss control present.
    assert!(
        harness.query_by_label("\u{26A0}").is_some(),
        "warning icon must render alongside text",
    );
    assert!(harness.query_by_label("\u{274C}").is_some());
}

/// A skip count above one renders the plural copy with the count.
#[test]
fn skipped_banner_renders_plural_copy() {
    let label = skipped_wallets_banner_text(3).expect("three skips yield banner text");

    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 200.0))
        .build_ui(move |ui| {
            MessageBanner::set_global(ui.ctx(), &label, MessageType::Warning);
            MessageBanner::show_global(ui);
        });
    harness.run();

    assert!(
        harness
            .query_by_label(
                "3 saved wallets couldn't be opened. Re-add them from their recovery phrases to restore them."
            )
            .is_some(),
        "skip banner must render the plural copy verbatim",
    );
}

/// Zero skips yields no banner text, so no banner is rendered.
#[test]
fn no_skips_renders_no_banner() {
    assert!(
        skipped_wallets_banner_text(0).is_none(),
        "zero skips must not produce banner text",
    );
}
