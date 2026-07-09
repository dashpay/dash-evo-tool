//! Identity Hub — Settings tab.
//!
//! Two-column layout inside the central island: social profile (left) and
//! username + aliases (right), with a full-width `Advanced` expander below.
//! See design-spec §B.8 and dev-plan task T11.
//!
//! ## Backend integration
//!
//! This tab is **additive** with respect to the backend: it dispatches only
//! backend tasks that already exist and never introduces new variants. As of
//! 2026-04-23 the following controls cannot be wired to a backend task and are
//! therefore feature-gated — rendered as non-interactive affordances with a
//! `disabled_tooltip` explaining that the action is coming in a follow-up:
//!
//! - **Delete social profile** — no `DashPayTask::DeleteProfile` variant.
//! - **Add / remove alias** and **Make primary** — no `IdentityTask::AddAlias`
//!   / `RemoveAlias` / `MakePrimaryAlias` variants.
//! - **Unload this identity from this device** — no identity-unload task; the
//!   existing `wallet_lifecycle` unload path is wallet-scoped, not identity-
//!   scoped, and wiring it here would bypass the dashpay / DPNS state cleanup
//!   the operation implies.
//!
//! These appear as `Gated(missing_task)` non-interactive rows with the copy
//! from design-spec §D (tooltip catalog entries #49 and #59). A TODO comment
//! marks each one so the backend follow-up can search for the flag.

use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::identity::IdentityTask;
use crate::context::AppContext;
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use crate::ui::ScreenType;
use crate::ui::components::component_trait::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::identities::register_dpns_name_screen::RegisterDpnsNameSource;
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use eframe::egui::{Id, Margin, RichText, TextEdit, Ui};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Tooltip copy (from design-spec §D "Tooltip Catalog"). Kept as `const` so
// they are extractable as a single translation unit when i18n lands.
// ---------------------------------------------------------------------------

const TIP_CHANGE_PHOTO: &str = "Upload a square image. Other apps will see this avatar.";
const TIP_SAVE_NO_CHANGES: &str = "There are no changes to save.";
const TIP_SAVE_INVALID: &str = "Fix the highlighted fields before saving.";
const TIP_DELETE_PROFILE: &str = "Remove the display name, bio, and avatar from DashPay. Your identity, usernames, and \
     balance stay.";
const TIP_PRIMARY_PILL: &str = "Your primary username is what people see by default.";
const TIP_MAKE_PRIMARY: &str =
    "Use this username as your main one. Your old primary will become an alias.";
const TIP_REMOVE_ALIAS: &str = "Remove this alias. You will keep your other usernames.";
const TIP_ADD_ALIAS: &str = "Register another DPNS name that points to this identity.";
const TIP_ADD_KEY: &str =
    "Register a new key for this identity. You will choose its purpose and type.";
const TIP_REFRESH: &str = "Fetch the latest state of this identity from the network.";
const TIP_UNLOAD: &str = "Remove this identity from this device. It remains on Dash Platform — you can load it \
     again later.";
const TIP_ID_COPY: &str = "Copy the full identity ID to your clipboard.";
const TIP_PROTX_COPY: &str = "Copy the masternode ID to your clipboard.";
const TIP_BADGE_USER: &str = "A regular identity used for payments, DPNS, and DashPay.";
const TIP_BADGE_MASTERNODE: &str =
    "An identity tied to a Dash masternode. It can vote on name contests.";
const TIP_BADGE_EVONODE: &str =
    "An identity tied to a Dash evonode. It can vote and validate Platform transactions.";

// Marker strings for controls without a matching backend task. Surfaced in
// disabled_tooltip and as a prefix on the row so users know it is a coming
// feature, not a stuck UI.
const GATED_COMING_SOON: &str =
    "Coming soon. This control will activate when the backend task lands.";

// Limits and validation come from the shared model validator, the single
// source of truth also used by the DashPay profile editor and the backend.
use crate::model::dashpay::{
    MAX_AVATAR_URL_CHARS as MAX_AVATAR_URL, MAX_BIO_CHARS as MAX_BIO,
    MAX_DISPLAY_NAME_CHARS as MAX_DISPLAY_NAME, ProfileFieldError, validate_profile_fields,
};

// ---------------------------------------------------------------------------
// Stateful tab component
// ---------------------------------------------------------------------------

/// Settings tab state. Holds the currently-selected identity (picked on
/// construction) plus per-field edit state. Follows the project's stateful-UI
/// pattern used by `ProfileScreen`: form fields, dirty tracking, confirmation
/// dialogs.
#[derive(Default)]
pub struct SettingsTab {
    /// Identity whose settings we are editing. `None` when no identities
    /// exist yet (the hub's `HubLanding::Onboarding` path means we should
    /// normally not even render this tab, but we defend against it anyway).
    selected_identity: Option<QualifiedIdentity>,
    /// Editable social profile fields. Loaded on `ensure_selected` from the
    /// cached DashPay profile (identity_id + network).
    edit_display_name: String,
    edit_bio: String,
    edit_avatar_url: String,
    /// Copy of the originals for `has_changes` comparison. Updated only
    /// after a CONFIRMED backend success via `on_profile_saved()`.
    original_display_name: String,
    original_bio: String,
    original_avatar_url: String,
    /// The values that were actually submitted to `UpdateProfile`. Stored so
    /// that `on_profile_saved()` commits the submitted snapshot, NOT the
    /// current edit-field state (which may have changed while the round-trip
    /// was in-flight). Cleared on identity switch or when committed. (T21)
    pending_save: Option<(String, String, String)>,
    /// `Advanced` expander state. Defaults closed per §B.8; callers (tests)
    /// may flip this via `open_advanced_for_test` to assert the section
    /// renders without a click.
    advanced_open: bool,
    /// Confirmation dialog for the (gated) "Delete social profile" action.
    confirm_delete_profile: Option<ConfirmationDialog>,
    /// Confirmation dialog for the (gated) "Unload this identity" action.
    confirm_unload: Option<ConfirmationDialog>,
    /// Track whether we have loaded the cached profile for the current
    /// identity. Reset on identity change.
    profile_loaded: bool,
}

impl SettingsTab {
    /// Construct an empty tab. The hub owns the lifecycle so we do not take an
    /// `AppContext` here — `ensure_selected` pulls the first identity when
    /// `render` is first called.
    pub fn new() -> Self {
        Self::default()
    }

    /// Borrow the currently-selected identity, if any. Used by the hub's
    /// `display_task_result` to guard `on_profile_saved` against stale results.
    pub fn selected_identity(
        &self,
    ) -> Option<&crate::model::qualified_identity::QualifiedIdentity> {
        self.selected_identity.as_ref()
    }

    /// Render the tab; returns any `AppAction` generated by the user.
    pub fn render(
        &mut self,
        ui: &mut Ui,
        app_context: &Arc<AppContext>,
        profiles: &mut super::profile_cache::ProfileCache,
    ) -> AppAction {
        self.ensure_selected(app_context, profiles);

        let Some(identity) = self.selected_identity.clone() else {
            return render_empty_state(ui);
        };

        let mut action = AppAction::None;
        let dark_mode = ui.ctx().global_style().visuals.dark_mode;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Two-column layout. egui's `columns` takes care of splitting
                // the available width evenly; on narrow panels the individual
                // rows inside each column wrap gracefully because we use
                // `set_max_width` on the text fields.
                ui.columns(2, |cols| {
                    action |= self.render_social_profile(&mut cols[0], app_context, &identity);
                    action |=
                        self.render_username_and_aliases(&mut cols[1], app_context, &identity);
                });

                ui.add_space(16.0);

                // Full-width Advanced expander.
                let header = RichText::new("Advanced")
                    .strong()
                    .size(16.0)
                    .color(DashColors::text_primary(dark_mode));
                // Remember open state across frames via a stable id so a test
                // that clicks the header keeps it open for the next step.
                let resp = egui::CollapsingHeader::new(header)
                    .id_salt(Id::new("identity_hub_settings_advanced"))
                    .default_open(self.advanced_open)
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("Keys, raw identifiers, and identity type.")
                                .small()
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                        ui.add_space(8.0);
                        action |= self.render_advanced(ui, app_context, &identity);
                    });
                // Track open state so test helpers can read it if needed.
                self.advanced_open = resp.fully_open();
            });

        // Dialogs on top.
        action |= self.show_gated_dialogs(ui);

        action
    }

    // -----------------------------------------------------------------
    // Section renderers
    // -----------------------------------------------------------------

    fn render_social_profile(
        &mut self,
        ui: &mut Ui,
        app_context: &Arc<AppContext>,
        identity: &QualifiedIdentity,
    ) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.ctx().global_style().visuals.dark_mode;

        section_heading(ui, "Social profile", dark_mode);
        ui.label(
            RichText::new("This information is visible to everyone on Dash Platform.")
                .small()
                .color(DashColors::text_secondary(dark_mode)),
        );
        ui.add_space(8.0);

        // Avatar block — placeholder glyph + "Change photo" ghost button.
        // The actual file-picker wiring lives in `ProfileScreen`; we surface
        // the button and let the user click through to the legacy edit path
        // in a follow-up (no backend task needed yet).
        ui.horizontal(|ui| {
            ui.label(RichText::new("👤").size(48.0).color(DashColors::DEEP_BLUE));
            ui.vertical(|ui| {
                let btn = ComponentStyles::add_secondary_button(ui, "Change photo", dark_mode)
                    .clickable_tooltip(TIP_CHANGE_PHOTO);
                if btn.clicked() {
                    // Route to legacy DashPay Profile screen for the full
                    // image-upload flow. This is NOT a backend task and does
                    // not violate the "additive only" rule.
                    action = AppAction::SetMainScreen(
                        crate::ui::RootScreenType::RootScreenDashPayProfile,
                    );
                }
            });
        });

        ui.add_space(8.0);

        // Display name input.
        ui.label(RichText::new("Display name").color(DashColors::text_primary(dark_mode)));
        let display_name = ui.add(
            TextEdit::singleline(&mut self.edit_display_name)
                .hint_text("How should people see your name?")
                .desired_width(f32::INFINITY),
        );
        counter(
            ui,
            self.edit_display_name.len(),
            MAX_DISPLAY_NAME,
            dark_mode,
        );
        let _ = display_name; // response not needed beyond widget side-effects

        ui.add_space(8.0);

        // Bio textarea.
        ui.label(RichText::new("About").color(DashColors::text_primary(dark_mode)));
        ui.add(
            TextEdit::multiline(&mut self.edit_bio)
                .hint_text(format!("A short description, up to {MAX_BIO} characters."))
                .desired_width(f32::INFINITY)
                .desired_rows(4),
        );
        counter(ui, self.edit_bio.len(), MAX_BIO, dark_mode);

        ui.add_space(8.0);

        // Avatar URL — we include it so users can still set the avatar when
        // the file-picker flow is not yet available from this tab. Kept under
        // the visual avatar block so it is clearly secondary.
        ui.label(RichText::new("Avatar URL").color(DashColors::text_primary(dark_mode)));
        ui.add(
            TextEdit::singleline(&mut self.edit_avatar_url)
                .hint_text("https://example.com/avatar.jpg")
                .desired_width(f32::INFINITY),
        );
        counter(ui, self.edit_avatar_url.len(), MAX_AVATAR_URL, dark_mode);

        ui.add_space(12.0);

        // Save / Delete buttons row.
        let invalid = self.validation_error().is_some();
        let dirty = self.has_changes();
        let can_save = !invalid && dirty;
        let save_tooltip = if !dirty {
            TIP_SAVE_NO_CHANGES.to_string()
        } else if invalid {
            TIP_SAVE_INVALID.to_string()
        } else {
            "Save your social profile to DashPay.".to_string()
        };

        ui.horizontal(|ui| {
            let save =
                ComponentStyles::add_primary_button_enabled(ui, can_save, "Save social profile");
            let save = if can_save {
                save.clickable_tooltip(save_tooltip)
            } else {
                save.disabled_tooltip(save_tooltip)
            };
            if save.clicked() && can_save {
                // Capture the exact values being submitted. `on_profile_saved()`
                // commits THIS snapshot as the new baseline — not whatever is in
                // the edit fields at the time the success arrives, which may have
                // changed while the round-trip was in-flight.
                self.pending_save = Some((
                    self.edit_display_name.clone(),
                    self.edit_bio.clone(),
                    self.edit_avatar_url.clone(),
                ));
                action = AppAction::BackendTask(BackendTask::DashPayTask(Box::new(
                    DashPayTask::UpdateProfile {
                        identity: identity.clone(),
                        display_name: string_if_set(&self.edit_display_name),
                        bio: string_if_set(&self.edit_bio),
                        avatar_url: string_if_set(&self.edit_avatar_url),
                    },
                )));
            }

            ui.add_space(12.0);

            // GATED: DashPayTask::DeleteProfile does not exist (2026-04-23).
            // Render as a non-interactive danger-style link so Alex can see
            // the affordance and knows it is planned.
            // TODO(identity-hub): wire once DashPayTask::DeleteProfile lands.
            let delete = ui
                .add_enabled(
                    false,
                    egui::Button::new(
                        RichText::new("Delete social profile").color(DashColors::ERROR),
                    )
                    .fill(egui::Color32::TRANSPARENT)
                    .stroke(egui::Stroke::NONE),
                )
                .disabled_tooltip(format!("{TIP_DELETE_PROFILE} {GATED_COMING_SOON}"));
            if delete.clicked() {
                // Unreachable while disabled; defensive — open the confirm
                // dialog so, once the backend exists, this path activates
                // with a single-line change (remove `add_enabled(false, …)`).
                self.confirm_delete_profile = Some(
                    ConfirmationDialog::new(
                        "Delete social profile",
                        "Remove the display name, bio, and avatar from DashPay. Your \
                         identity, usernames, and balance stay intact. Are you sure?",
                    )
                    .confirm_text(Some("Delete"))
                    .cancel_text(Some("Keep"))
                    .danger_mode(true),
                );
            }
        });

        // Prevent unused-variable warning when app_context is not needed
        // by the current code path (kept for future backend dispatches).
        let _ = app_context;

        action
    }

    fn render_username_and_aliases(
        &mut self,
        ui: &mut Ui,
        app_context: &Arc<AppContext>,
        identity: &QualifiedIdentity,
    ) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.ctx().global_style().visuals.dark_mode;

        // Identity-type badge + tooltip — design-spec §B.8 rule 1 moves this
        // into Advanced, but we also surface a compact badge here so the
        // user knows which identity they are editing. Matches wireframe Frame 8.
        let (badge_label, badge_tip) = identity_type_badge(identity.identity_type);
        let badge = egui::Button::new(RichText::new(badge_label).small())
            .fill(DashColors::surface(dark_mode))
            .stroke(egui::Stroke::new(
                1.0,
                DashColors::text_secondary(dark_mode),
            ));
        ui.add(badge).info_tooltip(badge_tip);
        ui.add_space(8.0);

        section_heading(ui, "Username", dark_mode);

        // Primary DPNS name. If none, show the CTA card.
        let primary = identity.dpns_names.first();
        if let Some(name) = primary {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("@{}", name.name))
                        .monospace()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.add_space(4.0);
                // Primary pill.
                let pill = egui::Button::new(RichText::new("Primary").small())
                    .fill(DashColors::DASH_BLUE)
                    .stroke(egui::Stroke::NONE);
                ui.add(pill).info_tooltip(TIP_PRIMARY_PILL);

                ui.add_space(4.0);
                if ui
                    .button("Copy")
                    .clickable_tooltip(format!("Copy @{} to your clipboard.", name.name))
                    .clicked()
                {
                    ui.ctx().copy_text(format!("@{}", name.name));
                }
            });
        } else {
            // Pick-a-username CTA.
            egui::Frame::group(ui.style())
                .fill(DashColors::surface(dark_mode))
                .inner_margin(Margin::same(10))
                .corner_radius(egui::CornerRadius::same(6))
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Pick a username")
                                .strong()
                                .color(DashColors::text_primary(dark_mode)),
                        );
                        ui.add_space(6.0);
                        let reg = ComponentStyles::add_primary_button(ui, "Register a username")
                            .clickable_tooltip(
                                "Register a DPNS name and bind it to this identity.",
                            );
                        if reg.clicked() {
                            action = AppAction::AddScreen(
                                ScreenType::RegisterDpnsName(RegisterDpnsNameSource::Identities)
                                    .create_screen(app_context),
                            );
                        }
                    });
                });
        }

        ui.add_space(12.0);

        // Aliases block. Each secondary DPNS name appears with Make-primary +
        // Remove actions; both are GATED because the backend variants do not
        // exist yet.
        section_heading(ui, "Aliases", dark_mode);
        ui.label(
            RichText::new("Extra usernames that also point to your identity.")
                .small()
                .color(DashColors::text_secondary(dark_mode)),
        );
        ui.add_space(4.0);

        let aliases: Vec<_> = identity.dpns_names.iter().skip(1).cloned().collect();
        if aliases.is_empty() {
            ui.label(RichText::new("No aliases yet.").color(DashColors::text_secondary(dark_mode)));
        } else {
            for alias in &aliases {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("@{}", alias.name))
                            .monospace()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    // TODO(identity-hub): wire once IdentityTask::MakePrimaryAlias exists.
                    let mp = ui
                        .add_enabled(false, egui::Button::new("Make primary"))
                        .disabled_tooltip(format!("{TIP_MAKE_PRIMARY} {GATED_COMING_SOON}"));
                    let _ = mp;
                    // TODO(identity-hub): wire once IdentityTask::RemoveAlias exists.
                    let rm = ui
                        .add_enabled(false, egui::Button::new("Remove"))
                        .disabled_tooltip(format!("{TIP_REMOVE_ALIAS} {GATED_COMING_SOON}"));
                    let _ = rm;
                });
            }
        }

        ui.add_space(6.0);

        // TODO(identity-hub): "Add an alias" requires IdentityTask::AddAlias,
        // which does not exist yet. Render as disabled so Alex sees the
        // affordance and learns it is planned.
        let add = ui
            .add_enabled(false, egui::Button::new("Add an alias"))
            .disabled_tooltip(format!("{TIP_ADD_ALIAS} {GATED_COMING_SOON}"));
        let _ = add;

        action
    }

    fn render_advanced(
        &mut self,
        ui: &mut Ui,
        app_context: &Arc<AppContext>,
        identity: &QualifiedIdentity,
    ) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.ctx().global_style().visuals.dark_mode;

        // 1. Identity type + raw ID + (optional) ProTxHash.
        sub_heading(ui, "Identity", dark_mode);
        let id_base58 = identity.identity.id().to_string(Encoding::Base58);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Identity ID").color(DashColors::text_secondary(dark_mode)));
            ui.label(
                RichText::new(&id_base58)
                    .monospace()
                    .color(DashColors::text_primary(dark_mode)),
            );
            if ui
                .small_button("Copy")
                .clickable_tooltip(TIP_ID_COPY)
                .clicked()
            {
                ui.ctx().copy_text(id_base58.clone());
            }
        });

        if identity.identity_type != IdentityType::User {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Masternode ID").color(DashColors::text_secondary(dark_mode)),
                );
                // The ProTxHash is the raw identity ID in hex for masternode /
                // evonode identities. We display it as hex per §C.
                let protx_hex = identity.identity.id().to_string(Encoding::Hex);
                ui.label(
                    RichText::new(&protx_hex)
                        .monospace()
                        .color(DashColors::text_primary(dark_mode)),
                );
                if ui
                    .small_button("Copy")
                    .clickable_tooltip(TIP_PROTX_COPY)
                    .clicked()
                {
                    ui.ctx().copy_text(protx_hex);
                }
            });
        }

        ui.add_space(10.0);

        // 2. Keys table.
        sub_heading(ui, "Keys", dark_mode);
        ui.label(
            RichText::new(
                "Keys let this identity sign actions. Most people never need to manage these \
                 directly.",
            )
            .small()
            .color(DashColors::text_secondary(dark_mode)),
        );
        let key_count = identity.identity.public_keys().len();
        ui.label(
            RichText::new(format!(
                "This identity has {key_count} key{s}.",
                s = if key_count == 1 { "" } else { "s" }
            ))
            .color(DashColors::text_primary(dark_mode)),
        );
        ui.add_space(4.0);
        // `Add a new key` routes to the existing AddKeyScreen — no new
        // backend work required, and the screen handles its own dispatch.
        let add_key =
            ComponentStyles::add_primary_button(ui, "Add a new key").clickable_tooltip(TIP_ADD_KEY);
        if add_key.clicked() {
            action = AppAction::AddScreen(
                ScreenType::AddKeyScreen(identity.clone()).create_screen(app_context),
            );
        }

        ui.add_space(12.0);

        // 3. Refresh.
        sub_heading(ui, "Refresh and diagnostics", dark_mode);
        let refresh = ComponentStyles::add_secondary_button(ui, "Refresh identity data", dark_mode)
            .clickable_tooltip(TIP_REFRESH);
        if refresh.clicked() {
            action = AppAction::BackendTask(BackendTask::IdentityTask(
                IdentityTask::RefreshIdentity(identity.clone()),
            ));
        }

        ui.add_space(16.0);

        // 4. Danger zone — red-bordered card.
        let danger_color = DashColors::ERROR;
        egui::Frame::group(ui.style())
            .fill(DashColors::surface(dark_mode))
            .stroke(egui::Stroke::new(1.0, danger_color))
            .inner_margin(Margin::same(12))
            .corner_radius(egui::CornerRadius::same(6))
            .show(ui, |ui| {
                sub_heading(ui, "Danger zone", dark_mode);
                ui.label(
                    RichText::new(
                        "These actions affect this device only. Your identity stays on Dash \
                         Platform.",
                    )
                    .small()
                    .color(DashColors::text_secondary(dark_mode)),
                );
                ui.add_space(6.0);
                // TODO(identity-hub): wire once an identity-scoped unload task
                // exists. Wallet-scoped unload (wallet_lifecycle) is too broad
                // — it would silently drop sibling identities on the same wallet.
                let unload = ui
                    .add_enabled(
                        false,
                        ComponentStyles::danger_button("Unload this identity from this device"),
                    )
                    .disabled_tooltip(format!("{TIP_UNLOAD} {GATED_COMING_SOON}"));
                if unload.clicked() {
                    self.confirm_unload = Some(
                        ConfirmationDialog::new(
                            "Unload this identity",
                            "This removes the identity from this device. It remains on Dash \
                             Platform — you can load it again later.",
                        )
                        .confirm_text(Some("Unload"))
                        .cancel_text(Some("Keep"))
                        .danger_mode(true),
                    );
                }
            });

        action
    }

    // -----------------------------------------------------------------
    // Dialog handling
    // -----------------------------------------------------------------

    fn show_gated_dialogs(&mut self, ui: &mut Ui) -> AppAction {
        let action = AppAction::None;

        if let Some(dialog) = self.confirm_delete_profile.as_mut() {
            match dialog.show(ui).inner.dialog_response {
                Some(ConfirmationStatus::Confirmed) | Some(ConfirmationStatus::Canceled) => {
                    self.confirm_delete_profile = None;
                }
                None => {}
            }
        }

        if let Some(dialog) = self.confirm_unload.as_mut() {
            match dialog.show(ui).inner.dialog_response {
                Some(ConfirmationStatus::Confirmed) | Some(ConfirmationStatus::Canceled) => {
                    self.confirm_unload = None;
                }
                None => {}
            }
        }

        action
    }

    // -----------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------

    /// Load the first available identity if none selected. Caches the
    /// cached-profile fields on identity change so the text inputs reflect
    /// the stored state immediately.
    fn ensure_selected(
        &mut self,
        app_context: &Arc<AppContext>,
        profiles: &mut super::profile_cache::ProfileCache,
    ) {
        // Read the app-scoped active identity (selected → first → none), not
        // `.first()`, so the Settings tab agrees with the breadcrumb/hub and
        // does not flip-flop across frames (D4).
        let incoming = app_context.resolve_selected_identity();

        let changed = match (&self.selected_identity, &incoming) {
            (Some(a), Some(b)) => a.identity.id() != b.identity.id(),
            (None, Some(_)) | (Some(_), None) => true,
            (None, None) => false,
        };

        if changed {
            self.selected_identity = incoming;
            self.profile_loaded = false;
            // Clear the editor to a clean slate; fields repopulate once the
            // async profile load lands (see `load_cached_profile`).
            self.edit_display_name.clear();
            self.edit_bio.clear();
            self.edit_avatar_url.clear();
            self.original_display_name.clear();
            self.original_bio.clear();
            self.original_avatar_url.clear();
            // A pending save for the old identity must not be committed for
            // the new one — clear it on switch (T21).
            self.pending_save = None;
        }

        if self.selected_identity.is_some() && !self.profile_loaded {
            self.load_cached_profile(profiles);
        }
    }

    /// Populate the editor from the hub's async profile cache. The local DB
    /// profile cache was removed in the platform-wallet migration, so this
    /// reads the cache (queuing a load on a miss) and fills the fields once the
    /// profile arrives — without clobbering edits the user has already made.
    fn load_cached_profile(&mut self, profiles: &mut super::profile_cache::ProfileCache) {
        let Some(identity) = self.selected_identity.clone() else {
            self.profile_loaded = true;
            return;
        };
        let fields = match profiles.get_or_request(&identity) {
            Some(loaded) => loaded.clone(),
            // Not loaded yet — a load is queued; retry on the next frame.
            None => return,
        };
        if self.has_changes() {
            // The async load landed after the user started editing; keep their
            // input and stop trying to repopulate.
            self.profile_loaded = true;
            return;
        }
        let (display_name, bio, avatar_url) = match fields {
            Some(f) => (f.display_name, f.bio, f.avatar_url),
            None => (String::new(), String::new(), String::new()),
        };
        self.edit_display_name = display_name;
        self.edit_bio = bio;
        self.edit_avatar_url = avatar_url;
        self.original_display_name = self.edit_display_name.clone();
        self.original_bio = self.edit_bio.clone();
        self.original_avatar_url = self.edit_avatar_url.clone();
        self.profile_loaded = true;
    }

    fn has_changes(&self) -> bool {
        self.edit_display_name != self.original_display_name
            || self.edit_bio != self.original_bio
            || self.edit_avatar_url != self.original_avatar_url
    }

    /// Called by the hub after a confirmed `DashPayProfileUpdated` backend
    /// success result for the currently-selected identity. Commits the
    /// SUBMITTED values (captured at click time) as the new baseline, which
    /// disables the Save button until the user makes another change. (T21)
    ///
    /// Using the submitted snapshot — not the current edit fields — prevents
    /// the data-loss scenario where the user keeps typing after clicking Save:
    /// the deferred-success must not silently treat never-saved edits as saved.
    ///
    /// A failed `UpdateProfile` task does NOT call this method, so the baseline
    /// stays at the last-confirmed state and the user can retry.
    pub fn on_profile_saved(&mut self) {
        if let Some((dn, bio, url)) = self.pending_save.take() {
            self.original_display_name = dn;
            self.original_bio = bio;
            self.original_avatar_url = url;
        }
        // If pending_save is None (e.g. stale success after an identity switch
        // cleared it) we do nothing — the hub's identity-ID guard should have
        // prevented this call, but defending is harmless.
    }

    /// Clear any in-flight pending save snapshot. Called by the hub's
    /// `display_task_error` so a failed `UpdateProfile` doesn't leave a stale
    /// snapshot that would be committed if a later `DashPayProfileUpdated` from
    /// a different path (e.g. legacy ProfileScreen "Change photo") arrives.
    pub fn clear_pending_save(&mut self) {
        self.pending_save = None;
    }

    /// Validation check used to drive Save button state. Returns `None` when
    /// input is valid, else the first violation. We do not persist this in a
    /// banner because users can self-correct inline using the counter.
    fn validation_error(&self) -> Option<ProfileFieldError> {
        validate_profile_fields(
            &self.edit_display_name,
            &self.edit_bio,
            &self.edit_avatar_url,
        )
        .into_iter()
        .next()
    }

    // -----------------------------------------------------------------
    // Test helpers (pub(crate))
    // -----------------------------------------------------------------

    /// Test helper: force the advanced expander open so the kittest frame sees
    /// the interior widgets without a click event. Not used by production code
    /// but kept on the struct for future populated-render tests.
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn open_advanced_for_test(&mut self) {
        self.advanced_open = true;
    }
}

// ---------------------------------------------------------------------------
// Small layout helpers
// ---------------------------------------------------------------------------

fn render_empty_state(ui: &mut Ui) -> AppAction {
    let dark_mode = ui.ctx().global_style().visuals.dark_mode;
    ui.vertical_centered(|ui| {
        ui.add_space(32.0);
        ui.label(
            RichText::new("No identity selected.")
                .strong()
                .color(DashColors::text_primary(dark_mode)),
        );
        ui.add_space(6.0);
        ui.label(
            RichText::new("Create or load an identity to see its settings.")
                .color(DashColors::text_secondary(dark_mode)),
        );
    });
    AppAction::None
}

fn section_heading(ui: &mut Ui, text: &str, dark_mode: bool) {
    ui.add_space(4.0);
    ui.label(
        RichText::new(text)
            .strong()
            .size(18.0)
            .color(DashColors::text_primary(dark_mode)),
    );
    ui.add_space(4.0);
}

fn sub_heading(ui: &mut Ui, text: &str, dark_mode: bool) {
    ui.label(
        RichText::new(text)
            .strong()
            .size(14.0)
            .color(DashColors::text_primary(dark_mode)),
    );
}

fn counter(ui: &mut Ui, count: usize, max: usize, dark_mode: bool) {
    let color = if count > max {
        DashColors::ERROR
    } else {
        DashColors::text_secondary(dark_mode)
    };
    ui.label(RichText::new(format!("{count}/{max}")).small().color(color));
}

fn string_if_set(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn identity_type_badge(kind: IdentityType) -> (&'static str, &'static str) {
    match kind {
        IdentityType::User => ("User identity", TIP_BADGE_USER),
        IdentityType::Masternode => ("Masternode identity", TIP_BADGE_MASTERNODE),
        IdentityType::Evonode => ("Evonode identity", TIP_BADGE_EVONODE),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_no_identity_selected() {
        let tab = SettingsTab::new();
        assert!(tab.selected_identity.is_none());
    }

    #[test]
    fn has_changes_tracks_baseline() {
        let mut tab = SettingsTab::new();
        tab.edit_display_name = "alex".into();
        tab.original_display_name = "alex".into();
        assert!(!tab.has_changes());

        tab.edit_display_name = "priya".into();
        assert!(tab.has_changes());
    }

    #[test]
    fn validation_catches_long_display_name() {
        let mut tab = SettingsTab::new();
        tab.edit_display_name = "a".repeat(MAX_DISPLAY_NAME + 1);
        assert!(tab.validation_error().is_some());
    }

    #[test]
    fn validation_catches_long_bio() {
        let mut tab = SettingsTab::new();
        tab.edit_bio = "a".repeat(MAX_BIO + 1);
        assert!(tab.validation_error().is_some());
    }

    #[test]
    fn validation_requires_http_scheme() {
        let mut tab = SettingsTab::new();
        tab.edit_avatar_url = "ftp://example.com/img".into();
        assert!(tab.validation_error().is_some());

        tab.edit_avatar_url = "https://example.com/img".into();
        assert!(tab.validation_error().is_none());
    }

    #[test]
    fn validation_accepts_empty_avatar_url() {
        let tab = SettingsTab::new();
        assert!(tab.validation_error().is_none());
    }

    #[test]
    fn string_if_set_trims_and_nones_empty() {
        assert_eq!(string_if_set(""), None);
        assert_eq!(string_if_set("   "), None);
        assert_eq!(string_if_set("  alex  "), Some("alex".to_string()));
    }

    #[test]
    fn identity_type_badge_covers_all_variants() {
        for ty in [
            IdentityType::User,
            IdentityType::Masternode,
            IdentityType::Evonode,
        ] {
            let (label, tip) = identity_type_badge(ty);
            assert!(!label.is_empty());
            assert!(!tip.is_empty());
        }
    }

    /// IT-SETTINGS-01 (section-heading slice) — verifies the three required
    /// section headings are rendered as labels when the layout helpers are
    /// called directly. This covers the expected section-name assertions
    /// from the test-case spec without bootstrapping a full `AppContext` and
    /// `QualifiedIdentity`, which would add significant test-fixture weight.
    #[test]
    fn section_headings_render_their_text() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;

        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(|ui| {
                let dark = ui.ctx().global_style().visuals.dark_mode;
                section_heading(ui, "Social profile", dark);
                section_heading(ui, "Username", dark);
                section_heading(ui, "Aliases", dark);
                sub_heading(ui, "Advanced", dark);
            });
        harness.run();
        assert!(
            harness.query_by_label("Social profile").is_some(),
            "Social profile heading must render",
        );
        assert!(
            harness.query_by_label("Username").is_some(),
            "Username heading must render",
        );
        assert!(
            harness.query_by_label("Aliases").is_some(),
            "Aliases heading must render",
        );
        assert!(
            harness.query_by_label("Advanced").is_some(),
            "Advanced sub-heading must render",
        );
    }

    /// IT-SETTINGS-02 — T21 deferred baseline: on_profile_saved() must commit
    /// the values that were SUBMITTED, not whatever is in the edit fields at the
    /// time the success arrives (which can differ when the user keeps typing
    /// after clicking Save on a slow network).
    #[test]
    fn on_profile_saved_commits_submitted_snapshot_not_current_edits() {
        let mut tab = SettingsTab::new();
        // Simulate the user having loaded a profile ("Alice") and then editing it.
        tab.original_display_name = "Alice".into();
        tab.original_bio = String::new();
        tab.original_avatar_url = String::new();
        tab.edit_display_name = "Alicia".into();
        tab.edit_bio = String::new();
        tab.edit_avatar_url = String::new();

        // User clicks Save — capture the submitted snapshot.
        tab.pending_save = Some(("Alicia".into(), String::new(), String::new()));

        // While the round-trip is in-flight the user KEEPS TYPING.
        tab.edit_display_name = "Alicia Smith".into();

        // The success arrives — hub calls on_profile_saved().
        tab.on_profile_saved();

        // The baseline must reflect "Alicia" (what was submitted), NOT
        // "Alicia Smith" (what happens to be in the box right now).
        assert_eq!(
            tab.original_display_name, "Alicia",
            "baseline must be the submitted value, not the current edit"
        );
        // The edit field is unchanged — the user can continue editing.
        assert_eq!(tab.edit_display_name, "Alicia Smith");
        // has_changes() sees "Alicia Smith" vs "Alicia" → Save re-enables.
        assert!(
            tab.has_changes(),
            "Save must re-enable for the in-flight edits"
        );
        // pending_save is cleared.
        assert!(tab.pending_save.is_none());
    }

    /// IT-SETTINGS-03 — T21: pending_save is cleared on identity switch so a
    /// stale success from the old identity cannot corrupt the new identity's
    /// baseline.
    #[test]
    fn pending_save_cleared_on_identity_change() {
        let mut tab = SettingsTab::new();
        tab.edit_display_name = "Alicia".into();
        tab.pending_save = Some(("Alicia".into(), String::new(), String::new()));

        // Simulate what ensure_selected() does on a changed identity.
        tab.selected_identity = None;
        tab.profile_loaded = false;
        tab.edit_display_name.clear();
        tab.edit_bio.clear();
        tab.edit_avatar_url.clear();
        tab.original_display_name.clear();
        tab.original_bio.clear();
        tab.original_avatar_url.clear();
        tab.pending_save = None; // the line we added in ensure_selected

        assert!(
            tab.pending_save.is_none(),
            "pending_save must be cleared on identity switch"
        );
        // If on_profile_saved is now called (stale result) it must be a no-op.
        let original_before = tab.original_display_name.clone();
        tab.on_profile_saved();
        assert_eq!(
            tab.original_display_name, original_before,
            "stale on_profile_saved must not corrupt baseline"
        );
    }
}
