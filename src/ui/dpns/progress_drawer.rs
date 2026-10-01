//! Vote progress drawer (VOTE-FR-083): bottom-right, collapsible, on every
//! screen. Non-blocking — the rest of the app stays interactive.

use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::contested_names::ContestedResourceTask;
use crate::context::AppContext;
use crate::model::dpns_voting::progress::{
    ProgressCounts, ProgressPhase, drawer_operations, progress_counts, progress_phase,
};
use crate::model::dpns_voting::{DpnsVoteFailure, DpnsVoteOutcome, DpnsVoteTargetStatus};
use crate::ui::RootScreenType;
use crate::ui::dpns::copy::{drawer_header, progress_row_status};
use crate::ui::theme::DashColors;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use eframe::egui::{self, Align2, Id, RichText, Sense, Ui};

const OPEN_ID: &str = "dpns_vote_progress_drawer_open";

/// AppState-owned drawer state; survives navigation. Dismissed operations
/// live in `AppContext` so the `Needs attention` row honours them too.
#[derive(Debug, Clone)]
pub struct DrawerState {
    /// Settled operations created before this time are not listed.
    since_ms: u64,
}

impl DrawerState {
    pub fn new(since_ms: u64) -> Self {
        Self { since_ms }
    }

    /// Settled operations created before this moment are not listed.
    pub fn since_ms(&self) -> u64 {
        self.since_ms
    }
}

/// Expand the drawer, e.g. right after a submit.
pub fn expand(ctx: &egui::Context) {
    ctx.data_mut(|data| data.insert_temp(Id::new(OPEN_ID), true));
}

fn is_open(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp(Id::new(OPEN_ID)).unwrap_or(false))
}

/// The action a row offers for an outcome, if any (VOTE-FR-083).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowAction {
    CheckAgain,
    ReviewAgain,
    AddVotingKey,
}

impl RowAction {
    fn label(self) -> &'static str {
        match self {
            Self::CheckAgain => "Check again",
            Self::ReviewAgain => "Review again",
            Self::AddVotingKey => "Add voting key",
        }
    }
}

/// The valid action for an outcome: never a resubmit while ambiguous.
pub fn row_action(outcome: &DpnsVoteOutcome) -> Option<RowAction> {
    match (outcome.status, outcome.failure) {
        (DpnsVoteTargetStatus::Unconfirmed, _) => Some(RowAction::CheckAgain),
        (_, Some(DpnsVoteFailure::VotingKeyMissing)) => Some(RowAction::AddVotingKey),
        (_, Some(DpnsVoteFailure::VotingEnded)) => None,
        (status, _) if status.is_reviewable_failure() => Some(RowAction::ReviewAgain),
        _ => None,
    }
}

fn choice_label(choice: ResourceVoteChoice) -> String {
    match choice {
        ResourceVoteChoice::Lock => "Lock".to_owned(),
        ResourceVoteChoice::Abstain => "Abstain".to_owned(),
        ResourceVoteChoice::TowardsIdentity(id) => {
            let encoded = id.to_string(Encoding::Base58);
            format!("Vote for {head}…", head = &encoded[..6])
        }
    }
}

fn paint_segments(ui: &mut Ui, counts: ProgressCounts) {
    let dark_mode = ui.visuals().dark_mode;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 6.0), Sense::hover());
    let total = counts.total.max(1) as f32;
    let mut x = rect.left();
    for (count, color) in [
        (counts.confirmed, DashColors::success_color(dark_mode)),
        (counts.failed, DashColors::error_color(dark_mode)),
        (counts.checking, DashColors::warning_color(dark_mode)),
        (counts.sending, DashColors::DASH_BLUE),
    ] {
        if count == 0 {
            continue;
        }
        let width = rect.width() * count as f32 / total;
        let segment = egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(width, 6.0));
        ui.painter().rect_filled(segment, 2.0, color);
        x += width;
    }
}

/// Render the drawer when there is something to show.
pub fn show(ctx: &egui::Context, app_context: &AppContext, state: &mut DrawerState) -> AppAction {
    let operations = app_context.dpns_vote_progress();
    let dismissed = app_context.dismissed_dpns_vote_operations();
    let shown = drawer_operations(&operations, state.since_ms, &dismissed);
    if shown.is_empty() {
        return AppAction::None;
    }
    let counts = progress_counts(shown.iter().copied());
    let mut open = is_open(ctx);
    let mut action = AppAction::None;
    egui::Area::new(Id::new("dpns_vote_progress_drawer"))
        .anchor(Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -16.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::window(ui.style()).show(ui, |ui| {
                ui.set_width(360.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(drawer_header(counts)).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button(if open { "Hide" } else { "Show" })
                            .clicked()
                        {
                            open = !open;
                        }
                        if !counts.in_flight() && ui.small_button("Dismiss").clicked() {
                            app_context.dismiss_dpns_vote_operations(
                                shown.iter().map(|operation| operation.id),
                            );
                        }
                    });
                });
                paint_segments(ui, counts);
                if !open {
                    return;
                }
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        for outcome in shown
                            .iter()
                            .flat_map(|operation| &operation.targets)
                            .filter(|outcome| progress_phase(outcome).is_some())
                        {
                            ui.horizontal_wrapped(|ui| {
                                let node =
                                    outcome.target.voter_alias.clone().unwrap_or_else(|| {
                                        let encoded =
                                            outcome.target.key.voter_id.to_string(Encoding::Base58);
                                        format!("{head}…", head = &encoded[..6])
                                    });
                                ui.label(format!(
                                    "{node} · {name}.dash · {choice}",
                                    name = outcome.target.contested_name,
                                    choice = choice_label(outcome.target.requested_choice),
                                ));
                                let color = match progress_phase(outcome) {
                                    Some(ProgressPhase::Failed) => {
                                        DashColors::error_color(ui.visuals().dark_mode)
                                    }
                                    Some(ProgressPhase::Checking) => {
                                        DashColors::warning_color(ui.visuals().dark_mode)
                                    }
                                    _ => DashColors::text_secondary(ui.visuals().dark_mode),
                                };
                                ui.label(
                                    RichText::new(progress_row_status(
                                        outcome.status,
                                        outcome.failure,
                                    ))
                                    .color(color),
                                );
                                if let Some(row) = row_action(outcome)
                                    && ui.small_button(row.label()).clicked()
                                {
                                    action = match row {
                                        RowAction::CheckAgain => AppAction::BackendTask(
                                            BackendTask::ContestedResourceTask(
                                                ContestedResourceTask::ReconcileDpnsVoteOperation(
                                                    outcome.operation_id,
                                                    app_context.network(),
                                                ),
                                            ),
                                        ),
                                        RowAction::ReviewAgain => {
                                            app_context.request_dpns_vote_review(outcome.clone());
                                            AppAction::SetMainScreenThenGoToMainScreen(
                                                RootScreenType::RootScreenDPNSActiveContests,
                                            )
                                        }
                                        RowAction::AddVotingKey => {
                                            AppAction::SetMainScreenThenGoToMainScreen(
                                                RootScreenType::RootScreenMasternodes,
                                            )
                                        }
                                    };
                                }
                            });
                        }
                    });
            });
        });
    ctx.data_mut(|data| data.insert_temp(Id::new(OPEN_ID), open));
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::dpns_voting::{
        DpnsVoteOperation, DpnsVoteTarget, DpnsVoteTargetKey, VoteTiming,
    };
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::platform::Identifier;

    fn outcome(status: DpnsVoteTargetStatus, failure: Option<DpnsVoteFailure>) -> DpnsVoteOutcome {
        let mut operation = DpnsVoteOperation::new(vec![DpnsVoteTarget {
            key: DpnsVoteTargetKey {
                network: Network::Testnet,
                voter_id: Identifier::from([1; 32]),
                vote_poll_id: Identifier::from([2; 32]),
            },
            voter_alias: None,
            contested_name: "alice".to_owned(),
            requested_choice: ResourceVoteChoice::Lock,
            current_choice: None,
            timing: VoteTiming::Now,
        }]);
        let mut outcome = operation.targets.remove(0);
        outcome.status = status;
        outcome.failure = failure;
        outcome
    }

    /// VOTE-FR-044/083: ambiguity offers only Check again; ended voting offers nothing.
    #[test]
    fn rows_offer_only_the_valid_action() {
        use DpnsVoteTargetStatus as S;
        assert_eq!(
            row_action(&outcome(
                S::Unconfirmed,
                Some(DpnsVoteFailure::ResultUnconfirmed)
            )),
            Some(RowAction::CheckAgain)
        );
        assert_eq!(
            row_action(&outcome(
                S::Rejected,
                Some(DpnsVoteFailure::PlatformRejected)
            )),
            Some(RowAction::ReviewAgain)
        );
        assert_eq!(
            row_action(&outcome(
                S::FailedBeforeSubmission,
                Some(DpnsVoteFailure::VotingKeyMissing)
            )),
            Some(RowAction::AddVotingKey)
        );
        assert_eq!(
            row_action(&outcome(
                S::FailedBeforeSubmission,
                Some(DpnsVoteFailure::VotingEnded)
            )),
            None
        );
        assert_eq!(row_action(&outcome(S::Confirming, None)), None);
        assert_eq!(row_action(&outcome(S::Confirmed, None)), None);
    }
}
