use super::super::TokenNameLanguage;
use super::*;
use crate::model::token_creator::{NameDraft, Section};
use crate::ui::state::token_creator::Proposal;

pub(super) fn show(ui: &mut Ui, state: &mut CreatorWorkspace) {
    card(ui, |ui| {
        heading(ui, "What are you creating?", "basics", &mut state.help);
        ui.label("Give people a recognizable name, then choose how the supply should work.");
        if let Some(name) = state.draft.names.first_mut() {
            text(
                ui,
                "Token name",
                "name",
                &mut name.singular,
                &mut state.help,
            );
            ui.small("Required. For example, StudioCredit.");
            text(
                ui,
                "Plural name",
                "plural",
                &mut name.plural,
                &mut state.help,
            );
            ui.small("Required. For example, StudioCredits.");
        }
        ui.small("Use at least 3 characters without spaces, within 25 UTF-8 bytes. Accented letters and other multibyte characters leave room for fewer characters.");
        egui::CollapsingHeader::new("Names in other languages").show(ui, |ui| {
            ui.label("Keep one set of names per language.");
            let mut remove = None;
            for (index, name) in state.draft.names.iter_mut().enumerate() {
                ui.push_id(index, |ui| {
                    let caption = label(ui, "Language", "localizations", &mut state.help);
                    egui::ComboBox::from_id_salt("language")
                        .selected_text(&name.language)
                        .show_ui(ui, |ui| {
                            for language in TokenNameLanguage::selection_order() {
                                ui.selectable_value(
                                    &mut name.language,
                                    language.iso_code().into(),
                                    language.ui_label(),
                                );
                            }
                        })
                        .response
                        .labelled_by(caption.id);
                    if index > 0 {
                        text(
                            ui,
                            "Token name",
                            "name",
                            &mut name.singular,
                            &mut state.help,
                        );
                        text(
                            ui,
                            "Plural name",
                            "plural",
                            &mut name.plural,
                            &mut state.help,
                        );
                        if ui.button("Remove language").clicked() {
                            remove = Some(index);
                        }
                    }
                    checkbox(
                        ui,
                        "Include this name in search",
                        "keywords",
                        &mut name.searchable,
                        &mut state.help,
                    );
                });
            }
            if let Some(index) = remove {
                state.draft.names.remove(index);
            }
            if ui.button("Add language").clicked() {
                state.draft.names.push(NameDraft {
                    language: "es".into(),
                    singular: String::new(),
                    plural: String::new(),
                    searchable: false,
                });
            }
        });
        checkbox(
            ui,
            "Capitalize token names",
            "capitalization",
            &mut state.draft.capitalize,
            &mut state.help,
        );
    });
    card(ui, |ui| {
        heading(ui, "Choose a supply policy", "policy", &mut state.help);
        ui.label("A starting point is a choice about power. Nothing is selected for you.");
        ui.horizontal_wrapped(|ui| {
            for (limited, caption, title) in [
                (false, "Compare fixed example", "Fixed supply"),
                (
                    true,
                    "Compare owner example",
                    "Owner-managed, with a hard cap",
                ),
            ] {
                if ui.button(caption).clicked() {
                    state.proposal = Some(Proposal {
                        title,
                        draft: state.draft.example_proposal(limited),
                    });
                }
            }
            if ui.button("Choose custom rules →").clicked() {
                state.draft.policy = Policy::Custom;
                state.section = Section::Supply;
            }
        });
        ui.label(policy_label(state.draft.policy));
        if state.undo.is_some() && ui.button("Undo last example or template").clicked() {
            state.undo_proposal();
        }
        for (title, body, key) in [
            (
                "Fixed supply",
                "1,000 tokens, permanently capped at 1,000. No one can mint more, freeze accounts, pause transfers or change the rules.",
                "fixed-policy",
            ),
            (
                "Owner-managed, with a hard cap",
                "1,000 tokens to start, up to 10,000. The owner can mint, burn their own tokens and pause or resume transfers. All other issuer powers stay off.",
                "owner-policy",
            ),
        ] {
            label(ui, title, key, &mut state.help);
            ui.label(body);
            ui.add_space(12.0);
        }
        ui.small("These are explicit custom examples, separate from the protocol templates below.");
        egui::CollapsingHeader::new("Existing protocol templates").show(ui, |ui| {
            ui.label("These retain the original protocol permissions. They differ from the two examples above. Compare every performer, administrator and allowance.");
            use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationPresetFeatures::*;
            for features in [MostRestrictive,WithOnlyEmergencyAction,WithMintingAndBurningActions,WithExtremeActions,WithAllAdvancedActions] {
                if ui.button(format!("Compare {}",preset_label(features))).clicked() { state.proposal = Some(Proposal { title: preset_label(features), draft: state.draft.preset_proposal(features) }); }
            }
        });
    });
}
