use crate::det_harness::DETHarness;

/// Start the Dash Evo Tool app and click on all icons in the left panel
/// to ensure they are clickable and correct screens are shown.
#[test]
fn test_left_panel() {
    // Create a test harness for the egui app

    DETHarness::new("left_panel_icon_clicks")
        .try_execute("app_startup", |det| {
            // label => text to find
            let buttons = vec![
                ("I", "Identities"),
                ("Q", "Contracts"),
                ("O", "Tokens"),
                ("C", "DPNS Subscreens"),
                ("W", "Wallets"),
                ("T", "Tools"),
                ("N", "Networks"),
            ];
            for (button, text) in buttons {
                let icon = det.get_by_label(button);
                icon.click();
                det.run();
                det.snapshot(&format!("app_startup.{}", button));
                let nodes: Vec<_> = det.query_all_by_label(text);
                tracing::debug!(count = nodes.len(), ?nodes, "Clicked on icon: {}", button);
            }
        })
        .expect("Failed to run app startup test");
}
