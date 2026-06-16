//! Helpers for shielded (ZK) operations in tests.
//!
//! Phase D retired DET's home-grown shielded subsystem; warm-up and key
//! binding are owned by the upstream coordinator (binding happens automatically
//! on wallet unlock), so the only helpers left are availability / skip / error
//! classification. The coordinator-store balance reads are `pub(crate)` and not
//! reachable from this external test crate — balance/sync verification lives in
//! the Phase-G det-cli self-test, which drives the public MCP read tools.

use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::feature_gate::FeatureGate;

/// Check whether the connected platform supports shielded operations
/// via the `FeatureGate::Shielded` protocol version check.
///
/// Returns `true` if shielded state transitions are available. Call this
/// early in shielded tests to skip proactively instead of waiting for an
/// error from the backend task.
pub fn is_shielded_available(app_context: &AppContext) -> bool {
    FeatureGate::Shielded.is_available(app_context)
}

/// Check `E2E_SKIP_SHIELDED` env var and skip the calling test if set.
///
/// Call this at the top of every shielded test function. Returns `true`
/// if the test should be skipped (caller should `return` early).
pub fn skip_if_shielded_disabled() -> bool {
    if std::env::var("E2E_SKIP_SHIELDED").is_ok() {
        tracing::info!("Skipping shielded test (E2E_SKIP_SHIELDED is set)");
        true
    } else {
        false
    }
}

/// Check whether a task error indicates the platform does not support
/// shielded operations (e.g., testnet without shielded support enabled).
///
/// Returns `true` for errors that indicate the platform lacks shielded support:
/// - Connection-related errors (CoreRpc, DapiConnectionRefused) — Core RPC
///   not available for shielded ops
/// - Platform rejection errors — state transition types for shielded ops
///   not recognized by testnet
/// - SDK/protocol errors containing unsupported-operation signals
///
/// TODO: This still falls back to Debug-string inspection for some error
/// variants (PlatformRejected, SdkError) because the SDK does not expose
/// typed variants for "unsupported state transition type" or deserialization
/// failures on unknown variants. Once the SDK adds typed errors for these
/// cases, replace the string checks with proper pattern matching.
pub fn is_platform_shielded_unsupported(
    err: &dash_evo_tool::backend_task::error::TaskError,
) -> bool {
    use dash_evo_tool::backend_task::error::TaskError;

    match err {
        // Typed variants that clearly indicate infrastructure unavailability
        TaskError::CoreRpc { .. }
        | TaskError::CoreRpcConnectionFailed { .. }
        | TaskError::DapiConnectionRefused { .. } => true,

        // Platform rejected or unclassified SDK errors — inspect Debug output
        // for shielded-specific signals until the SDK provides typed variants
        TaskError::PlatformRejected { .. } | TaskError::SdkError { .. } => {
            let msg = format!("{:?}", err).to_lowercase();
            msg.contains("not implemented")
                || msg.contains("not supported")
                || msg.contains("serializedobjectparsingerror")
                || msg.contains("unexpectedvariant")
                || msg.contains("variant 15")
                || msg.contains("variant 16")
                || msg.contains("variant 17")
                || msg.contains("variant 18")
                || msg.contains("variant 19")
        }

        _ => false,
    }
}
