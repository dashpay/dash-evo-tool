use crate::context::AppContext;
use crate::model::user_role::UserRole;
use dash_sdk::dpp::version::feature_initial_protocol_versions::CONTRACT_FEE_CLAIM_INITIAL_PROTOCOL_VERSION;

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
    /// The connected platform keeps contract fee pots (filled by document
    /// action fees) and accepts the transition that pays them out.
    ContractFeePots,
}

/// Whether the protocol version the connected network has proven is one this
/// build defines and is at least `activation`.
///
/// Every capability reads the version through this, so it fails closed both
/// before the network proves a version and for a version newer than any this
/// build knows ([`dash_sdk::ProtocolVersionInfo::newer_protocol_available`]):
/// this build cannot tell what an unknown version's rules allow.
fn known_protocol_version_at_least(ctx: &AppContext, activation: u32) -> bool {
    let info = ctx.protocol_version_info();
    !info.newer_protocol_available
        && info
            .verified_version
            .is_some_and(|version| version >= activation)
}

impl Capability {
    fn is_met(self, ctx: &AppContext) -> bool {
        match self {
            Capability::ContractFeePots => {
                known_protocol_version_at_least(ctx, CONTRACT_FEE_CLAIM_INITIAL_PROTOCOL_VERSION)
            }
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
    /// The connected platform provides this network-scoped capability.
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
    /// The Shielded account tab — the view of the wallet's shielded (Orchard)
    /// pool. Always available: DET configures the shielded coordinator on every
    /// network, so shielded balances can exist and must be viewable wherever the
    /// wallet runs. Distinct from [`FeatureGate::ShieldedOperations`], which gates
    /// *creating* shielded state transitions behind experimental access.
    Shielded,
    /// Shielded sending with experimental features enabled for the user's role.
    ShieldedOperations,
    /// DashPay social features — always available (placeholder for future restriction)
    DashPay,
    /// DashPay operations the user can actually invoke — pay buttons and the
    /// payment-history subscreen. Experimental only.
    DashPayOperations,
    /// Masternode operation — a Power User activity (the operator persona), so
    /// gated at [`UserRole::Power`]. Backs the Masternodes nav entry.
    Masternodes,
    /// The Developer-tools tier itself — features reserved for the Platform
    /// Developer role (raw protocol inspection, bulk operations, Devnet config).
    /// Gated at [`UserRole::Developer`]; the successor to the old overloaded
    /// developer-mode flag, which conflated this tier with Power-role disclosure.
    DeveloperTools,
    /// Viewing and claiming the fee pots of a contract. Offered only where
    /// the connected network keeps them.
    ContractFeePots,
}

impl FeatureGate {
    /// The checks that must ALL pass for this feature. An empty list means the
    /// feature is always available (the empty conjunction is `true`).
    fn checks(self) -> &'static [Check] {
        match self {
            FeatureGate::Shielded => &[],
            FeatureGate::ShieldedOperations => {
                &[Check::Experimental(ExperimentalFeature::Shielded)]
            }
            FeatureGate::DashPay => &[],
            FeatureGate::DashPayOperations => &[Check::Experimental(ExperimentalFeature::DashPay)],
            FeatureGate::Masternodes => &[Check::MinRole(UserRole::Power)],
            FeatureGate::DeveloperTools => &[Check::MinRole(UserRole::Developer)],
            FeatureGate::ContractFeePots => &[Check::Capability(Capability::ContractFeePots)],
        }
    }

    /// Evaluate whether this feature is available in the current context.
    pub fn is_available(self, ctx: &AppContext) -> bool {
        self.checks().iter().all(|c| c.is_met(ctx))
    }

    /// The first role, capability, or experimental check that fails, if any.
    pub fn first_unmet_check(self, ctx: &AppContext) -> Option<Check> {
        self.checks().iter().copied().find(|c| !c.is_met(ctx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::{prove_protocol_version, test_app_context};
    use std::sync::Arc;

    const ROLES: [UserRole; 3] = [UserRole::Everyday, UserRole::Power, UserRole::Developer];

    /// A context in `role`, on its own data dir — the app k/v store refuses a
    /// second open of the same path, so contexts cannot share one.
    fn ctx_with_role(role: UserRole) -> (tempfile::TempDir, Arc<AppContext>) {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_app_context(tmp.path());
        ctx.set_user_role(role);
        (tmp, ctx)
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

    #[test]
    fn shielded_operations_only_require_experimental_access() {
        for role in ROLES {
            let (_tmp, ctx) = ctx_with_role(role);
            assert_eq!(
                FeatureGate::ShieldedOperations.is_available(&ctx),
                role.at_least(UserRole::Power)
            );
            assert!(FeatureGate::Shielded.is_available(&ctx));
        }
    }

    /// Protocol version 14 features stay closed below version 14 and before
    /// the network's version is known, whatever the role.
    #[test]
    fn protocol_v14_gates_follow_the_connected_version() {
        // One entry per protocol version 14 gate this build offers.
        let gates = [FeatureGate::ContractFeePots];
        for role in ROLES {
            for gate in gates {
                // A proven version only rises, so each gate gets a fresh context.
                let (_tmp, ctx) = ctx_with_role(role);
                assert!(!gate.is_available(&ctx), "{gate:?} closed at boot");
                prove_protocol_version(&ctx, 13);
                assert!(!gate.is_available(&ctx), "{gate:?} closed on protocol 13");
                prove_protocol_version(&ctx, 14);
                assert!(gate.is_available(&ctx), "{gate:?} open on protocol 14");
            }
        }
    }

    /// A protocol version this build does not define closes every protocol
    /// capability: nothing here knows that version's rules. The newest
    /// version this build defines keeps them open.
    #[test]
    fn an_unknown_future_protocol_version_closes_every_protocol_capability() {
        use dash_sdk::dpp::version::LATEST_VERSION;
        let (_tmp, ctx) = ctx_with_role(UserRole::Developer);
        let capabilities = [Capability::ContractFeePots];

        prove_protocol_version(&ctx, LATEST_VERSION);
        for capability in capabilities {
            assert!(
                capability.is_met(&ctx),
                "{capability:?} on protocol {LATEST_VERSION}"
            );
        }

        prove_protocol_version(&ctx, LATEST_VERSION + 1);
        for capability in capabilities {
            assert!(
                !capability.is_met(&ctx),
                "{capability:?} on unknown protocol {}",
                LATEST_VERSION + 1
            );
        }
    }
}
