//! Helpers for routing DashPay mutations through the platform-wallet
//! changeset flow.
//!
//! Phase 9b migrates backend tasks away from direct `Database::*` DB
//! writes and onto mutation methods that emit changesets the
//! persister catches on flush. Every helper follows the same shape:
//! resolve the owner's `PlatformWallet`, acquire its state-mut
//! guard, call the mutation on `ManagedIdentity`, wrap the emitted
//! sub-changeset in a top-level [`PlatformWalletChangeSet`], and
//! queue it via `pw.queue_persist`.
//!
//! Silent no-op (with a `tracing::warn!` / `tracing::debug!`) if the
//! owner identity isn't present in the platform-wallet's
//! `IdentityManager` — losing the cache is not worth failing the
//! outer operation.
//!
//! The `_with_pw_blocking` variants take an already-resolved
//! `&PlatformWallet` and are only for the SPV frame loop in
//! `context::transaction_processing`, which holds a write guard on
//! the owning evo-tool `Wallet`. Going through `AppContext` there
//! would re-acquire a read guard on that same wallet and deadlock
//! deterministically on `std::sync::RwLock` (read-while-write on
//! the same thread). Both blocking helpers use
//! `PlatformWallet::state_mut_blocking`, so they MUST NOT be called
//! from a tokio async context.

use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use platform_wallet::PlatformWallet;
use platform_wallet::changeset::PlatformWalletChangeSet;
use platform_wallet::wallet::dashpay::{DashPayProfile, PaymentEntry};
use std::sync::Arc;

/// Resolve the `PlatformWallet` for a `QualifiedIdentity`. Shared by
/// all async cache helpers — logs a `tracing::warn!` and returns
/// `None` when the owner has no platform wallet (treat as no-op).
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

/// Route a `ManagedIdentity::set_dashpay_profile` mutation through
/// the platform-wallet changeset flow. Replaces the direct
/// `db.save_dashpay_profile` call.
pub(crate) async fn cache_profile(
    app_context: &AppContext,
    identity: &QualifiedIdentity,
    profile: Option<DashPayProfile>,
) {
    let Some(pw) = resolve_platform_wallet(app_context, identity) else {
        return;
    };
    let owner_id = identity.identity.id();
    let id_cs = {
        let mut state = pw.state_mut().await;
        let Some(managed) = state.identity_manager.managed_identity_mut(&owner_id) else {
            tracing::debug!(
                identity = %owner_id,
                "platform-wallet cache: identity not in IdentityManager"
            );
            return;
        };
        managed.set_dashpay_profile(profile)
    };
    pw.queue_persist(PlatformWalletChangeSet {
        identities: Some(id_cs),
        ..Default::default()
    });
}

/// Route a `ManagedIdentity::record_dashpay_payment` mutation through
/// the platform-wallet changeset flow. Replaces the direct
/// `db.save_payment` call.
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
    let id_cs = {
        let mut state = pw.state_mut().await;
        let Some(managed) = state.identity_manager.managed_identity_mut(&owner_id) else {
            tracing::debug!(
                identity = %owner_id,
                "platform-wallet cache: identity not in IdentityManager"
            );
            return;
        };
        managed.record_dashpay_payment(tx_id, entry)
    };
    pw.queue_persist(PlatformWalletChangeSet {
        identities: Some(id_cs),
        ..Default::default()
    });
}

/// Route a `ManagedIdentity::set_contact_bloom_registered_count`
/// mutation through the platform-wallet changeset flow. Replaces the
/// direct `db.update_bloom_registered_count` call.
pub(crate) async fn cache_contact_bloom_registered_count(
    app_context: &AppContext,
    identity: &QualifiedIdentity,
    contact_id: &Identifier,
    count: u32,
) {
    let Some(pw) = resolve_platform_wallet(app_context, identity) else {
        return;
    };
    let owner_id = identity.identity.id();
    let contact_cs = {
        let mut state = pw.state_mut().await;
        let Some(managed) = state.identity_manager.managed_identity_mut(&owner_id) else {
            tracing::debug!(
                identity = %owner_id,
                "platform-wallet cache: identity not in IdentityManager"
            );
            return;
        };
        managed.set_contact_bloom_registered_count(contact_id, count)
    };
    pw.queue_persist(PlatformWalletChangeSet {
        contacts: Some(contact_cs),
        ..Default::default()
    });
}

// --- Blocking variants (SPV transaction-processing frame loop) ---

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
    pw.queue_persist(PlatformWalletChangeSet {
        identities: Some(id_cs),
        ..Default::default()
    });
}

/// Blocking contact-index bump helper for the SPV frame loop.
/// Monotonic — if `index` is not greater than the current value, the
/// mutation emits an empty changeset and the persister has nothing
/// to write.
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
    pw.queue_persist(PlatformWalletChangeSet {
        contacts: Some(contact_cs),
        ..Default::default()
    });
}
