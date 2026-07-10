//! The role the user is operating in — the persona / progressive-disclosure axis.
//!
//! A stateless, ordered value type: `Everyday < Power < Developer`. Feature
//! availability is monotonic in it (Invariant I1: each role is a strict
//! superset of the one below), so every role check reads as "at least X".
//! See `docs/personas/README.md` and
//! `docs/ai-design/2026-07-10-persona-capability-gating/design.md`.

/// The role the user is operating in. Ordered: each role is a strict superset
/// of the one below, so "at least Power" is `role >= UserRole::Power`.
///
/// Explicit discriminants (`= 0/1/2`) pin the on-disk / atomic encoding:
/// [`from_u8`](Self::from_u8) reads this discriminant back out of the shared
/// runtime atomic, so reordering variants must not silently change a persisted
/// or in-flight role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum UserRole {
    /// Default view — Everyday User (Alex). Balance, send/receive, DPNS, and
    /// history. Account internals and address tables stay hidden.
    #[default]
    Everyday = 0,
    /// Detailed view — Power User (Priya). Full account breakdown, address
    /// tables and derivation paths, asset-lock management, refresh controls,
    /// key export, and masternode key paths.
    Power = 1,
    /// Developer tools — Platform Developer (Jordan). Everything in Power, plus
    /// raw credits, state-transition context, Devnet config, faucet, bulk
    /// operations, and signing overrides.
    Developer = 2,
}

impl UserRole {
    /// Canonical persisted string for this role. Paired with
    /// [`from_persisted`](Self::from_persisted); these strings are the on-disk
    /// contract, so keep them stable.
    pub fn as_str(self) -> &'static str {
        match self {
            UserRole::Everyday => "Everyday",
            UserRole::Power => "Power",
            UserRole::Developer => "Developer",
        }
    }

    /// Parse a persisted role string. Returns `Some` only for the canonical
    /// strings emitted by [`as_str`](Self::as_str).
    ///
    /// The legacy `UserMode` values (`"Advanced"`, `"Beginner"`), the empty
    /// string, and anything unknown all return `None` — a deliberate sentinel
    /// meaning "no explicit role was ever recorded." `None` must NOT collapse
    /// to a concrete role here: because the retired `UserMode` defaulted to
    /// `"Advanced"` for every user, mapping that literal to a role would
    /// silently promote the entire existing user base. The caller instead
    /// seeds the initial role once from the legacy `.env DEVELOPER_MODE` flag.
    pub fn from_persisted(s: &str) -> Option<Self> {
        match s {
            "Everyday" => Some(UserRole::Everyday),
            "Power" => Some(UserRole::Power),
            "Developer" => Some(UserRole::Developer),
            _ => None,
        }
    }

    /// Short UI label for the interface-mode selectors (Settings and the
    /// onboarding row share this vocabulary so a role picked in one is findable
    /// by name in the other). Distinct from [`as_str`](Self::as_str), which is
    /// the persisted wire string and must stay stable.
    pub fn label(self) -> &'static str {
        match self {
            UserRole::Everyday => "Default view",
            UserRole::Power => "Detailed view",
            UserRole::Developer => "Developer tools",
        }
    }

    /// One-line description of what this interface mode reveals, shown under the
    /// selectors on both surfaces.
    pub fn description(self) -> &'static str {
        match self {
            UserRole::Everyday => "Shows your balance, send and receive, and usernames.",
            UserRole::Power => "Adds account details, address tables, and masternode tools.",
            UserRole::Developer => "Adds raw protocol data, Devnet, and signing overrides.",
        }
    }

    /// Whether this role is at least `min` — the monotonic availability check
    /// (Invariant I1). Anything a lower role can do, a higher role can too.
    pub fn at_least(self, min: UserRole) -> bool {
        self >= min
    }

    /// Decode the `u8` discriminant stored in the shared runtime atomic.
    /// Unknown values fall back to the baseline role, mirroring the `None`
    /// default of [`from_persisted`](Self::from_persisted).
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => UserRole::Power,
            2 => UserRole::Developer,
            _ => UserRole::Everyday,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_everyday() {
        assert_eq!(UserRole::default(), UserRole::Everyday);
    }

    #[test]
    fn strict_superset_total_order() {
        // Invariant I1: Everyday < Power < Developer.
        assert!(UserRole::Everyday < UserRole::Power);
        assert!(UserRole::Power < UserRole::Developer);
        assert!(UserRole::Everyday < UserRole::Developer);
    }

    #[test]
    fn at_least_is_monotonic() {
        assert!(UserRole::Developer.at_least(UserRole::Power));
        assert!(UserRole::Power.at_least(UserRole::Power));
        assert!(!UserRole::Everyday.at_least(UserRole::Power));
        assert!(UserRole::Everyday.at_least(UserRole::Everyday));
        assert!(!UserRole::Power.at_least(UserRole::Developer));
    }

    #[test]
    fn canonical_strings_round_trip() {
        for role in [UserRole::Everyday, UserRole::Power, UserRole::Developer] {
            assert_eq!(UserRole::from_persisted(role.as_str()), Some(role));
        }
    }

    #[test]
    fn legacy_and_unknown_strings_are_sentinels() {
        // The retired `UserMode` variants and any garbage must decode to
        // `None` so they defer to the `.env` seed rather than mis-promoting.
        for s in ["Advanced", "Beginner", "", "developer", "power", "42"] {
            assert_eq!(
                UserRole::from_persisted(s),
                None,
                "'{s}' must be treated as a legacy sentinel, not a role"
            );
        }
    }

    #[test]
    fn labels_and_descriptions_are_distinct_per_role() {
        let roles = [UserRole::Everyday, UserRole::Power, UserRole::Developer];
        let labels: Vec<_> = roles.iter().map(|r| r.label()).collect();
        assert_eq!(labels, ["Default view", "Detailed view", "Developer tools"]);
        // Every role has a non-empty description and no two share one.
        for (i, a) in roles.iter().enumerate() {
            assert!(!a.description().is_empty());
            for b in &roles[i + 1..] {
                assert_ne!(a.description(), b.description());
            }
        }
    }

    #[test]
    fn u8_discriminant_round_trips() {
        for role in [UserRole::Everyday, UserRole::Power, UserRole::Developer] {
            assert_eq!(UserRole::from_u8(role as u8), role);
        }
        // Unknown discriminants fall back to the baseline role.
        assert_eq!(UserRole::from_u8(99), UserRole::Everyday);
    }
}
