use super::*;
use crate::model::token_creator::TokenDraft;
use std::{collections::BTreeMap, sync::LazyLock};

#[derive(serde::Deserialize)]
struct HelpTopic {
    title: String,
    body: String,
    example: String,
    consequence: Option<String>,
}
static TOPICS: LazyLock<BTreeMap<String, HelpTopic>> =
    LazyLock::new(|| serde_json::from_str(include_str!("help.json")).unwrap_or_default());

pub(super) fn topic_title(key: &str) -> &str {
    TOPICS
        .get(key)
        .map_or("Token setting", |topic| topic.title.as_str())
}

pub(super) fn show(context: &Context, state: &mut CreatorWorkspace) {
    if let Some(key) = state.help.clone() {
        let mut close = false;
        let response = egui::Modal::new(egui::Id::new("creator_help")).show(context, |ui| {
            ui.set_width(540.0_f32.min(context.content_rect().width() - 60.0));
            if let Some(topic) = TOPICS.get(&key) {
                ui.heading(&topic.title);
                ui.add_space(10.0);
                ui.label(&topic.body);
                ui.add_space(10.0);
                ui.strong("Example");
                ui.label(&topic.example);
                if let Some(consequence) = &topic.consequence {
                    ui.add_space(10.0);
                    ui.strong("Consequence");
                    ui.label(consequence);
                }
            } else {
                ui.heading("Help is unavailable");
                ui.label("Return to the form and open the tutorial for this setting.");
            }
            ui.add_space(12.0);
            ui.hyperlink_to("Open token tutorial ↗", TUTORIAL_URL);
            if ui.button("Back to the form").clicked() {
                close = true;
            }
        });
        if close || response.should_close() {
            state.help = None;
            context.data_mut(|data| {
                data.insert_temp(egui::Id::new("creator_restore_help_focus"), true)
            });
            context.request_repaint();
        }
        return;
    }
    if let Some(proposal) = &state.proposal {
        let mut apply = false;
        let mut cancel = false;
        let response = egui::Modal::new(egui::Id::new("creator_compare")).show(context, |ui| {
            ui.set_width(800.0_f32.min(context.content_rect().width() - 60.0));
            ui.heading(format!("Compare {}",proposal.title));
            ui.label("Your names, description, document types, keywords, and groups are kept. Inactive allocation parameters and scheduled rows are retained.");
            let mut rows: Vec<_> = comparison(&state.draft, &proposal.draft)
                .into_iter()
                .map(|(caption, before, after)| (caption.to_owned(), before, after))
                .collect();
            for kind in RuleKind::ALL {
                let before = state.draft.permissions.get(kind);
                let after = proposal.draft.permissions.get(kind);
                rows.push((format!("{} · performer", kind.label()), before.performer.label(), after.performer.label()));
                rows.push((format!("{} · administrator", kind.label()), before.administrator.label(), after.administrator.label()));
                for (caption, old, new) in [
                    ("allow disabling action", before.allow_disable_action, after.allow_disable_action),
                    ("allow removing administrator", before.allow_remove_administrator, after.allow_remove_administrator),
                    ("allow replacing administrator", before.allow_replace_administrator, after.allow_replace_administrator),
                ] {
                    rows.push((format!("{} · {caption}", kind.label()), supply::yes(old).into(), supply::yes(new).into()));
                }
            }
            egui::ScrollArea::vertical().max_height(context.content_rect().height() * 0.62).show(ui, |ui| {
                ui.strong("Changes to your draft");
                comparison_table(ui, "changed_settings", &rows, true);
                egui::CollapsingHeader::new("Full comparison including unchanged settings").show(ui, |ui| {
                    comparison_table(ui, "all_settings", &rows, false);
                });
            });
            ui.horizontal(|ui| {
                if ui.button("Keep my draft").clicked() { cancel = true; }
                if ComponentStyles::add_primary_button(ui,"Apply these changes").clicked() { apply = true; }
            });
        });
        if apply {
            state.apply_proposal();
        } else if cancel || response.should_close() {
            state.proposal = None;
        }
        return;
    }
    if let Some(kind) = state.rule_editor {
        let mut close = false;
        let mut groups = false;
        let response = egui::Modal::new(egui::Id::new("creator_rule")).show(context, |ui| {
            ui.set_width(600.0_f32.min(context.content_rect().width() - 60.0));
            heading(ui,kind.label(),kind.help_key(),&mut state.help);
            egui::ScrollArea::vertical().max_height(context.content_rect().height() * 0.65).show(ui, |ui| {
                let rule = state.draft.permissions.get_mut(kind);
                supply::actor(ui,"Who can perform this action","performer",&mut rule.performer,&state.draft.groups,state.draft.main_group,&mut state.help);
                ui.add_space(12.0);
                supply::actor(ui,"Who can change this rule","administrator",&mut rule.administrator,&state.draft.groups,state.draft.main_group,&mut state.help);
                ui.add_space(12.0);
                checkbox(ui,"Allow the action performer to be changed to No one","flags.0",&mut rule.allow_disable_action,&mut state.help);
                checkbox(ui,"Allow the administrator to be changed to No one","flags.1",&mut rule.allow_remove_administrator,&mut state.help);
                checkbox(ui,"Allow the administrator to replace itself","flags.2",&mut rule.allow_replace_administrator,&mut state.help);
                ui.small("These allowances are separate. An action with no current performer may still be enabled later when a rule administrator remains.");
                if ui.link("Open Groups").clicked() { groups = true; }
            });
            if ui.button("Done").clicked() { close = true; }
        });
        if groups {
            state.section = Section::Groups;
            state.rule_editor = None;
            state.notice = Some(format!(
                "Edit the groups, then return to the {} rule in Supply & permissions.",
                kind.label()
            ));
        } else if close || response.should_close() {
            state.rule_editor = None;
        }
    }
}

fn comparison_table(ui: &mut Ui, id: &str, rows: &[(String, String, String)], changed_only: bool) {
    let column_width = ((ui.available_width() - 16.0) / 3.0).max(1.0);
    egui::Grid::new(id)
        .num_columns(3)
        .max_col_width(column_width)
        .striped(true)
        .show(ui, |ui| {
            for caption in ["Setting", "Your draft", "After applying"] {
                ui.strong(caption);
            }
            ui.end_row();
            for (caption, before, after) in rows
                .iter()
                .filter(|(_, before, after)| !changed_only || before != after)
            {
                for value in [caption, before, after] {
                    ui.add(egui::Label::new(value).wrap());
                }
                ui.end_row();
            }
        });
}

fn comparison(before: &TokenDraft, after: &TokenDraft) -> Vec<(&'static str, String, String)> {
    let amount_label = |amount: &TokenAmountInput, decimals| {
        amount
            .parse(decimals)
            .map(|raw| format!("{} tokens ({raw} base units)", format_amount(raw, decimals)))
            .unwrap_or_else(|_| format!("{} (unfinished)", amount.text))
    };
    let maximum = |draft: &TokenDraft| {
        if draft.no_maximum {
            "No maximum".into()
        } else {
            amount_label(&draft.maximum, draft.decimals)
        }
    };
    let enabled = |value| {
        if value {
            "On".to_string()
        } else {
            "Off".to_string()
        }
    };
    vec![
        (
            "Policy",
            policy_label(before.policy).into(),
            policy_label(after.policy).into(),
        ),
        (
            "Initial supply",
            amount_label(&before.initial, before.decimals),
            amount_label(&after.initial, after.decimals),
        ),
        ("Maximum supply", maximum(before), maximum(after)),
        (
            "Decimal places",
            before.decimals.to_string(),
            after.decimals.to_string(),
        ),
        (
            "Ordinary transfers",
            enabled(before.transferable),
            enabled(after.transferable),
        ),
        (
            "Starts paused",
            enabled(before.start_paused),
            enabled(after.start_paused),
        ),
        (
            "Receiving while frozen",
            enabled(before.allow_receive_frozen),
            enabled(after.allow_receive_frozen),
        ),
        (
            "Initial and default mint recipient",
            recipient_label(&before.mint_recipient),
            recipient_label(&after.mint_recipient),
        ),
        (
            "Per-mint recipient choice",
            enabled(before.permissions.allow_mint_recipient_choice),
            enabled(after.permissions.allow_mint_recipient_choice),
        ),
        (
            "Recurring distribution",
            enabled(before.recurring.enabled),
            enabled(after.recurring.enabled),
        ),
        (
            "Scheduled allocations",
            enabled(before.scheduled_enabled),
            enabled(after.scheduled_enabled),
        ),
        (
            "Once per identity",
            enabled(before.once_enabled),
            enabled(after.once_enabled),
        ),
        (
            "History ledgers",
            history_label(before.history),
            history_label(after.history),
        ),
        (
            "Main group",
            before
                .main_group
                .map_or_else(|| "None".into(), |id| format!("Draft group {id}")),
            after
                .main_group
                .map_or_else(|| "None".into(), |id| format!("Draft group {id}")),
        ),
        (
            "Main group replacement authority",
            before.permissions.main_group_change.label(),
            after.permissions.main_group_change.label(),
        ),
    ]
}

fn history_label(history: [bool; 6]) -> String {
    [
        "Transfers",
        "Freezing",
        "Minting",
        "Burning",
        "Price changes",
        "Purchases",
    ]
    .into_iter()
    .zip(history)
    .map(|(label, on)| format!("{label}: {}", if on { "on" } else { "off" }))
    .collect::<Vec<_>>()
    .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_approved_help_topics_are_present_and_tutorial_is_exact() {
        assert_eq!(TOPICS.len(), 90);
        for kind in RuleKind::ALL {
            assert!(TOPICS.contains_key(kind.help_key()));
        }
        for kind in crate::model::token_creator::distributions::FunctionKind::ALL {
            assert!(TOPICS.contains_key(kind.help_key()));
            for parameter in kind.parameters() {
                assert!(TOPICS.contains_key(parameter.help_key()));
            }
        }
        assert_eq!(
            TUTORIAL_URL,
            "https://drive.google.com/drive/folders/1ECWS2d_-OeHXTW-Ur-qFlf_mZzYUIqQ1?usp=sharing"
        );
    }
    #[test]
    fn checkpoint_review_help_renders_the_approved_consequence() {
        use egui_kittest::kittest::Queryable;
        let mut state = CreatorWorkspace {
            help: Some("transferable".into()),
            ..Default::default()
        };
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(900.0, 700.0))
            .build_ui(move |ui| show(ui.ctx(), &mut state));
        harness.run();
        harness.get_by_label("Transferability cannot change after creation. Non-transferable tokens require protocol 14 or later.");
    }
}
