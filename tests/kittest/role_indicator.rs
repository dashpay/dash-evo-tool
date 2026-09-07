//! The nav-rail interface-mode indicator: hidden for the default role, and
//! carrying a distinct compact label for each raised role so switching modes in
//! Settings is reflected live in the bottom-left corner.

use crate::support::{mount_app, with_isolated_data_dir};
use dash_evo_tool::model::user_role::UserRole;
use dash_evo_tool::ui::RootScreenType;
use egui_kittest::kittest::Queryable;

/// Regression: the bottom-left indicator must read the live shared role and
/// change when the role changes. Below Power it is absent; at Power it reads
/// "🔧 Expert"; at Developer it reads "🔧 Dev" — never the same label for both
/// raised roles (the defect where switching produced no visible change).
#[test]
fn indicator_tracks_role_and_distinguishes_raised_modes() {
    with_isolated_data_dir(|| {
        let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
        let app_context = harness.state().current_app_context().clone();

        app_context.set_user_role(UserRole::Everyday);
        harness.run_steps(3);
        assert_eq!(
            harness.query_all_by_label("🔧 Expert").count(),
            0,
            "no interface-mode indicator below the Power role"
        );
        assert_eq!(
            harness.query_all_by_label("🔧 Dev").count(),
            0,
            "no interface-mode indicator below the Power role"
        );

        app_context.set_user_role(UserRole::Power);
        harness.run_steps(3);
        assert!(
            harness.query_by_label("🔧 Expert").is_some(),
            "the Power role must show the Expert indicator"
        );
        assert_eq!(
            harness.query_all_by_label("🔧 Dev").count(),
            0,
            "the Power role must not show the Developer indicator"
        );

        app_context.set_user_role(UserRole::Developer);
        harness.run_steps(3);
        assert!(
            harness.query_by_label("🔧 Dev").is_some(),
            "the Developer role must show its own distinct indicator"
        );
        assert_eq!(
            harness.query_all_by_label("🔧 Expert").count(),
            0,
            "raising to Developer must replace the Expert indicator, not duplicate it"
        );
    });
}
