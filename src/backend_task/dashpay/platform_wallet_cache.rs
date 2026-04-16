//! Transitional helpers for DashPay mutations that haven't moved into
//! platform-wallet's DashPayWallet yet.
//!
//! **Payments** are now handled internally:
//! - Received: [`DashPayWallet::try_record_incoming_payment`]
//! - Sent: [`DashPayWallet::send_payment`]
//!
//! **Profiles** still need this helper — profile.rs load/create/update
//! haven't been rewired to DashPayWallet methods yet (Phase 3).
//! Once they are, this file can be deleted.

use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use platform_wallet::PlatformWallet;
use platform_wallet::wallet::dashpay::DashPayProfile;
use std::sync::Arc;

/// Resolve the `PlatformWallet` for a `QualifiedIdentity`.
fn resolve_platform_wallet(
    app_context: &AppContext,
    identity: &QualifiedIdentity,
) -> Option<Arc<PlatformWallet>> {
    match app_context.platform_wallet_for_identity(identity) {
        Ok(pw) => Some(pw),
        Err(e) => {
            tracing::warn!(
                error = %e,
                identity = %identity.identity.id(),
                "platform-wallet cache: no platform wallet for identity"
            );
            None
        }
    }
}

/// Cache a DashPay profile on the owner's ManagedIdentity.
///
/// TODO: Remove once profile.rs uses DashPayWallet::sync() /
/// create_profile() / update_profile() directly.
pub(crate) async fn cache_profile(
    app_context: &AppContext,
    identity: &QualifiedIdentity,
    profile: Option<DashPayProfile>,
) {
    let Some(pw) = resolve_platform_wallet(app_context, identity) else {
        return;
    };
    let owner_id = identity.identity.id();
    let persister = pw.persister().clone();
    let mut state = pw.state_mut().await;
    let Some(managed) = state.identity_manager.managed_identity_mut(&owner_id) else {
        tracing::debug!(
            identity = %owner_id,
            "platform-wallet cache: identity not in IdentityManager"
        );
        return;
    };
    managed.set_dashpay_profile(profile, &persister);
}
