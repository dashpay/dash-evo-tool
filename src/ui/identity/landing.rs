//! Landing-state resolution for the Identities hub.
//!
//! The hub chooses what to render at the top level based on how many identities are
//! loaded on the active network. See design-spec §A.4.

/// Which top-level view the hub should render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HubLanding {
    /// Zero identities on the current network. Show the onboarding empty state.
    Onboarding,
    /// Exactly one identity. Render Identity Home directly for that identity.
    Home,
    /// Two or more identities. Render the identity picker grid until the user
    /// selects one.
    Picker,
}

impl HubLanding {
    /// Decide the landing state from the loaded-identity count on the active network.
    pub fn from_identity_count(count: usize) -> Self {
        match count {
            0 => HubLanding::Onboarding,
            1 => HubLanding::Home,
            _ => HubLanding::Picker,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_identities_is_onboarding() {
        assert_eq!(HubLanding::from_identity_count(0), HubLanding::Onboarding);
    }

    #[test]
    fn one_identity_is_home() {
        assert_eq!(HubLanding::from_identity_count(1), HubLanding::Home);
    }

    #[test]
    fn two_identities_is_picker() {
        assert_eq!(HubLanding::from_identity_count(2), HubLanding::Picker);
    }

    #[test]
    fn many_identities_is_picker() {
        assert_eq!(HubLanding::from_identity_count(42), HubLanding::Picker);
    }
}
