//! Tab identity for the hub screen.

use std::fmt;

/// One of the four tabs inside `IdentityHubScreen`.
///
/// Order matters: the tab bar renders these left to right in variant order.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdentityHubTab {
    /// Identity home: hero, quick actions, checklist, recent activity.
    #[default]
    Home,
    /// Contacts: requests, active contacts, sent requests. Gated on social profile.
    Contacts,
    /// Unified activity timeline (payments · funding · platform ops).
    Activity,
    /// Identity settings: social profile, username + aliases, advanced, danger zone.
    Settings,
}

impl IdentityHubTab {
    /// All tabs in display order.
    pub const ALL: [IdentityHubTab; 4] = [
        IdentityHubTab::Home,
        IdentityHubTab::Contacts,
        IdentityHubTab::Activity,
        IdentityHubTab::Settings,
    ];

    /// Alex-facing label used in the tab bar and topbar.
    pub fn label(self) -> &'static str {
        match self {
            IdentityHubTab::Home => "Home",
            IdentityHubTab::Contacts => "Contacts",
            IdentityHubTab::Activity => "Activity",
            IdentityHubTab::Settings => "Settings",
        }
    }

    /// Accessible description used by screen readers. Always a complete sentence.
    pub fn accessible_description(self) -> &'static str {
        match self {
            IdentityHubTab::Home => "Identity home. Balance, quick actions, and recent activity.",
            IdentityHubTab::Contacts => {
                "Contacts. Received requests, your contacts, and sent requests."
            }
            IdentityHubTab::Activity => {
                "Activity. A unified timeline of payments, funding, and platform actions."
            }
            IdentityHubTab::Settings => {
                "Settings. Social profile, usernames, keys, and advanced options."
            }
        }
    }
}

impl fmt::Display for IdentityHubTab {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_in_display_order() {
        assert_eq!(IdentityHubTab::ALL[0], IdentityHubTab::Home);
        assert_eq!(IdentityHubTab::ALL[1], IdentityHubTab::Contacts);
        assert_eq!(IdentityHubTab::ALL[2], IdentityHubTab::Activity);
        assert_eq!(IdentityHubTab::ALL[3], IdentityHubTab::Settings);
    }

    #[test]
    fn labels_are_non_empty_complete_words() {
        for tab in IdentityHubTab::ALL {
            let label = tab.label();
            assert!(!label.is_empty(), "tab label must not be empty");
            assert!(
                label.chars().next().unwrap().is_ascii_uppercase(),
                "tab label '{label}' must start with an uppercase letter"
            );
        }
    }

    #[test]
    fn accessible_description_is_complete_sentence() {
        for tab in IdentityHubTab::ALL {
            let desc = tab.accessible_description();
            assert!(
                desc.ends_with('.'),
                "accessible description '{desc}' must end with a period"
            );
        }
    }

    #[test]
    fn default_is_home() {
        assert_eq!(IdentityHubTab::default(), IdentityHubTab::Home);
    }

    /// Pins the exact tab-bar label sequence consumed by `IdentityHubTabBar`.
    #[test]
    fn labels_in_display_order() {
        let labels: Vec<&str> = IdentityHubTab::ALL.iter().map(|t| t.label()).collect();
        assert_eq!(labels, ["Home", "Contacts", "Activity", "Settings"]);
    }

    #[test]
    fn display_matches_label() {
        for tab in IdentityHubTab::ALL {
            assert_eq!(format!("{tab}"), tab.label());
        }
    }
}
