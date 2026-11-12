use super::*;
use crate::model::token_creator::distributions::{FunctionDraft, FunctionKind, evaluate_preview};
use crate::model::token_creator::{IntervalKind, ScheduleDraft, TimeUnit};
use crate::ui::components::local_schedule_input::LocalScheduleInput;
use crate::ui::components::{Component, ComponentResponse};
use dash_sdk::dpp::{dashcore::Network, version::PlatformVersion};

pub(super) fn show(
    ui: &mut Ui,
    state: &mut CreatorWorkspace,
    network: Network,
    version: &PlatformVersion,
) {
    heading(ui, "Distribution", "distribution", &mut state.help);
    ui.label("Three independent ways to distribute tokens. Turning one off keeps its settings in your draft.");
    ui.label("Distribution rules make tokens available to claim. They do not promise an automatic transfer into a wallet. Initial supply is separate.");
    card(ui, |ui| {
        heading(ui, "Recurring distribution", "recurring", &mut state.help);
        checkbox(
            ui,
            "Enable recurring distribution",
            "recurring",
            &mut state.draft.recurring.enabled,
            &mut state.help,
        );
        ui.label("Allocate tokens at repeated time, epoch or block intervals. Recipients must claim eligible allocations.");
        ui.label(recurring_authority(&state.draft));
        if state.draft.recurring.enabled {
            ui.push_id("recurring", |ui| {
                let recurring = &mut state.draft.recurring;
                label(ui,"Interval based on","intervalKind",&mut state.help);
                ui.horizontal(|ui| {
                    for (kind,caption) in [(IntervalKind::Time,"Elapsed time"),(IntervalKind::Blocks,"Blocks"),(IntervalKind::Epochs,"Platform epochs")] { ui.selectable_value(&mut recurring.interval_kind,kind,caption); }
                });
                text(ui,"Repeat every","interval",&mut recurring.interval,&mut state.help);
                if recurring.interval_kind == IntervalKind::Time {
                    label(ui,"Time unit","timeUnit",&mut state.help);
                    egui::ComboBox::from_label("Time unit").selected_text(recurring.time_unit.label()).show_ui(ui, |ui| { for unit in TimeUnit::ALL { ui.selectable_value(&mut recurring.time_unit,unit,unit.label()); } });
                    ui.small("Time intervals must align to full minutes. The minimum depends on the selected network.");
                }
                label(ui,"Recurring recipient","recipient",&mut state.help);
                supply::recipient(ui,&mut recurring.recipient,true,false,&mut state.help);
                if recurring.recipient == Recipient::EvoNodes && recurring.interval_kind != IntervalKind::Epochs { ui.colored_label(ui.visuals().error_fg_color,"Choose Epochs for EvoNode participation allocations."); }
                label(ui,"Emission function","recurring",&mut state.help);
                egui::ComboBox::from_label("Emission function").selected_text(recurring.function.label()).show_ui(ui, |ui| { for kind in FunctionKind::ALL { ui.selectable_value(&mut recurring.function,kind,kind.label()); } });
                help(ui,&mut state.help,recurring.function.help_key());
                let function = &mut recurring.functions[recurring.function as usize];
                if ui.button("Restore worked example parameters").clicked() { *function = FunctionDraft::example(recurring.function); }
                ui.small("Worked examples use 2 decimal places. At other precision, the displayed amounts stay the same and the raw coefficients have different effects.");
                for parameter in recurring.function.parameters() {
                    if parameter.is_amount() {
                        amount(ui, parameter.label(), parameter.help_key(), function.amounts.entry(*parameter).or_default(), state.draft.decimals, &mut state.help);
                    } else {
                        text(ui,parameter.label(),parameter.help_key(),function.values.entry(*parameter).or_default(),&mut state.help);
                    }
                    if parameter.optional() { ui.small("Blank means absent; zero is an explicit value."); }
                }
                if recurring.function == FunctionKind::Stepwise {
                    label(ui,"Phases","param.steps",&mut state.help);
                    let mut remove = None;
                    for (index,(start,value)) in function.steps.iter_mut().enumerate() {
                        ui.push_id(index, |ui| {
                            text(ui,"Starting period index","period-index",start,&mut state.help);
                            amount(ui,"Amount until the next phase","param.amount",value,state.draft.decimals,&mut state.help);
                            if ui.button("Remove phase").clicked() { remove = Some(index); }
                        });
                    }
                    if let Some(index) = remove { function.steps.remove(index); }
                    if ui.button("Add phase").clicked() { function.steps.push((String::new(),TokenAmountInput::default())); }
                }
                text(ui,"Assumed registration reference for preview","period-index",&mut state.preview_reference,&mut state.help);
                ui.small("Blank uses zero. The first row is illustrative; cumulative allocation starts at period 1 and excludes initial supply.");
                heading(ui,"Current-parameter preview","static-preview",&mut state.help);
                let reference = if state.preview_reference.trim().is_empty() { Some(0) } else { state.preview_reference.parse::<u64>().ok() };
                if let Some(reference) = reference {
                    match function.parse(state.draft.decimals,reference,version).and_then(|function| evaluate_preview(&function,reference,version)) {
                        Ok(rows) => {
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(),130.0),egui::Sense::hover());
                            let max = rows.iter().map(|row| row.amount).max().unwrap_or(1).max(1) as f64;
                            let points: Vec<_> = rows.iter().map(|row| egui::pos2(rect.left() + rect.width() * row.index as f32 / 10.0,rect.bottom() - rect.height() * (row.amount as f64 / max) as f32)).collect();
                            ui.painter().add(egui::Shape::line(points,egui::Stroke::new(2.0,ui.visuals().selection.bg_fill)));
                            egui::Grid::new("emission_table").striped(true).show(ui, |ui| {
                                for title in ["Period","Base units","Tokens","Cumulative tokens"] { ui.strong(title); } ui.end_row();
                                for row in rows { ui.label(row.index.to_string()); ui.label(row.amount.to_string()); ui.label(format_amount(row.amount,state.draft.decimals)); ui.label(format_amount(row.cumulative,state.draft.decimals)); ui.end_row(); }
                            });
                        }
                        Err(error) => { ui.colored_label(ui.visuals().error_fg_color,error.to_string()); }
                    }
                } else { ui.colored_label(ui.visuals().error_fg_color,"Enter a whole registration reference within the supported range."); }
                if let Err(error) = recurring.parse(state.draft.decimals,network,version) { ui.colored_label(ui.visuals().error_fg_color,error.to_string()); }
            });
        } else {
            ui.small(format!(
                "Saved settings: {function}, every {interval} {unit}, to {recipient}.",
                function = state.draft.recurring.function.label(),
                interval = state.draft.recurring.interval,
                unit = match state.draft.recurring.interval_kind {
                    IntervalKind::Time => state.draft.recurring.time_unit.label(),
                    IntervalKind::Blocks => "blocks",
                    IntervalKind::Epochs => "epochs",
                },
                recipient = recipient_label(&state.draft.recurring.recipient),
            ));
        }
        if ui
            .link("Edit permission to change recurring distribution")
            .clicked()
        {
            state.rule_editor = Some(RuleKind::Recurring);
        }
    });
    card(ui, |ui| {
        heading(ui, "Scheduled distributions", "scheduled", &mut state.help);
        checkbox(
            ui,
            "Enable scheduled distributions",
            "scheduled",
            &mut state.draft.scheduled_enabled,
            &mut state.help,
        );
        ui.label("Set specific dates, recipients and amounts. Recipients must claim eligible allocations. Editing or reopening this screen does not move the date.");
        if state.draft.scheduled_enabled {
            let mut remove = None;
            for row in &mut state.draft.scheduled {
                ui.push_id(row.id, |ui| {
                    ui.separator();
                    label(
                        ui,
                        "Allocation date and time (your local time)",
                        "scheduledDate",
                        &mut state.help,
                    );
                    let input = state.schedule_inputs.entry(row.id).or_insert_with(|| {
                        let input =
                            LocalScheduleInput::new().with_label("Allocation date and time");
                        row.timestamp
                            .and_then(|ms| i64::try_from(ms).ok())
                            .and_then(chrono::DateTime::from_timestamp_millis)
                            .map_or_else(|| input.clone(), |time| input.clone().with_time(time))
                    });
                    input.show(ui).inner.update(&mut row.timestamp);
                    if let Some(timestamp) = row.timestamp {
                        ui.small(format!(
                            "Allocation instant: {} ({timestamp} UTC milliseconds)",
                            crate::model::datetime::local_date_time_from_unix_millis(timestamp)
                        ));
                    }
                    label(ui, "Recipient", "recipient", &mut state.help);
                    supply::recipient(ui, &mut row.recipient, false, false, &mut state.help);
                    amount(
                        ui,
                        "Scheduled amount",
                        "scheduledAmount",
                        &mut row.amount,
                        state.draft.decimals,
                        &mut state.help,
                    );
                    if ui.button("Remove scheduled distribution").clicked() {
                        remove = Some(row.id);
                    }
                });
            }
            if let Some(id) = remove {
                state.draft.scheduled.retain(|row| row.id != id);
                state.schedule_inputs.remove(&id);
            }
            if ui.button("Add scheduled distribution").clicked() {
                let id = state.next_schedule_id;
                state.next_schedule_id = state.next_schedule_id.saturating_add(1);
                state.draft.scheduled.push(ScheduleDraft {
                    id,
                    timestamp: None,
                    recipient: Recipient::Owner,
                    amount: TokenAmountInput::default(),
                });
            }
        } else {
            ui.small(format!(
                "Off. {count} scheduled allocations are retained in this draft.",
                count = state.draft.scheduled.len()
            ));
        }
    });
    card(ui, |ui| {
        heading(ui, "Once per identity", "once", &mut state.help);
        checkbox(
            ui,
            "Enable once-per-identity distribution",
            "once",
            &mut state.draft.once_enabled,
            &mut state.help,
        );
        ui.label("Each eligible identity can claim once. One person can control multiple identities. Available supply can limit claims; tokens do not transfer automatically.");
        if state.draft.once_enabled {
            if version.protocol_version < 14 {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    "The selected network does not support this allocation yet.",
                );
            }
            amount(
                ui,
                "Amount per identity",
                "onceAmount",
                &mut state.draft.once_amount,
                state.draft.decimals,
                &mut state.help,
            );
        } else if !state.draft.once_amount.text.is_empty() {
            ui.small(format!(
                "Off. Saved amount: {amount} {unit}.",
                amount = state.draft.once_amount.text,
                unit = if state.draft.once_amount.base_units {
                    "base units"
                } else {
                    "tokens"
                },
            ));
        }
    });
}

pub(super) fn recurring_authority(draft: &crate::model::token_creator::TokenDraft) -> String {
    let rule = draft.permissions.get(RuleKind::Recurring);
    if rule.performer != ActorDraft::NoOne {
        let actor = rule.performer.label();
        if draft.recurring.enabled {
            format!("{actor} can change recurring distribution.")
        } else {
            format!("{actor} can enable recurring distribution later.")
        }
    } else if rule.administrator != ActorDraft::NoOne {
        format!(
            "{} can change who controls recurring distribution.",
            rule.administrator.label()
        )
    } else {
        "No one can enable or change recurring distribution later.".into()
    }
}
