use super::*;
use crate::model::token_creator::groups::GroupDraft;
use crate::model::token_creator::rules::parse_identity;
use crate::ui::state::token_creator::RuleFilter;
use dash_sdk::dpp::version::PlatformVersion;

pub(super) fn show(ui: &mut Ui, state: &mut CreatorWorkspace, version: &PlatformVersion) {
    card(ui, |ui| {
        heading(ui, "Supply & permissions", "policy", &mut state.help);
        ui.label("Enter amounts in tokens or base units. Changing precision preserves the displayed token amount.");
        if ui.available_width() >= 440.0 {
            ui.columns(2, |columns| {
                initial_supply(&mut columns[0], state);
                maximum_supply(&mut columns[1], state);
            });
        } else {
            initial_supply(ui, state);
            maximum_supply(ui, state);
        }
        ui.small("Burning tokens can free capacity for future mints; this is not a lifetime minting limit.");
        ui.horizontal(|ui| {
            let caption = label(ui, "Decimal places", "decimals", &mut state.help);
            let mut precision = state.draft.decimals;
            if ui
                .add(egui::DragValue::new(&mut precision).range(0..=16))
                .labelled_by(caption.id)
                .changed()
                && let Err(error) = state.draft.change_precision(precision)
            {
                state.error = Some(error);
            }
        });
        ui.small("Choose 0 to 16 decimal places. For example, 2 allows 1.25 tokens. Amounts that cannot be represented remain visible for correction.");
        ui.label(cap_consequence(&state.draft));
        subheading(ui, "Mint recipients", "mintRecipient", &mut state.help);
        ui.label("The default new-token recipient also receives the initial supply. Choosing a different recipient on a future mint is a separate permission.");
        label(
            ui,
            "Default recipient for newly created tokens",
            "mintRecipient",
            &mut state.help,
        );
        recipient(
            ui,
            &mut state.draft.mint_recipient,
            false,
            true,
            &mut state.help,
        );
        ui.small(format!(
            "Initial allocation: {}",
            recipient_label(&state.draft.mint_recipient)
        ));
        checkbox(
            ui,
            "Allow choosing a recipient on each mint",
            "mintOverride",
            &mut state.draft.permissions.allow_mint_recipient_choice,
            &mut state.help,
        );
        checkbox(
            ui,
            "Allow transfers between holders",
            "transferable",
            &mut state.draft.transferable,
            &mut state.help,
        );
        ui.small("Transferability is permanent after creation. Other configured token actions remain separate.");
        if !state.draft.transferable {
            ui.label("Ordinary transfers cannot be enabled later. Other token actions retain their own rules.");
            if version.protocol_version < 14 {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    "The selected network does not support this setting yet.",
                );
            }
        }
        checkbox(
            ui,
            "Start with transfers paused",
            "paused",
            &mut state.draft.start_paused,
            &mut state.help,
        );
        ui.small(
            "Distributions and other configured actions can continue while transfers are paused.",
        );
        if state.draft.start_paused
            && state
                .draft
                .permissions
                .get(RuleKind::Pause)
                .permanently_disabled()
        {
            ui.colored_label(ui.visuals().warn_fg_color,"No one can resume transfers with these rules. Change the pause rule or turn this setting off.");
        }
        checkbox(
            ui,
            "Allow transfers to frozen identities",
            "allowFrozen",
            &mut state.draft.allow_receive_frozen,
            &mut state.help,
        );
        ui.small("A frozen identity still cannot spend. This setting only controls receiving.");
    });
    card(ui, |ui| {
        subheading(
            ui,
            "Who can act, and who can change the rules?",
            "permissions",
            &mut state.help,
        );
        ui.label("All 13 action rules are in this draft. Open a rule to inspect its administrator and three change allowances.");
        ui.label("The performer may use the action. The administrator may change its rule within the three allowed changes.");
        ui.horizontal_wrapped(|ui| {
            for (filter, label) in [
                (RuleFilter::All, "All 13 rules"),
                (RuleFilter::Enabled, "Enabled powers"),
                (RuleFilter::Changeable, "Changeable rules"),
            ] {
                ui.selectable_value(&mut state.rule_filter, filter, label);
            }
        });
        for kind in RuleKind::ALL {
            let rule = state.draft.permissions.get(kind);
            let visible = match state.rule_filter {
                RuleFilter::All => true,
                RuleFilter::Enabled => rule.performer != ActorDraft::NoOne,
                RuleFilter::Changeable => {
                    rule.administrator != ActorDraft::NoOne
                        || rule.allow_disable_action
                        || rule.allow_remove_administrator
                        || rule.allow_replace_administrator
                }
            };
            if !visible {
                continue;
            }
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                ui.strong(kind.label());
                help(ui, &mut state.help, kind.help_key());
                if ui.button("Edit rule").clicked() {
                    state.rule_editor = Some(kind);
                }
            });
            ui.label(format!("Performer: {}", rule.performer.label()));
            ui.label(format!("Administrator: {}", rule.administrator.label()));
            ui.small(format!(
                "Disable action: {} · Remove administrator: {} · Replace administrator: {}",
                yes(rule.allow_disable_action),
                yes(rule.allow_remove_administrator),
                yes(rule.allow_replace_administrator)
            ));
            if rule.permanently_disabled() {
                ui.small("Permanently disabled by this configuration.");
            }
        }
    });
    card(ui, |ui| {
        subheading(
            ui,
            "Marketplace at creation",
            "rule.marketplace",
            &mut state.help,
        );
        ui.label("No marketplace sale or direct-purchase price is configured. The two sale-related rules above still determine who could change these settings later.");
    });
}

pub(super) fn cap_consequence(draft: &crate::model::token_creator::TokenDraft) -> &'static str {
    if draft.no_maximum {
        "No numerical supply cap is configured."
    } else if draft.maximum.parse(draft.decimals).is_err() {
        "Enter a valid maximum and inspect who can change it."
    } else if draft.permissions.get(RuleKind::Cap).permanently_disabled() {
        "The maximum is a permanent hard cap: no one can change it or change that rule."
    } else {
        "A permanent hard cap has not been established. Inspect the maximum-supply authorities and allowances."
    }
}

fn initial_supply(ui: &mut Ui, state: &mut CreatorWorkspace) {
    amount(
        ui,
        "Initial supply",
        "initial",
        &mut state.draft.initial,
        state.draft.decimals,
        &mut state.help,
    );
}

fn maximum_supply(ui: &mut Ui, state: &mut CreatorWorkspace) {
    ui.add_enabled_ui(!state.draft.no_maximum, |ui| {
        amount(
            ui,
            "Maximum outstanding supply",
            "maximum",
            &mut state.draft.maximum,
            state.draft.decimals,
            &mut state.help,
        );
    });
    checkbox(
        ui,
        "No maximum supply",
        "noMaximum",
        &mut state.draft.no_maximum,
        &mut state.help,
    );
}

pub(super) fn yes(value: bool) -> &'static str {
    if value { "allowed" } else { "not allowed" }
}

pub(super) fn actor(
    ui: &mut Ui,
    caption: &str,
    key: &str,
    value: &mut ActorDraft,
    groups: &[GroupDraft],
    main: Option<u16>,
    topic: &mut Option<String>,
) {
    let caption_response = label(ui, caption, key, topic);
    egui::ComboBox::from_id_salt(caption)
        .width(ui.available_width())
        .selected_text(value.label())
        .show_ui(ui, |ui| {
            ui.selectable_value(value, ActorDraft::NoOne, "No one");
            ui.selectable_value(value, ActorDraft::Owner, "Contract owner");
            if ui
                .selectable_label(
                    matches!(value, ActorDraft::Identity(_)),
                    "Specific identity",
                )
                .clicked()
                && !matches!(value, ActorDraft::Identity(_))
            {
                *value = ActorDraft::Identity(String::new());
            }
            ui.selectable_value(value, ActorDraft::MainGroup, "Main group");
            for group in groups {
                ui.selectable_value(
                    value,
                    ActorDraft::Group(group.position),
                    format!("{} (draft group {})", group.label, group.position),
                );
            }
        })
        .response
        .labelled_by(caption_response.id);
    if let ActorDraft::Identity(identity) = value {
        text(ui, "Identity ID", "identity", identity, topic);
        if !identity.is_empty() {
            match parse_identity(identity) {
                Ok(_) => {
                    ui.small(
                        "Valid ID format. Ownership and network existence are not asserted here.",
                    );
                }
                Err(error) => {
                    ui.colored_label(ui.visuals().error_fg_color, error.to_string());
                }
            }
        }
    }
    if *value == ActorDraft::MainGroup && main.is_none() {
        ui.colored_label(
            ui.visuals().error_fg_color,
            "Select a main group in Groups before using this authority.",
        );
    }
}

pub(super) fn recipient(
    ui: &mut Ui,
    value: &mut Recipient,
    evonodes: bool,
    none: bool,
    topic: &mut Option<String>,
) {
    let caption = ui.label(RichText::new("Recipient").family(Typography::strong_family()));
    egui::ComboBox::from_id_salt("Recipient")
        .width(ui.available_width())
        .selected_text(recipient_label(value))
        .show_ui(ui, |ui| {
            ui.selectable_value(value, Recipient::Owner, "Contract owner");
            if ui
                .selectable_label(matches!(value, Recipient::Identity(_)), "Specific identity")
                .clicked()
                && !matches!(value, Recipient::Identity(_))
            {
                *value = Recipient::Identity(String::new());
            }
            if evonodes {
                ui.selectable_value(value, Recipient::EvoNodes, "EvoNodes by participation");
            }
            if none {
                ui.selectable_value(value, Recipient::None, "No fixed destination");
            }
        })
        .response
        .labelled_by(caption.id);
    if let Recipient::Identity(identity) = value {
        text(ui, "Recipient identity ID", "mintIdentity", identity, topic);
        if !identity.is_empty() {
            match parse_identity(identity) {
                Ok(_) => {
                    ui.small("Valid ID format; network existence has not been checked.");
                }
                Err(error) => {
                    ui.colored_label(ui.visuals().error_fg_color, error.to_string());
                }
            }
        }
    }
}
