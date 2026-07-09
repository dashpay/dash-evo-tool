//! Masternode/evonode detail view (FR-5, reuse-heavy).
//!
//! Composition order is fixed (TC-FR5-01, human-requested late correction):
//!
//! 1. **Header** — conditional alias, shortened ProTxHash + copy-full-value,
//!    type badge, `IdentityStatus` dot + label.
//! 2. **Actions row** (FR-9) — `Withdraw` / `Top up` / `Transfer` entry points
//!    that push the *existing* credit screens scoped to this node's
//!    `QualifiedIdentity` (both node types). Evonode-only `Claim token
//!    rewards ›` cross-link (FR-11), absent for a plain masternode.
//! 3. **Keys section** (FR-10) — V/O/P presence, copyable voter-identity id,
//!    protection tier, `Add password protection…` (Tier-1 only), `Manage keys ›`
//!    into the existing key screens.
//! 4. **DPNS voting** — collapsible, count in the header. Populated in B5b.
//! 5. **Remove** — danger `ConfirmationDialog`; removes the node and its
//!    associated voter identity.
//!
//! The `‹ All masternodes` back row lives in the content panel, not the global
//! header. Every reused screen is pushed by type — no parallel MN-specific
//! reimplementation (NFR-1).

use std::sync::Arc;

use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use eframe::egui::{self, Color32, RichText, Ui};

use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::contested_name::MasternodeContestSummary;
use crate::model::qualified_identity::{IdentityType, MasternodeKeyPresence, QualifiedIdentity};
use crate::ui::ScreenType;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::identity::identity_picker_card::draw_type_badge;
use crate::ui::identity::identity_pill::shorten_id;
use crate::ui::theme::{ComponentStyles, DashColors};
use crate::wallet_backend::IdentityKeyView;
use crate::wallet_backend::secret_seam::SecretScheme;

/// The fixed top→bottom section order. Actions must precede Keys (TC-FR5-01).
pub const SECTION_ORDER: [&str; 5] = ["Header", "Actions", "Keys", "DPNS", "Remove"];

/// At-rest protection posture of a node's vault keys, reduced to what the detail
/// view needs: the tier label and whether an `Add password protection…` action
/// applies (only when there are unprotected vault keys to seal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProtectionTier {
    /// No vault-stored keys (read-only node or resident-plaintext only).
    NoVaultKeys,
    /// At least one unprotected (Tier-1) vault key, none protected.
    Unprotected,
    /// Every vault key is password-protected (Tier-2), or a mix.
    Protected,
}

impl ProtectionTier {
    fn label(self) -> &'static str {
        match self {
            ProtectionTier::Protected => "Keys: password-protected",
            // A read-only node has nothing sealed either — "unprotected" is the
            // accurate, non-alarming description of its at-rest posture.
            _ => "Keys: unprotected",
        }
    }

    /// `Add password protection…` is offered only when there are Tier-1 keys to
    /// seal (§FR-8 / NFR-4).
    fn offers_add_protection(self) -> bool {
        matches!(self, ProtectionTier::Unprotected)
    }
}

/// Outcome of rendering the detail view for one frame.
pub enum DetailOutcome {
    /// No terminal interaction this frame.
    None,
    /// Return to the card list (`‹ All masternodes`).
    Back,
    /// The node was removed — return to the list and reload.
    Removed,
    /// Push a reused screen / navigate. Boxed because `AppAction` is large.
    Forward(Box<AppAction>),
}

/// Masternode/evonode detail view state.
pub struct MasternodeDetailView {
    app_context: Arc<AppContext>,
    identity: QualifiedIdentity,
    node_id_hex_full: String,
    node_id_short: String,
    key_presence: MasternodeKeyPresence,
    contest_summary: MasternodeContestSummary,
    /// Collapsible DPNS section state (populated in B5b); collapsed by default.
    dpns_open: bool,
    remove_dialog: Option<ConfirmationDialog>,
}

impl MasternodeDetailView {
    pub fn new(app_context: &Arc<AppContext>, identity: QualifiedIdentity) -> Self {
        let node_id_hex_full = identity.identity.id().to_string(Encoding::Hex);
        let node_id_short = shorten_id(&node_id_hex_full);
        let key_presence = identity.masternode_key_presence();
        let voter_id = identity
            .associated_voter_identity
            .as_ref()
            .map(|(voter, _)| voter.id());
        let contest_summary = app_context
            .masternode_contest_summary(voter_id)
            .unwrap_or_default();
        Self {
            app_context: app_context.clone(),
            identity,
            node_id_hex_full,
            node_id_short,
            key_presence,
            contest_summary,
            dpns_open: false,
            remove_dialog: None,
        }
    }

    /// The node's identity id — used by the list screen to match the open node.
    pub fn node_id(&self) -> dash_sdk::platform::Identifier {
        self.identity.identity.id()
    }

    fn is_evonode(&self) -> bool {
        self.identity.identity_type == IdentityType::Evonode
    }

    /// Build an `AddScreen` action for a reused screen type, scoped to this node.
    fn push(&self, screen_type: ScreenType) -> AppAction {
        AppAction::AddScreen(screen_type.create_screen(&self.app_context))
    }

    fn badge_label(&self) -> &'static str {
        if self.is_evonode() {
            "Evonode"
        } else {
            "Masternode"
        }
    }

    /// Probe the at-rest protection posture of this node's vault keys.
    fn protection_tier(&self) -> ProtectionTier {
        let Ok(backend) = self.app_context.wallet_backend() else {
            return ProtectionTier::NoVaultKeys;
        };
        let view = IdentityKeyView::new(
            backend.secret_store(),
            self.identity.identity.id().to_buffer(),
        );
        let (mut protected, mut unprotected) = (0usize, 0usize);
        for (target, key_id) in self.identity.private_keys.keys_set() {
            match view.scheme(&target, key_id) {
                Ok(SecretScheme::Protected) => protected += 1,
                Ok(SecretScheme::Unprotected) => unprotected += 1,
                _ => {}
            }
        }
        match (protected, unprotected) {
            (0, 0) => ProtectionTier::NoVaultKeys,
            (0, _) => ProtectionTier::Unprotected,
            _ => ProtectionTier::Protected,
        }
    }

    pub fn show(&mut self, ui: &mut Ui, network_accent: Color32) -> DetailOutcome {
        let dark_mode = ui.style().visuals.dark_mode;
        let mut outcome = DetailOutcome::None;

        // Back row + Refresh (content-panel, not the global header). FR-7.
        ui.horizontal(|ui| {
            if ui
                .selectable_label(false, RichText::new("‹ All masternodes"))
                .clicked()
            {
                outcome = DetailOutcome::Back;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ComponentStyles::add_toolbar_button(ui, "Refresh", network_accent).clicked() {
                    let voter_id = self
                        .identity
                        .associated_voter_identity
                        .as_ref()
                        .map(|(voter, _)| voter.id());
                    self.contest_summary = self
                        .app_context
                        .masternode_contest_summary(voter_id)
                        .unwrap_or_default();
                }
            });
        });
        ui.separator();

        egui::ScrollArea::vertical().show(ui, |ui| {
            self.render_header(ui, dark_mode);
            ui.add_space(12.0);
            if let Some(action) = self.render_actions_row(ui, dark_mode) {
                outcome = DetailOutcome::Forward(Box::new(action));
            }
            ui.add_space(12.0);
            if let Some(action) = self.render_keys_section(ui, dark_mode) {
                outcome = DetailOutcome::Forward(Box::new(action));
            }
            ui.add_space(12.0);
            self.render_dpns_section(ui, dark_mode);
            ui.add_space(12.0);
            if self.render_remove_section(ui, dark_mode) {
                outcome = DetailOutcome::Removed;
            }
        });

        outcome
    }

    fn render_header(&self, ui: &mut Ui, dark_mode: bool) {
        // Conditional alias line — omitted entirely when unset (TC-FR5-02).
        if let Some(alias) = self
            .identity
            .alias
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            ui.label(
                RichText::new(alias)
                    .size(20.0)
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );
        }
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(&self.node_id_short)
                    .monospace()
                    .color(DashColors::text_secondary(dark_mode)),
            );
            // Copy the FULL ProTxHash, not the shortened display string (TC-FR5-03).
            if ui.button("⧉").on_hover_text("Copy ProTxHash").clicked() {
                ui.ctx().copy_text(self.node_id_hex_full.clone());
            }
            draw_type_badge(ui, self.badge_label(), dark_mode);
        });
        // Status dot + label (never colour-only — TC-FR5-05 / NFR-6).
        ui.horizontal(|ui| {
            let (rect, _) =
                ui.allocate_exact_size(egui::Vec2::new(10.0, 10.0), egui::Sense::hover());
            ui.painter()
                .circle_filled(rect.center(), 4.0, Color32::from(self.identity.status));
            ui.add_space(4.0);
            ui.label(
                RichText::new(self.identity.status.to_string())
                    .color(DashColors::text_primary(dark_mode)),
            );
        });
    }

    fn render_actions_row(&self, ui: &mut Ui, dark_mode: bool) -> Option<AppAction> {
        let mut action = None;
        ui.label(
            RichText::new("Actions")
                .strong()
                .color(DashColors::text_primary(dark_mode)),
        );
        ui.horizontal_wrapped(|ui| {
            // All three credit screens are reused, scoped to THIS node (FR-9).
            if ui.button("Withdraw").clicked() {
                action = Some(self.push(ScreenType::WithdrawalScreen(self.identity.clone())));
            }
            if ui.button("Top up").clicked() {
                action = Some(self.push(ScreenType::TopUpIdentity(self.identity.clone())));
            }
            if ui.button("Transfer").clicked() {
                action = Some(self.push(ScreenType::TransferScreen(self.identity.clone())));
            }
            // Evonode-only token-rewards cross-link (FR-11); absent for a plain
            // masternode (TC-FR11-02).
            if self.is_evonode()
                && ui
                    .button("Claim token rewards ›")
                    .on_hover_text("Claim this evonode's token rewards.")
                    .clicked()
            {
                // TODO(B8): scope the existing `ClaimTokensScreen` to the
                // evonode's reward token once that token context is resolved.
                // Routing to the Tokens area preserves reuse without fabricating
                // token info here.
                action = Some(AppAction::SetMainScreen(
                    crate::ui::RootScreenType::RootScreenMyTokenBalances,
                ));
            }
        });
        action
    }

    fn render_keys_section(&mut self, ui: &mut Ui, dark_mode: bool) -> Option<AppAction> {
        let mut action = None;
        ui.label(
            RichText::new("Keys")
                .strong()
                .color(DashColors::text_primary(dark_mode)),
        );

        // Compact V/O/P presence (glyph, not colour-only — NFR-6).
        ui.horizontal(|ui| {
            ui.label(RichText::new("Roles:").color(DashColors::text_secondary(dark_mode)));
            for (letter, present) in [
                ("V", self.key_presence.voting),
                ("O", self.key_presence.owner),
                ("P", self.key_presence.payout),
            ] {
                let text = if present {
                    RichText::new(letter)
                        .strong()
                        .color(DashColors::text_primary(dark_mode))
                } else {
                    RichText::new("·").color(DashColors::text_secondary(dark_mode))
                };
                ui.label(text);
            }
        });

        // Copyable voter-identity id, when a voter identity is loaded.
        if let Some((voter, _)) = self.identity.associated_voter_identity.as_ref() {
            let voter_full = voter.id().to_string(Encoding::Base58);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("Voter identity: {}", shorten_id(&voter_full)))
                        .color(DashColors::text_secondary(dark_mode)),
                );
                if ui
                    .button("⧉")
                    .on_hover_text("Copy voter identity ID")
                    .clicked()
                {
                    ui.ctx().copy_text(voter_full.clone());
                }
            });
        }

        // Protection tier + conditional Add-protection (FR-8 / NFR-4).
        let tier = self.protection_tier();
        ui.label(RichText::new(tier.label()).color(DashColors::text_secondary(dark_mode)));
        ui.horizontal(|ui| {
            if tier.offers_add_protection() && ui.button("Add password protection…").clicked() {
                // The seal flow (password entry → `IdentityTask::ProtectIdentityKeys`)
                // lives in the reused key screens; route there rather than
                // duplicating the password form on this page.
                action = Some(self.push(ScreenType::Keys(self.identity.identity.clone())));
            }
            if ui.button("Manage keys ›").clicked() {
                action = Some(self.push(ScreenType::Keys(self.identity.identity.clone())));
            }
        });
        action
    }

    fn render_dpns_section(&mut self, ui: &mut Ui, dark_mode: bool) {
        // Collapsed by default; open-contest count in the header. The voting
        // table + Cast votes land in B5b.
        let header = format!(
            "DPNS voting ({} open)",
            self.contest_summary.open_contest_count
        );
        egui::CollapsingHeader::new(header)
            .default_open(self.dpns_open)
            .show(ui, |ui| {
                ui.label(
                    RichText::new(
                        "There are no open name contests for this node to vote on right now.",
                    )
                    .color(DashColors::text_secondary(dark_mode)),
                );
                // TODO(B5b): inline per-contest voting table + Cast votes, and
                // the missing-voter-identity "Add voting key" scoped prompt.
            });
    }

    /// Returns `true` once the node has been removed.
    fn render_remove_section(&mut self, ui: &mut Ui, _dark_mode: bool) -> bool {
        if ui.button("Remove masternode").clicked() {
            self.remove_dialog = Some(
                ConfirmationDialog::new(
                    "Remove masternode",
                    "This removes the node and its voting identity from this device. \
                     You can load it again later with its ProTxHash.",
                )
                .danger_mode(true),
            );
        }

        let mut removed = false;
        if let Some(dialog) = self.remove_dialog.as_mut() {
            use crate::ui::components::component_trait::Component;
            let response = dialog.show(ui);
            if let Some(status) = response.inner.dialog_response {
                self.remove_dialog = None;
                if status == ConfirmationStatus::Confirmed {
                    removed = self.remove_node();
                }
            }
        }
        removed
    }

    /// Delete the node and its associated voter identity from local storage.
    fn remove_node(&self) -> bool {
        let node_id = self.identity.identity.id();
        if let Err(e) = self.app_context.delete_local_qualified_identity(&node_id) {
            tracing::warn!("Failed to remove masternode identity: {e}");
            return false;
        }
        if let Some((voter, _)) = self.identity.associated_voter_identity.as_ref()
            && let Err(e) = self
                .app_context
                .delete_local_qualified_identity(&voter.id())
        {
            tracing::warn!("Failed to remove voter identity: {e}");
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tc_fr5_01_actions_render_before_keys() {
        let actions = SECTION_ORDER.iter().position(|s| *s == "Actions").unwrap();
        let keys = SECTION_ORDER.iter().position(|s| *s == "Keys").unwrap();
        assert!(
            actions < keys,
            "Actions must render before Keys (TC-FR5-01)"
        );
    }

    #[test]
    fn section_order_is_header_actions_keys_dpns_remove() {
        assert_eq!(
            SECTION_ORDER,
            ["Header", "Actions", "Keys", "DPNS", "Remove"]
        );
    }

    #[test]
    fn protection_tier_label_and_add_gate() {
        assert_eq!(ProtectionTier::Unprotected.label(), "Keys: unprotected");
        assert_eq!(
            ProtectionTier::Protected.label(),
            "Keys: password-protected"
        );
        assert_eq!(ProtectionTier::NoVaultKeys.label(), "Keys: unprotected");
        assert!(ProtectionTier::Unprotected.offers_add_protection());
        assert!(!ProtectionTier::Protected.offers_add_protection());
        assert!(!ProtectionTier::NoVaultKeys.offers_add_protection());
    }
}
