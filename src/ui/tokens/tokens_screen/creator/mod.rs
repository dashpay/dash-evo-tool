//! The sequential Token Creator workspace and screen-specific controls.

mod basics;
mod dialogs;
mod distribution;
mod documents;
mod groups;
mod review;
mod supply;

use super::TokensScreen;
use crate::app::AppAction;
use crate::model::token_creator::amount::{TokenAmountInput, format_amount};
use crate::model::token_creator::rules::{ActorDraft, RuleKind};
use crate::model::token_creator::{Policy, Recipient, Section};
use crate::ui::state::token_creator::CreatorWorkspace;
use crate::ui::theme::ComponentStyles;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use egui::{Context, RichText, Ui};

const TUTORIAL_URL: &str =
    "https://drive.google.com/drive/folders/1ECWS2d_-OeHXTW-Ur-qFlf_mZzYUIqQ1?usp=sharing";

impl TokensScreen {
    pub(super) fn render_token_workspace(&mut self, context: &Context, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        self.creator_workspace.draft.reconcile_policy();
        if self
            .creator_workspace
            .reviewed_draft
            .as_ref()
            .is_some_and(|draft| *draft != self.creator_workspace.draft)
        {
            self.creator_workspace.prepared = None;
            self.creator_workspace.acknowledged = false;
        }
        ui.horizontal(|ui| {
            ui.heading("Token Creator");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.hyperlink_to("Open tutorial ↗", TUTORIAL_URL);
            });
        });
        ui.label("Design your token, understand its rules, then review before creating it.");
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("Network: {}", self.app_context.network));
            if let Some(identity) = &self.selected_identity {
                ui.label(format!("Owner: {}", identity.display_string()));
            } else {
                ui.label("Drafting is available. Choose an identity in Review when you are ready.");
            }
            help(ui, &mut self.creator_workspace.help, "readiness");
        });
        ui.add_space(12.0);
        egui::ScrollArea::horizontal()
            .id_salt("creator_sections")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (index, section) in Section::ALL.into_iter().enumerate() {
                        if ui
                            .add(
                                egui::Button::selectable(
                                    self.creator_workspace.section == section,
                                    format!("{}  {}", index + 1, section.label()),
                                )
                                .wrap_mode(egui::TextWrapMode::Extend),
                            )
                            .clicked()
                        {
                            self.creator_workspace.section = section;
                        }
                    }
                });
            });
        ui.add_space(16.0);
        if let Some(error) = &self.creator_workspace.error {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("{}: {error}", error.field),
                );
                if ui
                    .link(format!("Go to {}", error.section.label()))
                    .clicked()
                {
                    self.creator_workspace.section = error.section;
                }
            });
        }
        if let Some(notice) = &self.creator_workspace.notice {
            ui.horizontal_wrapped(|ui| {
                ui.label(notice);
                if let Some(section) = self.creator_workspace.error_section {
                    if ui.link(format!("Go to {}", section.label())).clicked() {
                        self.creator_workspace.section = section;
                    }
                }
            });
        }
        let wide = ui.available_width() >= 950.0;
        let width = if wide {
            ui.available_width() - 280.0
        } else {
            ui.available_width()
        };
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(width);
                    match self.creator_workspace.section {
                        Section::Basics => basics::show(ui, &mut self.creator_workspace),
                        Section::Supply => supply::show(
                            ui,
                            &mut self.creator_workspace,
                            self.app_context.platform_version(),
                        ),
                        Section::Distribution => distribution::show(
                            ui,
                            &mut self.creator_workspace,
                            self.app_context.network,
                            self.app_context.platform_version(),
                        ),
                        Section::Groups => groups::show(
                            ui,
                            &mut self.creator_workspace,
                            self.app_context.platform_version(),
                        ),
                        Section::History => documents::history(ui, &mut self.creator_workspace),
                        Section::Documents => documents::show(ui, &mut self.creator_workspace),
                        Section::Review => {
                            action |= self.render_workspace_review(ui);
                        }
                    }
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                self.creator_workspace.section != Section::Basics,
                                egui::Button::new("← Back"),
                            )
                            .clicked()
                        {
                            self.creator_workspace.navigate(false);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if self.creator_workspace.section != Section::Review
                                && ComponentStyles::add_primary_button(ui, "Continue →").clicked()
                            {
                                self.creator_workspace.navigate(true);
                            }
                        });
                    });
                },
            );
            if wide {
                ui.add_space(16.0);
                ui.vertical(|ui| {
                    ui.set_width(260.0);
                    summary(ui, &mut self.creator_workspace);
                });
            }
        });
        if !wide {
            ui.add_space(16.0);
            egui::CollapsingHeader::new("Your token · draft summary")
                .show(ui, |ui| summary(ui, &mut self.creator_workspace));
        }
        dialogs::show(context, &mut self.creator_workspace);
        action
    }
}

fn card(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    egui::Frame::group(ui.style())
        .fill(ui.visuals().panel_fill)
        .inner_margin(16)
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
    ui.add_space(12.0);
}

fn help(ui: &mut Ui, topic: &mut Option<String>, key: &str) {
    if ui
        .small_button("?")
        .on_hover_text("Explain this setting")
        .clicked()
    {
        *topic = Some(key.into());
    }
}

fn heading(ui: &mut Ui, text: &str, key: &str, topic: &mut Option<String>) {
    ui.horizontal_wrapped(|ui| {
        ui.heading(text);
        help(ui, topic, key);
    });
    ui.add_space(6.0);
}

fn label(ui: &mut Ui, text: &str, key: &str, topic: &mut Option<String>) {
    ui.horizontal_wrapped(|ui| {
        ui.strong(text);
        help(ui, topic, key);
    });
}

fn text(ui: &mut Ui, caption: &str, key: &str, value: &mut String, topic: &mut Option<String>) {
    label(ui, caption, key, topic);
    ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY));
    ui.add_space(8.0);
}

fn checkbox(ui: &mut Ui, caption: &str, key: &str, value: &mut bool, topic: &mut Option<String>) {
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(value, caption);
        help(ui, topic, key);
    });
}

fn amount(
    ui: &mut Ui,
    caption: &str,
    key: &str,
    value: &mut TokenAmountInput,
    decimals: u8,
    topic: &mut Option<String>,
) {
    label(ui, caption, key, topic);
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut value.text).desired_width(180.0));
        let mut raw = value.base_units;
        egui::ComboBox::from_id_salt((caption, "units"))
            .selected_text(if raw { "base units" } else { "tokens" })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut raw, false, "tokens");
                ui.selectable_value(&mut raw, true, "base units");
            });
        if let Err(error) = value.change_units(raw, decimals) {
            ui.colored_label(ui.visuals().error_fg_color, error.to_string());
        }
    });
    if !value.text.trim().is_empty() {
        match value.parse(decimals) {
            Ok(raw) => {
                ui.small(format!(
                    "{} tokens = {raw} base units",
                    format_amount(raw, decimals)
                ));
            }
            Err(error) => {
                ui.colored_label(ui.visuals().error_fg_color, error.to_string());
            }
        }
    }
    ui.add_space(8.0);
}

fn policy_label(policy: Policy) -> &'static str {
    match policy {
        Policy::Unselected => "Choose a policy",
        Policy::Fixed => "Fixed supply",
        Policy::LimitedOwner => "Owner-managed, with a hard cap",
        Policy::Custom => "Custom rules",
        Policy::Preset(features) => preset_label(features),
    }
}

fn preset_label(
    features: dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationPresetFeatures,
) -> &'static str {
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationPresetFeatures::*;
    match features {
        MostRestrictive => "Most restrictive",
        WithOnlyEmergencyAction => "Emergency action",
        WithMintingAndBurningActions => "Minting and burning",
        WithExtremeActions => "Extreme actions",
        WithAllAdvancedActions => "All advanced actions",
    }
}

fn recipient_label(recipient: &Recipient) -> String {
    match recipient {
        Recipient::Owner => "Contract owner".into(),
        Recipient::None => "No fixed destination (initial supply goes to the owner)".into(),
        Recipient::Identity(id) => format!("Identity {id}"),
        Recipient::EvoNodes => "EvoNodes by participation".into(),
    }
}

fn summary(ui: &mut Ui, state: &mut CreatorWorkspace) {
    card(ui, |ui| {
        ui.heading("Your token");
        ui.label(
            RichText::new(state.draft.names.first().map_or("Unnamed token", |name| {
                if name.singular.is_empty() {
                    "Unnamed token"
                } else {
                    &name.singular
                }
            }))
            .strong(),
        );
        ui.label(policy_label(state.draft.policy));
        ui.separator();
        for (label, amount) in [
            ("Initial supply", &state.draft.initial),
            ("Maximum supply", &state.draft.maximum),
        ] {
            if label == "Maximum supply" && state.draft.no_maximum {
                ui.label("Maximum supply: no configured limit");
            } else {
                ui.label(format!(
                    "{label}: {}",
                    amount
                        .parse(state.draft.decimals)
                        .map(|raw| format!("{} tokens", format_amount(raw, state.draft.decimals)))
                        .unwrap_or_else(|_| "not valid yet".into())
                ));
            }
        }
        ui.small(format!("{} decimal places", state.draft.decimals));
        ui.label(format!(
            "Initial recipient: {}",
            recipient_label(&state.draft.mint_recipient)
        ));
        ui.separator();
        ui.strong("Powers and administrators");
        for kind in RuleKind::ALL {
            let rule = state.draft.permissions.get(kind);
            if rule.performer != ActorDraft::NoOne
                || rule.administrator != ActorDraft::NoOne
                || !rule.permanently_disabled()
            {
                ui.label(kind.label());
                ui.small(format!("Performer: {}", rule.performer.label()));
                ui.small(format!("Administrator: {}", rule.administrator.label()));
                if rule.allow_disable_action
                    || rule.allow_remove_administrator
                    || rule.allow_replace_administrator
                {
                    ui.small("Later changes are allowed. Review the rule's three allowances.");
                }
            }
        }
        if state
            .draft
            .permissions
            .rules
            .iter()
            .all(|rule| rule.permanently_disabled())
        {
            ui.label("All 13 action rules are permanently disabled.");
        }
        ui.separator();
        ui.small("Draft only. Nothing is created or charged before the final confirmation.");
    });
}
