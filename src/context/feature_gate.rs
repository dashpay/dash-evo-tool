use crate::context::AppContext;
use crate::model::user_role::UserRole;

/// The platform protocol version that first defines the shielded state
/// transitions (shield, shielded transfer, unshield, shield from asset lock,
/// shielded withdrawal) — `None` while they remain unshipped, which is the case
/// on every released version today, so [`Capability::ShieldedProtocol`] is unmet
/// on every network.
///
/// TODO: set this to the activation version when upstream ships the shielded state
/// transitions, then re-check the shielded gates (the tripwire test below fails
/// until they are). Do NOT infer activation from
/// `FeatureVersionBounds::max_version > 0` instead: `check_version` is
/// `v >= min && v <= max`, so `{min: 0, max: 0}` is a legitimately checkable v0
/// bound — `identity_create_state_transition`, a live feature, ships exactly that
/// triple — not an "undefined" marker. A shielded transition released at v0 would
/// read as permanently absent. See the PR #879 review.
const SHIELDED_ACTIVATION_PROTOCOL_VERSION: Option<u32> = None;

/// A runtime capability of the connected platform, evaluated against the live
/// context. Independent of the user's role — it answers "does the connected
/// network support this?", not "which role has the user selected?".
///
/// Capabilities are network-scoped by construction: `is_met` reads live state
/// off the passed-in [`AppContext`], which is one instance per network, so a
/// check against mainnet's context sees mainnet's protocol version and testnet's
/// sees testnet's. New variants must follow the same rule — read from `ctx`,
/// never a network-agnostic constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    /// The connected platform protocol version defines all shielded state
    /// transitions (shield, shielded transfer, unshield, shield from asset
    /// lock, shielded withdrawal).
    ShieldedProtocol,
}

impl Capability {
    fn is_met(self, ctx: &AppContext) -> bool {
        match self {
            Capability::ShieldedProtocol => match SHIELDED_ACTIVATION_PROTOCOL_VERSION {
                // Not shipped anywhere yet, so no network can offer it.
                None => false,
                // The version fetched from the connected network, not the hardcoded
                // default — the capability is per-network. The boot value (0, "not
                // fetched yet") is below any activation version, so a network whose
                // version is not known yet reads as unmet rather than optimistic.
                Some(activation) => ctx.platform_protocol_version() >= activation,
            },
        }
    }
}

/// A feature that has not stabilised yet (the stability axis). Distinct from
/// role: when such a feature stabilises, its check is removed and it unlocks
/// for every role — misfiling it as a role gate would permanently hide the
/// stabilised feature from ordinary users.
///
/// Availability is resolved through [`AppContext::experimental_enabled`], which
/// today tracks the retired dev-mode gating (`>= Power`) so behaviour is
/// unchanged; stabilising a feature later is a one-line flip that unlocks it for
/// every role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExperimentalFeature {
    /// Shielded (ZK) send and shielded-tab surfaces — not stable yet.
    Shielded,
    /// DashPay payments and payment-history surfaces — not stable yet.
    DashPay,
}

/// One predicate contributing to a feature's availability. A feature is
/// available iff ALL of its checks are met (AND semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Check {
    /// The user is operating in at least this role (role axis).
    MinRole(UserRole),
    /// The connected platform provides this capability (platform axis,
    /// network-scoped by construction — see [`Capability`]).
    Capability(Capability),
    /// An experimental feature that has not stabilised yet (stability axis).
    /// When the feature stabilises this check is removed and it unlocks for
    /// EVERY role.
    Experimental(ExperimentalFeature),
    // FUTURE — not built now (YAGNI). A context predicate ("wallet loaded",
    // "identity exists", …) would be added here as one variant, e.g.
    // `Context(ContextPredicate)`, evaluated in `is_met`. No such variant is
    // introduced today because no current callsite gates availability on one.
}

impl Check {
    fn is_met(&self, ctx: &AppContext) -> bool {
        match self {
            Check::MinRole(min) => ctx.user_role().at_least(*min),
            Check::Capability(cap) => cap.is_met(ctx),
            Check::Experimental(f) => ctx.experimental_enabled(*f),
        }
    }
}

/// Named feature gate. Each variant resolves to a conjunction of [`Check`]s;
/// the feature is available iff all of them pass.
///
/// Adding a new gate:
/// 1. Add a variant here
/// 2. Add its check list to `checks()`
/// 3. Use `FeatureGate::Variant.is_available(&ctx)` at the callsite
///
/// # Usage
///
/// ```ignore
/// if FeatureGate::Shielded.is_available(&ctx) {
///     items.push(something);
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FeatureGate {
    /// Shielded (ZK) transactions — requires the current platform version to
    /// define all shielded state transitions. Backs the Shielded account tab.
    Shielded,
    /// Shielded operations the user can actually invoke — shielded send sources
    /// and shielded destinations. Both axes must hold: the connected network has
    /// to define the shielded state transitions ([`FeatureGate::Shielded`]) *and*
    /// the feature is still experimental. Offering a shielded destination the
    /// network cannot settle is a dead end, so this never reduces to the
    /// experimental flag alone.
    ShieldedOperations,
    /// DashPay social features — always available (placeholder for future restriction)
    DashPay,
    /// DashPay operations the user can actually invoke — pay buttons and the
    /// payment-history subscreen. Experimental only: DashPay has no network
    /// capability gate today, unlike [`FeatureGate::ShieldedOperations`].
    DashPayOperations,
    /// Masternode operation — a Power User activity (the operator persona), so
    /// gated at [`UserRole::Power`]. Backs the Masternodes nav entry.
    Masternodes,
    /// The Developer-tools tier itself — features reserved for the Platform
    /// Developer role (raw protocol inspection, bulk operations, Devnet config).
    /// Gated at [`UserRole::Developer`]; the successor to the old overloaded
    /// developer-mode flag, which conflated this tier with Power-role disclosure.
    DeveloperTools,
}

impl FeatureGate {
    /// The checks that must ALL pass for this feature. An empty list means the
    /// feature is always available (the empty conjunction is `true`).
    fn checks(self) -> &'static [Check] {
        match self {
            FeatureGate::Shielded => &[Check::Capability(Capability::ShieldedProtocol)],
            FeatureGate::ShieldedOperations => &[
                Check::Capability(Capability::ShieldedProtocol),
                Check::Experimental(ExperimentalFeature::Shielded),
            ],
            FeatureGate::DashPay => &[],
            FeatureGate::DashPayOperations => &[Check::Experimental(ExperimentalFeature::DashPay)],
            FeatureGate::Masternodes => &[Check::MinRole(UserRole::Power)],
            FeatureGate::DeveloperTools => &[Check::MinRole(UserRole::Developer)],
        }
    }

    /// Evaluate whether this feature is available in the current context.
    pub fn is_available(self, ctx: &AppContext) -> bool {
        self.checks().iter().all(|c| c.is_met(ctx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::test_app_context;
    use dash_sdk::dpp::version::PlatformVersion;
    use std::sync::Arc;

    const ROLES: [UserRole; 3] = [UserRole::Everyday, UserRole::Power, UserRole::Developer];

    /// Highest protocol version probed by [`known_protocol_versions`]. Well past
    /// the versions upstream defines today, so the probe cannot silently miss one.
    const MAX_PROBED_PROTOCOL_VERSION: u32 = 40;

    /// A context in `role`, on its own data dir — the app k/v store refuses a
    /// second open of the same path, so contexts cannot share one.
    fn ctx_with_role(role: UserRole) -> (tempfile::TempDir, Arc<AppContext>) {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_app_context(tmp.path());
        ctx.set_user_role(role);
        (tmp, ctx)
    }

    /// Every protocol version upstream actually defines.
    fn known_protocol_versions() -> Vec<u32> {
        (1..=MAX_PROBED_PROTOCOL_VERSION)
            .filter(|v| PlatformVersion::get_optional(*v).is_some())
            .collect()
    }

    /// Masternode operation is the operator persona's activity: Power and above.
    #[test]
    fn masternodes_requires_at_least_power() {
        for role in ROLES {
            let (_tmp, ctx) = ctx_with_role(role);
            assert_eq!(
                FeatureGate::Masternodes.is_available(&ctx),
                role.at_least(UserRole::Power),
                "Masternodes availability for {role:?}"
            );
        }
    }

    /// The developer tier is reserved for the Developer role — a Power user must
    /// not reach it (that conflation is exactly what the old dev-mode flag did).
    #[test]
    fn developer_tools_requires_the_developer_role() {
        for role in ROLES {
            let (_tmp, ctx) = ctx_with_role(role);
            assert_eq!(
                FeatureGate::DeveloperTools.is_available(&ctx),
                role == UserRole::Developer,
                "DeveloperTools availability for {role:?}"
            );
        }
    }

    /// The empty conjunction is `true`: a gate with no checks is available to
    /// every role, unconditionally.
    #[test]
    fn empty_check_list_is_always_available() {
        assert!(
            FeatureGate::DashPay.checks().is_empty(),
            "this test is about the empty-conjunction case"
        );
        for role in ROLES {
            let (_tmp, ctx) = ctx_with_role(role);
            assert!(
                FeatureGate::DashPay.is_available(&ctx),
                "DashPay nav entry must stay available for {role:?}"
            );
        }
    }

    /// The experimental axis is wired: DashPay *operations* (pay buttons, payment
    /// history) track today's `experimental_enabled` rule — Power and above — while
    /// the DashPay nav entry above stays open to everyone.
    #[test]
    fn dashpay_operations_track_the_experimental_axis() {
        for role in ROLES {
            let (_tmp, ctx) = ctx_with_role(role);
            assert_eq!(
                FeatureGate::DashPayOperations.is_available(&ctx),
                ctx.experimental_enabled(ExperimentalFeature::DashPay),
                "DashPayOperations must follow the experimental axis for {role:?}"
            );
            assert_eq!(
                FeatureGate::DashPayOperations.is_available(&ctx),
                role.at_least(UserRole::Power),
                "…which today means Power and above ({role:?})"
            );
        }
    }

    /// An unfetched protocol version (0, the boot value) means "we do not know what
    /// this network supports" — the capability must read as unmet, never optimistic.
    #[test]
    fn shielded_capability_is_unmet_before_a_version_is_fetched() {
        let (_tmp, ctx) = ctx_with_role(UserRole::Developer);

        assert_eq!(ctx.platform_protocol_version(), 0, "boot value");
        assert!(!Capability::ShieldedProtocol.is_met(&ctx));
        assert!(
            !FeatureGate::Shielded.is_available(&ctx),
            "the shielded tab stays hidden until the network's version is known"
        );
    }

    /// Tripwire. Shielded state transitions have not shipped, so
    /// [`SHIELDED_ACTIVATION_PROTOCOL_VERSION`] is still `None`: the capability is
    /// unmet on every protocol version upstream defines, and the "capability met"
    /// half of the AND cannot be exercised yet.
    ///
    /// This test fails the moment that constant names a version upstream actually
    /// ships. That is the point: whoever activates the capability must then
    /// re-check the shielded gates and add the missing
    /// `capability ∧ role ⇒ available` case below.
    #[test]
    fn no_known_protocol_version_reaches_the_shielded_activation_version() {
        let (_tmp, ctx) = ctx_with_role(UserRole::Developer);
        let versions = known_protocol_versions();
        assert!(!versions.is_empty(), "the probe must find some versions");

        for version in versions {
            ctx.set_platform_protocol_version(version);
            assert!(
                !Capability::ShieldedProtocol.is_met(&ctx),
                "protocol v{version} now reaches the shielded activation version \
                 ({SHIELDED_ACTIVATION_PROTOCOL_VERSION:?}) — the shielded gates have a \
                 reachable capability and need re-checking"
            );
        }
    }

    /// AND semantics. `ShieldedOperations` is the first multi-check gate: it needs
    /// the experimental axis *and* the network capability. A Developer passes the
    /// experimental check outright, so the gate can only be closed by the failing
    /// capability — proving the capability check is genuinely part of the
    /// conjunction and not decoration. A gate that reduced to `experimental_enabled`
    /// (the bug this replaces) would be available here.
    #[test]
    fn shielded_operations_fails_when_only_the_experimental_axis_passes() {
        let (_tmp, ctx) = ctx_with_role(UserRole::Developer);

        assert!(
            ctx.experimental_enabled(ExperimentalFeature::Shielded),
            "a Developer passes the experimental check"
        );
        assert!(
            !Capability::ShieldedProtocol.is_met(&ctx),
            "no network defines the shielded state transitions yet"
        );
        assert!(
            !FeatureGate::ShieldedOperations.is_available(&ctx),
            "one failing check must close the conjunction: shielded send options \
             must not be offered on a network that cannot settle them"
        );
    }

    /// A failing capability closes the gate for every role — no role can buy its
    /// way past a network that does not support the feature.
    #[test]
    fn shielded_operations_is_unavailable_for_every_role_today() {
        for role in ROLES {
            let (_tmp, ctx) = ctx_with_role(role);
            assert!(
                !FeatureGate::ShieldedOperations.is_available(&ctx),
                "ShieldedOperations must stay closed for {role:?} while the capability is unmet"
            );
        }
    }
}
