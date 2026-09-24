//! Identities-hub view state that renders nothing.
//!
//! The active identity itself is app-scoped (`AppContext::selected_identity_id`),
//! so this holds only the within-session view concerns the breadcrumb switcher
//! owns: whether the user explicitly opened the picker, and the inline-search
//! buffers for the wallet / identity dropdowns. Per the module-placement
//! discriminator (renders nothing → `ui/state`).

/// Which hub surface to render, derived from the loaded-identity count, the
/// explicit app-scoped selection, and the picker override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HubView {
    /// No identities loaded — first-run onboarding.
    Onboarding,
    /// Identity picker grid (choose-an-identity).
    Picker,
    /// A single identity's home (the resolved active identity).
    Home,
}

/// Resolve the hub surface. `has_explicit_active` is whether the app-scoped
/// selection points at a still-loaded identity (the *raw* selection, not the
/// first-identity fallback); `picker_override` is set when the user clicked the
/// `Identities` crumb to deliberately open the picker.
pub fn effective_view(
    loaded_count: usize,
    has_explicit_active: bool,
    picker_override: bool,
) -> HubView {
    if loaded_count == 0 {
        HubView::Onboarding
    } else if picker_override {
        HubView::Picker
    } else if has_explicit_active || loaded_count == 1 {
        // Explicit choice, or a lone identity to auto-select.
        HubView::Home
    } else {
        // Two or more identities and none chosen — choose one first.
        HubView::Picker
    }
}

/// Within-session hub view state. Renders nothing.
#[derive(Default)]
pub struct HubSelection {
    /// The user explicitly opened the picker (via the `Identities` crumb) even
    /// though an identity is active. Cleared when an identity is chosen.
    picker_override: bool,
    /// Inline-search buffer for the wallet dropdown; live only while open.
    wallet_search: String,
    /// Inline-search buffer for the identity dropdown; live only while open.
    identity_search: String,
}

impl HubSelection {
    /// Whether the user has explicitly opened the picker.
    pub fn picker_override(&self) -> bool {
        self.picker_override
    }

    /// Open the picker (the `Identities` crumb click).
    pub fn open_picker(&mut self) {
        self.picker_override = true;
    }

    /// Clear the picker override — call when an identity is chosen.
    pub fn clear_picker_override(&mut self) {
        self.picker_override = false;
    }

    pub fn wallet_search_mut(&mut self) -> &mut String {
        &mut self.wallet_search
    }

    pub fn identity_search_mut(&mut self) -> &mut String {
        &mut self.identity_search
    }

    pub fn wallet_search(&self) -> &str {
        &self.wallet_search
    }

    pub fn identity_search(&self) -> &str {
        &self.identity_search
    }

    /// Clear both inline-search buffers (on popup close).
    pub fn clear_searches(&mut self) {
        self.wallet_search.clear();
        self.identity_search.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_picker_sets_and_clear_resets_override() {
        let mut s = HubSelection::default();
        assert!(!s.picker_override());
        s.open_picker();
        assert!(s.picker_override());
        s.clear_picker_override();
        assert!(!s.picker_override());
    }

    #[test]
    fn clear_searches_empties_both_buffers() {
        let mut s = HubSelection::default();
        s.wallet_search_mut().push_str("abc");
        s.identity_search_mut().push_str("xyz");
        s.clear_searches();
        assert!(s.wallet_search().is_empty());
        assert!(s.identity_search().is_empty());
    }

    #[test]
    fn effective_view_state_machine() {
        // No identities → onboarding regardless of the other inputs.
        assert_eq!(effective_view(0, false, false), HubView::Onboarding);
        assert_eq!(effective_view(0, true, true), HubView::Onboarding);
        // Explicit picker override wins when identities exist.
        assert_eq!(effective_view(1, true, true), HubView::Picker);
        assert_eq!(effective_view(3, true, true), HubView::Picker);
        // One identity, none explicitly chosen → auto Home.
        assert_eq!(effective_view(1, false, false), HubView::Home);
        // An explicit active identity → Home.
        assert_eq!(effective_view(3, true, false), HubView::Home);
        // Two or more, none chosen → Picker.
        assert_eq!(effective_view(2, false, false), HubView::Picker);
    }
}
