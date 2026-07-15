//! Hub-facing shim over the generalized [`global_nav_switcher`].
//!
//! The Identities hub keeps its original `BreadcrumbEffect` API and behavior:
//! this module builds the hub's [`PageNavSpec`] (interactive wallet + app-global
//! identity pills, `Identities` segment-1), delegates rendering to
//! [`global_nav_switcher::render`], and maps the generalized
//! [`GlobalNavEffect`] back to [`BreadcrumbEffect`]. A self-navigation to the
//! hub root (the `Identities` link) maps to [`BreadcrumbEffect::OpenPicker`],
//! preserving the pre-generalization behavior.
//!
//! [`global_nav_switcher`]: crate::ui::components::global_nav_switcher

use crate::context::AppContext;
use crate::ui::RootScreenType;
use crate::ui::components::global_nav_switcher::{self, GlobalNavEffect};
use crate::ui::state::global_nav::{IdentityPillScope, PageNavSpec, PillConsumption};
use crate::ui::state::hub_selection::HubSelection;
use dash_sdk::platform::Identifier;
use eframe::egui::Ui;
use std::sync::Arc;

use crate::model::wallet::WalletSeedHash;

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

/// The hub's page-nav spec: `Identities` segment-1 linking to the hub root, an
/// interactive (consumed) wallet pill, and the app-global identity pill.
fn hub_spec() -> PageNavSpec {
    PageNavSpec::new("Identities", RootScreenType::RootScreenIdentityHub)
        .with_wallet_pill(PillConsumption::Consumed)
        .with_identity_pill(IdentityPillScope::AppGlobalUser, PillConsumption::Consumed)
}

/// Map a generalized effect to the hub's `BreadcrumbEffect`. A self-navigation
/// to the hub root is the `Identities` link → open the picker.
fn map_effect(effect: GlobalNavEffect) -> BreadcrumbEffect {
    match effect {
        GlobalNavEffect::None => BreadcrumbEffect::None,
        GlobalNavEffect::NavigateToRoot(RootScreenType::RootScreenIdentityHub) => {
            BreadcrumbEffect::OpenPicker
        }
        // The hub's segment-1 only ever targets the hub itself.
        GlobalNavEffect::NavigateToRoot(_) => BreadcrumbEffect::None,
        GlobalNavEffect::SwitchWallet(hash) => BreadcrumbEffect::SwitchWallet(hash),
        GlobalNavEffect::SelectIdentity(id) => BreadcrumbEffect::SelectIdentity(id),
        // The hub never composes a page-scoped object pill.
        GlobalNavEffect::SelectPageObject(_) => BreadcrumbEffect::None,
        GlobalNavEffect::AddWallet => BreadcrumbEffect::AddWallet,
        GlobalNavEffect::AddIdentityCreate => BreadcrumbEffect::AddIdentityCreate,
        GlobalNavEffect::AddIdentityLoad => BreadcrumbEffect::AddIdentityLoad,
        GlobalNavEffect::CreateTestIdentities => BreadcrumbEffect::CreateTestIdentities,
    }
}

/// Render the hub breadcrumb switcher. Delegates to the generalized global-nav
/// switcher with the hub's spec and maps the effect back.
pub fn render(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    selection: &mut HubSelection,
) -> BreadcrumbEffect {
    map_effect(global_nav_switcher::render(
        ui,
        app_context,
        &hub_spec(),
        selection,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `Identities` link (self-navigation to the hub root) opens the picker,
    /// preserving the pre-generalization hub behavior.
    #[test]
    fn self_navigation_maps_to_open_picker() {
        assert_eq!(
            map_effect(GlobalNavEffect::NavigateToRoot(
                RootScreenType::RootScreenIdentityHub
            )),
            BreadcrumbEffect::OpenPicker
        );
    }

    /// The hub never surfaces a page-scoped object selection.
    #[test]
    fn page_scoped_effects_are_dropped_on_the_hub() {
        assert_eq!(
            map_effect(GlobalNavEffect::SelectPageObject(Identifier::new([5; 32]))),
            BreadcrumbEffect::None
        );
    }

    /// Wallet/identity switches and add flows pass through unchanged.
    #[test]
    fn common_effects_pass_through() {
        assert_eq!(
            map_effect(GlobalNavEffect::SwitchWallet([1; 32])),
            BreadcrumbEffect::SwitchWallet([1; 32])
        );
        let id = Identifier::new([2; 32]);
        assert_eq!(
            map_effect(GlobalNavEffect::SelectIdentity(id)),
            BreadcrumbEffect::SelectIdentity(id)
        );
        assert_eq!(
            map_effect(GlobalNavEffect::AddWallet),
            BreadcrumbEffect::AddWallet
        );
        assert_eq!(
            map_effect(GlobalNavEffect::CreateTestIdentities),
            BreadcrumbEffect::CreateTestIdentities
        );
    }
}
