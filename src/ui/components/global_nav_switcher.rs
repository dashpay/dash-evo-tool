//! Page-aware global navigation switcher.
//!
//! Generalizes the Identities-hub breadcrumb into a switcher rendered on every
//! root page. Composes `segment-1 link › 💼 wallet pill › 👤 identity/object
//! pill` per a [`PageNavSpec`], owns the wallet / identity dropdown `Popup`s,
//! and returns a typed [`GlobalNavEffect`] for the shell to apply.
//!
//! It is a pure UI component — it reads the app-scoped selection from
//! `AppContext` and reports an effect; the shell applies it (components render,
//! screens decide). Reuses [`BreadcrumbPill`]/[`IdentityPill`] and the existing
//! [`BreadcrumbPillMode`] verbatim — no new pill widget.
//!
//! Per-state modes follow design-spec §A.3 / §7; tooltips are verbatim from
//! design-spec §D (§7.1). Wallet-scoped identity lists use the *stored*
//! `wallet_hash` filter, never `associated_wallets.keys().next()` (R1).

use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::user_role::UserRole;
use crate::model::wallet::WalletSeedHash;
use crate::model::wallet_association::{NOT_IN_WALLET_LABEL, WalletAssociation};
use crate::ui::RootScreenType;
use crate::ui::components::breadcrumb_pill::{BreadcrumbPill, BreadcrumbPillMode};
use crate::ui::identity::identity_hero_card::HeroIdentityKind;
use crate::ui::identity::identity_pill::{IdentityPill, display_label};
use crate::ui::state::global_nav::{
    IdentityPillScope, PageNavSpec, PageObjectItem, PillConsumption,
};
use crate::ui::state::hub_selection::HubSelection;
use crate::ui::theme::DashColors;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use eframe::egui::{self, RichText, Sense, Ui};
use std::sync::Arc;

/// Inline search appears once a wallet's identity list reaches this size (§A.3).
const SEARCH_THRESHOLD: usize = 7;

/// A typed switcher outcome the shell applies. Generalizes the hub's
/// `BreadcrumbEffect`: adds [`GlobalNavEffect::SelectPageObject`] for the
/// page-scoped object pill, kept distinct from [`GlobalNavEffect::SelectIdentity`]
/// so a page-scoped selection never writes the app-global identity (FR-6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlobalNavEffect {
    /// No interaction this frame.
    None,
    /// Segment-1 was clicked — navigate to the page's root screen.
    NavigateToRoot(RootScreenType),
    /// Switch the operating wallet.
    SwitchWallet(WalletSeedHash),
    /// Select the app-global User identity.
    SelectIdentity(Identifier),
    /// Select a page-scoped object (the masternode/evonode in view). **Never**
    /// the app-global identity — the structural FR-6 boundary.
    SelectPageObject(Identifier),
    /// "Set up another wallet" — route to the Wallets screen.
    AddWallet,
    /// "Add another identity" → create a new identity.
    AddIdentityCreate,
    /// "Add another identity" → load an existing identity.
    AddIdentityLoad,
    /// Dev-mode: bulk-create test identities.
    CreateTestIdentities,
}

/// Wallet-pill mode by HD-wallet count: 0 → placeholder, 1 → subdued (info
/// only), ≥2 → interactive (opens the wallet dropdown). §A.3 / §7.
fn wallet_pill_mode(wallet_count: usize) -> BreadcrumbPillMode {
    match wallet_count {
        0 => BreadcrumbPillMode::Placeholder,
        1 => BreadcrumbPillMode::Subdued,
        _ => BreadcrumbPillMode::Interactive,
    }
}

/// tt-2 — interactive wallet pill (≥2 wallets). Verbatim, design-spec §D.
fn tt_wallet_interactive() -> &'static str {
    "Switch between your wallets. Each wallet can own several identities."
}

/// tt-3 — subdued wallet pill (exactly 1 wallet). Verbatim, design-spec §D #3
/// (the brief's "…to switch between them." is a paraphrase — this is canonical).
fn tt_wallet_subdued(wallet_name: &str) -> String {
    format!(
        "This identity is funded by {wallet_name}. Set up another wallet on the Wallets screen \
         to unlock switching."
    )
}

/// tt-4 — interactive identity pill. Verbatim, design-spec §D.
fn tt_identity(wallet_name: &str) -> String {
    format!("Switch between identities in {wallet_name} or add a new one.")
}

/// Short hex of a seed hash, for a wallet with no alias.
fn short_hex(hash: &WalletSeedHash) -> String {
    let mut s = String::with_capacity(10);
    for b in hash.iter().take(4) {
        s.push_str(&format!("{b:02x}"));
    }
    s.push('…');
    s
}

/// Loaded HD wallets as `(seed_hash, display_name)`, sorted by hash for a
/// stable order. Name = alias, else a short hex of the seed hash.
fn gather_wallets(app_context: &Arc<AppContext>) -> Vec<(WalletSeedHash, String)> {
    let Ok(wallets) = app_context.wallets.read() else {
        return Vec::new();
    };
    wallets
        .iter()
        .map(|(hash, w)| {
            let name = w
                .read()
                .ok()
                .and_then(|w| w.alias.clone())
                .filter(|a| !a.trim().is_empty())
                .unwrap_or_else(|| short_hex(hash));
            (*hash, name)
        })
        .collect()
}

/// Identity display label (Local nickname → DPNS → short id). The switcher
/// reads no social profile, so the display-name tier is empty.
fn identity_label(qi: &QualifiedIdentity) -> String {
    let dpns = qi.dpns_names.first().map(|n| n.name.as_str());
    display_label(
        qi.alias.as_deref(),
        None,
        dpns,
        &qi.identity.id().to_string(Encoding::Base58),
    )
}

/// First uppercase alphanumeric of the label, for the avatar monogram.
fn monogram_initial(label: &str) -> Option<char> {
    label
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
}

/// The selected page-scoped item, if the selection resolves against `items`. A
/// stale selection (not in `items`) resolves to `None`, so the pill falls back
/// to its placeholder.
fn resolve_page_object(
    items: &[PageObjectItem],
    selected: Option<Identifier>,
) -> Option<&PageObjectItem> {
    selected.and_then(|id| items.iter().find(|it| it.id == id))
}

/// The pill label for a page-scoped object: the selected item's label, or the
/// placeholder when nothing is selected (or the selection is stale).
fn page_object_label(
    placeholder: &str,
    items: &[PageObjectItem],
    selected: Option<Identifier>,
) -> String {
    resolve_page_object(items, selected)
        .map(|it| it.label.clone())
        .unwrap_or_else(|| placeholder.to_string())
}

/// A page-scoped pill with no items to offer is a placeholder — there is
/// nothing to pick, so it never opens a dropdown.
fn page_object_mode(items: &[PageObjectItem]) -> BreadcrumbPillMode {
    if items.is_empty() {
        BreadcrumbPillMode::Placeholder
    } else {
        BreadcrumbPillMode::Interactive
    }
}

/// Identity-primary context shared by the wallet + app-global identity pills.
/// Derived once per frame, mirroring the hub's original derivation.
struct AppGlobalContext {
    active_id: Option<Identifier>,
    pill_identity: Option<QualifiedIdentity>,
    active_is_wallet_less: bool,
    active_wallet: Option<WalletSeedHash>,
    active_wallet_name: String,
    scoped: Vec<QualifiedIdentity>,
    no_wallet: Vec<QualifiedIdentity>,
}

/// Derive the app-global (identity-primary) context: the active identity, the
/// wallet derived from it, the wallet-scoped identity list, and the no-wallet
/// group. Identical logic to the original hub switcher.
fn derive_app_global_context(
    app_context: &Arc<AppContext>,
    wallets: &[(WalletSeedHash, String)],
) -> AppGlobalContext {
    // FR-6: the app-global identity pill and its dropdown (including the
    // wallet-less "no wallet on this device" group) list User identities only —
    // masternode/evonode identities never appear on everyday-user surfaces
    // (TC-NAV-17). The wallet-scoped list below is wallet-owned, so it is
    // User-only by construction (masternodes are wallet-less).
    let all_identities = app_context.load_local_user_identities().unwrap_or_default();
    let all_ids: Vec<Identifier> = all_identities.iter().map(|qi| qi.identity.id()).collect();
    let active_id = app_context.selected_identity_id();
    // The identity pill reflects an *explicitly* chosen identity (or a lone
    // auto-selected one). In the ≥2-none-chosen picker state it stays a
    // placeholder (§7) — never the first-identity fallback, which would
    // duplicate a picker-grid label and disagree with "no identity chosen".
    let pill_target_id = crate::model::selected_identity::keep_if_loaded(active_id, &all_ids)
        .or_else(|| (all_ids.len() == 1).then(|| all_ids[0]));
    let pill_identity = pill_target_id
        .and_then(|id| all_identities.iter().find(|qi| qi.identity.id() == id))
        .cloned();
    let active_is_wallet_less = pill_identity
        .as_ref()
        .is_some_and(|qi| qi.wallet_index.is_none());

    // The wallet segment is DERIVED from the active identity (identity-primary).
    // A wallet-less active identity → no active wallet → empty wallet segment,
    // so the pill never shows a wallet belonging to a different identity.
    let active_wallet = if active_is_wallet_less {
        None
    } else {
        app_context
            .selected_wallet_hash()
            .filter(|h| wallets.iter().any(|(wh, _)| wh == h))
            .or_else(|| wallets.first().map(|(h, _)| *h))
    };
    let active_wallet_name = active_wallet
        .and_then(|h| wallets.iter().find(|(wh, _)| *wh == h))
        .map(|(_, n)| n.clone())
        .unwrap_or_default();

    // Identities owned by the active wallet (stored `wallet_hash` filter — R1).
    let scoped: Vec<QualifiedIdentity> = active_wallet
        .and_then(|h| {
            app_context
                .load_local_qualified_identities_for_wallet(&h)
                .ok()
        })
        .unwrap_or_default();
    // Identities with no wallet on this device (imported by id).
    let no_wallet: Vec<QualifiedIdentity> = all_identities
        .iter()
        .filter(|qi| qi.wallet_index.is_none())
        .cloned()
        .collect();

    AppGlobalContext {
        active_id,
        pill_identity,
        active_is_wallet_less,
        active_wallet,
        active_wallet_name,
        scoped,
        no_wallet,
    }
}

/// Render the switcher for `spec`. Reads the app-scoped selection; mutates only
/// the `selection` search buffers; returns the user's effect for the shell to
/// apply.
pub fn render(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    spec: &PageNavSpec,
    selection: &mut HubSelection,
) -> GlobalNavEffect {
    let dark_mode = ui.ctx().global_style().visuals.dark_mode;
    let mut effect = GlobalNavEffect::None;

    let wallets = gather_wallets(app_context);
    let wallet_count = wallets.len();

    // Identity-primary derivation is only needed when the third pill is the
    // app-global user identity (avoids an extra DB read on pages whose pill is
    // page-scoped, e.g. Masternodes).
    let app_global = matches!(
        spec.identity_pill(),
        Some((IdentityPillScope::AppGlobalUser, _))
    );
    let ctx_data = app_global.then(|| derive_app_global_context(app_context, &wallets));

    ui.horizontal(|ui| {
        // --- Segment 1: page-aware link --------------------------------------
        let link = ui.add(
            egui::Label::new(RichText::new(spec.segment1_label()).color(DashColors::DASH_BLUE))
                .sense(Sense::click()),
        );
        if link.clicked() {
            effect = GlobalNavEffect::NavigateToRoot(spec.segment1_target());
        }

        // --- Segment 2: wallet -----------------------------------------------
        // A page may show a fixed, read-only wallet association for the object in
        // view (e.g. a wallet-less masternode) instead of the app-scoped pill.
        if let Some((association, tooltip)) = spec.object_wallet() {
            ui.label(RichText::new("›").color(DashColors::text_secondary(dark_mode)));
            render_object_wallet_pill(ui, association, tooltip);
        } else if let Some(consumption) = spec.wallet_pill() {
            ui.label(RichText::new("›").color(DashColors::text_secondary(dark_mode)));
            render_wallet_pill(
                ui,
                consumption,
                &wallets,
                wallet_count,
                app_context,
                ctx_data.as_ref(),
                dark_mode,
                &mut effect,
            );
        }

        // --- Segment 3: identity / page-object pill --------------------------
        if let Some((scope, consumption)) = spec.identity_pill() {
            ui.label(RichText::new("›").color(DashColors::text_secondary(dark_mode)));
            match scope {
                IdentityPillScope::AppGlobalUser => {
                    // `ctx_data` is always `Some` here (derived when app_global).
                    if let Some(data) = ctx_data.as_ref() {
                        render_app_global_identity_pill(
                            ui,
                            consumption,
                            app_context,
                            data,
                            selection,
                            wallet_count,
                            dark_mode,
                            &mut effect,
                        );
                    }
                }
                IdentityPillScope::PageScopedObject {
                    placeholder,
                    tooltip,
                    items,
                    selected,
                } => {
                    render_page_object_pill(
                        ui,
                        consumption,
                        placeholder,
                        tooltip,
                        items,
                        *selected,
                        dark_mode,
                        &mut effect,
                    );
                }
            }
        }
    });

    effect
}

/// Render the wallet pill. `Consumed` reproduces the count-based
/// placeholder/subdued/interactive logic (identity-primary when `ctx_data` is
/// present); `Unwired` renders a subdued, non-interactive pill with a
/// how-to-change tooltip and emits no effect.
#[allow(clippy::too_many_arguments)]
fn render_wallet_pill(
    ui: &mut Ui,
    consumption: &PillConsumption,
    wallets: &[(WalletSeedHash, String)],
    wallet_count: usize,
    app_context: &Arc<AppContext>,
    ctx_data: Option<&AppGlobalContext>,
    dark_mode: bool,
    effect: &mut GlobalNavEffect,
) {
    // Resolve the active wallet: identity-primary when available, else the
    // plain app-scoped selection (or the first wallet).
    let (active_wallet, active_wallet_name, active_is_wallet_less) = match ctx_data {
        Some(d) => (
            d.active_wallet,
            d.active_wallet_name.clone(),
            d.active_is_wallet_less,
        ),
        None => {
            let active = app_context
                .selected_wallet_hash()
                .filter(|h| wallets.iter().any(|(wh, _)| wh == h))
                .or_else(|| wallets.first().map(|(h, _)| *h));
            let name = active
                .and_then(|h| wallets.iter().find(|(wh, _)| *wh == h))
                .map(|(_, n)| n.clone())
                .unwrap_or_default();
            (active, name, false)
        }
    };

    if let PillConsumption::Unwired { tooltip } = consumption {
        // Dimmed, no caret, no visible tag — the value stays visible, the
        // explanation lives in the tooltip (FR-GLOBAL-NAV-2 rule 3).
        if active_wallet.is_some() {
            BreadcrumbPill::new(active_wallet_name)
                .with_icon("💼")
                .subdued(true)
                .with_tooltip(tooltip.clone())
                .show(ui);
        } else {
            BreadcrumbPill::placeholder("(no wallet yet)")
                .with_tooltip(tooltip.clone())
                .show(ui);
        }
        return;
    }

    // Consumed: the original count-based rendering.
    let wallet_mode = if active_is_wallet_less {
        BreadcrumbPillMode::Placeholder
    } else {
        wallet_pill_mode(wallet_count)
    };
    match wallet_mode {
        BreadcrumbPillMode::Placeholder => {
            let label = if active_is_wallet_less {
                "(no wallet)"
            } else {
                "(no wallet yet)"
            };
            BreadcrumbPill::placeholder(label).show(ui);
        }
        BreadcrumbPillMode::Subdued => {
            BreadcrumbPill::new(active_wallet_name.clone())
                .with_icon("💼")
                .subdued(true)
                .with_tooltip(tt_wallet_subdued(&active_wallet_name))
                .show(ui);
        }
        BreadcrumbPillMode::Interactive => {
            let resp = BreadcrumbPill::new(active_wallet_name.clone())
                .with_icon("💼")
                .with_tooltip(tt_wallet_interactive())
                .show(ui);
            if let Some(anchor) = resp.response.clone() {
                let popup_id = ui.make_persistent_id("global_nav_wallet_switcher");
                egui::Popup::new(popup_id, ui.ctx().clone(), &anchor, anchor.layer_id)
                    .open_memory(resp.clicked.then_some(egui::SetOpenCommand::Toggle))
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .frame(egui::Frame::popup(ui.style()).fill(DashColors::popup_fill(dark_mode)))
                    .show(|ui| {
                        ui.set_min_width(220.0);
                        for (h, name) in wallets {
                            let is_active = active_wallet == Some(*h);
                            if ui
                                .selectable_label(is_active, format!("💼 {name}"))
                                .clicked()
                            {
                                *effect = GlobalNavEffect::SwitchWallet(*h);
                                ui.close();
                            }
                        }
                        ui.separator();
                        if ui.button("Set up another wallet").clicked() {
                            *effect = GlobalNavEffect::AddWallet;
                            ui.close();
                        }
                    });
            }
        }
    }
}

/// Render the fixed, read-only wallet indicator for the object in view. Names
/// the owning wallet subdued when one controls the object, else shows the muted
/// "not in a wallet" placeholder — never an interactive pill, so a wallet-less
/// object (e.g. a masternode) never appears to belong to an arbitrary wallet.
fn render_object_wallet_pill(ui: &mut Ui, association: &WalletAssociation, tooltip: &str) {
    match association {
        WalletAssociation::InWallet(name) => {
            BreadcrumbPill::new(name.clone())
                .with_icon("💼")
                .subdued(true)
                .with_tooltip(tooltip.to_string())
                .show(ui);
        }
        WalletAssociation::NotInWallet => {
            BreadcrumbPill::placeholder(NOT_IN_WALLET_LABEL)
                .with_tooltip(tooltip.to_string())
                .show(ui);
        }
    }
}

/// Render the app-global User identity pill. `Consumed` reproduces the hub's
/// identity dropdown (scoped list + no-wallet group + add flows); `Unwired`
/// renders a subdued, non-interactive pill with a how-to-change tooltip.
#[allow(clippy::too_many_arguments)]
fn render_app_global_identity_pill(
    ui: &mut Ui,
    consumption: &PillConsumption,
    app_context: &Arc<AppContext>,
    data: &AppGlobalContext,
    selection: &mut HubSelection,
    wallet_count: usize,
    dark_mode: bool,
    effect: &mut GlobalNavEffect,
) {
    let Some(active_qi) = data.pill_identity.as_ref() else {
        // No identity in scope: placeholder reflects whether a wallet exists.
        let label = if wallet_count == 0 {
            "(no identity yet)"
        } else {
            "(choose an identity)"
        };
        let pill = BreadcrumbPill::placeholder(label);
        match consumption {
            PillConsumption::Unwired { tooltip } => pill.with_tooltip(tooltip.clone()).show(ui),
            PillConsumption::Consumed => pill.show(ui),
        };
        return;
    };

    let label = identity_label(active_qi);
    let kind: HeroIdentityKind = active_qi.identity_type.into();
    let dpns = active_qi.dpns_names.first().map(|n| n.name.clone());
    let id_b58 = active_qi.identity.id().to_string(Encoding::Base58);

    if let PillConsumption::Unwired { tooltip } = consumption {
        // Subdued, non-interactive: the value shows dimmed with no caret.
        IdentityPill::new(active_qi.alias.as_deref(), dpns.as_deref(), &id_b58)
            .with_avatar(kind, monogram_initial(&label))
            .with_mode(BreadcrumbPillMode::Subdued)
            .with_tooltip(tooltip.clone())
            .show(ui);
        return;
    }

    let resp = IdentityPill::new(active_qi.alias.as_deref(), dpns.as_deref(), &id_b58)
        .with_avatar(kind, monogram_initial(&label))
        .with_tooltip(tt_identity(&data.active_wallet_name))
        .show(ui);

    if let Some(anchor) = resp.response.clone() {
        let popup_id = ui.make_persistent_id("global_nav_identity_switcher");
        egui::Popup::new(popup_id, ui.ctx().clone(), &anchor, anchor.layer_id)
            .open_memory(resp.clicked.then_some(egui::SetOpenCommand::Toggle))
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .frame(egui::Frame::popup(ui.style()).fill(DashColors::popup_fill(dark_mode)))
            .show(|ui| {
                ui.set_min_width(240.0);

                // Inline search once the scoped list is long (§A.3).
                let filter = if data.scoped.len() >= SEARCH_THRESHOLD {
                    ui.add(
                        egui::TextEdit::singleline(selection.identity_search_mut())
                            .hint_text("Search identities"),
                    );
                    selection.identity_search().trim().to_lowercase()
                } else {
                    String::new()
                };

                for qi in &data.scoped {
                    let row = identity_label(qi);
                    if !filter.is_empty() && !row.to_lowercase().contains(&filter) {
                        continue;
                    }
                    let id = qi.identity.id();
                    let is_active = data.active_id == Some(id);
                    if ui.selectable_label(is_active, row).clicked() {
                        *effect = GlobalNavEffect::SelectIdentity(id);
                        ui.close();
                    }
                }

                if !data.no_wallet.is_empty() {
                    ui.separator();
                    ui.label(
                        RichText::new("Identities without a wallet on this device")
                            .small()
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                    for qi in &data.no_wallet {
                        let id = qi.identity.id();
                        let is_active = data.active_id == Some(id);
                        if ui.selectable_label(is_active, identity_label(qi)).clicked() {
                            *effect = GlobalNavEffect::SelectIdentity(id);
                            ui.close();
                        }
                    }
                }

                ui.separator();
                if ui.button("Create a new identity").clicked() {
                    *effect = GlobalNavEffect::AddIdentityCreate;
                    ui.close();
                }
                if ui.button("Load an existing identity").clicked() {
                    *effect = GlobalNavEffect::AddIdentityLoad;
                    ui.close();
                }
                if app_context.user_role().at_least(UserRole::Power)
                    && ui.button("Create multiple test identities").clicked()
                {
                    *effect = GlobalNavEffect::CreateTestIdentities;
                    ui.close();
                }
            });
    }
}

/// Render the page-scoped object pill (masternode/evonode in view). `Consumed`
/// opens a dropdown of `items` and emits [`GlobalNavEffect::SelectPageObject`]
/// — never `SelectIdentity`; `Unwired` renders a subdued, non-interactive pill.
/// `tooltip` is the page's own copy for the interactive pill.
#[allow(clippy::too_many_arguments)]
fn render_page_object_pill(
    ui: &mut Ui,
    consumption: &PillConsumption,
    placeholder: &str,
    tooltip: &str,
    items: &[PageObjectItem],
    selected: Option<Identifier>,
    dark_mode: bool,
    effect: &mut GlobalNavEffect,
) {
    let label = page_object_label(placeholder, items, selected);
    // The glyph belongs to the selected object; a placeholder carries none.
    let icon = resolve_page_object(items, selected).and_then(|it| it.icon.clone());

    if let PillConsumption::Unwired {
        tooltip: how_to_change,
    } = consumption
    {
        let mut pill = BreadcrumbPill::new(label).subdued(true);
        if let Some(icon) = icon {
            pill = pill.with_icon(icon);
        }
        pill.with_tooltip(how_to_change.clone()).show(ui);
        return;
    }

    let mut pill = BreadcrumbPill::new(label).with_mode(page_object_mode(items));
    if let Some(icon) = icon {
        pill = pill.with_icon(icon);
    }
    // An empty page offers nothing to pick: placeholder only, no dropdown.
    if items.is_empty() {
        pill.show(ui);
        return;
    }

    let resp = pill.with_tooltip(tooltip.to_string()).show(ui);
    if let Some(anchor) = resp.response.clone() {
        let popup_id = ui.make_persistent_id("global_nav_object_switcher");
        egui::Popup::new(popup_id, ui.ctx().clone(), &anchor, anchor.layer_id)
            .open_memory(resp.clicked.then_some(egui::SetOpenCommand::Toggle))
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .frame(egui::Frame::popup(ui.style()).fill(DashColors::popup_fill(dark_mode)))
            .show(|ui| {
                ui.set_min_width(240.0);
                for it in items {
                    let is_active = selected == Some(it.id);
                    let row = match &it.icon {
                        Some(icon) => format!("{icon} {}", it.label),
                        None => it.label.clone(),
                    };
                    if ui.selectable_label(is_active, row).clicked() {
                        *effect = GlobalNavEffect::SelectPageObject(it.id);
                        ui.close();
                    }
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> Identifier {
        Identifier::new([byte; 32])
    }

    #[test]
    fn short_hex_is_stable_prefix() {
        let h = [0xABu8; 32];
        assert_eq!(short_hex(&h), "abababab…");
    }

    #[test]
    fn monogram_initial_picks_first_alphanumeric_uppercase() {
        assert_eq!(monogram_initial("alex.dash"), Some('A'));
        assert_eq!(monogram_initial("  9lives"), Some('9'));
        assert_eq!(monogram_initial("…"), None);
    }

    /// Wallet-pill mode resolver (moved verbatim from the hub switcher).
    #[test]
    fn wallet_pill_mode_by_count() {
        assert_eq!(wallet_pill_mode(0), BreadcrumbPillMode::Placeholder);
        assert_eq!(wallet_pill_mode(1), BreadcrumbPillMode::Subdued);
        assert_eq!(wallet_pill_mode(2), BreadcrumbPillMode::Interactive);
        assert_eq!(wallet_pill_mode(9), BreadcrumbPillMode::Interactive);
    }

    /// Verbatim tooltip strings (regression guard for the design-spec wording).
    #[test]
    fn tooltips_are_verbatim() {
        assert_eq!(
            tt_wallet_interactive(),
            "Switch between your wallets. Each wallet can own several identities."
        );
        assert_eq!(
            tt_wallet_subdued("Main Wallet"),
            "This identity is funded by Main Wallet. Set up another wallet on the Wallets screen \
             to unlock switching."
        );
        assert_eq!(
            tt_identity("Main Wallet"),
            "Switch between identities in Main Wallet or add a new one."
        );
    }

    fn node_items() -> Vec<PageObjectItem> {
        vec![
            PageObjectItem {
                id: id(1),
                label: "mn-east-01".to_string(),
                icon: Some("🖥".to_string()),
            },
            PageObjectItem {
                id: id(2),
                label: "evo-west-02".to_string(),
                icon: Some("◆".to_string()),
            },
        ]
    }

    /// TC-NAV-04 foundation — a page-scoped selection resolves the pill label to
    /// the selected item; a stale or absent selection falls back to placeholder.
    #[test]
    fn page_object_label_resolves_selection_else_placeholder() {
        let items = node_items();
        assert_eq!(
            page_object_label("(choose a masternode)", &items, Some(id(2))),
            "evo-west-02"
        );
        // Nothing selected → placeholder.
        assert_eq!(
            page_object_label("(choose a masternode)", &items, None),
            "(choose a masternode)"
        );
        // Stale selection (not in items) → placeholder.
        assert_eq!(
            page_object_label("(choose a masternode)", &items, Some(id(9))),
            "(choose a masternode)"
        );
    }

    /// The pill's glyph is the selected object's own; a placeholder (nothing or
    /// a stale id selected) carries none.
    #[test]
    fn page_object_icon_belongs_to_the_selected_item() {
        let items = node_items();
        assert_eq!(
            resolve_page_object(&items, Some(id(2))).and_then(|it| it.icon.as_deref()),
            Some("◆")
        );
        assert!(resolve_page_object(&items, None).is_none());
        assert!(resolve_page_object(&items, Some(id(9))).is_none());
    }

    /// A page with no objects to offer renders a placeholder pill (no dropdown);
    /// one or more objects make it interactive.
    #[test]
    fn page_object_pill_is_placeholder_until_objects_exist() {
        assert_eq!(page_object_mode(&[]), BreadcrumbPillMode::Placeholder);
        assert_eq!(
            page_object_mode(&node_items()),
            BreadcrumbPillMode::Interactive
        );
    }

    /// TC-NAV-16 foundation — a page-scoped object selection maps to
    /// `SelectPageObject`, never `SelectIdentity`. Guards the FR-6 boundary at
    /// the effect level (the switcher's PageScopedObject arm emits only this).
    #[test]
    fn page_object_effect_is_never_select_identity() {
        let picked = GlobalNavEffect::SelectPageObject(id(3));
        assert!(matches!(picked, GlobalNavEffect::SelectPageObject(x) if x == id(3)));
        assert!(!matches!(picked, GlobalNavEffect::SelectIdentity(_)));
    }
}
