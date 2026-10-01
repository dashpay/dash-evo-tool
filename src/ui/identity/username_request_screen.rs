//! Request status page: where one username request stands and what happens next.

use std::sync::Arc;

use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use egui::{RichText, Ui};

use crate::app::AppAction;
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::dpns_usernames::{RequestPhase, TallyStanding, UsernameRequest};
use crate::model::user_role::UserRole;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::identity::identity_pill::{display_label, shorten_id};
use crate::ui::identity::register_dpns_name_screen::status_line;
use crate::ui::identity::username_copy::{
    Tone, format_date, format_date_time, phase_label, what_happens_next,
};
use crate::ui::identity::usernames_card::register_action_for;
use crate::ui::theme::{ComponentStyles, DashColors};
use crate::ui::{MessageType, RootScreenType, ScreenLike};

/// Pushed page showing the status of one of an identity's username requests.
pub struct UsernameRequestScreen {
    pub app_context: Arc<AppContext>,
    identity_id: Identifier,
    normalized_label: String,
    refreshing: bool,
    /// Whether a loaded node can vote; computed on arrival, not per frame.
    can_vote: bool,
    /// The identity's display name; computed on arrival, not per frame.
    identity_label: String,
}

impl UsernameRequestScreen {
    pub fn new(
        app_context: &Arc<AppContext>,
        identity_id: Identifier,
        normalized_label: String,
    ) -> Self {
        let mut screen = Self {
            app_context: app_context.clone(),
            identity_id,
            normalized_label,
            refreshing: false,
            can_vote: false,
            identity_label: String::new(),
        };
        screen.refresh();
        screen
    }

    /// The identity this page belongs to.
    pub fn identity_id(&self) -> Identifier {
        self.identity_id
    }

    /// The normalized label of the request shown.
    pub fn normalized_label(&self) -> &str {
        &self.normalized_label
    }

    fn request(&self) -> Option<UsernameRequest> {
        self.app_context
            .username_requests_for(&self.identity_id)
            .into_iter()
            .find(|r| r.normalized_label == self.normalized_label)
    }

    fn load_identity_label(&self) -> String {
        let identity = self
            .app_context
            .load_local_user_identities()
            .unwrap_or_default()
            .into_iter()
            .find(|qi| qi.identity.id() == self.identity_id);
        match identity {
            Some(qi) => display_label(
                crate::model::dpns_usernames::user_alias(&qi),
                None,
                self.app_context.main_username(&qi).as_deref(),
                &qi.identity.id().to_string(Encoding::Base58),
            ),
            None => shorten_id(&self.identity_id.to_string(Encoding::Base58)),
        }
    }

    fn render_timeline(ui: &mut Ui, request: &UsernameRequest, dark_mode: bool) {
        let (joining, voting, done) = match request.phase {
            RequestPhase::Joinable => (true, false, false),
            RequestPhase::Voting => (false, true, false),
            _ => (false, false, true),
        };
        if let Some(at) = request.requested_at {
            status_line(
                ui,
                Tone::Positive,
                &format!("Requested {}", format_date(at)),
                dark_mode,
            );
        }
        if let Some(join_end) = request.join_end {
            let text = if joining {
                format!(
                    "Open for other requests until {}",
                    format_date_time(join_end)
                )
            } else {
                format!("Open for other requests. Ended {}.", format_date(join_end))
            };
            let tone = if joining {
                Tone::Caution
            } else {
                Tone::Positive
            };
            status_line(ui, tone, &text, dark_mode);
        }
        if let Some(end) = request.end {
            let (tone, text) = if done {
                (
                    Tone::Positive,
                    format!("Community vote ended {}.", format_date(end)),
                )
            } else if voting {
                (
                    Tone::Caution,
                    format!("Community vote. Ends around {}.", format_date_time(end)),
                )
            } else {
                (
                    Tone::Neutral,
                    format!("Community vote until {}", format_date_time(end)),
                )
            };
            status_line(ui, tone, &text, dark_mode);
        }
        let (tone, result) = match request.phase {
            RequestPhase::Joinable | RequestPhase::Voting => {
                (Tone::Neutral, "Result: Not decided yet.")
            }
            RequestPhase::Won => (Tone::Positive, "Result: The name is yours."),
            RequestPhase::Lost => (Tone::Negative, "Result: The name went to someone else."),
            RequestPhase::Locked => (Tone::Negative, "Result: The name is locked for good."),
            RequestPhase::NoWinner => (Tone::Negative, "Result: No one got the name."),
        };
        status_line(ui, tone, result, dark_mode);
    }

    fn render_tally(ui: &mut Ui, request: &UsernameRequest) {
        let tally = &request.tally;
        let standing = match tally.standing() {
            TallyStanding::Leading => "Leading",
            TallyStanding::Tied => "Tied",
            TallyStanding::Trailing | TallyStanding::LockAhead => "",
        };
        let lock_standing = if tally.standing() == TallyStanding::LockAhead {
            "Leading"
        } else {
            ""
        };
        egui::Grid::new("username_request_tally")
            .num_columns(3)
            .spacing([24.0, 4.0])
            .show(ui, |ui| {
                ui.label("You");
                ui.label(standing);
                ui.label(tally.you.to_string());
                ui.end_row();
                for (id, votes) in &tally.others {
                    ui.label(format!(
                        "Other request ({id})",
                        id = shorten_id(&id.to_string(Encoding::Base58))
                    ));
                    ui.label("");
                    ui.label(votes.to_string());
                    ui.end_row();
                }
                ui.label("Lock, so no one gets it");
                ui.label(lock_standing);
                ui.label(tally.lock.to_string());
                ui.end_row();
                ui.label("Abstain");
                ui.label("");
                ui.label(tally.abstain.to_string());
                ui.end_row();
            });
    }

    fn render_body(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.style().visuals.dark_mode;
        if ui.link("‹ Back to profile").clicked() {
            action = AppAction::PopScreen;
        }
        let Some(request) = self.request() else {
            ui.label("This request is no longer listed for this identity.");
            return action;
        };
        let name = request.label.clone();
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.heading(format!("Your request for @{name}"));
            ui.label(phase_label(request.phase));
        });
        ui.label(
            RichText::new(if request.tally.others.is_empty() {
                "This name needs a community vote, so Dash masternodes decide who gets it."
            } else {
                "Other people asked for the same name, so Dash masternodes vote on who gets it."
            })
            .color(DashColors::text_secondary(dark_mode)),
        );
        ui.add_space(10.0);
        Self::render_timeline(ui, &request, dark_mode);

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("How the vote is going").strong());
            let label = if self.refreshing {
                "Refreshing…"
            } else {
                "Refresh status"
            };
            if ui
                .add_enabled(!self.refreshing, egui::Button::new(label))
                .clicked()
            {
                self.refreshing = true;
                action = AppAction::BackendTask(BackendTask::IdentityTask(
                    IdentityTask::RefreshMyUsernameRequests,
                ));
            }
        });
        Self::render_tally(ui, &request);
        ui.label(
            RichText::new(format!(
                "Evonodes count as 4 votes. Votes can change until the vote ends. Last updated {time}.",
                time = format_date_time(request.last_updated)
            ))
            .small()
            .color(DashColors::text_secondary(dark_mode)),
        );

        ui.add_space(12.0);
        ui.label(RichText::new("What happens next").strong());
        for rule in what_happens_next(&name) {
            ui.label(format!("• {rule}"));
        }

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ComponentStyles::add_secondary_button(ui, "Get a username without a vote", dark_mode)
                .clicked()
            {
                action = register_action_for(&self.app_context, self.identity_id);
            }
            if self.can_vote && ui.link("Your nodes can vote on this name").clicked() {
                self.app_context
                    .request_dpns_votes_for_name(self.normalized_label.clone());
                action = AppAction::SetMainScreenThenGoToMainScreen(
                    RootScreenType::RootScreenDPNSActiveContests,
                );
            }
        });
        action
    }
}

impl ScreenLike for UsernameRequestScreen {
    fn refresh(&mut self) {
        self.identity_label = self.load_identity_label();
        self.can_vote = self.app_context.user_role().at_least(UserRole::Power)
            && self
                .app_context
                .load_local_qualified_identities()
                .unwrap_or_default()
                .iter()
                .any(|qi| qi.associated_voter_identity.is_some());
    }

    fn refresh_on_arrival(&mut self) {
        self.refresh();
    }

    fn display_message(&mut self, _message: &str, message_type: MessageType) {
        if matches!(message_type, MessageType::Error | MessageType::Warning) {
            self.refreshing = false;
        }
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        if matches!(
            result,
            BackendTaskSuccessResult::MyUsernameRequestsRefreshed
        ) {
            self.refreshing = false;
        }
    }

    fn ui(&mut self, ui: &mut Ui) -> AppAction {
        let identity_label = self.identity_label.clone();
        let name = self
            .request()
            .map_or_else(|| self.normalized_label.clone(), |r| r.label);
        let crumb = format!("@{name}");
        let breadcrumbs = vec![
            (
                "Identities",
                AppAction::SetMainScreen(RootScreenType::RootScreenIdentityHub),
            ),
            (identity_label.as_str(), AppAction::PopScreen),
            (crumb.as_str(), AppAction::None),
        ];
        let mut action = add_top_panel(ui, &self.app_context, breadcrumbs, vec![]);
        action |= add_left_panel(ui, &self.app_context, RootScreenType::RootScreenIdentityHub);
        action |= island_central_panel(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| self.render_body(ui))
                .inner
        });
        action
    }
}
