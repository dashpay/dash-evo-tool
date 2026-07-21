//! Page-scoped global-nav model: which breadcrumb pills a root page composes,
//! how each pill participates, and the page-scoped object selection kept
//! distinct from the app-global user-identity selection.
//!
//! Renders nothing (module-placement discriminator → `ui/state`). The
//! `global_nav_switcher` component reads a [`PageNavSpec`] and renders it.
//!
//! FR-6 boundary: [`IdentityPillScope::PageScopedObject`] carries its own
//! selection and never writes `AppContext::selected_identity_id` — the switcher
//! maps it to a distinct `SelectPageObject` effect, not `SelectIdentity`.

use crate::model::wallet_association::WalletAssociation;
use crate::ui::RootScreenType;
use dash_sdk::platform::Identifier;

/// One selectable object in a page-scoped pill dropdown, e.g. a loaded
/// masternode/evonode identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageObjectItem {
    /// Identity id of the object (the masternode/evonode identity).
    pub id: Identifier,
    /// Display label for the dropdown row and the pill when this item is active.
    pub label: String,
    /// Type glyph shown ahead of the label, e.g. `HeroIdentityKind::type_glyph`.
    /// `None` renders the label alone.
    pub icon: Option<String>,
}

/// How a breadcrumb pill participates on a given page (FR-GLOBAL-NAV-2 rule 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PillConsumption {
    /// The page consumes this selection — render interactive, two-way bound.
    Consumed,
    /// Not yet wired on this page — render subdued (dimmed, no caret, no text
    /// tag) with a hover tooltip telling the user how to change the selection.
    Unwired { tooltip: String },
}

impl PillConsumption {
    /// Whether the page consumes this selection (interactive pill).
    pub fn is_consumed(&self) -> bool {
        matches!(self, Self::Consumed)
    }

    /// The how-to-change tooltip for an unwired pill; `None` when consumed.
    pub fn tooltip(&self) -> Option<&str> {
        match self {
            Self::Unwired { tooltip } => Some(tooltip),
            Self::Consumed => None,
        }
    }
}

/// What the third breadcrumb pill represents on a page (FR-GLOBAL-NAV-3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityPillScope {
    /// The app-global User identity (`AppContext::selected_identity_id`). The
    /// switcher reads/writes the app-scoped selection for this variant.
    AppGlobalUser,
    /// A page-scoped object in view (the masternode/evonode on the Masternodes
    /// page). This variant **never** writes `AppContext::selected_identity_id`;
    /// the switcher maps its selection to `SelectPageObject`. This is the
    /// structural FR-6 boundary in code.
    PageScopedObject {
        /// Label shown when nothing is selected, e.g. `(no masternode yet)`.
        placeholder: String,
        /// Hover tooltip for the interactive pill. Page-owned copy, so the
        /// switcher stays free of page-specific wording.
        tooltip: String,
        /// The objects the dropdown offers.
        items: Vec<PageObjectItem>,
        /// The currently selected object, if any.
        selected: Option<Identifier>,
    },
}

impl IdentityPillScope {
    /// Build a page-scoped object scope from its page-owned copy and items.
    pub fn page_scoped_object(
        placeholder: impl Into<String>,
        tooltip: impl Into<String>,
        items: Vec<PageObjectItem>,
        selected: Option<Identifier>,
    ) -> Self {
        Self::PageScopedObject {
            placeholder: placeholder.into(),
            tooltip: tooltip.into(),
            items,
            selected,
        }
    }

    /// Whether this scope is page-scoped (and therefore never touches the
    /// app-global identity selection).
    pub fn is_page_scoped(&self) -> bool {
        matches!(self, Self::PageScopedObject { .. })
    }

    /// The page-scoped selection, if any. Always `None` for [`AppGlobalUser`],
    /// which resolves its selection from `AppContext`, not from this scope — a
    /// page-scoped object is never the app-global identity (FR-6 boundary).
    ///
    /// [`AppGlobalUser`]: IdentityPillScope::AppGlobalUser
    pub fn page_scoped_selection(&self) -> Option<Identifier> {
        match self {
            Self::PageScopedObject { selected, .. } => *selected,
            Self::AppGlobalUser => None,
        }
    }
}

/// Per-page global-nav composition: the page-aware segment-1 (label + link
/// target) and which pills the page shows / how each participates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageNavSpec {
    segment1_label: String,
    segment1_target: RootScreenType,
    wallet_pill: Option<PillConsumption>,
    identity_pill: Option<(IdentityPillScope, PillConsumption)>,
    /// A fixed, read-only wallet association for the object in view (e.g. the
    /// masternode on the Masternodes page), with its page-owned tooltip. When
    /// set, the switcher renders this in the wallet segment instead of the
    /// app-scoped [`wallet_pill`](Self::wallet_pill): a wallet-less object shows
    /// the "not in a wallet" indicator rather than an arbitrary loaded wallet.
    object_wallet: Option<(WalletAssociation, String)>,
}

impl PageNavSpec {
    /// A spec with the given page-aware segment-1 and no pills yet — compose
    /// pills via [`with_wallet_pill`](Self::with_wallet_pill) and
    /// [`with_identity_pill`](Self::with_identity_pill).
    pub fn new(segment1_label: impl Into<String>, segment1_target: RootScreenType) -> Self {
        Self {
            segment1_label: segment1_label.into(),
            segment1_target,
            wallet_pill: None,
            identity_pill: None,
            object_wallet: None,
        }
    }

    /// A fully-unwired everyday-page spec (the Phase-A rollout default): both
    /// the wallet pill and the app-global identity pill render subdued with
    /// how-to-change tooltips. Pages with no identity context use
    /// [`new`](Self::new) + [`with_wallet_pill`](Self::with_wallet_pill) alone.
    pub fn unwired_everyday(
        segment1_label: impl Into<String>,
        segment1_target: RootScreenType,
        wallet_tooltip: impl Into<String>,
        identity_tooltip: impl Into<String>,
    ) -> Self {
        Self::new(segment1_label, segment1_target)
            .with_wallet_pill(PillConsumption::Unwired {
                tooltip: wallet_tooltip.into(),
            })
            .with_identity_pill(
                IdentityPillScope::AppGlobalUser,
                PillConsumption::Unwired {
                    tooltip: identity_tooltip.into(),
                },
            )
    }

    /// Add the wallet pill with the given participation mode.
    pub fn with_wallet_pill(mut self, consumption: PillConsumption) -> Self {
        self.wallet_pill = Some(consumption);
        self
    }

    /// Add the third (identity/object) pill with its scope and participation.
    pub fn with_identity_pill(
        mut self,
        scope: IdentityPillScope,
        consumption: PillConsumption,
    ) -> Self {
        self.identity_pill = Some((scope, consumption));
        self
    }

    /// Show a fixed, read-only wallet association for the object in view instead
    /// of the app-scoped wallet pill. `tooltip` is the page-owned copy shown on
    /// hover. Overrides any [`with_wallet_pill`](Self::with_wallet_pill) in the
    /// wallet segment.
    pub fn with_object_wallet(
        mut self,
        association: WalletAssociation,
        tooltip: impl Into<String>,
    ) -> Self {
        self.object_wallet = Some((association, tooltip.into()));
        self
    }

    /// The page-aware segment-1 label (e.g. `Masternodes`).
    pub fn segment1_label(&self) -> &str {
        &self.segment1_label
    }

    /// The root screen segment-1 links to.
    pub fn segment1_target(&self) -> RootScreenType {
        self.segment1_target
    }

    /// The wallet-pill participation, or `None` if the page shows no wallet pill.
    pub fn wallet_pill(&self) -> Option<&PillConsumption> {
        self.wallet_pill.as_ref()
    }

    /// The fixed, read-only wallet association for the object in view (with its
    /// page-owned tooltip), or `None` when the wallet segment is app-scoped.
    pub fn object_wallet(&self) -> Option<&(WalletAssociation, String)> {
        self.object_wallet.as_ref()
    }

    /// The third-pill scope + participation, or `None` if the page shows no
    /// identity/object pill (per-page composition — FR-GLOBAL-NAV-2 rule 4).
    pub fn identity_pill(&self) -> Option<&(IdentityPillScope, PillConsumption)> {
        self.identity_pill.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> Identifier {
        Identifier::new([byte; 32])
    }

    /// Foundation for TC-NAV-01 — segment-1 is page-driven (label + target).
    #[test]
    fn segment1_is_page_aware() {
        let spec = PageNavSpec::new("Masternodes", RootScreenType::RootScreenIdentityHub);
        assert_eq!(spec.segment1_label(), "Masternodes");
        assert_eq!(
            spec.segment1_target(),
            RootScreenType::RootScreenIdentityHub
        );
    }

    /// TC-NAV-15 — a page with no identity/object context shows only the wallet
    /// pill; there is no third segment at all.
    #[test]
    fn wallet_only_page_has_no_identity_pill() {
        let spec = PageNavSpec::new("Wallets", RootScreenType::RootScreenWalletsBalances)
            .with_wallet_pill(PillConsumption::Consumed);
        assert!(spec.wallet_pill().is_some());
        assert!(spec.identity_pill().is_none());
    }

    /// TC-NAV-13 / TC-NAV-14 — an unwired pill's participation carries a
    /// non-empty how-to-change tooltip; a consumed pill carries none.
    #[test]
    fn unwired_pill_carries_nonempty_tooltip() {
        let unwired = PillConsumption::Unwired {
            tooltip: "Change the active wallet from the Wallets tab.".to_string(),
        };
        assert!(!unwired.is_consumed());
        assert_eq!(
            unwired.tooltip(),
            Some("Change the active wallet from the Wallets tab.")
        );

        let consumed = PillConsumption::Consumed;
        assert!(consumed.is_consumed());
        assert!(consumed.tooltip().is_none());
    }

    /// TC-FR6-07 foundation — the page-scoped object selection is isolated from
    /// the app-global identity: `PageScopedObject` exposes its own selection,
    /// `AppGlobalUser` never does.
    #[test]
    fn page_scoped_object_is_isolated_from_app_global() {
        let scope = IdentityPillScope::page_scoped_object(
            "(no masternode yet)",
            "Switch between your loaded masternodes and evonodes.",
            vec![PageObjectItem {
                id: id(7),
                label: "mn-east-01".to_string(),
                icon: Some("🖥".to_string()),
            }],
            Some(id(7)),
        );
        assert!(scope.is_page_scoped());
        assert_eq!(scope.page_scoped_selection(), Some(id(7)));

        let app_global = IdentityPillScope::AppGlobalUser;
        assert!(!app_global.is_page_scoped());
        assert_eq!(app_global.page_scoped_selection(), None);
    }

    /// The Phase-A rollout default composes both pills subdued with tooltips.
    #[test]
    fn unwired_everyday_composes_both_subdued() {
        let spec = PageNavSpec::unwired_everyday(
            "Contracts",
            RootScreenType::RootScreenDocumentQuery,
            "Change the active wallet from the Wallets tab.",
            "Change the active identity from the Identity Hub.",
        );
        let wallet = spec.wallet_pill().expect("wallet pill present");
        assert!(!wallet.is_consumed());
        assert!(wallet.tooltip().is_some_and(|t| !t.is_empty()));

        let (scope, consumption) = spec.identity_pill().expect("identity pill present");
        assert_eq!(*scope, IdentityPillScope::AppGlobalUser);
        assert!(!consumption.is_consumed());
        assert!(consumption.tooltip().is_some_and(|t| !t.is_empty()));
    }
}
