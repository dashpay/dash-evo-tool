//! IT-LEFT-PANEL — the left nav rail: nav items read as clickable (pointer
//! cursor on hover), including the text label beneath each icon.

use crate::support::{mount_app, with_isolated_data_dir};
use dash_evo_tool::ui::RootScreenType;
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;

/// The reported bug: hovering a nav item's text label (e.g. "Settings") gave no
/// pointer cursor, so it did not read as clickable. The label is now a clickable
/// widget, so hovering it shows the pointing-hand cursor. `Role::Label` selects
/// the text label (its own widget), never the icon button beside it.
#[test]
fn nav_label_hover_shows_pointer_cursor() {
    with_isolated_data_dir(|| {
        let mut harness = mount_app(RootScreenType::RootScreenWalletsBalances);
        harness.run();

        let label = harness.get_by_role_and_label(Role::Label, "Settings");
        label.hover();
        harness.run();

        assert_eq!(
            harness.output().platform_output.cursor_icon,
            egui::CursorIcon::PointingHand,
            "hovering a nav item's text label must show the pointing-hand cursor"
        );
    });
}
