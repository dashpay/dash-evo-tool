//! Masternode/evonode detail view (FR-5).
//!
//! Section order (header, actions, keys, DPNS voting, remove) is fixed by
//! design; each action pushes an existing screen — no parallel MN-specific
//! reimplementation (NFR-1). See `docs/ai-design/2026-07-09-masternode-page-design/`.

use std::sync::Arc;

use chrono::{LocalResult, TimeZone, Utc};
use chrono_humanize::HumanTime;
use dash_sdk::dpp::identity::TimestampMillis;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use eframe::egui::{self, Color32, RichText, Ui};

use std::collections::BTreeMap;

use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;

use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::contested_names::ContestedResourceTask;
use crate::backend_task::identity::{IdentityInputToLoad, IdentityLoadMode, IdentityTask};
use crate::context::AppContext;
use crate::model::contested_name::{ContestedName, MasternodeContestSummary};
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_identity::{
    IdentityType, MasternodeKeyPresence, PrivateKeyTarget, QualifiedIdentity,
};
use crate::model::secret::Secret;
use crate::ui::components::MessageBanner;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::password_input::PasswordInput;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::identity::identity_picker_card::draw_type_badge;
use crate::ui::identity::identity_pill::shorten_id;
use crate::ui::masternodes::card::{
    PLATFORM_IDENTITY_STATUS_TOOLTIP, platform_identity_status_label,
};
use crate::ui::masternodes::{TIP_OWNER_KEY, TIP_PAYOUT_KEY, TIP_VOTING_KEY, key_status_tokens};
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};
use crate::ui::tokens::claim_tokens_screen::ClaimTokensScreen;
use crate::ui::tokens::tokens_screen::IdentityTokenBasicInfo;
use crate::ui::{MessageType, Screen, ScreenType};
use crate::wallet_backend::IdentityKeyView;
use crate::wallet_backend::secret_seam::SecretScheme;

/// §7 copy: shown when the node has no voting key loaded.
const MISSING_VOTER_MESSAGE: &str =
    "This node has no voting key loaded. Add its voting private key to cast votes.";
/// §7 copy: shown when the node has a voter identity but no open contests.
const NO_OPEN_CONTESTS_MESSAGE: &str =
    "There are no open name contests for this node to vote on right now.";

/// The collapsible DPNS section header, with the open-contest count (TC-DPNS-02).
fn dpns_section_header(open_contest_count: usize) -> String {
    format!("DPNS name contests to vote on ({open_contest_count})")
}

/// Framing shown once above the per-contest vote controls, so a masternode
/// owner unfamiliar with DPNS contested voting understands what is being
/// decided.
const CONTEST_INTRO_MESSAGE: &str = "Several identities want the same name. Cast this node's vote to help decide who receives it, or to lock the name so no one gets it.";
/// Nudge shown under a contest that still has no vote picked, so the user knows
/// why the Cast votes button stays disabled.
const NO_SELECTION_HINT: &str =
    "No vote picked yet. Choose Abstain, Lock, or a candidate above to set this node's vote.";
/// Tooltip on an enabled Cast votes button.
const CAST_ENABLED_HINT: &str = "Submit this node's vote for every name you picked.";
/// Tooltip on a disabled Cast votes button, explaining what unlocks it.
const CAST_DISABLED_HINT: &str =
    "Pick Abstain, Lock, or a candidate for at least one name to enable this.";

/// The full DPNS domain a contest is fighting over: DPNS names register under
/// `.dash`, so append it to the normalized label (shown bare elsewhere) to make
/// clear this is a real domain registration.
fn contest_display_name(normalized_name: &str) -> String {
    format!("{normalized_name}.dash")
}

/// A candidate choice label carrying the candidate's current vote tally, so the
/// voter sees the standing before picking. Phrased to avoid singular/plural
/// verb agreement for later translation.
fn candidate_choice_label(candidate_name: &str, votes: u32) -> String {
    format!("Vote for {candidate_name} (votes so far: {votes})")
}

/// Render data for one open contest, snapshotted before the choice-writing
/// loop so it does not borrow `open_contests` while `vote_selections` mutates.
struct ContestVoteRow {
    name: String,
    end_time: Option<TimestampMillis>,
    /// `(candidate id, candidate name, votes so far)` for each contestant.
    candidates: Vec<(dash_sdk::platform::Identifier, String, u32)>,
}

/// A one-line status for a contest: how many identities are competing and when
/// voting closes. Keeps the deadline absolute (ISO) plus a relative hint, and
/// degrades cleanly when the end time has not loaded yet.
fn contest_status_line(candidate_count: usize, end_time: Option<TimestampMillis>) -> String {
    let count = format!("Identities competing for this name: {candidate_count}.");
    match end_time {
        Some(end_time) => match Utc.timestamp_millis_opt(end_time as i64) {
            LocalResult::Single(dt) => {
                let iso = dt.format("%Y-%m-%d %H:%M:%S");
                let relative = HumanTime::from(dt);
                format!("{count} Voting ends {iso} UTC ({relative}).")
            }
            _ => format!("{count} The voting deadline is unavailable."),
        },
        None => format!("{count} The voting deadline is still loading."),
    }
}

/// The fixed top→bottom section order. Actions must precede Keys (TC-FR5-01).
pub const SECTION_ORDER: [&str; 5] = ["Header", "Actions", "Keys", "DPNS", "Remove"];

/// Tooltip for an authentication key — Platform-only, so it has no DIP-3 role
/// counterpart and lives here rather than in the shared masternode tooltips.
const TIP_AUTH_KEY: &str = "An authentication key signs this identity's actions on Dash Platform.";

/// A short role name for a masternode key and its tooltip, aligned with the Dash
/// Core DIP-3 ProRegTx roles. Voter-identity keys are always the voting key; on
/// the main identity, the Platform Owner and Transfer keys of a masternode
/// identity mirror the ProTx owner key and payout address respectively. Unknown
/// purposes fall back to their name with no tooltip.
fn key_role_label(
    target: &PrivateKeyTarget,
    key: &dash_sdk::platform::IdentityPublicKey,
) -> (String, Option<&'static str>) {
    role_label_and_tip(
        *target == PrivateKeyTarget::PrivateKeyOnVoterIdentity,
        key.purpose(),
    )
}

/// The pure label/tooltip mapping behind [`key_role_label`], split out so it can
/// be unit-tested without constructing an `IdentityPublicKey`.
fn role_label_and_tip(
    is_voter: bool,
    purpose: dash_sdk::dpp::identity::Purpose,
) -> (String, Option<&'static str>) {
    use dash_sdk::dpp::identity::Purpose;
    if is_voter {
        return ("Voting".to_string(), Some(TIP_VOTING_KEY));
    }
    match purpose {
        Purpose::OWNER => ("Owner".to_string(), Some(TIP_OWNER_KEY)),
        Purpose::TRANSFER => ("Payout address".to_string(), Some(TIP_PAYOUT_KEY)),
        Purpose::AUTHENTICATION => ("Authentication".to_string(), Some(TIP_AUTH_KEY)),
        other => (format!("{other:?}"), None),
    }
}

/// Button labels (and DIP-3-aligned tooltips) for the "Manage keys" list, one
/// per entry of `keys`, in order.
///
/// Each label is the key's role word (`Owner`/`Payout address`/`Voting`/…)
/// plus a `(disabled)` marker for keys platform has retired: a node that
/// rotates its payout address keeps the old, disabled Payout key on-chain
/// next to the new active one, so a role word alone is not unique. When two
/// keys would still collide (e.g. two retired Payout keys), the key id
/// disambiguates them, so every button carries a distinct, correct label.
fn manage_keys_labels(
    keys: &[(PrivateKeyTarget, dash_sdk::platform::IdentityPublicKey)],
) -> Vec<(String, Option<&'static str>)> {
    let base: Vec<(String, Option<&'static str>)> = keys
        .iter()
        .map(|(target, key)| {
            let (role, tip) = key_role_label(target, key);
            let mut label = format!("{role} key");
            if key.is_disabled() {
                label.push_str(" (disabled)");
            }
            (label, tip)
        })
        .collect();

    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for (label, _) in &base {
        *counts.entry(label.as_str()).or_default() += 1;
    }

    base.iter()
        .zip(keys.iter())
        .map(|((label, tip), (_, key))| {
            if counts.get(label.as_str()).copied().unwrap_or(0) > 1 {
                (format!("{label} #{}", key.id()), *tip)
            } else {
                (label.clone(), *tip)
            }
        })
        .collect()
}

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
    contest_summary: MasternodeContestSummary,
    /// Open contests this node can still vote on (loaded at construction /
    /// refresh). Active/open only — scheduled/past history lives on the DPNS
    /// Scheduled Votes screen (§10.7).
    open_contests: Vec<ContestedName>,
    /// Per-contest pending vote choice, keyed by normalized contested name.
    vote_selections: BTreeMap<String, ResourceVoteChoice>,
    /// The scoped, in-place "Add voting key" prompt (US-3 / §10.8) — distinct
    /// from FR-4's load form. `Some` while the prompt is open.
    voter_key_prompt: Option<PasswordInput>,
    remove_dialog: Option<ConfirmationDialog>,
}

#[cfg(test)]
impl MasternodeDetailView {
    /// Open the `Add voting key` prompt on a key, as typing into it would.
    pub(crate) fn set_voter_key_prompt_for_test(&mut self, value: &str) {
        let mut prompt = PasswordInput::new();
        prompt.set_text(value);
        self.voter_key_prompt = Some(prompt);
    }

    /// Whether the `Add voting key` prompt is open — and thus holding a key.
    pub(crate) fn has_voter_key_prompt_for_test(&self) -> bool {
        self.voter_key_prompt.is_some()
    }
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
        let open_contests = Self::load_open_contests(app_context, voter_id);
        Self {
            app_context: app_context.clone(),
            identity,
            node_id_hex_full,
            node_id_short,
            key_presence,
            contest_summary,
            open_contests,
            vote_selections: BTreeMap::new(),
            voter_key_prompt: None,
            remove_dialog: None,
        }
    }

    /// Load the contests this node can still vote on. Empty when the node has no
    /// voting key (no voter id) or the read fails.
    fn load_open_contests(
        app_context: &Arc<AppContext>,
        voter_id: Option<dash_sdk::platform::Identifier>,
    ) -> Vec<ContestedName> {
        let Some(voter_id) = voter_id else {
            return Vec::new();
        };
        app_context
            .ongoing_contested_names()
            .unwrap_or_default()
            .into_iter()
            .filter(|contest| contest.is_open_for_voter(&voter_id))
            .collect()
    }

    /// Refresh the DPNS contest summary + open-contest list from the store.
    fn refresh_contests(&mut self) {
        let voter_id = self
            .identity
            .associated_voter_identity
            .as_ref()
            .map(|(voter, _)| voter.id());
        self.contest_summary = self
            .app_context
            .masternode_contest_summary(voter_id)
            .unwrap_or_default();
        self.open_contests = Self::load_open_contests(&self.app_context, voter_id);
    }

    /// Build the network re-fetch dispatched by the detail Refresh button:
    /// refresh this node's identity, plus a DPNS contests re-query
    /// when the node has a voter identity that can vote.
    fn refresh_from_network(&self) -> AppAction {
        let mut tasks = vec![BackendTask::IdentityTask(IdentityTask::RefreshIdentity(
            self.identity.clone(),
        ))];
        if self.identity.associated_voter_identity.is_some() {
            tasks.push(BackendTask::ContestedResourceTask(
                ContestedResourceTask::QueryDPNSContests,
            ));
        }
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
                    // Re-read the local contest cache immediately (optimistic)
                    // AND dispatch a network re-fetch of this node plus the DPNS
                    // contests — Refresh must reach the network.
                    self.refresh_contests();
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
                    RichText::new(format!("Voter identity: {}", shorten_id(&voter_full)))
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

        // Per-key "Manage keys" list. Each key opens its own `KeyInfoScreen` —
        // the real, interactive per-key screen with view/sign/seal actions —
        // not the static read-only `KeysScreen` table. This mirrors
        // `identities_screen.rs`: one button per key, each pushing
        // `Screen::KeyInfoScreen`.
        ui.add_space(4.0);
        ui.label(
            RichText::new("Manage keys")
                .strong()
                .color(DashColors::text_primary(dark_mode)),
        );
        let keys = self.identity_keys();
        let labels = manage_keys_labels(&keys);
        for ((target, key), (label, tip)) in keys.into_iter().zip(labels) {
            let button = ui.button(format!("{label} ›"));
            let button = match tip {
                Some(tip) => button.clickable_tooltip(tip),
                None => button,
            };
            if button.clicked() {
                action = Some(self.open_key_info(target, &key));
            }
        }

        // Add-protection CTA (FR-8): the seal form (password entry →
        // `IdentityTask::ProtectIdentityKeys`, which seals the whole identity)
        // lives inside `KeyInfoScreen`. Open the first held key so the user
        // lands directly on the interactive seal flow.
        if tier.offers_add_protection()
            && let Some((target, key)) = self.first_protectable_key()
            && ui.button("Add password protection…").clicked()
        {
            action = Some(self.open_key_info_with_protection_prompt(target, &key));
        }
        action
    }

    /// Every key of this node, main-identity keys first then voter-identity
    /// keys, each paired with the `PrivateKeyTarget` that scopes it. Backs the
    /// per-key "Manage keys" list and the Add-protection routing.
    fn identity_keys(&self) -> Vec<(PrivateKeyTarget, dash_sdk::platform::IdentityPublicKey)> {
        let mut keys = Vec::new();
        for key in self.identity.identity.public_keys().values() {
            keys.push((PrivateKeyTarget::PrivateKeyOnMainIdentity, key.clone()));
        }
        if let Some((voter, _)) = self.identity.associated_voter_identity.as_ref() {
            for key in voter.public_keys().values() {
                keys.push((PrivateKeyTarget::PrivateKeyOnVoterIdentity, key.clone()));
            }
        }
        keys
    }

    /// The first key whose private material this node actually holds — the only
    /// keys that can be sealed. Used to route the Add-protection CTA straight
    /// into an interactive `KeyInfoScreen` seal flow.
    fn first_protectable_key(
        &self,
    ) -> Option<(PrivateKeyTarget, dash_sdk::platform::IdentityPublicKey)> {
        self.identity_keys().into_iter().find(|(target, key)| {
            self.identity
                .private_keys
                .get_cloned_private_key_data_and_wallet_info(&(target.clone(), key.id()))
                .is_some()
        })
    }

    /// Build the `AddScreen` action that opens `KeyInfoScreen` for one key,
    /// carrying its held private-key data if any. Mirrors the
    /// per-key push in `identities_screen.rs`.
    fn open_key_info(
        &self,
        target: PrivateKeyTarget,
        key: &dash_sdk::platform::IdentityPublicKey,
    ) -> AppAction {
        self.open_key_info_with_mode(target, key, KeyInfoOpenMode::Normal)
    }

    /// Open `KeyInfoScreen` directly in the add-protection confirmation flow.
    fn open_key_info_with_protection_prompt(
        &self,
        target: PrivateKeyTarget,
        key: &dash_sdk::platform::IdentityPublicKey,
    ) -> AppAction {
        self.open_key_info_with_mode(target, key, KeyInfoOpenMode::WithProtectionPrompt)
    }

    fn open_key_info_with_mode(
        &self,
        target: PrivateKeyTarget,
        key: &dash_sdk::platform::IdentityPublicKey,
        mode: KeyInfoOpenMode,
    ) -> AppAction {
        let holding = self
            .identity
            .private_keys
            .get_cloned_private_key_data_and_wallet_info(&(target, key.id()));
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
        AppAction::AddScreen(Screen::KeyInfoScreen(screen))
    }

    /// Render the collapsible DPNS voting section (collapsed by default,
    /// open-contest count in the header). Inline voting reuses the existing
    /// `vote_on_dpns_name` backend (locked decision #1 — not a deep-link).
    fn render_dpns_section(&mut self, ui: &mut Ui, dark_mode: bool) -> Option<AppAction> {
        // When the node has no voter key, the "Add voting key" CTA is
        // the primary next step — render it above, outside the collapsed-by-
        // default DPNS section, so it is visible without expanding anything.
        // The empty DPNS section (no contests possible without a voter) is
        // omitted in that state.
        if self.identity.associated_voter_identity.is_none() {
            return self.render_missing_voter(ui, dark_mode);
        }

        let mut action = None;
        let header = dpns_section_header(self.contest_summary.open_contest_count);
        egui::CollapsingHeader::new(header)
            .default_open(false)
            .show(ui, |ui| {
                if self.open_contests.is_empty() {
                    ui.label(
                        RichText::new(NO_OPEN_CONTESTS_MESSAGE)
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                } else {
                    action = self.render_vote_table(ui, dark_mode);
                }
            });
        action
    }

    /// Missing-voter-identity state (US-3 / §10.9): an actionable message plus a
    /// scoped in-place `Add voting key` prompt — never the raw error, never
    /// FR-4's load form.
    fn render_missing_voter(&mut self, ui: &mut Ui, dark_mode: bool) -> Option<AppAction> {
        let mut action = None;
        ui.label(RichText::new(MISSING_VOTER_MESSAGE).color(DashColors::warning_color(dark_mode)));

        match self.voter_key_prompt.as_mut() {
            None => {
                if ui.button("Add voting key").clicked() {
                    // Node context is already bound (`self.identity`) — the
                    // prompt only asks for the voting key, no ProTxHash re-entry.
                    self.voter_key_prompt = Some(
                        PasswordInput::new()
                            .with_hint_text("Voting private key (WIF or hex)")
                            .with_monospace(),
                    );
                }
            }
            Some(prompt) => {
                prompt.show(ui);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.voter_key_prompt = None;
                    }
                    let has_key = !self
                        .voter_key_prompt
                        .as_ref()
                        .map(PasswordInput::is_empty)
                        .unwrap_or(true);
                    if ui.add_enabled(has_key, egui::Button::new("Save")).clicked() {
                        action = self.submit_voter_key();
                    }
                });
            }
        }
        action
    }

    /// Close the `Add voting key` prompt, zeroizing the key typed into it. Called
    /// when the Masternodes tab is left: the tab is a root screen that outlives
    /// navigation, and an unsubmitted key must not.
    pub fn clear_secrets(&mut self) {
        self.voter_key_prompt = None;
    }

    /// Build the scoped voter-key update: re-load THIS node (context pre-bound)
    /// with just the entered voting key, updating its voter identity in place.
    /// Distinct from FR-4's load form and exempt from duplicate-ProTxHash
    /// rejection (§10.8).
    fn submit_voter_key(&mut self) -> Option<AppAction> {
        let voting_key = self.voter_key_prompt.as_mut()?.take_secret();
        self.voter_key_prompt = None;
        let input = IdentityInputToLoad {
            identity_id_input: self.node_id_hex_full.clone(),
            identity_type: self.identity.identity_type,
            alias_input: self.identity.alias.clone().unwrap_or_default(),
            voting_private_key_input: voting_key,
            owner_private_key_input: Secret::default(),
            payout_address_private_key_input: Secret::default(),
            keys_input: vec![],
            derive_keys_from_wallets: false,
            selected_wallet_seed_hash: None,
            encryption_password: None,
            // In-place update: merge the new voting key into the already-loaded
            // node, preserving its Owner/Payout keys (§10.8). Never overwrite.
            load_mode: IdentityLoadMode::MergeIntoExisting,
            // This view gates on nothing: the load opens a record of its own rather
            // than adopting one another caller is waiting on.
            load_token: None,
        };
        Some(AppAction::BackendTask(BackendTask::IdentityTask(
            IdentityTask::LoadIdentity(input),
        )))
    }

    /// Per-contest voting choices + Cast votes, dispatching the existing
    /// `VoteOnDPNSNames` backend for the selected choices.
    fn render_vote_table(&mut self, ui: &mut Ui, dark_mode: bool) -> Option<AppAction> {
        let mut action = None;
        // Collect the render data up front so the choice-writing loop does not
        // borrow `self.open_contests` while mutating `self.vote_selections`.
        let contests: Vec<ContestVoteRow> = self
            .open_contests
            .iter()
            .map(|contest| {
                let candidates = contest
                    .contestants
                    .as_ref()
                    .map(|list| {
                        list.iter()
                            .map(|c| (c.id, c.name.clone(), c.votes))
                            .collect()
                    })
                    .unwrap_or_default();
                ContestVoteRow {
                    name: contest.normalized_contested_name.clone(),
                    end_time: contest.end_time,
                    candidates,
                }
            })
            .collect();

        ui.label(RichText::new(CONTEST_INTRO_MESSAGE).color(DashColors::text_secondary(dark_mode)));

        for contest in &contests {
            ui.separator();
            ui.label(
                RichText::new(contest_display_name(&contest.name))
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );
            ui.label(
                RichText::new(contest_status_line(
                    contest.candidates.len(),
                    contest.end_time,
                ))
                .color(DashColors::text_secondary(dark_mode)),
            );
            let selected = self.vote_selections.get(&contest.name).copied();
            ui.horizontal_wrapped(|ui| {
                if ui
                    .selectable_label(selected == Some(ResourceVoteChoice::Abstain), "Abstain")
                    .clicked()
                {
                    self.vote_selections
                        .insert(contest.name.clone(), ResourceVoteChoice::Abstain);
                }
                if ui
                    .selectable_label(selected == Some(ResourceVoteChoice::Lock), "Lock")
                    .clicked()
                {
                    self.vote_selections
                        .insert(contest.name.clone(), ResourceVoteChoice::Lock);
                }
                // Candidate choices are scoped to THIS contest's contestants.
                for (candidate_id, candidate_name, votes) in &contest.candidates {
                    let choice = ResourceVoteChoice::TowardsIdentity(*candidate_id);
                    if ui
                        .selectable_label(
                            selected == Some(choice),
                            candidate_choice_label(candidate_name, *votes),
                        )
                        .clicked()
                    {
                        self.vote_selections.insert(contest.name.clone(), choice);
                    }
                }
            });
            if selected.is_none() {
                ui.label(
                    RichText::new(NO_SELECTION_HINT).color(DashColors::text_secondary(dark_mode)),
                );
            }
        }

        ui.separator();
        let votes: Vec<(String, ResourceVoteChoice)> = self
            .vote_selections
            .iter()
            .filter(|(name, _)| contests.iter().any(|c| &c.name == *name))
            .map(|(name, choice)| (name.clone(), *choice))
            .collect();
        let has_votes = !votes.is_empty();
        if ui
            .add_enabled(has_votes, egui::Button::new("Cast votes"))
            .on_hover_text(CAST_ENABLED_HINT)
            .on_disabled_hover_text(CAST_DISABLED_HINT)
            .clicked()
        {
            action = Some(AppAction::BackendTask(BackendTask::ContestedResourceTask(
                ContestedResourceTask::VoteOnDPNSNames(votes, vec![self.identity.clone()]),
            )));
        }
        action
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
            use crate::ui::components::component_trait::Component;
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

    /// Build a masternode key with a chosen id / purpose / disabled state.
    fn mn_key(
        id: dash_sdk::dpp::identity::KeyID,
        purpose: dash_sdk::dpp::identity::Purpose,
        disabled: bool,
    ) -> dash_sdk::platform::IdentityPublicKey {
        use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
        use dash_sdk::dpp::identity::{KeyType, SecurityLevel};
        use dash_sdk::dpp::platform_value::BinaryData;
        IdentityPublicKeyV0 {
            id,
            key_type: KeyType::ECDSA_HASH160,
            purpose,
            security_level: SecurityLevel::CRITICAL,
            read_only: true,
            data: BinaryData::new(vec![id as u8; 20]),
            disabled_at: disabled.then_some(1),
            contract_bounds: None,
        }
        .into()
    }

    /// An evonode that has rotated its payout address holds two `TRANSFER`
    /// (Payout) keys on its main identity — the active new one and the disabled
    /// old one — plus the owner key and a voter-identity voting key. Every
    /// "Manage keys" button must get a distinct, correct label: the disabled
    /// payout key is marked `(disabled)` instead of colliding with the active
    /// one under a bare "Payout key".
    #[test]
    fn manage_keys_labels_disambiguate_rotated_evonode_payout_keys() {
        use dash_sdk::dpp::identity::Purpose;
        let keys = vec![
            (
                PrivateKeyTarget::PrivateKeyOnMainIdentity,
                mn_key(0, Purpose::TRANSFER, false),
            ),
            (
                PrivateKeyTarget::PrivateKeyOnMainIdentity,
                mn_key(1, Purpose::OWNER, false),
            ),
            (
                PrivateKeyTarget::PrivateKeyOnMainIdentity,
                mn_key(2, Purpose::TRANSFER, true),
            ),
            (
                PrivateKeyTarget::PrivateKeyOnVoterIdentity,
                mn_key(0, Purpose::VOTING, false),
            ),
        ];

        let labels: Vec<String> = manage_keys_labels(&keys)
            .into_iter()
            .map(|(label, _tip)| label)
            .collect();
        assert_eq!(
            labels,
            vec![
                "Payout address key".to_string(),
                "Owner key".to_string(),
                "Payout address key (disabled)".to_string(),
                "Voting key".to_string(),
            ]
        );
        // No two buttons ever share a label.
        let unique: std::collections::BTreeSet<_> = labels.iter().collect();
        assert_eq!(unique.len(), labels.len(), "labels must be unique");
    }

    /// When even the role + `(disabled)` marker still collides — a payout
    /// address rotated twice leaves two disabled Payout keys — the key id
    /// breaks the tie so every button stays unique.
    #[test]
    fn manage_keys_labels_fall_back_to_key_id_on_residual_collision() {
        use dash_sdk::dpp::identity::Purpose;
        let keys = vec![
            (
                PrivateKeyTarget::PrivateKeyOnMainIdentity,
                mn_key(0, Purpose::TRANSFER, false),
            ),
            (
                PrivateKeyTarget::PrivateKeyOnMainIdentity,
                mn_key(2, Purpose::TRANSFER, true),
            ),
            (
                PrivateKeyTarget::PrivateKeyOnMainIdentity,
                mn_key(3, Purpose::TRANSFER, true),
            ),
        ];

        let labels: Vec<String> = manage_keys_labels(&keys)
            .into_iter()
            .map(|(label, _tip)| label)
            .collect();
        assert_eq!(
            labels,
            vec![
                "Payout address key".to_string(),
                "Payout address key (disabled) #2".to_string(),
                "Payout address key (disabled) #3".to_string(),
            ]
        );
        let unique: std::collections::BTreeSet<_> = labels.iter().collect();
        assert_eq!(unique.len(), labels.len(), "labels must be unique");
    }

    #[test]
    fn role_labels_follow_dip3_protx_terms() {
        use dash_sdk::dpp::identity::Purpose;
        // A voter-identity key is always the voting key, regardless of purpose.
        assert_eq!(
            role_label_and_tip(true, Purpose::AUTHENTICATION),
            ("Voting".to_string(), Some(TIP_VOTING_KEY))
        );
        // Main-identity roles mirror the DIP-3 ProRegTx owner key and payout
        // address; the Platform Transfer key surfaces as "Payout address".
        assert_eq!(
            role_label_and_tip(false, Purpose::OWNER),
            ("Owner".to_string(), Some(TIP_OWNER_KEY))
        );
        assert_eq!(
            role_label_and_tip(false, Purpose::TRANSFER),
            ("Payout address".to_string(), Some(TIP_PAYOUT_KEY))
        );
        assert_eq!(
            role_label_and_tip(false, Purpose::AUTHENTICATION),
            ("Authentication".to_string(), Some(TIP_AUTH_KEY))
        );
        // An unmapped purpose keeps its name and carries no tooltip.
        assert_eq!(
            role_label_and_tip(false, Purpose::ENCRYPTION),
            (format!("{:?}", Purpose::ENCRYPTION), None)
        );
    }

    #[test]
    fn tc_dpns_02_header_shows_open_contest_count() {
        assert_eq!(dpns_section_header(3), "DPNS name contests to vote on (3)");
        assert_eq!(dpns_section_header(0), "DPNS name contests to vote on (0)");
    }

    #[test]
    fn contest_name_gets_dash_suffix() {
        // The normalized label is shown bare elsewhere; the vote section spells
        // out the full `.dash` domain so the user knows it is a registration.
        assert_eq!(contest_display_name("det"), "det.dash");
    }

    #[test]
    fn candidate_label_carries_current_tally() {
        let label = candidate_choice_label("alice", 5);
        assert!(
            label.contains("Vote for alice"),
            "names the candidate: {label}"
        );
        assert!(label.contains('5'), "shows the running tally: {label}");
    }

    #[test]
    fn status_line_reports_candidate_count() {
        let line = contest_status_line(2, None);
        assert!(
            line.contains("Identities competing for this name: 2."),
            "counts contestants: {line}"
        );
        assert!(
            line.contains("still loading"),
            "degrades when the deadline is absent: {line}"
        );
    }

    #[test]
    fn status_line_renders_absolute_deadline() {
        // 2021-01-01T00:00:00Z in milliseconds.
        let line = contest_status_line(3, Some(1_609_459_200_000));
        assert!(
            line.contains("Identities competing for this name: 3."),
            "counts contestants: {line}"
        );
        assert!(
            line.contains("2021-01-01 00:00:00 UTC"),
            "shows the absolute ISO deadline: {line}"
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
        ks.private_keys.insert(
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
