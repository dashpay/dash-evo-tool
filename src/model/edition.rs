//! The build-time application **edition** — which surfaces the compiled binary
//! exposes. Selected once at compile time (the `masternode-owner-edition` Cargo
//! feature) and read through [`Edition::CURRENT`].
//!
//! An edition is a *UX restriction*, not a security or binary-size boundary: the
//! Cargo feature does not dead-code-eliminate the hidden screens (they stay
//! enum-reachable and compiled), and hiding a screen does not stop the
//! background subsystems (shielded coordinator, event bridge, identity sweeps)
//! that boot regardless of navigation. The edition only decides which root
//! screens are *reachable* through the navigation.
//!
//! The policy is a single pure predicate — [`Edition::allows`] — over
//! [`RootScreenType`], composed with the user's [`UserRole`] at the enforcement
//! sites via [`Edition::permits`] (the Developer role is a full escape hatch).
//! Design: `docs/ai-design/2026-07-13-masternode-owner-edition/design.md`.

use crate::model::settings::RootScreenType;
use crate::model::user_role::UserRole;

/// Which edition this binary is. Chosen at compile time; read via
/// [`Edition::CURRENT`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Edition {
    /// The complete application — every screen is reachable (gated only by the
    /// role/capability system, never by the edition).
    Full,
    /// A stripped-down build for masternode owners performing an urgent upgrade:
    /// only the Masternodes screen and Settings are reachable. Everything else
    /// is hidden from navigation.
    MasternodeOwner,
}

impl Edition {
    /// The edition this binary was compiled as.
    #[cfg(feature = "masternode-owner-edition")]
    pub const CURRENT: Edition = Edition::MasternodeOwner;

    /// The edition this binary was compiled as.
    #[cfg(not(feature = "masternode-owner-edition"))]
    pub const CURRENT: Edition = Edition::Full;

    /// Whether this edition exposes `screen` at all — the pure edition policy,
    /// independent of the user's role. [`Edition::Full`] exposes everything;
    /// [`Edition::MasternodeOwner`] exposes only the Masternodes screen and
    /// Settings (the network chooser).
    ///
    /// This is the single source of truth for the edition's scope. It carries no
    /// role logic on purpose: the Developer escape hatch is applied by
    /// [`permits`](Self::permits), keeping this predicate pure and unit-testable.
    pub fn allows(self, screen: RootScreenType) -> bool {
        match self {
            Edition::Full => true,
            Edition::MasternodeOwner => matches!(
                screen,
                RootScreenType::RootScreenMasternodes | RootScreenType::RootScreenNetworkChooser
            ),
        }
    }

    /// Whether `screen` is reachable given this edition **and** the user's
    /// `role`. The Developer role is a full escape hatch: it lifts the edition
    /// restriction entirely, so every screen becomes reachable again (subject to
    /// its own feature gate, evaluated separately at the callsite).
    pub fn permits(self, screen: RootScreenType, role: UserRole) -> bool {
        self.allows(screen) || role.at_least(UserRole::Developer)
    }

    /// The screen this edition prefers to land on when a persisted or requested
    /// target is not reachable — before falling back to
    /// [`always_reachable_screen`](Self::always_reachable_screen).
    pub fn home_screen(self) -> RootScreenType {
        match self {
            Edition::Full => RootScreenType::RootScreenIdentities,
            Edition::MasternodeOwner => RootScreenType::RootScreenMasternodes,
        }
    }

    /// A screen this edition **always** permits and that carries no further
    /// feature gate — a guaranteed-reachable floor for the clamp so navigation
    /// can never strand the user on a hidden screen. Settings stays reachable in
    /// the masternode-owner edition precisely so a user who has dropped below
    /// Power can still get back to the role selector.
    pub fn always_reachable_screen(self) -> RootScreenType {
        match self {
            Edition::Full => RootScreenType::RootScreenIdentities,
            Edition::MasternodeOwner => RootScreenType::RootScreenNetworkChooser,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every root screen there is — kept in sync with `RootScreenType::from_int`
    /// so the policy is exercised against the whole surface, not a sample.
    fn all_root_screens() -> Vec<RootScreenType> {
        (0..=64).filter_map(RootScreenType::from_int).collect()
    }

    #[test]
    fn full_edition_allows_every_screen() {
        for screen in all_root_screens() {
            assert!(
                Edition::Full.allows(screen),
                "Full edition must expose {screen:?}"
            );
        }
    }

    #[test]
    fn masternode_owner_allows_only_masternodes_and_settings() {
        for screen in all_root_screens() {
            let expected = matches!(
                screen,
                RootScreenType::RootScreenMasternodes | RootScreenType::RootScreenNetworkChooser
            );
            assert_eq!(
                Edition::MasternodeOwner.allows(screen),
                expected,
                "MasternodeOwner edition scope for {screen:?}"
            );
        }
    }

    #[test]
    fn developer_role_is_a_full_escape_hatch() {
        // Below Developer: only the edition's own scope is permitted.
        for role in [UserRole::Everyday, UserRole::Power] {
            assert!(
                !Edition::MasternodeOwner.permits(RootScreenType::RootScreenWalletsBalances, role),
                "a hidden screen must stay hidden at {role:?}"
            );
        }
        // Developer lifts the restriction entirely.
        for screen in all_root_screens() {
            assert!(
                Edition::MasternodeOwner.permits(screen, UserRole::Developer),
                "Developer must reach {screen:?} (escape hatch)"
            );
        }
    }

    #[test]
    fn permits_keeps_the_edition_scope_below_developer() {
        // The two in-scope screens are permitted at any role; the escape hatch is
        // not the only way to reach them.
        for role in [UserRole::Everyday, UserRole::Power, UserRole::Developer] {
            assert!(Edition::MasternodeOwner.permits(RootScreenType::RootScreenMasternodes, role));
            assert!(
                Edition::MasternodeOwner.permits(RootScreenType::RootScreenNetworkChooser, role)
            );
        }
    }

    #[test]
    fn full_edition_permits_everything_regardless_of_role() {
        for screen in all_root_screens() {
            assert!(Edition::Full.permits(screen, UserRole::Everyday));
        }
    }

    #[test]
    fn fallback_screens_are_always_reachable_in_their_edition() {
        // The always-reachable floor must be in the edition's own `allows` scope
        // (so the escape hatch is never load-bearing for the fallback) at every
        // role, including the lowest.
        for edition in [Edition::Full, Edition::MasternodeOwner] {
            let floor = edition.always_reachable_screen();
            assert!(
                edition.allows(floor),
                "{edition:?} floor {floor:?} must be in the edition scope"
            );
            assert!(
                edition.permits(floor, UserRole::Everyday),
                "{edition:?} floor {floor:?} must be reachable at the lowest role"
            );
        }
    }

    #[test]
    fn current_edition_matches_the_compiled_feature() {
        #[cfg(feature = "masternode-owner-edition")]
        assert_eq!(Edition::CURRENT, Edition::MasternodeOwner);
        #[cfg(not(feature = "masternode-owner-edition"))]
        assert_eq!(Edition::CURRENT, Edition::Full);
    }
}
