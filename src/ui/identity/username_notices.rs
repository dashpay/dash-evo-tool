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
    // Outcomes not yet recorded as seen. The mark is re-sent until the store
    // confirms it, so a frame whose action is replaced by another dispatch
    // (e.g. a profile load) cannot lose it; the context bounds the retries.
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
    if !unrecorded.is_empty()
        && matches!(action, AppAction::None)
        && app_context.try_dispatch_username_seen_mark(&identity_id, ui.ctx().cumulative_pass_nr())
    {
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
    use crate::backend_task::error::TaskError;
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

    /// A store that keeps failing the "seen" write does not get a new task (or
    /// a new error banner) on every frame: the next frame stays quiet, and the
    /// retries stop for the session after a few failures.
    #[test]
    fn failing_seen_store_is_not_retried_every_frame() {
        use crate::model::dpns_usernames::{SEEN_MARK_MAX_FAILURES, UsernameRequest};
        use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
        use crate::model::qualified_identity::{IdentityStatus, IdentityType};
        use crate::utils::egui_mpsc::SenderAsync;
        use crate::wallet_backend::DetKv;
        use crate::wallet_backend::kv_test_support::FailingKv;

        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let store = Arc::new(FailingKv::default());
        ctx.set_det_kv_override_for_test(DetKv::from_store(store.clone()));
        let identity = QualifiedIdentity {
            identity: dash_sdk::dpp::identity::Identity::create_basic_identity(
                [0x61; 32].into(),
                dash_sdk::dpp::version::PlatformVersion::latest(),
            )
            .expect("identity"),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: KeyStorage::default(),
            dpns_names: Vec::new(),
            associated_wallets: Default::default(),
            secret_access: None,
            wallet_index: None,
            top_ups: Default::default(),
            status: IdentityStatus::Active,
            network: dash_sdk::dpp::dashcore::Network::Testnet,
        };
        let id = identity.identity.id();
        let mut won = UsernameRequest::submitted(
            "ali",
            0,
            ContestDurations {
                total: Duration::ZERO,
                join: Duration::ZERO,
            },
            None,
        );
        won.phase = RequestPhase::Won;
        ctx.store_username_requests(&id, vec![won]).expect("store");
        store.fail_next_puts_containing("det:username_outcomes_seen:", usize::MAX);

        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let app_context = ctx.clone();
        let mut harness = egui_kittest::Harness::builder().build_ui_state(
            move |ui, last: &mut Option<AppAction>| {
                let action = render(ui, &app_context, &identity);
                if !matches!(action, AppAction::None) {
                    *last = Some(action);
                }
            },
            None,
        );
        let mut failures = 0;
        for _ in 0..20 {
            harness.step();
            let Some(AppAction::BackendTask(task)) = harness.state_mut().take() else {
                continue;
            };
            let (tx, _rx) = tokio::sync::mpsc::channel(8);
            let sender = SenderAsync::new(tx, egui::Context::default());
            let result = rt.block_on(ctx.run_backend_task(task, sender));
            assert!(matches!(result, Err(TaskError::UsernameStorage { .. })));
            failures += 1;
            // Right after a failure, the next frame must not dispatch again.
            harness.step();
            assert!(
                harness.state_mut().take().is_none(),
                "a failing store was retried on the very next frame"
            );
            // Skip the backoff so the test exercises the session cap.
            ctx.username_cache_mut()
                .seen_marks
                .get_mut(&id)
                .expect("attempts")
                .expire_backoff_for_test();
        }
        assert_eq!(failures, SEEN_MARK_MAX_FAILURES);
    }
}
