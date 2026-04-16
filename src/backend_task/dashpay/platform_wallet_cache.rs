//! Transitional helpers for DashPay mutations that haven't moved into
//! platform-wallet's DashPayWallet yet.
//!
//! **Received payments** are now handled internally by
//! [`DashPayWallet::try_record_incoming_payment`] — no helper needed.
//!
//! **Sent payments** still need this helper because `send_payment` hasn't
//! moved into DashPayWallet yet (requires CoreWallet integration).
//! Once DashPayWallet owns the send flow, this file can be deleted.

use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use platform_wallet::PlatformWallet;
use platform_wallet::wallet::dashpay::PaymentEntry;
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

/// Record a sent payment on the owner's ManagedIdentity.
///
/// TODO: Remove once DashPayWallet::send_payment() owns the full
/// send flow (address derivation + CoreWallet tx + record).
pub(crate) async fn cache_payment(
    app_context: &AppContext,
    identity: &QualifiedIdentity,
    tx_id: String,
    entry: PaymentEntry,
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
    managed.record_dashpay_payment(tx_id, entry, &persister);
}
