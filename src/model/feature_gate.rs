use crate::context::AppContext;

/// Named feature gate. Each variant maps to a predicate over `AppContext`.
///
/// Adding a new gate:
/// 1. Add variant here
/// 2. Implement predicate in `is_available()`
/// 3. Use at UI callsite: `FeatureGate::Shielded.is_available(&ctx)`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FeatureGate {
    /// Shielded (ZK) transactions — requires protocol version >= 12
    Shielded,
    /// DashPay social features — always available (placeholder for future restriction)
    DashPay,
    /// Expert/developer mode — unlocks advanced UI elements
    DeveloperMode,
    /// SPV backend mode — currently requires developer mode
    SpvBackend,
}

impl FeatureGate {
    /// Evaluate whether this feature is available in the current context.
    pub fn is_available(self, ctx: &AppContext) -> bool {
        match self {
            FeatureGate::Shielded => ctx.supports_shielded(),
            FeatureGate::DashPay => true, // Always for now; future: network/version gate
            FeatureGate::DeveloperMode => ctx.is_developer_mode(),
            FeatureGate::SpvBackend => ctx.is_developer_mode(),
        }
    }
}

/// Check whether all given gates are available.
pub fn all_available(gates: &[FeatureGate], ctx: &AppContext) -> bool {
    gates.iter().all(|g| g.is_available(ctx))
}

/// Check whether any of the given gates is available.
pub fn any_available(gates: &[FeatureGate], ctx: &AppContext) -> bool {
    gates.iter().any(|g| g.is_available(ctx))
}
