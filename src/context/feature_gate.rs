use crate::context::AppContext;
use crate::model::user_role::UserRole;
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
    /// The connection has verified a Platform version supported by DET.
    SupportedNetwork,
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
            Check::SupportedNetwork => ctx.ensure_platform_write_supported().is_ok(),
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
    /// *creating* shielded state transitions behind the network's protocol
    /// capability.
    Shielded,
    /// Shielded sending on a verified, supported network, with experimental
    /// features enabled for the user's role.
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
            FeatureGate::Shielded => &[],
            FeatureGate::ShieldedOperations => &[
                Check::SupportedNetwork,
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

    /// The first check that fails to hold for this gate, or `None` if every
    /// check passes (equivalent to `is_available` returning `true`). Lets a
    /// caller surface which axis is actually blocking availability — e.g.
    /// distinguishing "the network doesn't support this yet" (a
    /// [`Check::SupportedNetwork`]) from "your interface mode doesn't unlock this" (a
    /// [`Check::Experimental`]) — instead of a single opaque `false`.
    pub fn first_unmet_check(self, ctx: &AppContext) -> Option<Check> {
        self.checks().iter().copied().find(|c| !c.is_met(ctx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::{prove_protocol_version, test_app_context};
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
        (14..=MAX_PROBED_PROTOCOL_VERSION)
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

    #[test]
    fn shielded_operations_need_a_supported_network_and_unlocked_role() {
        for role in ROLES {
            let (_tmp, ctx) = ctx_with_role(role);
            assert!(!FeatureGate::ShieldedOperations.is_available(&ctx));
            for version in known_protocol_versions() {
                prove_protocol_version(&ctx, version);
                assert_eq!(
                    FeatureGate::ShieldedOperations.is_available(&ctx),
                    role.at_least(UserRole::Power)
                );
                assert!(FeatureGate::Shielded.is_available(&ctx));
            }
        }
    }

    #[test]
    fn shielded_operations_report_network_then_role_requirements() {
        let (_tmp, ctx) = ctx_with_role(UserRole::Everyday);
        assert_eq!(
            FeatureGate::ShieldedOperations.first_unmet_check(&ctx),
            Some(Check::SupportedNetwork)
        );
        prove_protocol_version(&ctx, 14);
        assert_eq!(
            FeatureGate::ShieldedOperations.first_unmet_check(&ctx),
            Some(Check::Experimental(ExperimentalFeature::Shielded))
        );
        ctx.set_user_role(UserRole::Developer);
        assert_eq!(
            FeatureGate::ShieldedOperations.first_unmet_check(&ctx),
            None
        );
    }

    #[test]
    fn unsupported_network_keeps_balances_visible_but_refuses_shielded_operations() {
        use dash_sdk::dpp::version::LATEST_VERSION;
        for version in [13, LATEST_VERSION + 1] {
            let (_tmp, ctx) = ctx_with_role(UserRole::Developer);
            prove_protocol_version(&ctx, version);
            assert!(FeatureGate::Shielded.is_available(&ctx));
            assert!(!FeatureGate::ShieldedOperations.is_available(&ctx));
        }
    }
}
