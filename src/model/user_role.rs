//! The role the user is operating in — the persona / progressive-disclosure axis.
//!
//! A stateless, ordered value type: `Everyday < Power < Developer`. Feature
//! availability is monotonic in it (Invariant I1: each role is a strict
//! superset of the one below), so every role check reads as "at least X".
//! See `docs/personas/README.md` and
//! `docs/ai-design/2026-07-10-persona-capability-gating/design.md`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// The role the user is operating in. Ordered: each role is a strict superset
/// of the one below, so "at least Power" is `role >= UserRole::Power`.
///
/// Explicit discriminants (`= 0/1/2`) pin the on-disk encoding and the one
/// [`UserRoleCell`] stores, so reordering variants must not silently change a
/// persisted or in-flight role.
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
    /// The role of an account that never recorded one — a fresh install, or a
    /// settings blob written before the role field existed.
    ///
    /// Deliberately not [`Default::default`] (Everyday): those accounts come from
    /// builds that exposed the power-user surface unconditionally, so starting
    /// them anywhere lower would read as lost functionality. Everyday is a choice
    /// the user opts *into*.
    ///
    /// Applies only when the stored role is *known* to be absent. A role that
    /// could not be **read** is a different case entirely — see
    /// [`LEAST_PRIVILEGED`](Self::LEAST_PRIVILEGED).
    pub const WHEN_UNSET: UserRole = UserRole::Power;

    /// The role to fall back to when the stored one cannot be trusted — an
    /// unreadable settings store, or a role encoding that does not decode.
    ///
    /// The opposite decision from [`WHEN_UNSET`](Self::WHEN_UNSET), and
    /// deliberately so: "the user never chose a role" is a fact, and Power is the
    /// right answer to it; "we do not know which role the user chose" is an
    /// absence of facts, and answering it with Power would let a transient glitch
    /// grant capability the user may never have had. Unknown resolves down.
    pub const LEAST_PRIVILEGED: UserRole = UserRole::Everyday;

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
    /// meaning "no explicit role was ever recorded." `None` must NOT collapse to
    /// a concrete role here: the caller resolves it to [`WHEN_UNSET`](Self::WHEN_UNSET),
    /// and folding that decision into the decoder would make an unknown string
    /// indistinguishable from a deliberate choice.
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

    /// Decode the `u8` discriminant [`UserRoleCell`] stores. Unknown values fall
    /// back to [`LEAST_PRIVILEGED`](Self::LEAST_PRIVILEGED) — a value that cannot
    /// be trusted must not grant capability.
    const fn from_u8(v: u8) -> Self {
        match v {
            1 => UserRole::Power,
            2 => UserRole::Developer,
            _ => UserRole::LEAST_PRIVILEGED,
        }
    }
}

/// The app-global [`UserRole`], shared by every per-network `AppContext`.
///
/// Clone it to wire a sibling context to the *same* role: every clone reads and
/// writes one shared value, so a role set through any handle is immediately
/// observed by all of them — never a per-context copy, which would let a feature
/// gate render from a stale value. Cheap to clone and safe to share across
/// threads; the atomic encoding is an implementation detail.
///
/// ```
/// # use dash_evo_tool::model::user_role::{UserRole, UserRoleCell};
/// let cell = UserRoleCell::new(UserRole::Everyday);
/// let sibling = cell.clone();
/// cell.set(UserRole::Developer);
/// assert_eq!(sibling.get(), UserRole::Developer);
/// ```
#[derive(Debug, Clone)]
pub struct UserRoleCell(Arc<AtomicU8>);

impl UserRoleCell {
    /// A cell holding `role`, shared by every clone of the returned handle.
    pub fn new(role: UserRole) -> Self {
        Self(Arc::new(AtomicU8::new(role as u8)))
    }

    /// The role currently held.
    pub fn get(&self) -> UserRole {
        UserRole::from_u8(self.0.load(Ordering::Relaxed))
    }

    /// Publish `role` to every holder of this cell.
    pub fn set(&self, role: UserRole) {
        self.0.store(role as u8, Ordering::Relaxed);
    }
}

impl Default for UserRoleCell {
    /// A cell at [`UserRole::default`] — the pre-seed value at boot, before the
    /// persisted settings are read.
    fn default() -> Self {
        Self::new(UserRole::default())
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

    /// The two fallbacks answer different questions and must not converge: a
    /// *known* absence of a role opts into the power surface (`WHEN_UNSET`),
    /// whereas an *unknown* role grants the least of it (`LEAST_PRIVILEGED`).
    /// Collapsing them is the bug this pair exists to prevent — a settings read
    /// that merely failed would otherwise elevate the session.
    #[test]
    fn unknown_and_unset_roles_resolve_in_opposite_directions() {
        assert_eq!(
            UserRole::LEAST_PRIVILEGED,
            [UserRole::Everyday, UserRole::Power, UserRole::Developer]
                .into_iter()
                .min()
                .expect("the role order has a floor"),
            "the untrusted-value fallback must be the lowest role there is"
        );
        assert!(
            UserRole::WHEN_UNSET.at_least(UserRole::LEAST_PRIVILEGED)
                && UserRole::WHEN_UNSET != UserRole::LEAST_PRIVILEGED,
            "a role that was never recorded must resolve strictly above one that \
             could not be read"
        );
    }

    #[test]
    fn canonical_strings_round_trip() {
        for role in [UserRole::Everyday, UserRole::Power, UserRole::Developer] {
            assert_eq!(UserRole::from_persisted(role.as_str()), Some(role));
        }
    }

    #[test]
    fn legacy_and_unknown_strings_are_sentinels() {
        // The retired `UserMode` variants and any garbage must decode to `None`
        // so they defer to `WHEN_UNSET` rather than mis-promoting.
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

    /// Every role survives a cell round-trip, so the encoding the cell hides
    /// never reinterprets a value on the way through.
    #[test]
    fn cell_round_trips_every_role() {
        let cell = UserRoleCell::default();
        assert_eq!(cell.get(), UserRole::default());

        for role in [UserRole::Everyday, UserRole::Power, UserRole::Developer] {
            cell.set(role);
            assert_eq!(cell.get(), role);
        }
    }

    /// Clones share one value in both directions — the property `AppContext`
    /// relies on to keep every per-network context on the same role.
    #[test]
    fn cell_clones_share_one_value() {
        let cell = UserRoleCell::new(UserRole::Everyday);
        let sibling = cell.clone();

        cell.set(UserRole::Power);
        assert_eq!(sibling.get(), UserRole::Power, "a write is seen by a clone");

        sibling.set(UserRole::Developer);
        assert_eq!(
            cell.get(),
            UserRole::Developer,
            "a clone's write is seen by the original"
        );
    }
}
