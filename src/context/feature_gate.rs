use crate::context::AppContext;
use crate::model::user_role::UserRole;
use dash_sdk::dpp::version::PlatformVersion;

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
            Capability::ShieldedProtocol => {
                // Use the protocol version fetched from the network (not the
                // hardcoded default) to look up the correct PlatformVersion.
                // Returns false when the version hasn't been fetched yet (0)
                // or doesn't support shielded state transitions.
                let proto = ctx.platform_protocol_version();
                let Some(pv) = PlatformVersion::get_optional(proto) else {
                    return false;
                };
                let st = &pv.dpp.state_transition_serialization_versions;
                // All shielded state transition types must be present (max_version > 0
                // means the feature has been defined in this protocol version).
                st.shield_state_transition.max_version > 0
                    && st.shielded_transfer_state_transition.max_version > 0
                    && st.unshield_state_transition.max_version > 0
                    && st.shield_from_asset_lock_state_transition.max_version > 0
                    && st.shielded_withdrawal_state_transition.max_version > 0
            }
        }
    }
}

/// A feature that has not stabilised yet (the stability axis). Distinct from
/// role: when such a feature stabilises, its check is removed and it unlocks
/// for every role — misfiling it as a role gate would permanently hide the
/// stabilised feature from ordinary users.
///
/// Empty until Phase 2 reclassifies the "dev-mode as a stability flag"
/// callsites; the type exists now so [`Check::Experimental`] compiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExperimentalFeature {}

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
    /// define all shielded state transitions.
    Shielded,
    /// DashPay social features — always available (placeholder for future restriction)
    DashPay,
    /// Expert/developer mode — unlocks advanced UI elements. Temporarily mapped
    /// to "at least Power" during the compat-shim window; the callsites behind
    /// it are reclassified per persona in a later pass.
    DeveloperMode,
}

impl FeatureGate {
    /// The checks that must ALL pass for this feature. An empty list means the
    /// feature is always available (the empty conjunction is `true`).
    fn checks(self) -> &'static [Check] {
        match self {
            FeatureGate::Shielded => &[Check::Capability(Capability::ShieldedProtocol)],
            FeatureGate::DashPay => &[],
            FeatureGate::DeveloperMode => &[Check::MinRole(UserRole::Power)],
        }
    }

    /// Evaluate whether this feature is available in the current context.
    pub fn is_available(self, ctx: &AppContext) -> bool {
        self.checks().iter().all(|c| c.is_met(ctx))
    }
}
