use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::ui::MessageType;
use dash_evo_tool::ui::components::MessageBanner;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

#[test]
fn old_network_banner_explains_recovery_without_protocol_jargon() {
    let error = TaskError::PlatformNetworkTooOld {
        verified_version: 13,
    };
    let text = error.to_string();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, 300.0))
        .build_ui(move |ui| {
            let handle = MessageBanner::set_global(ui.ctx(), error.to_string(), MessageType::Error);
            handle.with_details(&error);
            handle.disable_auto_dismiss();
            MessageBanner::show_global(ui);
        });
    harness.run();
    assert!(harness.query_by_label(&text).is_some());
    assert!(text.contains("Wait for the network to upgrade"));
    assert!(text.contains("Settings"));
    assert!(!text.to_lowercase().contains("protocol"));
}
