//! Editing a schedule keeps its original target and submits an optimistic update.

use super::*;
use crate::ui::components::modal_chrome::{ModalChromeConfig, modal_chrome};

pub(super) struct ScheduledVoteEditor {
    original: ScheduledDpnsVoteRow,
    key: DpnsVoteTargetKey,
    node_label: String,
    choices: Vec<(ResourceVoteChoice, String)>,
    choice: ResourceVoteChoice,
    date: String,
    hour: u32,
    minute: u32,
    time_changed: bool,
}

pub(super) enum EditOutcome {
    KeepOpen,
    Cancel,
    Save(ContestedResourceTask),
}

impl ScheduledVoteEditor {
    pub(super) fn new(
        original: ScheduledDpnsVoteRow,
        key: DpnsVoteTargetKey,
        node_label: String,
        choices: Vec<(ResourceVoteChoice, String)>,
    ) -> Option<Self> {
        let timestamp = i64::try_from(original.vote.unix_timestamp).ok()?;
        let time = DateTime::from_timestamp_millis(timestamp)?;
        Some(Self {
            choice: original.vote.choice,
            original,
            key,
            node_label,
            choices,
            date: time.format("%Y-%m-%d").to_string(),
            hour: time.hour(),
            minute: time.minute(),
            time_changed: false,
        })
    }

    pub(super) fn scheduled_key(&self) -> DpnsScheduledVoteKey {
        DpnsScheduledVoteKey {
            network: self.key.network,
            voter_id: self.key.voter_id,
            contested_name: self.original.vote.contested_name.clone(),
        }
    }

    fn task(&self, now_ms: u64) -> Option<ContestedResourceTask> {
        if self.original.status != DpnsVoteTargetStatus::Scheduled {
            return None;
        }
        // A choice-only edit must preserve seconds/milliseconds in an imported schedule.
        let timestamp = if self.time_changed {
            crate::model::dpns_vote_schedule::parse_utc_schedule(
                &self.date,
                self.hour,
                self.minute,
            )?
        } else {
            self.original.vote.unix_timestamp
        };
        if timestamp <= now_ms
            || !self
                .choices
                .iter()
                .any(|(choice, _)| *choice == self.choice)
        {
            return None;
        }
        Some(ContestedResourceTask::EditScheduledDpnsVote {
            operation_id: self.original.journal_target.as_ref().map(|(id, _)| *id),
            key: self.key.clone(),
            expected_choice: self.original.vote.choice,
            expected_timestamp: self.original.vote.unix_timestamp,
            choice: self.choice,
            unix_timestamp: timestamp,
        })
    }

    pub(super) fn show(&mut self, ctx: &egui::Context) -> EditOutcome {
        let chrome = modal_chrome(
            ctx,
            ModalChromeConfig {
                title: "Edit scheduled vote".into(),
                overlay_id: egui::Id::new("edit_scheduled_vote"),
                overlay_order: egui::Order::Middle,
                window_order: egui::Order::Foreground,
                resizable: false,
                show_close_button: true,
                blocks_input: true,
                inner_margin: 16,
            },
            |ui| {
                ui.label(format!(
                    "{} — {}.dash",
                    self.node_label, self.original.vote.contested_name
                ));
                egui::ComboBox::from_id_salt("edited_vote_choice")
                    .selected_text(
                        self.choices
                            .iter()
                            .find(|(choice, _)| *choice == self.choice)
                            .map(|(_, label)| label.as_str())
                            .unwrap_or("Choose a vote"),
                    )
                    .show_ui(ui, |ui| {
                        for (choice, label) in &self.choices {
                            ui.selectable_value(&mut self.choice, *choice, label);
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label("Cast on (UTC):");
                    self.time_changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut self.date)
                                .hint_text("YYYY-MM-DD")
                                .desired_width(100.0),
                        )
                        .changed();
                    self.time_changed |= ui
                        .add(
                            egui::DragValue::new(&mut self.hour)
                                .prefix("Hour: ")
                                .range(0..=23),
                        )
                        .changed();
                    self.time_changed |= ui
                        .add(
                            egui::DragValue::new(&mut self.minute)
                                .prefix("Minute: ")
                                .range(0..=59),
                        )
                        .changed();
                });
                ui.label("Keep Dash Evo Tool running and connected until the scheduled time.");
                let task = self.task(Utc::now().timestamp_millis() as u64);
                if task.is_none() {
                    ui.colored_label(DashColors::warning_color(ui.visuals().dark_mode),
                    "Choose a future date and time. Only votes that have not started can be edited.");
                }
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        return EditOutcome::Cancel;
                    }
                    if ui
                        .add_enabled(task.is_some(), Button::new("Save changes"))
                        .clicked()
                        && let Some(task) = task
                    {
                        return EditOutcome::Save(task);
                    }
                    EditOutcome::KeepOpen
                })
                .inner
            },
        );
        if chrome.closed_via_x || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            EditOutcome::Cancel
        } else {
            chrome.inner.unwrap_or(EditOutcome::KeepOpen)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_task::contested_names::ScheduledDPNSVote;

    #[test]
    fn voting_ui_schedule_edit_preserves_target_and_compares_original_values() {
        let key = DpnsVoteTargetKey {
            network: dash_sdk::dpp::dashcore::Network::Testnet,
            voter_id: Identifier::from([1; 32]),
            vote_poll_id: Identifier::from([2; 32]),
        };
        let row = ScheduledDpnsVoteRow {
            failure: None,
            vote: ScheduledDPNSVote {
                voter_id: key.voter_id,
                contested_name: "alice".into(),
                choice: ResourceVoteChoice::Lock,
                unix_timestamp: 1_900_000_012_345,
                executed_successfully: false,
            },
            journal_target: Some((DpnsVoteOperationId::from_bytes([3; 16]), key.clone())),
            status: DpnsVoteTargetStatus::Scheduled,
        };
        let mut editor = ScheduledVoteEditor::new(
            row.clone(),
            key.clone(),
            "node-one".into(),
            vec![
                (ResourceVoteChoice::Lock, "Lock".into()),
                (ResourceVoteChoice::Abstain, "Abstain".into()),
            ],
        )
        .unwrap();
        editor.choice = ResourceVoteChoice::Abstain;
        let ContestedResourceTask::EditScheduledDpnsVote {
            operation_id,
            key: edited_key,
            expected_choice,
            expected_timestamp,
            choice,
            unix_timestamp,
        } = editor.task(1).unwrap()
        else {
            panic!("expected edit");
        };
        assert_eq!(edited_key, key);
        assert_eq!(operation_id, row.journal_target.map(|(id, _)| id));
        assert_eq!(expected_choice, ResourceVoteChoice::Lock);
        assert_eq!(choice, ResourceVoteChoice::Abstain);
        assert_eq!(expected_timestamp, row.vote.unix_timestamp);
        assert_eq!(unix_timestamp, row.vote.unix_timestamp);
        assert!(editor.task(unix_timestamp).is_none());
        editor.original.status = DpnsVoteTargetStatus::Queued;
        assert!(editor.task(1).is_none());
        editor.original.status = DpnsVoteTargetStatus::Scheduled;
        editor.time_changed = true;
        editor.date = "2031-02-03".into();
        editor.hour = 4;
        editor.minute = 5;
        let ContestedResourceTask::EditScheduledDpnsVote { unix_timestamp, .. } =
            editor.task(1).unwrap()
        else {
            panic!("expected edit");
        };
        assert_eq!(
            unix_timestamp,
            crate::model::dpns_vote_schedule::parse_utc_schedule("2031-02-03", 4, 5).unwrap()
        );
    }
}
