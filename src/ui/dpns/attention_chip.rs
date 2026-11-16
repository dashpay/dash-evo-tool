//! Top-bar voting attention chips (VOTE-FR-072): shown on every screen to
//! operators at the Power role with at least one voting node.

use crate::app::AppAction;
use crate::context::AppContext;
use crate::context::feature_gate::FeatureGate;
use crate::model::dpns::urgency_window;
use crate::model::dpns_voting::operator::{AttentionSummary, TimeLeft, time_left};
use crate::ui::RootScreenType;
use crate::ui::components::pill::accent_pill_button;
use crate::ui::dpns::copy::{deadline_line, needs_vote_chip_label, unresolved_chip_label};
use crate::ui::theme::DashColors;
use crate::utils::time::now_ms;
use eframe::egui::Ui;

/// Chips to render for one summary; pure so it is testable without a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionChips {
    /// Label and whether the soonest deadline is urgent.
    pub needs_vote: Option<(String, bool)>,
    pub unresolved: Option<String>,
    pub tooltip: String,
}

/// Decide which chips show. `visible` is the role + voting-node gate.
pub fn attention_chips(
    summary: &AttentionSummary,
    visible: bool,
    now_ms: u64,
    urgency: std::time::Duration,
) -> AttentionChips {
    if !visible || summary.voting_nodes == 0 {
        return AttentionChips {
            needs_vote: None,
            unresolved: None,
            tooltip: String::new(),
        };
    }
    let urgent = summary.is_urgent(now_ms, urgency);
    let soonest: Option<TimeLeft> = summary.soonest_end.and_then(|end| time_left(end, now_ms));
    let needs_vote = (summary.needs_decision > 0).then(|| {
        (
            needs_vote_chip_label(summary.needs_decision, soonest.filter(|_| urgent)),
            urgent,
        )
    });
    let unresolved = (summary.unresolved > 0).then(|| unresolved_chip_label(summary.unresolved));
    let tooltip = summary
        .soonest_names
        .iter()
        .map(|(name, end)| deadline_line(name, end.and_then(|end| time_left(end, now_ms))))
        .collect::<Vec<_>>()
        .join("\n");
    AttentionChips {
        needs_vote,
        unresolved,
        tooltip,
    }
}

/// Render the chips; a click opens Masternodes ▸ Votes.
pub fn show(ui: &mut Ui, app_context: &AppContext) -> AppAction {
    let chips = attention_chips(
        &app_context.dpns_vote_attention(),
        FeatureGate::Masternodes.is_available(app_context),
        now_ms(),
        urgency_window(app_context.network()),
    );
    if render_chips(ui, &chips) {
        // Resolves to Masternodes ▸ Votes ▸ To decide (see `resolve_root_screen`).
        AppAction::SetMainScreenThenGoToMainScreen(RootScreenType::RootScreenDPNSActiveContests)
    } else {
        AppAction::None
    }
}

/// Paint `chips`; returns whether one was activated.
fn render_chips(ui: &mut Ui, chips: &AttentionChips) -> bool {
    let dark_mode = ui.visuals().dark_mode;
    let mut clicked = false;
    if let Some(label) = &chips.unresolved {
        clicked |= accent_pill_button(
            ui,
            label,
            DashColors::warning_color(dark_mode),
            Some("Dash Evo Tool is still checking these votes with Platform. Don't submit them again."),
        )
        .clicked();
    }
    if let Some((label, urgent)) = &chips.needs_vote {
        let accent = if *urgent {
            DashColors::warning_color(dark_mode)
        } else {
            DashColors::text_secondary(dark_mode)
        };
        let tooltip = (!chips.tooltip.is_empty()).then_some(chips.tooltip.as_str());
        clicked |= accent_pill_button(ui, label, accent, tooltip).clicked();
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Key, accesskit::Role};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    const HOUR: u64 = 3_600_000;

    fn summary(needs: usize, unresolved: usize, soonest_end: Option<u64>) -> AttentionSummary {
        AttentionSummary {
            needs_decision: needs,
            soonest_end,
            unresolved,
            soonest_names: vec![("alice".to_owned(), soonest_end)],
            voting_nodes: 2,
        }
    }

    /// VOTE-TC-085: an urgent mainnet deadline turns the chip amber with the time.
    #[test]
    fn urgent_chip_names_the_soonest_deadline() {
        let chips = attention_chips(
            &summary(1, 0, Some(3 * HOUR + 1)),
            true,
            0,
            Duration::from_secs(86_400),
        );
        assert_eq!(
            chips.needs_vote,
            Some((
                "1 name needs your vote · first ends in 3 hours".to_owned(),
                true
            ))
        );
        assert_eq!(chips.tooltip, "alice.dash ends in 3 hours.");
    }

    #[test]
    fn calm_chip_omits_the_deadline() {
        let chips = attention_chips(
            &summary(2, 0, Some(30 * HOUR)),
            true,
            0,
            Duration::from_secs(86_400),
        );
        assert_eq!(
            chips.needs_vote,
            Some(("2 names need your vote".to_owned(), false))
        );
    }

    /// VOTE-TC-086: hidden below the Power role or without a voting node.
    #[test]
    fn chips_hide_without_the_gate_or_voting_nodes() {
        let hidden = attention_chips(&summary(3, 1, Some(HOUR)), false, 0, Duration::ZERO);
        assert_eq!(hidden.needs_vote, None);
        assert_eq!(hidden.unresolved, None);

        let mut keyless = summary(3, 1, Some(HOUR));
        keyless.voting_nodes = 0;
        let hidden = attention_chips(&keyless, true, 0, Duration::ZERO);
        assert_eq!((hidden.needs_vote, hidden.unresolved), (None, None));
    }

    /// VOTE-TC-087: an unconfirmed target shows the second chip on its own.
    #[test]
    fn unresolved_chip_shows_without_pending_decisions() {
        let chips = attention_chips(&summary(0, 1, None), true, 0, Duration::ZERO);
        assert_eq!(chips.needs_vote, None);
        assert_eq!(
            chips.unresolved,
            Some("1 vote is still being checked".to_owned())
        );
    }

    /// QA-001: each chip is a button a keyboard user reaches with Tab and
    /// activates with Enter or Space, and focus shows on screen.
    #[test]
    fn chips_take_keyboard_focus_and_activate_with_enter_or_space() {
        const UNRESOLVED: &str = "1 vote is still being checked";
        for key in [Key::Enter, Key::Space] {
            let chips = AttentionChips {
                needs_vote: Some(("1 name needs your vote".to_owned(), false)),
                unresolved: Some(UNRESOLVED.to_owned()),
                tooltip: String::new(),
            };
            let activations = Rc::new(Cell::new(0));
            let counted = Rc::clone(&activations);
            let mut harness = Harness::builder().build_ui(move |ui| {
                ui.horizontal(|ui| {
                    if render_chips(ui, &chips) {
                        counted.set(counted.get() + 1);
                    }
                });
            });
            harness.run();
            let unfocused = harness.output().shapes.clone();

            harness.key_press(Key::Tab);
            harness.run();
            assert!(
                harness
                    .get_by_role_and_label(Role::Button, UNRESOLVED)
                    .is_focused(),
                "Tab must focus the first chip"
            );
            assert!(
                harness.output().shapes != unfocused,
                "the focused chip must show a focus ring"
            );

            harness.key_press(key);
            harness.run();
            assert_eq!(activations.get(), 1, "{key:?} must activate the chip");
        }
    }
}
