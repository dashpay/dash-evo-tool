//! Home notices for an identity's username requests: one card per pending
//! request and a one-time banner per finished outcome.

use std::collections::BTreeSet;
use std::sync::Arc;

use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use egui::{Frame, Id, Margin, RichText, Ui};

use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::identity::IdentityTask;
use crate::context::AppContext;
use crate::model::dpns_usernames::{RequestPhase, UsernameRequest};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::ScreenType;
use crate::ui::identity::register_dpns_name_screen::status_line;
use crate::ui::identity::username_copy::{Tone, format_date, phase_label, standing_line};
use crate::ui::identity::usernames_card::register_action_for;
use crate::ui::theme::{ComponentStyles, DashColors};

/// Outcome banners shown in this session, so a banner marked seen stays up until dismissed.
const SESSION_BANNERS_ID: &str = "identity_username_outcome_banners";

/// Per-session banner state, kept in egui temp data.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SessionBanners {
    /// Shown this session; stays up after being recorded as seen.
    shown: BTreeSet<String>,
    /// Dismissed this session; hidden even before the seen flag is stored.
    dismissed: BTreeSet<String>,
}

/// Text of the one-time banner for a finished request, if it gets one.
pub fn outcome_banner_text(request: &UsernameRequest) -> Option<String> {
    let name = &request.label;
    match request.phase {
        RequestPhase::Won => Some(format!(
            "You're @{name}. People can now find and pay you by this name."
        )),
        RequestPhase::Lost => Some(format!(
            "@{name} went to someone else. You can choose a different username."
        )),
        RequestPhase::Locked => Some(format!(
            "No one can register @{name} anymore. The community vote locked it."
        )),
        RequestPhase::NoWinner => Some(format!(
            "The vote for @{name} ended without a winner. You can choose a different username."
        )),
        RequestPhase::Joinable | RequestPhase::Voting => None,
    }
}

fn banner_key(identity_id: &Identifier, request: &UsernameRequest) -> String {
    format!(
        "{identity_id}:{}:{:?}",
        request.normalized_label, request.phase
    )
}

/// Render pending cards and outcome banners for `identity`.
pub fn render(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
) -> AppAction {
    let mut action = AppAction::None;
    let dark_mode = ui.style().visuals.dark_mode;
    let identity_id = identity.identity.id();
    let requests = app_context.username_requests_for(&identity_id);
    let session_id = Id::new(SESSION_BANNERS_ID);
    let mut session: SessionBanners = ui
        .ctx()
        .data(|data| data.get_temp(session_id))
        .unwrap_or_default();
    let before = session.clone();
    // Outcomes not yet recorded as seen. The mark is re-sent on every frame
    // until the store confirms it, so a frame whose action is replaced by
    // another dispatch (e.g. a profile load) cannot lose it; the task is idempotent.
    let mut unrecorded: Vec<UsernameRequest> = Vec::new();

    for request in &requests {
        let Some(text) = outcome_banner_text(request) else {
            continue;
        };
        let key = banner_key(&identity_id, request);
        let recorded = app_context.username_outcome_seen(&identity_id, request);
        if !recorded {
            unrecorded.push(request.clone());
        }
        if session.dismissed.contains(&key) || (recorded && !session.shown.contains(&key)) {
            continue;
        }
        session.shown.insert(key.clone());
        let tone = if request.phase == RequestPhase::Won {
            Tone::Positive
        } else {
            Tone::Neutral
        };
        notice_frame(ui, dark_mode, |ui| {
            ui.horizontal_wrapped(|ui| {
                status_line(ui, tone, &text, dark_mode);
                if matches!(request.phase, RequestPhase::Lost | RequestPhase::NoWinner)
                    && ui.button("Choose another username").clicked()
                {
                    action = register_action_for(app_context, identity_id);
                }
                if ui.button("Dismiss").clicked() {
                    session.shown.remove(&key);
                    session.dismissed.insert(key.clone());
                }
            });
        });
    }
    if !unrecorded.is_empty() && matches!(action, AppAction::None) {
        action = mark_seen_action(identity_id, unrecorded);
    }
    if session != before {
        ui.ctx()
            .data_mut(|data| data.insert_temp(session_id, session));
    }

    for request in requests.iter().filter(|r| r.phase.is_pending()) {
        notice_frame(ui, dark_mode, |ui| {
            status_line(ui, Tone::Caution, phase_label(request.phase), dark_mode);
            let name = &request.label;
            let first = match request.end {
                Some(end) => format!(
                    "@{name} is waiting for a community vote. It ends around {date}.",
                    date = format_date(end)
                ),
                None => format!("@{name} is waiting for a community vote."),
            };
            ui.label(RichText::new(first).strong());
            ui.label(standing_line(
                request.tally.standing(),
                request.tally.others.len(),
            ));
            if ComponentStyles::add_secondary_button(ui, "View status", dark_mode).clicked() {
                action = AppAction::AddScreen(
                    ScreenType::UsernameRequestStatus {
                        identity_id,
                        normalized_label: request.normalized_label.clone(),
                    }
                    .create_screen(app_context),
                );
            }
        });
    }
    action
}

fn mark_seen_action(identity_id: Identifier, requests: Vec<UsernameRequest>) -> AppAction {
    AppAction::BackendTask(BackendTask::IdentityTask(
        IdentityTask::MarkUsernameOutcomesSeen {
            identity_id,
            requests,
        },
    ))
}

fn notice_frame(ui: &mut Ui, dark_mode: bool, add: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(DashColors::surface(dark_mode))
        .inner_margin(Margin::symmetric(12, 10))
        .corner_radius(6.0)
        .show(ui, add);
    ui.add_space(8.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::dpns::ContestDurations;
    use std::time::Duration;

    #[test]
    fn banners_exist_only_for_outcomes() {
        let mut request = UsernameRequest::submitted(
            "ali",
            0,
            ContestDurations {
                total: Duration::ZERO,
                join: Duration::ZERO,
            },
            None,
        );
        assert!(outcome_banner_text(&request).is_none());
        request.phase = RequestPhase::Won;
        assert_eq!(
            outcome_banner_text(&request).as_deref(),
            Some("You're @ali. People can now find and pay you by this name.")
        );
        request.phase = RequestPhase::Locked;
        assert_eq!(
            outcome_banner_text(&request).as_deref(),
            Some("No one can register @ali anymore. The community vote locked it.")
        );
    }
}
