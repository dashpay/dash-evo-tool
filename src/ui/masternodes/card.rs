//! Masternode/evonode card for the Masternodes grid — reuses
//! `identity_picker_card.rs`'s visual language, adding voter-readiness,
//! key-presence, and DPNS-status rows (colour always paired with text, NFR-6).

use crate::model::contested_name::{MasternodeContestSummary, MasternodeVoteStateSummary};
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_identity::{IdentityStatus, IdentityType, MasternodeKeyPresence};
use crate::ui::identity::identity_picker_card::{
    CARD_HEIGHT, CARD_MIN_WIDTH, draw_monogram, draw_type_badge,
};
use crate::ui::masternodes::key_status_tokens;
use crate::ui::theme::DashColors;
use eframe::egui::{
    self, Color32, CornerRadius, Frame, Margin, Response, RichText, Sense, Stroke, Ui, Vec2,
    WidgetInfo, WidgetType,
};

pub(crate) const PLATFORM_IDENTITY_STATUS_TOOLTIP: &str = "This shows whether this masternode's Platform identity can currently be found. It does not show Core network or PoSe status.";

pub(crate) fn platform_identity_status_label(status: IdentityStatus) -> String {
    format!("Platform identity: {status}")
}

/// Heading for a masternode card: the alias when set, otherwise the shortened
/// ProTxHash. Mirrors TC-FR3-02 / TC-FR3-03.
pub fn card_heading(alias: Option<&str>, shortened_pro_tx_hash: &str) -> String {
    match alias.map(str::trim).filter(|s| !s.is_empty()) {
        Some(alias) => alias.to_string(),
        None => shortened_pro_tx_hash.to_string(),
    }
}

/// Sub-line for a masternode card: the shortened ProTxHash shown beneath the
/// heading only when an alias provides the heading. When the ProTxHash is
/// already the heading it is not repeated. Mirrors TC-FR3-03.
pub fn card_sub_line(alias: Option<&str>, shortened_pro_tx_hash: &str) -> Option<String> {
    alias
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|_| shortened_pro_tx_hash.to_string())
}

/// Format a masternode Platform balance for display.
pub fn balance_label(credits: u64) -> String {
    format_credits_as_dash(credits)
}

/// Voter-readiness label. Green + `Voting ready` when a voting key is loaded,
/// warning + `No voting key` otherwise (§7 copy).
pub fn voter_readiness_label(voting_present: bool) -> &'static str {
    if voting_present {
        "Voting ready"
    } else {
        "No voting key"
    }
}

/// DPNS status line with count-first precedence (§10.1): open contests first,
/// then failed or pending scheduled votes, then none.
pub fn dpns_status_line(summary: MasternodeContestSummary) -> String {
    if summary.vote_state == MasternodeVoteStateSummary::Unavailable {
        if summary.open_contest_count == 0 {
            "DPNS voting status unavailable".to_owned()
        } else {
            format!(
                "Vote state unavailable for {} active contests",
                summary.open_contest_count
            )
        }
    } else if summary.open_contest_count > 0 {
        if summary.vote_state == MasternodeVoteStateSummary::Checking {
            format!(
                "Checking votes for {} active contests",
                summary.open_contest_count
            )
        } else if summary.needs_vote_count == 0 {
            "Votes cast in all active contests".to_owned()
        } else if summary.needs_vote_count == 1 {
            format!(
                "{} active contests · 1 needs a vote",
                summary.open_contest_count
            )
        } else {
            format!(
                "{} active contests · {} need votes",
                summary.open_contest_count, summary.needs_vote_count
            )
        }
    } else if summary.has_failed_scheduled_vote {
        "Scheduled vote needs attention".to_string()
    } else if summary.has_scheduled_vote {
        "Vote scheduled".to_string()
    } else {
        "No open contests".to_string()
    }
}

/// Response from [`MasternodeCard::show`]: whether the card was activated, plus
/// the node id echoed from construction so a grid can route selection without
/// tracking indices.
#[derive(Clone, Debug)]
pub struct MasternodeCardResponse {
    pub clicked: bool,
    pub node_id: String,
}

/// A single masternode/evonode card. Built from already-resolved display data
/// so the component stays free of `AppContext` and is unit-testable.
#[derive(Clone, Debug)]
pub struct MasternodeCard {
    node_id: String,
    node_id_short: String,
    alias: Option<String>,
    balance_credits: Option<u64>,
    node_type: IdentityType,
    key_presence: MasternodeKeyPresence,
    contest_summary: MasternodeContestSummary,
    status: IdentityStatus,
}

impl MasternodeCard {
    pub fn new(
        node_id: impl Into<String>,
        node_id_short: impl Into<String>,
        node_type: IdentityType,
        key_presence: MasternodeKeyPresence,
        contest_summary: MasternodeContestSummary,
        status: IdentityStatus,
    ) -> Self {
        Self {
            node_id: node_id.into(),
            node_id_short: node_id_short.into(),
            alias: None,
            balance_credits: None,
            node_type,
            key_presence,
            contest_summary,
            status,
        }
    }

    pub fn with_alias(mut self, alias: Option<String>) -> Self {
        self.alias = alias.filter(|s| !s.trim().is_empty());
        self
    }

    /// Attach the Platform balance shown on the card.
    pub fn with_balance_credits(mut self, credits: u64) -> Self {
        self.balance_credits = Some(credits);
        self
    }

    /// Heading string — exposed for tests (TC-FR3-02 / 03).
    pub fn heading(&self) -> String {
        card_heading(self.alias.as_deref(), &self.node_id_short)
    }

    /// Sub-line string — exposed for tests.
    pub fn sub_line(&self) -> Option<String> {
        card_sub_line(self.alias.as_deref(), &self.node_id_short)
    }

    /// Badge pill text (`Masternode` / `Evonode`).
    pub fn badge_label(&self) -> &'static str {
        match self.node_type {
            IdentityType::Evonode => "Evonode",
            // A User identity never reaches this grid; fall back to Masternode.
            _ => "Masternode",
        }
    }

    pub fn show(&self, ui: &mut Ui) -> MasternodeCardResponse {
        let dark_mode = ui.ctx().global_style().visuals.dark_mode;
        let border = Stroke::new(1.0, DashColors::border(dark_mode));
        let fill = DashColors::surface(dark_mode);
        let heading = self.heading();
        let heading_for_a11y = heading.clone();

        let frame = Frame::new()
            .fill(fill)
            .stroke(border)
            .corner_radius(CornerRadius::same(16))
            .inner_margin(Margin::symmetric(16, 16));

        let desired_size = Vec2::new(CARD_MIN_WIDTH, CARD_HEIGHT);
        let mut platform_identity_status_rect = None;

        let inner = frame.show(ui, |ui| {
            ui.set_min_size(desired_size);
            ui.set_max_width(desired_size.x);
            ui.vertical(|ui| {
                // Top row: monogram (left) + type badge (right).
                ui.horizontal(|ui| {
                    draw_monogram(ui, &heading, false, dark_mode);
                    ui.add_space(8.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                        draw_type_badge(ui, self.badge_label(), dark_mode);
                    });
                });
                ui.add_space(12.0);

                // Heading + optional ProTxHash sub-line.
                ui.label(
                    RichText::new(&heading)
                        .color(DashColors::text_primary(dark_mode))
                        .strong()
                        .size(16.0),
                );
                if let Some(sub) = self.sub_line() {
                    ui.label(
                        RichText::new(sub)
                            .color(DashColors::text_secondary(dark_mode))
                            .size(13.0),
                    );
                }
                if let Some(credits) = self.balance_credits {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(balance_label(credits))
                            .monospace()
                            .color(DashColors::text_primary(dark_mode))
                            .strong()
                            .size(13.0),
                    );
                }
                ui.add_space(8.0);

                // Voter readiness (colour + text — NFR-6).
                let voting = self.key_presence.voting;
                let (voter_color, voter_label) = if voting {
                    (
                        DashColors::success_color(dark_mode),
                        voter_readiness_label(true),
                    )
                } else {
                    (
                        DashColors::warning_color(dark_mode),
                        voter_readiness_label(false),
                    )
                };
                draw_status_row(ui, voter_color, voter_label, dark_mode);

                // Compact key status: `Keys: V O P` (present emphasised).
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Keys:")
                            .color(DashColors::text_secondary(dark_mode))
                            .size(13.0),
                    );
                    // The whole card is a `Sense::click()` target, so the letters
                    // carry no per-letter tooltip here — a Help cursor inside a
                    // clickable card would fight its PointingHand affordance. The
                    // role explanations live on the detail screen's `Roles:` row.
                    for token in key_status_tokens(self.key_presence) {
                        if token.present {
                            ui.label(
                                RichText::new(token.letter)
                                    .color(DashColors::text_primary(dark_mode))
                                    .strong()
                                    .size(13.0),
                            );
                        } else {
                            ui.label(
                                RichText::new("·")
                                    .color(DashColors::text_secondary(dark_mode))
                                    .size(13.0),
                            );
                        }
                    }
                });

                // DPNS status line (count-first precedence).
                ui.label(
                    RichText::new(dpns_status_line(self.contest_summary))
                        .color(DashColors::text_secondary(dark_mode))
                        .size(13.0),
                );
                ui.add_space(4.0);

                let status_response = draw_status_row(
                    ui,
                    Color32::from(self.status),
                    &platform_identity_status_label(self.status),
                    dark_mode,
                );
                platform_identity_status_rect = Some(status_response.rect);
            });
        });

        // Whole card = one click target with an accessible label (NFR-6).
        let rect = inner.response.rect;
        let id = ui.id().with(("masternode-card", &self.node_id));
        let response: Response = ui.interact(rect, id, Sense::click());
        let response = if platform_identity_status_rect.is_some_and(|status_rect| {
            ui.input(|input| {
                input
                    .pointer
                    .hover_pos()
                    .is_some_and(|pointer| status_rect.contains(pointer))
            })
        }) {
            response.on_hover_text(PLATFORM_IDENTITY_STATUS_TOOLTIP)
        } else {
            response
        };
        if response.hovered() {
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(16),
                Stroke::new(1.5, DashColors::border_light(dark_mode)),
                egui::StrokeKind::Inside,
            );
        }
        response.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Button, true, format!("Open {heading_for_a11y}"))
        });

        MasternodeCardResponse {
            clicked: response.clicked(),
            node_id: self.node_id.clone(),
        }
    }
}

/// Paint a small status dot followed by its text label. The label is always
/// present, so status is never conveyed by colour alone (NFR-6).
fn draw_status_row(ui: &mut Ui, color: Color32, label: &str, dark_mode: bool) -> Response {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(10.0, 10.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 4.0, color);
        ui.add_space(4.0);
        ui.label(
            RichText::new(label)
                .color(DashColors::text_primary(dark_mode))
                .size(13.0),
        );
    })
    .response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_identity_status_copy_is_explicit() {
        assert_eq!(
            platform_identity_status_label(IdentityStatus::Active),
            "Platform identity: Active"
        );
        assert_eq!(
            platform_identity_status_label(IdentityStatus::NotFound),
            "Platform identity: Not Found"
        );
    }

    #[test]
    fn tc_fr3_02_heading_is_shortened_pro_tx_hash_when_no_alias() {
        assert_eq!(card_heading(None, "9a3f…d7e2"), "9a3f…d7e2");
        assert_eq!(card_sub_line(None, "9a3f…d7e2"), None);
    }

    #[test]
    fn tc_fr3_03_alias_heading_with_pro_tx_hash_sub_line() {
        assert_eq!(card_heading(Some("mn-east-01"), "9a3f…d7e2"), "mn-east-01");
        assert_eq!(
            card_sub_line(Some("mn-east-01"), "9a3f…d7e2"),
            Some("9a3f…d7e2".to_string())
        );
    }

    #[test]
    fn blank_alias_falls_through_to_pro_tx_hash() {
        assert_eq!(card_heading(Some("   "), "9a3f…d7e2"), "9a3f…d7e2");
        assert_eq!(card_sub_line(Some("   "), "9a3f…d7e2"), None);
    }

    #[test]
    fn balance_label_uses_canonical_dash_formatting() {
        for (credits, expected) in [
            (0, "0 DASH"),
            (2_890_000_000, "0.0289 DASH"),
            (100_000_000_000, "1 DASH"),
        ] {
            assert_eq!(balance_label(credits), expected);
        }
    }

    #[test]
    fn tc_fr3_06_07_voter_readiness_copy() {
        assert_eq!(voter_readiness_label(true), "Voting ready");
        assert_eq!(voter_readiness_label(false), "No voting key");
    }

    #[test]
    fn tc_fr3_09_dpns_open_contest_count() {
        let summary = MasternodeContestSummary {
            open_contest_count: 3,
            needs_vote_count: 1,
            has_scheduled_vote: false,
            ..Default::default()
        };
        assert_eq!(
            dpns_status_line(summary),
            "3 active contests · 1 needs a vote"
        );
    }

    #[test]
    fn tc_fr3_10_dpns_no_open_contests() {
        assert_eq!(
            dpns_status_line(MasternodeContestSummary::default()),
            "No open contests"
        );
    }

    #[test]
    fn tc_fr3_11_count_takes_precedence_over_scheduled() {
        // Both an open contest AND a scheduled vote present → count wins.
        let summary = MasternodeContestSummary {
            open_contest_count: 2,
            needs_vote_count: 1,
            has_scheduled_vote: true,
            ..Default::default()
        };
        assert_eq!(
            dpns_status_line(summary),
            "2 active contests · 1 needs a vote"
        );
    }

    #[test]
    fn dpns_scheduled_shown_only_when_no_open_contests() {
        let summary = MasternodeContestSummary {
            open_contest_count: 0,
            needs_vote_count: 0,
            has_scheduled_vote: true,
            ..Default::default()
        };
        assert_eq!(dpns_status_line(summary), "Vote scheduled");
    }

    #[test]
    fn terminal_scheduled_failure_needs_attention() {
        let summary = MasternodeContestSummary {
            has_scheduled_vote: false,
            has_failed_scheduled_vote: true,
            ..Default::default()
        };

        assert_eq!(dpns_status_line(summary), "Scheduled vote needs attention");
    }

    #[test]
    fn dpns_status_reports_checking_instead_of_all_votes_cast() {
        let summary = MasternodeContestSummary {
            open_contest_count: 3,
            vote_state: MasternodeVoteStateSummary::Checking,
            ..Default::default()
        };

        assert_eq!(
            dpns_status_line(summary),
            "Checking votes for 3 active contests"
        );
    }

    #[test]
    fn dpns_status_reports_unavailable_instead_of_all_votes_cast() {
        let summary = MasternodeContestSummary {
            open_contest_count: 2,
            vote_state: MasternodeVoteStateSummary::Unavailable,
            ..Default::default()
        };

        assert_eq!(
            dpns_status_line(summary),
            "Vote state unavailable for 2 active contests"
        );
    }

    #[test]
    fn dpns_status_reports_unavailable_when_the_summary_read_failed() {
        assert_eq!(
            dpns_status_line(MasternodeContestSummary::unavailable()),
            "DPNS voting status unavailable"
        );
    }

    #[test]
    fn tc_fr3_04_05_badge_label_by_type() {
        let card = MasternodeCard::new(
            "id",
            "9a3f…d7e2",
            IdentityType::Masternode,
            MasternodeKeyPresence::default(),
            MasternodeContestSummary::default(),
            IdentityStatus::Active,
        );
        assert_eq!(card.badge_label(), "Masternode");

        let evo = MasternodeCard::new(
            "id",
            "9a3f…d7e2",
            IdentityType::Evonode,
            MasternodeKeyPresence::default(),
            MasternodeContestSummary::default(),
            IdentityStatus::Active,
        );
        assert_eq!(evo.badge_label(), "Evonode");
    }
}
