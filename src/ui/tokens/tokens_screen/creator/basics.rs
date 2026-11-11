use super::super::TokenNameLanguage;
use super::*;
use crate::model::token_creator::{DraftError, NameDraft, Section};
use crate::ui::state::token_creator::Proposal;

pub(super) fn show(ui: &mut Ui, state: &mut CreatorWorkspace) {
    card(ui, |ui| {
        heading(ui, "What are you creating?", "basics", &mut state.help);
        ui.label("Give people a recognizable name, then choose how the supply should work.");
        let mut remove = None;
        for (index, name) in state.draft.names.iter_mut().enumerate() {
            ui.push_id(index, |ui| {
                if index > 0 {
                    ui.separator();
                }
                ui.horizontal(|ui| {
                    label(ui, "Language", "localizations", &mut state.help);
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
                        });
                    if index > 0 && ui.small_button("Remove language").clicked() {
                        remove = Some(index);
                    }
                });
                text(
                    ui,
                    "Singular token name",
                    "name",
                    &mut name.singular,
                    &mut state.help,
                );
                text(
                    ui,
                    "Plural token name",
                    "plural",
                    &mut name.plural,
                    &mut state.help,
                );
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
        checkbox(
            ui,
            "Capitalize token names",
            "capitalization",
            &mut state.draft.capitalize,
            &mut state.help,
        );
        text(
            ui,
            "Description (optional)",
            "description",
            &mut state.draft.description,
            &mut state.help,
        );
        ui.small("Names require 3 to 25 characters without spaces. A description may contain spaces and must have 3 to 100 characters when provided.");
        label(ui, "Decimal precision", "decimals", &mut state.help);
        let mut precision = state.draft.decimals;
        if ui
            .add(egui::DragValue::new(&mut precision).range(0..=16))
            .changed()
        {
            if let Err(error) = state.draft.change_precision(precision) {
                state.error = Some(DraftError::new(
                    Section::Basics,
                    "Decimal precision",
                    error.into(),
                ));
            }
        }
        ui.small("Changing precision preserves displayed token amounts. Values that would need rounding must be corrected first.");
    });
    card(ui, |ui| {
        heading(ui, "Choose a supply policy", "policy", &mut state.help);
        ui.label(
            "A starting point is a choice about power. Compare all changes before applying it.",
        );
        ui.strong(policy_label(state.draft.policy));
        ui.add_space(8.0);
        for (limited, title, body, key) in [
            (
                false,
                "Fixed supply",
                "1,000 tokens, permanently capped at 1,000. No issuer powers or rule administrators.",
                "fixed-policy",
            ),
            (
                true,
                "Owner-managed, with a hard cap",
                "Start with 1,000 tokens, capped at 10,000. The owner can mint, burn their own tokens, and pause or resume transfers.",
                "owner-policy",
            ),
        ] {
            label(ui, title, key, &mut state.help);
            ui.label(body);
            if ui
                .button(if limited {
                    "Compare owner example"
                } else {
                    "Compare fixed example"
                })
                .clicked()
            {
                state.proposal = Some(Proposal {
                    title,
                    draft: state.draft.example_proposal(limited),
                });
            }
            ui.add_space(12.0);
        }
        if ui.button("Choose custom rules →").clicked() {
            state.draft.policy = Policy::Custom;
            state.section = Section::Supply;
        }
        if state.undo.is_some() && ui.button("Undo last example or template").clicked() {
            state.undo_proposal();
        }
        egui::CollapsingHeader::new("Existing protocol templates").show(ui, |ui| {
            ui.label("These retain the original protocol permissions. They differ from the two examples above. Compare every performer, administrator and allowance.");
            use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationPresetFeatures::*;
            for features in [MostRestrictive,WithOnlyEmergencyAction,WithMintingAndBurningActions,WithExtremeActions,WithAllAdvancedActions] {
                if ui.button(format!("Compare {}",preset_label(features))).clicked() { state.proposal = Some(Proposal { title: preset_label(features), draft: state.draft.preset_proposal(features) }); }
            }
        });
    });
}
