//! Page-scoped view-model for the Masternodes global-nav breadcrumb (B7).
//!
//! Builds the Masternodes page's [`PageNavSpec`]: a page-aware `Masternodes`
//! segment-1, an **interactive** wallet pill (funds Top up — FR-9) and an
//! **interactive** page-scoped node pill (`🖥 mn-east-01 ›` — FR-GLOBAL-NAV-3),
//! two-way bound with the card grid and the detail view. Renders nothing
//! (module-placement discriminator → `ui/state`).
//!
//! FR-6 boundary: the node pill carries a **page-scoped** selection, distinct
//! from the app-global user identity. The switcher maps it to
//! `GlobalNavEffect::SelectPageObject`, never `SelectIdentity`, so a masternode
//! can never become the app-global identity, nor surface in the everyday-user
//! identity picker. The boundary is additionally enforced at the resolution
//! layer (B1).

use dash_sdk::platform::Identifier;

use crate::model::qualified_identity::IdentityType;
use crate::model::wallet_association::WalletAssociation;
use crate::ui::RootScreenType;
use crate::ui::identity::identity_hero_card::HeroIdentityKind;
use crate::ui::masternodes::card::card_heading;
use crate::ui::state::global_nav::{
    IdentityPillScope, PageNavSpec, PageObjectItem, PillConsumption,
};

/// Node-pill label when no node is loaded yet.
const NO_NODES_PLACEHOLDER: &str = "(no masternode yet)";
/// Node-pill label when nodes are loaded but none is open.
const NO_NODE_SELECTED_PLACEHOLDER: &str = "(choose a masternode)";
/// Hover tooltip for the interactive node pill.
const TT_NODE_PILL: &str = "Switch between your loaded masternodes and evonodes.";
/// Hover tooltip for the read-only wallet indicator on the Masternodes page,
/// explaining why no wallet is shown for the node.
const TT_NODE_WALLET: &str =
    "This masternode's keys aren't controlled by any wallet loaded in this app.";

/// Dropdown label for a loaded node: its alias when set, otherwise the
/// shortened ProTxHash — the same heading rule as its card, so the pill and the
/// grid always name a node identically.
pub fn node_pill_item(
    node_id: Identifier,
    alias: Option<&str>,
    node_id_short: &str,
    node_type: IdentityType,
) -> PageObjectItem {
    let kind: HeroIdentityKind = node_type.into();
    PageObjectItem {
        id: node_id,
        label: card_heading(alias, node_id_short),
        icon: Some(kind.type_glyph().to_string()),
    }
}

/// The node-pill placeholder: nothing loaded yet vs. loaded but none open.
fn node_placeholder(node_count: usize) -> &'static str {
    if node_count == 0 {
        NO_NODES_PLACEHOLDER
    } else {
        NO_NODE_SELECTED_PLACEHOLDER
    }
}

/// Build the Masternodes page's global-nav spec: page-aware segment-1, a
/// read-only wallet indicator (`wallet_association`) for the node in view, and
/// an interactive node pill listing `items` with `selected` (the node whose
/// detail view is open) shown on the pill.
///
/// The wallet segment is a fixed, read-only indicator rather than an interactive
/// pill: a masternode/evonode is loaded from raw owner/voting/payout keys, so no
/// wallet on this device controls it — showing an arbitrary loaded wallet there
/// would falsely imply ownership.
pub fn masternodes_page_nav_spec(
    items: Vec<PageObjectItem>,
    selected: Option<Identifier>,
    wallet_association: WalletAssociation,
) -> PageNavSpec {
    let placeholder = node_placeholder(items.len());
    PageNavSpec::new("Masternodes", RootScreenType::RootScreenMasternodes)
        .with_object_wallet(wallet_association, TT_NODE_WALLET)
        .with_identity_pill(
            IdentityPillScope::page_scoped_object(placeholder, TT_NODE_PILL, items, selected),
            PillConsumption::Consumed,
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> Identifier {
        Identifier::new([byte; 32])
    }

    /// The Masternodes breadcrumb carries segment-1, a read-only wallet
    /// indicator (never an interactive wallet pill — masternodes are wallet-less)
    /// and an interactive page-scoped node pill carrying the node in view
    /// (FR-GLOBAL-NAV-3).
    #[test]
    fn spec_has_segment1_readonly_wallet_and_interactive_node_pill() {
        let items = vec![node_pill_item(
            id(7),
            Some("mn-east-01"),
            "abcd…ef01",
            IdentityType::Masternode,
        )];
        let spec = masternodes_page_nav_spec(items, Some(id(7)), WalletAssociation::NotInWallet);

        assert_eq!(spec.segment1_label(), "Masternodes");
        assert_eq!(
            spec.segment1_target(),
            RootScreenType::RootScreenMasternodes
        );
        // The wallet segment is a read-only association, not an interactive pill.
        assert!(
            spec.wallet_pill().is_none(),
            "the Masternodes page shows no interactive wallet pill"
        );
        let (assoc, tooltip) = spec.object_wallet().expect("read-only wallet indicator");
        assert_eq!(*assoc, WalletAssociation::NotInWallet);
        assert_eq!(tooltip, TT_NODE_WALLET);

        let (scope, consumption) = spec.identity_pill().expect("node pill");
        assert!(consumption.is_consumed(), "the node pill is interactive");
        assert!(
            scope.is_page_scoped(),
            "the node pill is page-scoped, never the app-global identity",
        );
        assert_eq!(scope.page_scoped_selection(), Some(id(7)));
    }

    /// The placeholder distinguishes "none loaded" from "none open", mirroring
    /// the identity pill's `(no identity yet)` / `(choose an identity)` rule.
    #[test]
    fn placeholder_distinguishes_no_nodes_from_none_selected() {
        assert_eq!(node_placeholder(0), "(no masternode yet)");
        assert_eq!(node_placeholder(3), "(choose a masternode)");
    }

    /// A node's pill label follows its card heading (alias, else shortened
    /// ProTxHash) and its glyph follows its type.
    #[test]
    fn node_item_label_follows_card_heading_and_glyph_follows_type() {
        let aliased = node_pill_item(
            id(1),
            Some("mn-east-01"),
            "abcd…ef01",
            IdentityType::Masternode,
        );
        assert_eq!(aliased.label, "mn-east-01");
        assert_eq!(aliased.icon.as_deref(), Some("\u{1F5A5}"));

        let anonymous = node_pill_item(id(2), None, "abcd…ef01", IdentityType::Masternode);
        assert_eq!(anonymous.label, "abcd…ef01");

        let evonode = node_pill_item(id(3), None, "beef…cafe", IdentityType::Evonode);
        assert_eq!(evonode.icon.as_deref(), Some("\u{25C6}"));
    }
}
