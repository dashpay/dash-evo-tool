//! Kittest coverage for `DashPayScreen` typed-error routing.
//!
//! F95 regression: the embedded `ContactRequests` classifies a
//! missing-encryption-key failure and surfaces an inline "Add Encryption
//! Key" affordance, but only if `DashPayScreen` forwards `display_task_error`
//! to the active subscreen. Before the fix `DashPayScreen` used the
//! `ScreenLike` default (always `false`), so the classification never ran.

use crate::support::with_isolated_data_dir;
use dash_evo_tool::backend_task::dashpay::errors::DashPayError;
use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::ui::ScreenLike;
use dash_evo_tool::ui::dashpay::{DashPayScreen, DashPaySubscreen};
use egui_kittest::Harness;

/// On the Contacts subscreen, a `MissingEncryptionKey` error must route to
/// the embedded `ContactRequests`, which claims it (returns `true`) and
/// arms its inline recovery affordance. An unrelated error must NOT be
/// claimed, so it falls through to the global banner.
#[test]
fn missing_encryption_key_error_routes_to_embedded_contact_requests() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(50).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("create AppState")
                .with_animations(false)
        });
        harness.run_steps(3);

        let app_context = harness.state().current_app_context().clone();
        let mut screen = DashPayScreen::new(&app_context, DashPaySubscreen::Contacts);

        let handled =
            screen.display_task_error(&TaskError::DashPay(DashPayError::MissingEncryptionKey));
        assert!(
            handled,
            "Contacts subscreen must claim the missing-encryption-key error so the inline \
             recovery affordance can render"
        );

        let unrelated = screen.display_task_error(&TaskError::DocumentNotFound);
        assert!(
            !unrelated,
            "an unrelated error must fall through to the global banner, not be swallowed"
        );
    });
}

/// Non-Contacts subscreens have no classifier embedded, so they must not
/// claim the error — it belongs to the global banner there.
#[test]
fn profile_subscreen_does_not_claim_dashpay_errors() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(50).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("create AppState")
                .with_animations(false)
        });
        harness.run_steps(3);

        let app_context = harness.state().current_app_context().clone();
        let mut screen = DashPayScreen::new(&app_context, DashPaySubscreen::Profile);

        let handled =
            screen.display_task_error(&TaskError::DashPay(DashPayError::MissingEncryptionKey));
        assert!(
            !handled,
            "Profile subscreen has no embedded classifier and must defer to the global banner"
        );
    });
}
