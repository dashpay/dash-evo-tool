//! The identity keys list — every key of one identity, and the way into each.
//!
//! This is the only route into [`KeyInfoScreen`] that does not depend on the
//! identity already holding a usable key. Every other route reaches it from an
//! action (transfer, withdraw, a token operation) gated on holding the key that
//! action needs, which is false for precisely the identities the restore offer
//! below exists to help. Keeping this list ungated is the point of it.

use crate::app::AppAction;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::legacy_recovery::RecoveryItem;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::user_role::UserRole;
use crate::ui::components::MessageBanner;
use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::legacy_recovery_section::{LegacyRecoverySection, completion_message};
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::masternodes::{KeyVocabulary, identity_keys, manage_keys_labels};
use crate::ui::state::legacy_recovery::LegacyRecoveryState;
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};
use crate::ui::{MessageType, RootScreenType, Screen, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::platform::IdentityPublicKey;
use eframe::egui::{self, RichText, ScrollArea};
use std::sync::Arc;

/// The crumb this screen contributes to a pushed Key Info screen's breadcrumb.
const PARENT_CRUMB: &str = "Keys";

/// Shown for a key whose private half this install holds.
const HELD: &str = "This key is saved on this device.";
/// Shown for a key this install has only the public half of. The one fact a
/// user with stranded keys came here to find, so it is stated in words rather
/// than signalled by colour alone.
const NOT_HELD: &str = "This key is not saved on this device.";

pub struct KeysScreen {
    pub identity: QualifiedIdentity,
    pub app_context: Arc<AppContext>,
    /// The offer to restore keys this identity left behind in the previous
    /// version's saved data (issue #889). Scoped to the identity, so it belongs
    /// on the list rather than inside any one key.
    recovery: LegacyRecoveryState,
    /// A queued restore (the approved items), drained in `ui()`.
    pending_recovery_restore: Option<Vec<RecoveryItem>>,
}

impl ScreenLike for KeysScreen {
    /// Re-read the record *and* re-arm detection.
    ///
    /// Both, or the offer goes stale: a restore run from the Key Info screen
    /// pushed on top of this one writes the record behind this screen's back, so
    /// returning to it with only `reload_identity` shows the restored keys as
    /// held while still offering to restore them — and pressing Restore then
    /// reports there was nothing left to do. Re-arming is cheap and self-guards
    /// on an install with nothing to read.
    ///
    /// This is the only arrival hook that runs: the framework dispatches
    /// `refresh_on_arrival` for root screens, and `refresh` for the screen a
    /// `PopScreenAndRefresh` reveals — which is what this screen is.
    fn refresh(&mut self) {
        self.reload_identity();
        self.recovery.completed();
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        match backend_task_success_result {
            BackendTaskSuccessResult::LegacyRecoveryCandidates { identity_id, plan } => {
                self.recovery.offered(identity_id, plan);
            }
            BackendTaskSuccessResult::LegacyRecoveryCompleted {
                identity_id,
                ref applied,
                ..
            } => {
                // Results reach whichever screen is visible when they arrive,
                // so only this identity's own restore has anything to say here
                // or wrote the record the list is rendered from.
                if self.recovery.completed_for(identity_id) {
                    self.reload_identity();
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        completion_message(!applied.is_empty()),
                        MessageType::Success,
                    );
                }
            }
            _ => {}
        }
    }

    /// Return a recovery operation to its offer when the error was its own.
    ///
    /// Never claims the error (always `false`): the user has to see it, and
    /// `AppState`'s generic banner is how it gets to them. This exists for the
    /// typed error, which `display_message` does not receive — a restore that
    /// only ends on *any* error would be re-armed mid-flight by an unrelated
    /// task's failure and could then be dispatched twice.
    fn display_task_error(&mut self, error: &TaskError) -> bool {
        if self.recovery.owns_error(error) {
            self.recovery.failed();
        }
        false
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut action = add_top_panel(
            ui,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Keys", AppAction::None),
            ],
            vec![],
        );

        // The hub is where this screen is opened from, and the only nav entry
        // that leads back to it — `RootScreenIdentities` is no longer in the
        // nav, so selecting it here would highlight nothing.
        action |= add_left_panel(ui, &self.app_context, RootScreenType::RootScreenIdentityHub);

        action |= island_central_panel(ui, |ui| {
            let mut inner_action = AppAction::None;
            let dark_mode = ui.ctx().global_style().visuals.dark_mode;

            ui.horizontal(|ui| {
                if ComponentStyles::add_secondary_button(ui, "Back", dark_mode)
                    .clickable_tooltip("Return to the identity you came from.")
                    .clicked()
                {
                    inner_action |= AppAction::PopScreenAndRefresh;
                }
                ui.heading(
                    RichText::new("Identity Keys").color(DashColors::text_primary(dark_mode)),
                );
            });
            ui.separator();

            ScrollArea::vertical().show(ui, |ui| {
                // Above the list: the offer is about the identity, not about
                // any one key, and a user whose keys are missing must not have
                // to read past the keys they do have to find the remedy.
                self.render_recovery_section(ui);
                inner_action |= self.render_key_list(ui, dark_mode);
            });

            inner_action
        });

        // The passive check goes out once per opened screen, a restore only
        // after the user pressed Restore, so the two never contend for
        // `action`, which keeps only its most recent value.
        if let Some(task) = self.recovery.ensure_checked() {
            action |= AppAction::BackendTask(task);
        }
        if let Some(approved) = self.pending_recovery_restore.take()
            && let Some(task) = self.recovery.restore(approved)
        {
            action |= AppAction::BackendTask(task);
        }

        action
    }
}

impl KeysScreen {
    pub fn new(identity: QualifiedIdentity, app_context: &Arc<AppContext>) -> Self {
        let recovery = LegacyRecoveryState::new(app_context, identity.identity.id());
        Self {
            identity,
            app_context: app_context.clone(),
            recovery,
            pending_recovery_restore: None,
        }
    }

    /// One row per key: what it is for, whether this device holds it, and the
    /// way into its own page. Ungated — a key the device does not hold is
    /// exactly the key a user comes here to do something about.
    fn render_key_list(&self, ui: &mut egui::Ui, dark_mode: bool) -> AppAction {
        let mut action = AppAction::None;
        let keys = identity_keys(&self.identity);

        if keys.is_empty() {
            ui.add_space(8.0);
            // Not "no keys saved on this device": these rows come from the
            // identity's on-chain public keys, and that phrase means held=false
            // per-row on this same screen — a different statement entirely.
            ui.label(
                RichText::new("No keys were found for this identity.")
                    .color(DashColors::text_primary(dark_mode)),
            );
            ui.label(
                RichText::new("Refresh this identity to load its keys from Dash Platform.")
                    .small()
                    .color(DashColors::text_secondary(dark_mode)),
            );
            return action;
        }

        let expert = self.app_context.user_role().at_least(UserRole::Power);
        let vocabulary = KeyVocabulary::from(self.identity.identity_type);
        let labels = manage_keys_labels(vocabulary, &keys);
        for ((_, key), (label, tip)) in keys.into_iter().zip(labels) {
            // Where this key's private half actually is, whichever store filed
            // it. A presence check rather than a fetch: cloning the entry copies
            // raw key bytes out of the vault unscrubbed, and this runs every
            // frame for every key.
            let filed_at = self.identity.private_keys.candidates(&key).next();
            let held = if filed_at.is_some() { HELD } else { NOT_HELD };
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let button =
                    ComponentStyles::add_secondary_button(ui, format!("{label} ›"), dark_mode);
                let button = match tip {
                    Some(tip) => button.clickable_tooltip(tip),
                    None => button,
                };
                if button.clicked() {
                    let holding = filed_at.as_ref().and_then(|placement| {
                        self.identity
                            .private_keys
                            .get_cloned_private_key_data_and_wallet_info(placement)
                    });
                    action |= AppAction::AddScreen(Screen::KeyInfoScreen(
                        KeyInfoScreen::new(
                            self.identity.clone(),
                            key.clone(),
                            holding,
                            &self.app_context,
                        )
                        .with_parent(PARENT_CRUMB),
                    ));
                }
                ui.label(RichText::new(held).color(DashColors::text_secondary(dark_mode)));
            });
            if expert {
                Self::render_expert_detail(ui, &key, dark_mode);
            }
        }
        action
    }

    /// The on-chain specifics of one key, for the Expert view. Everyday view
    /// gets the role word and held state, which is what it can act on.
    fn render_expert_detail(ui: &mut egui::Ui, key: &IdentityPublicKey, dark_mode: bool) {
        ui.label(
            RichText::new(format!(
                "Key {id} · {purpose:?} · {security:?} · {key_type:?}{read_only}",
                id = key.id(),
                purpose = key.purpose(),
                security = key.security_level(),
                key_type = key.key_type(),
                read_only = if key.read_only() { " · read-only" } else { "" },
            ))
            .small()
            .monospace()
            .color(DashColors::text_secondary(dark_mode)),
        );
    }

    /// Render the offer to restore this identity's keys from the previous
    /// version's saved data, queueing what the user approved. Renders nothing
    /// when detection found nothing, so it appears only where it has something
    /// to say and retires itself once a restore lands.
    fn render_recovery_section(&mut self, ui: &mut egui::Ui) {
        let restoring = self.recovery.is_restoring();
        let Some(plan) = self.recovery.plan().filter(|plan| !plan.is_empty()) else {
            return;
        };
        ui.add_space(8.0);
        // `changed_value` hands back a reference into the response, so the
        // approved set has to be cloned out before the response is dropped.
        if let Some(approved) =
            LegacyRecoverySection::new(plan, KeyVocabulary::from(self.identity.identity_type))
                .restoring(restoring)
                .show(ui)
                .inner
                .changed_value()
        {
            self.pending_recovery_restore = Some(approved.clone());
        }
        ui.add_space(8.0);
        ui.separator();
    }

    /// Re-read this screen's identity from the store after a backend task wrote
    /// it, so the held/not-held column and the rows themselves reflect a
    /// restore that has landed. A read failure leaves the clone alone and says
    /// so — the change landed, this screen just cannot show it.
    fn reload_identity(&mut self) {
        let identity_id = self.identity.identity.id();
        match self.app_context.get_local_qualified_identity(&identity_id) {
            Ok(Some(fresh)) => self.identity = fresh,
            Ok(None) => {}
            Err(error) => {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "This identity's keys could not be reloaded. Close this list and open it \
                     again to see them.",
                    MessageType::Error,
                )
                .with_details(error);
            }
        }
    }
}
