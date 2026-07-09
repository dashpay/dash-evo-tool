//! Page-scoped view-model for the Masternodes global-nav pill (B7).
//!
//! Builds the Masternodes page's [`PageNavSpec`]: a page-aware `Masternodes`
//! segment-1, an **interactive** wallet pill (funds Top up — FR-9), and a
//! **page-scoped** masternode pill via [`IdentityPillScope::PageScopedObject`].
//!
//! The masternode selection this spec carries lives on the Masternodes page and
//! is **never** written to `AppContext::selected_identity_id` — that is the FR-6
//! boundary in code (complementing B1's resolution-layer filter). Renders
//! nothing (module-placement discriminator → `ui/state`).

use dash_sdk::platform::Identifier;

use crate::ui::RootScreenType;
use crate::ui::state::global_nav::{
    IdentityPillScope, PageNavSpec, PageObjectItem, PillConsumption,
};

/// Placeholder shown on the masternode pill when no node is loaded (§10.4).
pub const PLACEHOLDER_EMPTY: &str = "(no masternode yet)";
/// Placeholder shown on the masternode pill on the list, before a node is
/// chosen (§10.4).
pub const PLACEHOLDER_CHOOSE: &str = "Choose a masternode";
/// How-to tooltip on the subdued masternode pill when no node is loaded.
const TT_NO_MASTERNODE: &str = "Load a masternode to select one.";

/// Build the Masternodes page's global-nav spec.
///
/// `items` are the loaded MN/Evonode nodes (id + display label); `selected` is
/// the node whose detail view is open (`None` on the empty/list views — the
/// pill resets to its placeholder on `‹ All masternodes`, §10.4).
///
/// With no nodes the masternode pill renders subdued (Placeholder mode,
/// `(no masternode yet)`); with ≥1 node it is interactive (two-way with the card
/// grid + detail). The wallet pill is always interactive.
pub fn masternodes_page_nav_spec(
    items: Vec<PageObjectItem>,
    selected: Option<Identifier>,
) -> PageNavSpec {
    let (placeholder, identity_consumption) = if items.is_empty() {
        (
            PLACEHOLDER_EMPTY,
            PillConsumption::Unwired {
                tooltip: TT_NO_MASTERNODE.to_string(),
            },
        )
    } else {
        (PLACEHOLDER_CHOOSE, PillConsumption::Consumed)
    };

    PageNavSpec::new("Masternodes", RootScreenType::RootScreenMasternodes)
        .with_wallet_pill(PillConsumption::Consumed)
        .with_identity_pill(
            IdentityPillScope::page_scoped_object(placeholder, items, selected),
            identity_consumption,
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> Identifier {
        Identifier::new([byte; 32])
    }

    fn item(byte: u8, label: &str) -> PageObjectItem {
        PageObjectItem {
            id: id(byte),
            label: label.to_string(),
        }
    }

    /// TC-NAV-04 — with zero nodes the masternode pill is a subdued placeholder.
    #[test]
    fn empty_uses_placeholder_and_subdued_pill() {
        let spec = masternodes_page_nav_spec(vec![], None);
        assert_eq!(spec.segment1_label(), "Masternodes");
        assert_eq!(
            spec.segment1_target(),
            RootScreenType::RootScreenMasternodes
        );
        // Wallet pill interactive (TC-NAV-03).
        assert!(spec.wallet_pill().expect("wallet pill").is_consumed());

        let (scope, consumption) = spec.identity_pill().expect("identity pill");
        assert!(scope.is_page_scoped());
        assert!(!consumption.is_consumed(), "empty → subdued placeholder");
        match scope {
            IdentityPillScope::PageScopedObject {
                placeholder,
                items,
                selected,
            } => {
                assert_eq!(placeholder, PLACEHOLDER_EMPTY);
                assert!(items.is_empty());
                assert!(selected.is_none());
            }
            _ => panic!("expected page-scoped object pill"),
        }
    }

    /// TC-NAV-05/07 — with ≥1 node the pill is interactive and lists the nodes;
    /// the placeholder is `Choose a masternode` until one is selected.
    #[test]
    fn populated_pill_is_interactive_and_lists_nodes() {
        let items = vec![item(1, "mn-east-01"), item(2, "evo-west-02")];
        let spec = masternodes_page_nav_spec(items.clone(), None);
        let (scope, consumption) = spec.identity_pill().expect("identity pill");
        assert!(consumption.is_consumed(), "≥1 node → interactive dropdown");
        match scope {
            IdentityPillScope::PageScopedObject {
                placeholder,
                items: spec_items,
                selected,
            } => {
                assert_eq!(placeholder, PLACEHOLDER_CHOOSE);
                assert_eq!(spec_items, &items);
                assert!(selected.is_none());
            }
            _ => panic!("expected page-scoped object pill"),
        }
    }

    /// The detail-view selection is reflected in the pill and never surfaces as
    /// an app-global identity selection (page-scoped selection only).
    #[test]
    fn selected_node_is_reflected_in_page_scoped_selection() {
        let items = vec![item(1, "mn-east-01")];
        let spec = masternodes_page_nav_spec(items, Some(id(1)));
        let (scope, _) = spec.identity_pill().expect("identity pill");
        assert_eq!(scope.page_scoped_selection(), Some(id(1)));
    }
}
