use crate::context::AppContext;
use crate::spv::CoreBackendMode;
use dash_sdk::dpp::version::PlatformVersion;
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
/// ui.feature_gated(&ctx, FeatureGate::DeveloperMode, |ui| {
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
    /// Shielded (ZK) transactions — requires all shielded state transition features
    /// to be present in the current platform version (shield, shielded transfer,
    /// unshield, shield from asset lock, shielded withdrawal).
    Shielded,
    /// DashPay social features — always available (placeholder for future restriction)
    DashPay,
    /// Expert/developer mode — unlocks advanced UI elements
    DeveloperMode,
    /// SPV backend mode — active when the app is running in SPV (light client) mode
    SpvBackend,
}

impl FeatureGate {
    /// Evaluate whether this feature is available in the current context.
    pub fn is_available(self, ctx: &AppContext) -> bool {
        match self {
            FeatureGate::Shielded => {
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
            FeatureGate::DashPay => true, // Always for now; future: network/version gate
            FeatureGate::DeveloperMode => ctx.is_developer_mode(),
            FeatureGate::SpvBackend => ctx.core_backend_mode() == CoreBackendMode::Spv,
        }
    }
}

/// Extension trait on [`egui::Ui`] for feature-gated UI sections.
pub trait FeatureGateUiExt {
    /// Render a multi-widget block only when the gate is available.
    /// When unavailable, nothing is rendered and no layout space is allocated.
    fn feature_gated(
        &mut self,
        ctx: &AppContext,
        gate: FeatureGate,
        add_contents: impl FnOnce(&mut Ui),
    );
}

impl FeatureGateUiExt for Ui {
    fn feature_gated(
        &mut self,
        ctx: &AppContext,
        gate: FeatureGate,
        add_contents: impl FnOnce(&mut Ui),
    ) {
        if gate.is_available(ctx) {
            add_contents(self);
        }
    }
}
