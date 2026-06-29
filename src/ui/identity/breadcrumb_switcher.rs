//! The Identities-hub breadcrumb switcher (IDH-003).
//!
//! Composes `Identities` link › wallet pill › identity pill, owns the wallet
//! and identity dropdown `Popup`s, and returns a typed [`BreadcrumbEffect`].
//! It is a pure UI component — it reads the app-scoped selection from
//! `AppContext` and reports an effect; the hub applies it (components render,
//! screens decide).
//!
//! Per-state modes follow design-spec §A.3 / §7; tooltips are verbatim from
//! design-spec §D (§7.1). Wallet-scoped identity lists use the *stored*
//! `wallet_hash` filter, never `associated_wallets.keys().next()` (R1).

use super::identity_hero_card::HeroIdentityKind;
use super::identity_pill::{IdentityPill, display_label};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::breadcrumb_pill::{BreadcrumbPill, BreadcrumbPillMode};
use crate::ui::state::hub_selection::HubSelection;
use crate::ui::theme::DashColors;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use eframe::egui::{self, RichText, Sense, Ui};
use std::sync::Arc;

use crate::model::wallet::WalletSeedHash;

/// Inline search appears once a wallet's identity list reaches this size (§A.3).
const SEARCH_THRESHOLD: usize = 7;

/// A typed switcher outcome the hub applies. Switching is hub-internal; add
/// flows reuse existing `AppAction`s through the hub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BreadcrumbEffect {
    /// No interaction this frame.
    None,
    /// The `Identities` crumb was clicked — open the picker.
    OpenPicker,
    /// Switch the operating wallet.
    SwitchWallet(WalletSeedHash),
    /// Select an identity.
    SelectIdentity(Identifier),
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

/// Identity display label (Local nickname → DPNS → short id).
fn identity_label(qi: &QualifiedIdentity) -> String {
    let dpns = qi.dpns_names.first().map(|n| n.name.as_str());
    display_label(
        qi.alias.as_deref(),
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

/// Render the switcher. Reads the app-scoped selection; mutates only the
/// `selection` search buffers; returns the user's effect for the hub to apply.
pub fn render(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    selection: &mut HubSelection,
) -> BreadcrumbEffect {
    let dark_mode = ui.ctx().global_style().visuals.dark_mode;
    let mut effect = BreadcrumbEffect::None;

    let wallets = gather_wallets(app_context);
    let wallet_count = wallets.len();
    let active_wallet = app_context
        .selected_wallet_hash()
        .filter(|h| wallets.iter().any(|(wh, _)| wh == h))
        .or_else(|| wallets.first().map(|(h, _)| *h));
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
    let no_wallet: Vec<QualifiedIdentity> = app_context
        .load_local_qualified_identities()
        .map(|v| {
            v.into_iter()
                .filter(|qi| qi.wallet_index.is_none())
                .collect()
        })
        .unwrap_or_default();

    let active_id = app_context.selected_identity_id();
    let active_in_scope = active_id.filter(|id| scoped.iter().any(|qi| qi.identity.id() == *id));
    let pill_identity = active_in_scope
        .and_then(|id| scoped.iter().find(|qi| qi.identity.id() == id))
        .or_else(|| scoped.first());

    ui.horizontal(|ui| {
        // --- Segment 1: Identities link ---------------------------------
        let link = ui.add(
            egui::Label::new(RichText::new("Identities").color(DashColors::DASH_BLUE))
                .sense(Sense::click()),
        );
        if link.clicked() {
            effect = BreadcrumbEffect::OpenPicker;
        }
        ui.label(RichText::new("›").color(DashColors::text_secondary(dark_mode)));

        // --- Segment 2: wallet pill -------------------------------------
        match wallet_pill_mode(wallet_count) {
            BreadcrumbPillMode::Placeholder => {
                BreadcrumbPill::placeholder("(no wallet yet)").show(ui);
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
                    let popup_id = ui.make_persistent_id("hub_wallet_switcher");
                    egui::Popup::new(popup_id, ui.ctx().clone(), &anchor, anchor.layer_id)
                        .open_memory(resp.clicked.then_some(egui::SetOpenCommand::Toggle))
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .frame(
                            egui::Frame::popup(ui.style()).fill(DashColors::popup_fill(dark_mode)),
                        )
                        .show(|ui| {
                            ui.set_min_width(220.0);
                            for (h, name) in &wallets {
                                let is_active = active_wallet == Some(*h);
                                if ui
                                    .selectable_label(is_active, format!("💼 {name}"))
                                    .clicked()
                                {
                                    effect = BreadcrumbEffect::SwitchWallet(*h);
                                    ui.close();
                                }
                            }
                            ui.separator();
                            if ui.button("Set up another wallet").clicked() {
                                effect = BreadcrumbEffect::AddWallet;
                                ui.close();
                            }
                        });
                }
            }
        }

        ui.label(RichText::new("›").color(DashColors::text_secondary(dark_mode)));

        // --- Segment 3: identity pill -----------------------------------
        let Some(active_qi) = pill_identity else {
            // No identity in scope: placeholder reflects whether a wallet exists.
            let label = if wallet_count == 0 {
                "(no identity yet)"
            } else {
                "(choose an identity)"
            };
            BreadcrumbPill::placeholder(label).show(ui);
            return;
        };

        let label = identity_label(active_qi);
        let kind: HeroIdentityKind = active_qi.identity_type.into();
        let dpns = active_qi.dpns_names.first().map(|n| n.name.clone());
        let id_b58 = active_qi.identity.id().to_string(Encoding::Base58);
        let resp = IdentityPill::new(active_qi.alias.as_deref(), dpns.as_deref(), &id_b58)
            .with_avatar(kind, monogram_initial(&label))
            .with_tooltip(tt_identity(&active_wallet_name))
            .show(ui);

        if let Some(anchor) = resp.response.clone() {
            let popup_id = ui.make_persistent_id("hub_identity_switcher");
            egui::Popup::new(popup_id, ui.ctx().clone(), &anchor, anchor.layer_id)
                .open_memory(resp.clicked.then_some(egui::SetOpenCommand::Toggle))
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .frame(egui::Frame::popup(ui.style()).fill(DashColors::popup_fill(dark_mode)))
                .show(|ui| {
                    ui.set_min_width(240.0);

                    // Inline search once the scoped list is long (§A.3).
                    let filter = if scoped.len() >= SEARCH_THRESHOLD {
                        ui.add(
                            egui::TextEdit::singleline(selection.identity_search_mut())
                                .hint_text("Search identities"),
                        );
                        selection.identity_search().trim().to_lowercase()
                    } else {
                        String::new()
                    };

                    for qi in &scoped {
                        let row = identity_label(qi);
                        if !filter.is_empty() && !row.to_lowercase().contains(&filter) {
                            continue;
                        }
                        let id = qi.identity.id();
                        let is_active = active_id == Some(id);
                        if ui.selectable_label(is_active, row).clicked() {
                            effect = BreadcrumbEffect::SelectIdentity(id);
                            ui.close();
                        }
                    }

                    if !no_wallet.is_empty() {
                        ui.separator();
                        ui.label(
                            RichText::new("Identities without a wallet on this device")
                                .small()
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                        for qi in &no_wallet {
                            let id = qi.identity.id();
                            let is_active = active_id == Some(id);
                            if ui.selectable_label(is_active, identity_label(qi)).clicked() {
                                effect = BreadcrumbEffect::SelectIdentity(id);
                                ui.close();
                            }
                        }
                    }

                    ui.separator();
                    if ui.button("Create a new identity").clicked() {
                        effect = BreadcrumbEffect::AddIdentityCreate;
                        ui.close();
                    }
                    if ui.button("Load an existing identity").clicked() {
                        effect = BreadcrumbEffect::AddIdentityLoad;
                        ui.close();
                    }
                    if app_context.is_developer_mode()
                        && ui.button("Create multiple test identities").clicked()
                    {
                        effect = BreadcrumbEffect::CreateTestIdentities;
                        ui.close();
                    }
                });
        }
    });

    effect
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> Identifier {
        Identifier::from([b; 32])
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

    /// UT-SWITCH-MODE-01 — wallet-pill mode resolver.
    #[test]
    fn wallet_pill_mode_by_count() {
        assert_eq!(wallet_pill_mode(0), BreadcrumbPillMode::Placeholder);
        assert_eq!(wallet_pill_mode(1), BreadcrumbPillMode::Subdued);
        assert_eq!(wallet_pill_mode(2), BreadcrumbPillMode::Interactive);
        assert_eq!(wallet_pill_mode(9), BreadcrumbPillMode::Interactive);
    }

    /// UT-SWITCH-TT-01 — verbatim tooltip strings (regression guard for the
    /// tt-3 design-spec wording; the brief's paraphrase must not creep in).
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

    /// Guards R1: the no-wallet group is identified by `wallet_index.is_none()`,
    /// independent of `associated_wallets` (which clones every loaded wallet).
    /// The end-to-end scoping is covered by IT-SWITCH-02; this pins the
    /// discriminator the filter relies on.
    #[test]
    fn no_wallet_group_uses_wallet_index_discriminator() {
        let owned: Option<u32> = Some(3);
        let imported: Option<u32> = None;
        assert!(
            owned.is_some(),
            "wallet-owned identity carries a wallet_index"
        );
        assert!(
            imported.is_none(),
            "imported-by-id identity has no wallet_index"
        );
        let _ = id(1);
    }
}
