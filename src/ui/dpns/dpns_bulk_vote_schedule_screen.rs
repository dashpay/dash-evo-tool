use crate::app::AppAction;
use crate::backend_task::contested_names::ScheduledDPNSVote;
use crate::backend_task::{contested_names::ContestedResourceTask, BackendTask};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::{MessageType, ScreenLike};
use chrono::{Duration, Utc};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use eframe::egui::Context;
use eframe::egui::{self, Color32};
use std::sync::Arc;

use super::dpns_contested_names_screen::SelectedVote;
use super::dpns_vote_scheduling_screen::VoteOption;

pub struct BulkScheduleVoteScreen {
    pub app_context: Arc<AppContext>,
    // All the name+choice pairs the user SHIFT-clicked
    pub selected_votes: Vec<SelectedVote>,

    // The user picks which identities to use,
    // plus how far in the future each identity’s vote is cast.
    pub identities: Vec<QualifiedIdentity>,
    pub identity_options: Vec<VoteOption>,
    message: Option<(MessageType, String)>,
}

impl BulkScheduleVoteScreen {
    pub fn new(app_context: &Arc<AppContext>, selected_votes: Vec<SelectedVote>) -> Self {
        // Query local voting identities from app_context
        let identities = app_context
            .db
            .get_local_voting_identities(&app_context)
            .unwrap_or_default();

        // Initialize each identity’s “VoteOption” to “None” or “Scheduled(0,0,0)”
        let identity_options = identities
            .iter()
            .map(|_| VoteOption::Scheduled {
                days: 0,
                hours: 0,
                minutes: 0,
            })
            .collect();

        Self {
            app_context: app_context.clone(),
            selected_votes,
            identities,
            identity_options,
            message: None,
        }
    }

    fn schedule_votes(&mut self) -> AppAction {
        // For each identity that is set to “Scheduled”
        // build up a list of ScheduledDPNSVote for *all* selected votes
        let mut all_scheduled = Vec::new();
        for (identity, option) in self.identities.iter().zip(self.identity_options.iter()) {
            if let VoteOption::Scheduled {
                days,
                hours,
                minutes,
            } = option
            {
                // Convert days/hours/minutes into a single timestamp
                let now = Utc::now();
                let offset = Duration::days(*days as i64)
                    + Duration::hours(*hours as i64)
                    + Duration::minutes(*minutes as i64);
                let scheduled_time = (now + offset).timestamp_millis() as u64;

                // For each selected vote in selected_votes
                for sv in &self.selected_votes {
                    // You might want to check if scheduled_time is before the contest’s end_time.
                    // But here we do a simple approach:

                    let scheduled_vote = ScheduledDPNSVote {
                        contested_name: sv.contested_name.clone(),
                        voter_id: identity.identity.id().clone(),
                        choice: sv.vote_choice.clone(),
                        unix_timestamp: scheduled_time,
                        executed_successfully: false,
                    };
                    all_scheduled.push(scheduled_vote);
                }
            }
        }

        if all_scheduled.is_empty() {
            self.message = Some((
                MessageType::Error,
                "No votes selected or scheduled.".to_string(),
            ));
            return AppAction::None;
        }

        // Send them off to the backend
        AppAction::BackendTask(BackendTask::ContestedResourceTask(
            ContestedResourceTask::ScheduleDPNSVotes(all_scheduled),
        ))
    }
}

impl ScreenLike for BulkScheduleVoteScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = AppAction::None;
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Bulk-Schedule Votes");

            // Show the table of selected votes
            for sv in &self.selected_votes {
                ui.label(format!(
                    "Name: {}, Choice: {}",
                    sv.contested_name, sv.vote_choice
                ));
            }
            ui.separator();

            // Show your identity scheduling combos, same style as ScheduleVoteScreen
            // self.identity_options[i] => None or Scheduled(…)
            // etc.

            // “Schedule Votes” button
            if ui.button("Submit All Schedules").clicked() {
                action = self.schedule_votes();
            }

            // Show self.message if any
            if let Some((msg_type, msg_text)) = &self.message {
                match msg_type {
                    MessageType::Error => ui.colored_label(Color32::RED, msg_text),
                    MessageType::Success => ui.colored_label(Color32::GREEN, msg_text),
                    MessageType::Info => ui.label(msg_text),
                };
            }
        });
        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.message = Some((message_type, message.to_string()));
    }
}
