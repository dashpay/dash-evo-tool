//! Helpers for routing DashPay mutations through the platform-wallet
//! changeset flow.
//!
//! Phase 9b migrates backend tasks away from direct `Database::*` DB
//! writes and onto mutation methods that emit changesets the
//! persister catches on flush. Every helper here follows the same
//! three-step pattern:
//!
//! 1. Resolve the `PlatformWallet` handle for the owner identity.
//! 2. Acquire the wallet-manager write lock and call the mutation
//!    method on `ManagedIdentity`, capturing the emitted
//!    sub-changeset.
//! 3. Wrap the sub-changeset in a top-level
//!    [`PlatformWalletChangeSet`] and queue it for persistence via
//!    `pw.queue_persist`.
//!
//! Silent no-op with a `tracing::warn!` / `tracing::debug!` if the
//! owner identity isn't present in the platform-wallet's
//! `IdentityManager` — the caller's direct-write fallback has been
//! removed, but losing the cache is not worth failing the outer
//! operation.

use crate::context::AppContext;
use dash_sdk::platform::Identifier;
use platform_wallet::PlatformWallet;
use platform_wallet::changeset::PlatformWalletChangeSet;
use platform_wallet::wallet::dashpay::PaymentEntry;
use std::sync::Arc;

/// Resolve the `PlatformWallet` for `owner_id` by looking it up
/// through evo-tool's wallet bridge. Returns `None` (with a
/// `tracing::warn!`) if no wallet holds the identity — the caller
/// should treat that as a no-op.
///
/// Sync because both `get_identity_by_id` and
/// `platform_wallet_for_identity` are sync. Shared by the async
/// and blocking cache helpers.
fn resolve_platform_wallet_by_owner(
    app_context: &AppContext,
    owner_id: &Identifier,
) -> Option<Arc<PlatformWallet>> {
    // Load the QualifiedIdentity so we can hand it to
    // `platform_wallet_for_identity`, which is the one path that
    // knows how to resolve the owner's wallet seed hash.
    let wallets_snapshot = match app_context.wallets.read() {
        Ok(wallets) => wallets.clone(),
        Err(_) => {
            tracing::warn!(
                identity = %owner_id,
                "platform-wallet cache: failed to snapshot wallets"
            );
            return None;
        }
    };
    let qi = match app_context.db.get_identity_by_id(
        owner_id,
        app_context,
        &wallets_snapshot,
    ) {
        Ok(Some(qi)) => qi,
        Ok(None) => {
            tracing::debug!(
                identity = %owner_id,
                "platform-wallet cache: owner identity not found in DB"
            );
            return None;
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                identity = %owner_id,
                "platform-wallet cache: identity lookup failed"
            );
            return None;
        }
    };
    match app_context.platform_wallet_for_identity(&qi) {
        Ok(pw) => Some(pw),
        Err(e) => {
            tracing::warn!(
                error = %e,
                identity = %owner_id,
                "platform-wallet cache: no platform wallet for identity"
            );
            None
        }
    }
}

/// Route a `ManagedIdentity::set_contact_bloom_registered_count`
/// mutation through the platform-wallet changeset flow.
///
/// Replaces the direct `db.update_bloom_registered_count` call.
pub(crate) async fn cache_contact_bloom_registered_count(
    app_context: &AppContext,
    owner_id: &Identifier,
    contact_id: &Identifier,
    count: u32,
) {
    let Some(pw) = resolve_platform_wallet_by_owner(app_context, owner_id) else {
        return;
    };
    let contact_cs = {
        let mut state = pw.state_mut().await;
        let Some(managed) = state.identity_manager.managed_identity_mut(owner_id) else {
            tracing::debug!(
                identity = %owner_id,
                "platform-wallet cache: identity not in IdentityManager"
            );
            return;
        };
        managed.set_contact_bloom_registered_count(contact_id, count)
    };
    let cs = PlatformWalletChangeSet {
        contacts: Some(contact_cs),
        ..Default::default()
    };
    pw.queue_persist(cs);
}

// --- Blocking variants (SPV transaction-processing frame loop) ---
//
// These helpers take an already-resolved `&PlatformWallet` rather
// than looking one up through `AppContext`. The SPV frame loop in
// `context::transaction_processing` holds a write guard on the
// owning evo-tool `Wallet` (`wallet_arc.write()`) while matching
// transactions against wallet addresses. `resolve_platform_wallet_by_owner`
// would try to re-acquire a read guard on that same wallet via
// `AppContext::platform_wallet_for_identity`, which deadlocks
// deterministically on `std::sync::RwLock` (read-while-write on
// the same thread). The caller must have `wallet.platform_wallet`
// already in hand before entering the helper.
//
// All three use `PlatformWallet::state_mut_blocking`, so they MUST
// NOT be called from within a tokio async context.

/// Blocking payment-cache helper for the SPV frame loop. Records a
/// DashPay payment entry on the owner's `ManagedIdentity` and queues
/// the emitted changeset. See the module header for the deadlock
/// rationale behind the `&PlatformWallet` parameter.
pub(crate) fn cache_payment_with_pw_blocking(
    pw: &PlatformWallet,
    owner_id: &Identifier,
    tx_id: String,
    entry: PaymentEntry,
) {
    let id_cs = {
        let mut state = pw.state_mut_blocking();
        let Some(managed) = state.identity_manager.managed_identity_mut(owner_id) else {
            tracing::debug!(
                identity = %owner_id,
                "platform-wallet cache: identity not in IdentityManager"
            );
            return;
        };
        managed.record_dashpay_payment(tx_id, entry)
    };
    let cs = PlatformWalletChangeSet {
        identities: Some(id_cs),
        ..Default::default()
    };
    pw.queue_persist(cs);
}

/// Blocking contact-derivation bump helper for the SPV frame loop.
/// Routes a `ManagedIdentity::bump_contact_highest_receive_index`
/// mutation through the platform-wallet changeset flow. Monotonic —
/// if `index` is not greater than the current value in the
/// platform-wallet's in-memory state, the mutation emits an empty
/// changeset and the persister has nothing to write. See the module
/// header for the deadlock rationale behind the `&PlatformWallet`
/// parameter.
pub(crate) fn cache_contact_highest_receive_index_with_pw_blocking(
    pw: &PlatformWallet,
    owner_id: &Identifier,
    contact_id: &Identifier,
    index: u32,
) {
    let contact_cs = {
        let mut state = pw.state_mut_blocking();
        let Some(managed) = state.identity_manager.managed_identity_mut(owner_id) else {
            tracing::debug!(
                identity = %owner_id,
                "platform-wallet cache: identity not in IdentityManager"
            );
            return;
        };
        managed.bump_contact_highest_receive_index(contact_id, index)
    };
    let cs = PlatformWalletChangeSet {
        contacts: Some(contact_cs),
        ..Default::default()
    };
    pw.queue_persist(cs);
}

