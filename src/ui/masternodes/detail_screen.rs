//! Masternode/evonode detail view (FR-5).
//!
//! Section order (header, actions, keys, DPNS voting, remove) is fixed by
//! design; each action pushes an existing screen — no parallel MN-specific
//! reimplementation (NFR-1). See `docs/ai-design/2026-07-09-masternode-page-design/`.

use std::sync::Arc;

use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
#[cfg(test)]
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use eframe::egui::{self, Color32, RichText, Ui};

#[cfg(test)]
use std::collections::BTreeMap;

use crate::app::AppAction;
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::legacy_recovery::RecoveryItem;
use crate::model::qualified_identity::{IdentityType, MasternodeKeyPresence, QualifiedIdentity};
use crate::ui::components::MessageBanner;
use crate::ui::components::component_trait::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::legacy_recovery_section::host_offer;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::identity::identity_picker_card::draw_type_badge;
use crate::ui::identity::identity_pill::shorten_id;
use crate::ui::masternodes::card::{
    PLATFORM_IDENTITY_STATUS_TOOLTIP, platform_identity_status_label,
};
use crate::ui::masternodes::{KeyVocabulary, identity_keys, key_status_tokens, manage_keys_labels};
use crate::ui::state::legacy_recovery::LegacyRecoveryState;
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};
use crate::ui::tokens::claim_tokens_screen::ClaimTokensScreen;
use crate::ui::tokens::tokens_screen::IdentityTokenBasicInfo;
use crate::ui::{MessageType, Screen, ScreenType};
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

enum KeyInfoOpenMode {
    Normal,
    WithProtectionPrompt,
}

/// Masternode/evonode detail view state.
pub struct MasternodeDetailView {
    app_context: Arc<AppContext>,
    identity: QualifiedIdentity,
    node_id_hex_full: String,
    node_id_short: String,
    key_presence: MasternodeKeyPresence,
    remove_dialog: Option<ConfirmationDialog>,
    /// The offer to restore keys this node left behind in the previous
    /// version's saved data (issue #889).
    recovery: LegacyRecoveryState,
}

#[cfg(test)]
impl MasternodeDetailView {
    /// Whether a recovery offer is currently on screen for this node.
    pub(crate) fn has_recovery_offer_for_test(&self) -> bool {
        self.recovery.has_offer()
    }

    /// Put a detected plan on offer, as the check's own result does — without
    /// the egui context [`Self::absorb_recovery_result`] needs to report one.
    pub(crate) fn set_recovery_plan(
        &mut self,
        identity_id: dash_sdk::platform::Identifier,
        plan: crate::model::legacy_recovery::RecoveryPlan,
    ) {
        self.recovery.offered(identity_id, plan);
    }

    /// The key roles this view believes the node holds.
    pub(crate) fn key_presence_for_test(&self) -> MasternodeKeyPresence {
        self.key_presence
    }

    /// Dispatch this node's restore, as pressing Restore does, and report
    /// whether it went out.
    pub(crate) fn start_recovery_restore_for_test(&mut self) -> bool {
        self.recovery.restore(vec![]).is_some()
    }

    /// Whether a restore is still in flight, so the Restore button stays
    /// disabled.
    pub(crate) fn is_restoring_for_test(&self) -> bool {
        self.recovery.is_restoring()
    }
}

impl MasternodeDetailView {
    pub fn new(app_context: &Arc<AppContext>, identity: QualifiedIdentity) -> Self {
        let node_id_hex_full = identity.identity.id().to_string(Encoding::Hex);
        let node_id_short = shorten_id(&node_id_hex_full);
        let key_presence = identity.masternode_key_presence();
        let recovery = LegacyRecoveryState::new(app_context, identity.identity.id());
        Self {
            app_context: app_context.clone(),
            identity,
            node_id_hex_full,
            node_id_short,
            key_presence,
            remove_dialog: None,
            recovery,
        }
    }

    /// Route a finished backend task into this node's recovery offer, reporting
    /// whether this node's own restore finished.
    pub(crate) fn absorb_recovery_result(
        &mut self,
        ctx: &egui::Context,
        result: &BackendTaskSuccessResult,
    ) -> bool {
        self.recovery.absorb_result(ctx, result)
    }

    /// Re-read this node from the store and re-arm its recovery check.
    ///
    /// The view holds the identity it was opened with, and its key-presence
    /// line and recovery offer are both derived from it. A restore run from a
    /// pushed Key Info screen never reaches this view — that screen is on top,
    /// so it receives the result — which leaves the node page still offering
    /// keys that are already back, and still warning about a voting key it now
    /// holds. Called on arrival, so returning from a pushed screen recomputes
    /// both. Vote selections and any open prompt survive: they belong to the
    /// user's session, not to the record.
    pub(crate) fn refresh_from_store(&mut self) {
        let node_id = self.identity.identity.id();
        if let Ok(identities) = self.app_context.load_local_masternode_identities()
            && let Some(identity) = identities
                .into_iter()
                .find(|qi| qi.identity.id() == node_id)
        {
            self.key_presence = identity.masternode_key_presence();
            self.identity = identity;
        }
        self.recovery.completed();
    }

    /// End this view's recovery operation when the failure that arrived is that
    /// operation's own — every error reaches whichever screen is visible.
    pub(crate) fn absorb_recovery_error(&mut self, context: &BackendTaskContext) {
        self.recovery.absorb_error(context);
    }

    /// Build the network re-fetch dispatched by the detail Refresh button:
    /// refresh this node's identity, plus a DPNS contests re-query
    /// when the node has a voter identity that can vote.
    fn refresh_from_network(&self) -> AppAction {
        let tasks = vec![BackendTask::IdentityTask(IdentityTask::RefreshIdentity(
            self.identity.clone(),
        ))];
        AppAction::BackendTasks(tasks, crate::app::BackendTasksExecutionMode::Concurrent)
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
        // TODO: a mixed state (some Tier-1, some Tier-2) currently maps to
        // Protected, so the aggregate "Add password protection…" CTA is hidden
        // even though unprotected keys remain. This is mitigated by the per-key
        // Manage-keys list (each unprotected key can still be sealed from its
        // KeyInfoScreen); a dedicated "partially protected" tier could re-offer
        // the aggregate CTA.
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
                    outcome = DetailOutcome::Forward(Box::new(self.refresh_from_network()));
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
            if let Some(action) = self.render_dpns_section(ui, dark_mode) {
                outcome = DetailOutcome::Forward(Box::new(action));
            }
            ui.add_space(12.0);
            if self.render_remove_section(ui, dark_mode) {
                outcome = DetailOutcome::Removed;
            }
        });

        // Passive detection, dispatched once per opened view. It never competes
        // with a click made this frame: the click already owns the outcome, and
        // the check simply goes out on the next frame instead.
        if matches!(outcome, DetailOutcome::None)
            && let Some(task) = self.recovery.ensure_checked()
        {
            outcome = DetailOutcome::Forward(Box::new(AppAction::BackendTask(task)));
        }

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
            // `small_button` is text-height, so it stays vertically centered with
            // the monospace hash and the badge; a full-size button towers over them.
            if ui
                .small_button("Copy")
                .on_hover_text("Copy ProTxHash")
                .clicked()
            {
                ui.ctx().copy_text(self.node_id_hex_full.clone());
            }
            draw_type_badge(ui, self.badge_label(), dark_mode);
        });
        let balance = format_credits_as_dash(self.identity.identity.balance());
        ui.label(
            RichText::new(format!("Balance: {balance}"))
                .monospace()
                .strong()
                .size(14.0)
                .color(DashColors::text_primary(dark_mode)),
        );
        ui.horizontal(|ui| {
            let (rect, _) =
                ui.allocate_exact_size(egui::Vec2::new(10.0, 10.0), egui::Sense::hover());
            ui.painter()
                .circle_filled(rect.center(), 4.0, Color32::from(self.identity.status));
            ui.add_space(4.0);
            ui.label(
                RichText::new(platform_identity_status_label(self.identity.status))
                    .color(DashColors::text_primary(dark_mode)),
            );
        })
        .response
        .on_hover_text(PLATFORM_IDENTITY_STATUS_TOOLTIP);
    }

    fn render_actions_row(&self, ui: &mut Ui, dark_mode: bool) -> Option<AppAction> {
        let mut action = None;
        ui.label(
            RichText::new("Actions")
                .strong()
                .color(DashColors::text_primary(dark_mode)),
        );
        ui.horizontal_wrapped(|ui| {
            // The withdrawal screen is reused, scoped to THIS node (FR-9).
            if ui.button("Withdraw").clicked() {
                action = Some(self.push(ScreenType::WithdrawalScreen(self.identity.clone())));
            }
            // Evonode-only token-rewards cross-link (FR-11); absent for a plain
            // masternode (TC-FR11-02).
            if self.is_evonode()
                && ui
                    .button("Claim token rewards ›")
                    .on_hover_text("Claim this evonode's token rewards.")
                    .clicked()
            {
                action = Some(self.claim_token_rewards_action(ui.ctx()));
            }
        });
        action
    }

    /// Route the evonode "Claim token rewards" CTA (FR-11). When this
    /// evonode holds exactly one token in the local registry, push a
    /// `ClaimTokensScreen` scoped to it (the real claim flow). With zero or
    /// several tokens the correct target is ambiguous, so fall back to the My
    /// Tokens area where the user picks the token to claim.
    fn claim_token_rewards_action(&self, ctx: &egui::Context) -> AppAction {
        let fallback =
            AppAction::SetMainScreen(crate::ui::RootScreenType::RootScreenMyTokenBalances);
        let node_id = self.identity.identity.id();
        let mut mine: Vec<_> = self
            .app_context
            .identity_token_balances()
            .unwrap_or_default()
            .into_iter()
            .filter(|(key, _)| key.identity_id == node_id)
            .map(|(_, balance)| balance)
            .collect();
        if mine.len() != 1 {
            return fallback;
        }
        let itb = mine.remove(0);
        match self.app_context.get_contract_by_token_id(&itb.token_id) {
            Ok(Some(contract)) => {
                let basic = IdentityTokenBasicInfo {
                    token_id: itb.token_id,
                    token_alias: itb.token_alias.clone(),
                    identity_id: itb.identity_id,
                    contract_id: itb.data_contract_id,
                    token_position: itb.token_position,
                };
                AppAction::AddScreen(Screen::ClaimTokensScreen(ClaimTokensScreen::new(
                    basic,
                    contract,
                    itb.token_config,
                    &self.app_context,
                )))
            }
            _ => {
                MessageBanner::set_global(
                    ctx,
                    "This token's details aren't available yet. Open My Tokens to claim.",
                    MessageType::Info,
                );
                fallback
            }
        }
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
            ui.label(RichText::new("Roles:").color(DashColors::text_secondary(dark_mode)))
                .info_tooltip(
                    "Shows which of this node's keys are loaded: V is the voting key, O is the \
                     owner key, and P is the payout address key.",
                );
            // Each letter explains its own role on hover, so a user who hovers `V`
            // is told about the voting key rather than having to read the whole
            // legend on the `Roles:` label.
            for token in key_status_tokens(self.key_presence) {
                let text = if token.present {
                    RichText::new(token.letter)
                        .strong()
                        .color(DashColors::text_primary(dark_mode))
                } else {
                    RichText::new("·").color(DashColors::text_secondary(dark_mode))
                };
                ui.label(text).info_tooltip(token.tooltip);
            }
        });

        // Copyable voter-identity id, when a voter identity is loaded.
        if let Some((voter, _)) = self.identity.associated_voter_identity.as_ref() {
            let voter_full = voter.id().to_string(Encoding::Base58);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!(
                        "Voter identity: {voter}",
                        voter = shorten_id(&voter_full)
                    ))
                    .color(DashColors::text_secondary(dark_mode)),
                );
                // `small_button` keeps the copy affordance text-height and
                // vertically centered with the voter-identity label.
                if ui
                    .small_button("Copy")
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

        // Per-key "Manage keys" list. Each key opens its own `KeyInfoScreen`,
        // the interactive per-key screen with view/sign/seal actions. This
        // mirrors `identities_screen.rs` and the identity keys list: one button
        // per key, each pushing `Screen::KeyInfoScreen` with the target the row
        // found the material at.
        ui.add_space(4.0);
        ui.label(
            RichText::new("Manage keys")
                .strong()
                .color(DashColors::text_primary(dark_mode)),
        );
        let keys = identity_keys(&self.identity);
        // This page only ever shows masternode and evonode identities.
        let labels = manage_keys_labels(KeyVocabulary::from(self.identity.identity_type), &keys);
        for ((_, key), (label, tip)) in keys.into_iter().zip(labels) {
            let button = ui.button(format!("{label} ›"));
            let button = match tip {
                Some(tip) => button.clickable_tooltip(tip),
                None => button,
            };
            if button.clicked() {
                action = Some(self.open_key_info(&key));
            }
        }

        // Add-protection CTA (FR-8): the seal form (password entry →
        // `IdentityTask::ProtectIdentityKeys`, which seals the whole identity)
        // lives inside `KeyInfoScreen`. Open the first held key so the user
        // lands directly on the interactive seal flow.
        if tier.offers_add_protection()
            && let Some(key) = self.first_protectable_key()
            && ui.button("Add password protection…").clicked()
        {
            action = Some(self.open_key_info_with_protection_prompt(&key));
        }

        if let Some(approved) = self.render_recovery_section(ui)
            && let Some(task) = self.recovery.restore(approved)
        {
            action = Some(AppAction::BackendTask(task));
        }
        action
    }

    /// Render the offer at the foot of the keys section, returning the items the
    /// user approved this frame.
    fn render_recovery_section(&self, ui: &mut Ui) -> Option<Vec<RecoveryItem>> {
        if !self.recovery.has_offer() {
            return None;
        }
        ui.add_space(8.0);
        // This page only ever shows masternode and evonode identities.
        host_offer(
            &self.recovery,
            KeyVocabulary::from(self.identity.identity_type),
            ui,
        )
    }

    /// The first key whose private material this node actually holds — the only
    /// keys that can be sealed. Used to route the Add-protection CTA straight
    /// into an interactive `KeyInfoScreen` seal flow.
    ///
    /// Resolves "held" through `candidates()`, the same rule every other
    /// resolution site on this identity uses — a structural `(target, key_id)`
    /// probe would miss material filed under a target other than the one
    /// `identity_keys` structurally pairs the key with (e.g. a main-identity
    /// voting key filed under the voter placement by an older build), and could
    /// match a different key that merely shares the id. `candidates()` only
    /// checks presence, so no raw key bytes are cloned out of the vault here —
    /// unlike `open_key_info_with_mode`, which needs the actual secret and thus
    /// pays for the clone.
    fn first_protectable_key(&self) -> Option<dash_sdk::platform::IdentityPublicKey> {
        identity_keys(&self.identity)
            .into_iter()
            // Presence only: this gates a button, it never acts on the
            // placement, so which of several placements is the liveliest one
            // makes no difference to the answer.
            .find(|(_, key)| self.identity.private_keys.candidates(key).next().is_some())
            .map(|(_, key)| key)
    }

    /// Build the `AddScreen` action that opens `KeyInfoScreen` for one key,
    /// carrying its held private-key data if any. Mirrors the
    /// per-key push in `identities_screen.rs`.
    fn open_key_info(&self, key: &dash_sdk::platform::IdentityPublicKey) -> AppAction {
        self.open_key_info_with_mode(key, KeyInfoOpenMode::Normal)
    }

    /// Open `KeyInfoScreen` directly in the add-protection confirmation flow.
    fn open_key_info_with_protection_prompt(
        &self,
        key: &dash_sdk::platform::IdentityPublicKey,
    ) -> AppAction {
        self.open_key_info_with_mode(key, KeyInfoOpenMode::WithProtectionPrompt)
    }

    fn open_key_info_with_mode(
        &self,
        key: &dash_sdk::platform::IdentityPublicKey,
        mode: KeyInfoOpenMode,
    ) -> AppAction {
        // Where this key's private half actually is, by the one rule every
        // surface uses. A structural target alone would miss material filed under
        // the retired purpose-derived convention — a main-identity voting key
        // entered by hand — and report a key as unheld here while the identity
        // keys list shows it as saved on this device.
        let holding = self.identity.private_keys.held_private_key_data(key);
        let identity = self.identity.clone();
        let key = key.clone();
        let screen = match mode {
            KeyInfoOpenMode::Normal => {
                KeyInfoScreen::new(identity, key, holding, &self.app_context)
            }
            KeyInfoOpenMode::WithProtectionPrompt => {
                KeyInfoScreen::new_with_protection_prompt(identity, key, holding, &self.app_context)
            }
        };
        // No target is handed over: the screen resolves the placement itself, so
        // there is nothing for this caller to get wrong or for the `ScreenType`
        // round trip to drop.
        AppAction::AddScreen(Screen::KeyInfoScreen(screen))
    }

    fn render_dpns_section(&mut self, ui: &mut Ui, _dark_mode: bool) -> Option<AppAction> {
        ComponentStyles::add_secondary_button(ui, "DPNS Voting", ui.visuals().dark_mode)
            .clicked()
            .then(|| {
                AppAction::SetMainScreen(crate::ui::RootScreenType::RootScreenDPNSActiveContests)
            })
    }

    /// Returns `true` once the node has been removed.
    fn render_remove_section(&mut self, ui: &mut Ui, _dark_mode: bool) -> bool {
        let migration_in_progress = self.app_context.migration_status().state().is_in_progress();
        if ui
            .add_enabled(
                !migration_in_progress,
                egui::Button::new("Remove masternode"),
            )
            .on_disabled_hover_text(
                "Wait for the storage update to finish before removing this masternode.",
            )
            .clicked()
        {
            self.remove_dialog = Some(
                ConfirmationDialog::new(
                    "Remove masternode",
                    "This removes the node and its voting identity from this device. \
                     You can load it again later with its ProTxHash.",
                )
                .danger_mode(true)
                // §7 confirm verb (TC-US4-02).
                .confirm_text(Some("Remove masternode")),
            );
        }

        let mut removed = false;
        if let Some(dialog) = self.remove_dialog.as_mut() {
            let response = dialog.show(ui);
            if let Some(status) = response.inner.dialog_response {
                self.remove_dialog = None;
                if status == ConfirmationStatus::Confirmed {
                    removed = self.remove_node(ui.ctx());
                }
            }
        }
        removed
    }

    /// Delete the node and its associated voter identity from local storage.
    /// On the primary delete failing, surface an actionable error banner rather
    /// than failing silently, and keep the detail view open so the user can
    /// retry. The secondary voter-identity delete failing is non-fatal (the node
    /// is already gone) and only logged.
    fn remove_node(&self, ctx: &egui::Context) -> bool {
        let node_id = self.identity.identity.id();
        if let Err(e) = self.app_context.delete_local_qualified_identity(&node_id) {
            MessageBanner::set_global(
                ctx,
                "This masternode couldn't be removed from this device. Try again in a moment.",
                MessageType::Error,
            )
            .with_details(e);
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
    use crate::model::secret::Secret;

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

    /// TC-FR8-07 — a load-time / after-load Tier-2 seal is reflected by
    /// `protection_tier()`: an unsealed keyed node reports `Unprotected`, and
    /// once its keys are sealed under a password it reports `Protected` (so the
    /// detail view shows "Keys: password-protected" and stops offering
    /// Add-protection). Drives the real `IdentityKeyView` scheme path on an
    /// offline wired `AppContext` — no network I/O.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tc_fr8_07_protection_tier_reflects_tier2_seal() {
        use crate::app::TaskResult;
        use crate::app_dir::ensure_env_file;
        use crate::context::connection_status::ConnectionStatus;
        use crate::database::test_helpers::create_database_at_path;
        use crate::model::qualified_identity::IdentityStatus;
        use crate::model::qualified_identity::PrivateKeyTarget;
        use crate::model::qualified_identity::encrypted_key_storage::{KeyStorage, PrivateKeyData};
        use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
        use crate::utils::egui_mpsc::SenderAsync;
        use crate::utils::tasks::TaskManager;
        use dash_sdk::dpp::dashcore::Network;
        use dash_sdk::dpp::identity::Identity;
        use dash_sdk::dpp::version::PlatformVersion;
        use dash_sdk::platform::{Identifier, IdentityPublicKey};

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let data_dir = temp_dir.path().to_path_buf();
        ensure_env_file(&data_dir);
        let db = Arc::new(create_database_at_path(&data_dir.join("data.db")).expect("db"));
        let app_kv = AppContext::open_app_kv(&data_dir).expect("app kv");
        let secret_store = AppContext::open_secret_store(&data_dir).expect("secret store");
        let ctx = AppContext::new(
            data_dir,
            Network::Testnet,
            db,
            Arc::new(TaskManager::new()),
            Arc::new(ConnectionStatus::new()),
            egui::Context::default(),
            app_kv,
            secret_store,
            crate::model::user_role::UserRoleCell::default(),
        )
        .expect("offline testnet AppContext::new");
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");

        // A masternode-shaped identity carrying one owner key on the main
        // identity — enough for `protection_tier` to have a key to inspect.
        let pv = PlatformVersion::latest();
        let owner = IdentityPublicKey::random_key(1, Some(1), pv);
        let mut ks = KeyStorage::default();
        ks.insert_at(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, owner.id()),
            (
                QualifiedIdentityPublicKey::from(owner),
                PrivateKeyData::Clear([0xA0; 32]),
            ),
        );
        let identity =
            Identity::create_basic_identity(Identifier::random(), pv).expect("basic identity");
        let qi = QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::Masternode,
            alias: None,
            private_keys: ks,
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        };
        let identity_id = qi.identity.id();
        ctx.insert_local_qualified_identity(&qi, &None)
            .expect("insert masternode identity");

        // Before sealing: the key is keyless (Tier-1) → Unprotected.
        let view = MasternodeDetailView::new(&ctx, qi.clone());
        assert_eq!(
            view.protection_tier(),
            ProtectionTier::Unprotected,
            "an unsealed keyed node must report Unprotected",
        );
        assert!(
            view.protection_tier().offers_add_protection(),
            "an unsealed node must offer Add-protection",
        );

        // Seal the node's keys Tier-2 via the real backend task (the same task
        // the FR-8 seal flow dispatches).
        ctx.run_backend_task(
            BackendTask::IdentityTask(IdentityTask::ProtectIdentityKeys {
                identity_id,
                password: Secret::new("one-identity-password"),
                hint: None,
            }),
            SenderAsync::new(
                tokio::sync::mpsc::channel::<TaskResult>(4).0,
                ctx.egui_ctx().clone(),
            ),
        )
        .await
        .expect("seal task must succeed");

        // After sealing: the detail view reports Protected and stops offering
        // Add-protection. Rebuild the view to re-read the vault scheme.
        let view = MasternodeDetailView::new(&ctx, qi);
        assert_eq!(
            view.protection_tier(),
            ProtectionTier::Protected,
            "a Tier-2 sealed node must report Protected",
        );
        assert!(
            !view.protection_tier().offers_add_protection(),
            "a sealed node must not re-offer Add-protection",
        );

        ctx.wallet_backend().expect("backend").shutdown().await;
    }
}
