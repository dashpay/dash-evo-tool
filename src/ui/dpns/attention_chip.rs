//! Top-bar voting attention chips (VOTE-FR-072): shown on every screen to
//! operators at the Power role with at least one voting node.

use crate::app::AppAction;
use crate::context::AppContext;
use crate::context::feature_gate::FeatureGate;
use crate::model::dpns::urgency_window;
use crate::model::dpns_voting::operator::{AttentionSummary, TimeLeft, time_left};
use crate::ui::RootScreenType;
use crate::ui::components::pill::accent_pill;
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
    let dark_mode = ui.visuals().dark_mode;
    let mut clicked = false;
    if let Some(label) = &chips.unresolved {
        clicked |= accent_pill(
            ui,
            label,
            DashColors::warning_color(dark_mode),
            Some("Dash Evo Tool is still checking these votes with Platform. Don't submit them again."),
        )
        .interact(eframe::egui::Sense::click())
        .clicked();
    }
    if let Some((label, urgent)) = &chips.needs_vote {
        let accent = if *urgent {
            DashColors::warning_color(dark_mode)
        } else {
            DashColors::text_secondary(dark_mode)
        };
        let tooltip = (!chips.tooltip.is_empty()).then_some(chips.tooltip.as_str());
        clicked |= accent_pill(ui, label, accent, tooltip)
            .interact(eframe::egui::Sense::click())
            .clicked();
    }
    if clicked {
        // Resolves to Masternodes ▸ Votes ▸ To decide (see `resolve_root_screen`).
        AppAction::SetMainScreenThenGoToMainScreen(RootScreenType::RootScreenDPNSActiveContests)
    } else {
        AppAction::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
