//! Cancellable local export, with direct path entry when the native chooser is unavailable.

use super::*;
use crate::ui::state::token_creator::json_export::ContractJsonExport;
use dash_sdk::dpp::dashcore::Network;

pub(super) fn show(
    context: &Context,
    export: &mut Option<ContractJsonExport>,
    network: Network,
) -> AppAction {
    let Some(state) = export.as_mut() else {
        return AppAction::None;
    };
    let mut action = AppAction::None;
    let mut close = false;
    let saving = state.saving.is_some();
    let response = egui::Modal::new(egui::Id::new("token_contract_json_export")).show(context, |ui| {
        ui.set_width(540.0_f32.min(context.content_rect().width() - 2.0 * Spacing::LG));
        ui.heading("Save contract JSON");
        ui.label("Save the reviewed contract exactly as shown. Enter a file path or browse for a location.");
        ui.add_space(Spacing::SM);
        ui.add_enabled_ui(!saving && state.picker.is_none(), |ui| {
            let label = ui.label("File path");
            if ui.add(styled_text_edit_singleline(&mut state.path).desired_width(f32::INFINITY)).labelled_by(label.id).changed() {
                state.path_edited();
            }
            if ui.button("Browse…").clicked() {
                action = state.browse(network);
            }
        });
        if state.picker.is_some() {
            ui.label("Choose a location in the file chooser. If it does not appear, cancel choosing and enter a file path here.");
            if ui.button("Cancel file chooser").clicked() {
                state.cancel_picker();
            }
        }
        if let Some(notice) = &state.notice { ui.label(notice); }
        if let Some(path) = &state.replacement {
            ui.label(format!("A file already exists at {path}. Replace its contents?", path = path.display()));
        }
        if saving {
            ui.spinner();
            ui.label("Saving the contract file. Wait for the save to finish.");
        }
        ui.add_space(Spacing::MD);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_enabled_ui(!saving && state.picker.is_none() && !state.path.is_empty(), |ui| {
                if state.replacement.is_some() {
                    if ComponentStyles::add_danger_button(ui, "Replace file").clicked() { action = state.save(network, true); }
                } else if ComponentStyles::add_primary_button(ui, "Save").clicked() {
                    action = state.save(network, false);
                }
            });
            if ui.add_enabled(!saving, egui::Button::new("Cancel")).clicked() { close = true; }
        });
    });
    if !saving && (close || response.should_close()) {
        *export = None;
        // Closing the modal takes precedence over a click dispatched in this frame.
        action = AppAction::None;
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_task::{BackendTask, BackendTaskSuccessResult, tokens::TokenTask};
    use egui_kittest::kittest::{NodeT, Queryable};

    #[test]
    fn token_json_export_pending_chooser_can_be_cancelled_to_save_directly() {
        let mut harness = egui_kittest::Harness::new_ui_state(
            |ui, state: &mut (Option<ContractJsonExport>, Vec<AppAction>)| {
                let action = show(ui.ctx(), &mut state.0, Network::Testnet);
                if !matches!(action, AppAction::None) {
                    state.1.push(action);
                }
            },
            (
                Some(ContractJsonExport::new("reviewed JSON".into())),
                Vec::new(),
            ),
        );
        harness.get_by_label("Browse…").click();
        harness.run_steps(4);
        assert_eq!(harness.state().1.len(), 1);
        let AppAction::BackendTaskWithContext {
            context: pending, ..
        } = &harness.state().1[0]
        else {
            panic!("Expected chooser task")
        };
        let pending = pending.clone();
        harness.get_by_label("Cancel file chooser").click();
        harness.run_steps(2);
        let export = harness.state_mut().0.as_mut().unwrap();
        export.path = "/chosen/contract.json".into();
        export.path_edited();
        export.result(
            &pending,
            &BackendTaskSuccessResult::TokenContractJsonPath(Some("/late/contract.json".into())),
        );
        harness.run_steps(2);
        harness.get_by_label("Save").click();
        harness.run_steps(2);
        assert_eq!(harness.state().1.len(), 2);
        let AppAction::BackendTaskWithContext {
            task: BackendTask::TokenTask(task),
            ..
        } = &harness.state().1[1]
        else {
            panic!("Expected save task")
        };
        assert!(
            matches!(task.as_ref(), TokenTask::SaveTokenContractJson { path, json, replace: false } if path == std::path::Path::new("/chosen/contract.json") && json == "reviewed JSON")
        );
        assert!(
            harness
                .get_by_label("Cancel")
                .accesskit_node()
                .is_disabled()
        );
        harness.get_by_label("Saving the contract file. Wait for the save to finish.");
        assert!(harness.state().0.is_some());
    }
}
