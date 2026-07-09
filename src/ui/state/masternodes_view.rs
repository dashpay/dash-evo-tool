//! Page-scoped view-model for the Masternodes global-nav breadcrumb (B7).
//!
//! Builds the Masternodes page's [`PageNavSpec`]: a page-aware `Masternodes`
//! segment-1 and an **interactive** wallet pill (funds Top up — FR-9).
//!
//! The page deliberately carries **no** object/identity pill. Masternode and
//! evonode identities are never wallet-linked (`wallet_info` is always `None`
//! for them — locked decision #4). The breadcrumb's "wallet pill + object pill"
//! pairing expresses a genuine wallet↔identity relationship elsewhere in the
//! app (a wallet switch can reconcile a User identity); applying it here would
//! falsely imply a wallet↔masternode relationship that does not exist. Node
//! selection is driven entirely by card-click → detail and the detail /
//! load-form `‹ All masternodes` back link. Renders nothing (module-placement
//! discriminator → `ui/state`).
//!
//! The FR-6 boundary (a masternode never becoming the app-global identity) is
//! enforced structurally at the resolution layer (B1), independent of whether
//! any pill renders in this breadcrumb.

use crate::ui::RootScreenType;
use crate::ui::state::global_nav::{PageNavSpec, PillConsumption};

/// Build the Masternodes page's global-nav spec: a page-aware `Masternodes`
/// segment-1 plus an interactive wallet pill. No object/identity pill — see the
/// module docs for why (locked decision #4).
pub fn masternodes_page_nav_spec() -> PageNavSpec {
    PageNavSpec::new("Masternodes", RootScreenType::RootScreenMasternodes)
        .with_wallet_pill(PillConsumption::Consumed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Masternodes breadcrumb exposes segment-1 and an interactive wallet
    /// pill, and — deliberately — NO object/identity pill (locked decision #4:
    /// masternodes are never wallet-linked, so a wallet↔object pairing would
    /// misrepresent the relationship). Card-click/detail drive node selection.
    #[test]
    fn spec_has_segment1_and_wallet_pill_but_no_object_pill() {
        let spec = masternodes_page_nav_spec();
        assert_eq!(spec.segment1_label(), "Masternodes");
        assert_eq!(
            spec.segment1_target(),
            RootScreenType::RootScreenMasternodes
        );
        // Wallet pill interactive (FR-9 Top up).
        assert!(spec.wallet_pill().expect("wallet pill").is_consumed());
        // No object/identity pill on this page.
        assert!(
            spec.identity_pill().is_none(),
            "the Masternodes breadcrumb must carry no object/identity pill",
        );
    }
}
