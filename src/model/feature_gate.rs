use crate::context::AppContext;
use egui::Ui;

/// Named feature gate. Each variant maps to a predicate over `AppContext`.
///
/// Adding a new gate:
/// 1. Add variant here
/// 2. Implement predicate in `is_available()`
/// 3. Use at UI callsite (three patterns available)
///
/// # Usage patterns
///
/// **Single widget** (using egui built-in):
/// ```ignore
/// ui.add_visible(FeatureGate::Shielded.is_available(&ctx), egui::Button::new("Shield"));
/// ```
///
/// **Multi-widget section** (using extension trait):
/// ```ignore
/// use crate::model::feature_gate::FeatureGateUiExt;
/// ui.feature_gated(FeatureGate::DeveloperMode, &ctx, |ui| {
///     ui.label("Debug info");
///     ui.button("Advanced");
/// });
/// ```
///
/// **Conditional data** (direct predicate):
/// ```ignore
/// if FeatureGate::Shielded.is_available(&ctx) {
///     items.push(something);
/// }
/// ```
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

/// Extension trait on [`egui::Ui`] for feature-gated UI sections.
pub trait FeatureGateUiExt {
    /// Render a multi-widget block only when the gate is available.
    /// When unavailable, nothing is rendered and no layout space is allocated.
    fn feature_gated(
        &mut self,
        gate: FeatureGate,
        ctx: &AppContext,
        add_contents: impl FnOnce(&mut Ui),
    );
}

impl FeatureGateUiExt for Ui {
    fn feature_gated(
        &mut self,
        gate: FeatureGate,
        ctx: &AppContext,
        add_contents: impl FnOnce(&mut Ui),
    ) {
        if gate.is_available(ctx) {
            add_contents(self);
        }
    }
}
