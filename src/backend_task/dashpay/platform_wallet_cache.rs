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
//! `&PlatformWallet` and are called from
//! `context::transaction_processing`, which holds a write guard on
//! the owning evo-tool `Wallet` on the egui main thread (ZMQ tx
//! finality path). Going through `AppContext` there would re-acquire
//! a read guard on that same wallet and deadlock deterministically on
//! `std::sync::RwLock`. They cannot use `state_mut_blocking()`
//! either, because the main thread is inside the tokio runtime and
//! `tokio::sync::RwLock::blocking_write` panics in that context.
//! Instead they dispatch the mutation to a `tokio::spawn`ed task that
//! runs on a worker thread and uses the async `state_mut().await`
//! path. Mutations are fire-and-forget — the persister catches the
//! changeset on the next flush.

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

// --- Deferred variants (called from main-thread ZMQ tx finality) ---

/// Payment-cache helper called from the main thread on ZMQ tx
/// finality. Records a DashPay payment entry on the owner's
/// `ManagedIdentity` and queues the emitted changeset.
///
/// Mutation runs in a `tokio::spawn`ed task — the main thread cannot
/// take the wallet-manager write lock directly (tokio's
/// `blocking_write` panics inside the runtime context). Fire-and-
/// forget: the persister catches the changeset on the next flush.
pub(crate) fn cache_payment_with_pw_blocking(
    pw: &PlatformWallet,
    owner_id: &Identifier,
    tx_id: String,
    entry: PaymentEntry,
) {
    let pw = pw.clone();
    let owner_id = *owner_id;
    tokio::spawn(async move {
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
    });
}

/// Contact-index bump helper called from the main thread on ZMQ tx
/// finality. Monotonic — if `index` is not greater than the current
/// value, the mutation emits an empty changeset and the persister
/// has nothing to write. Mutation runs in a `tokio::spawn`ed task
/// (see [`cache_payment_with_pw_blocking`] for rationale).
pub(crate) fn cache_contact_highest_receive_index_with_pw_blocking(
    pw: &PlatformWallet,
    owner_id: &Identifier,
    contact_id: &Identifier,
    index: u32,
) {
    let pw = pw.clone();
    let owner_id = *owner_id;
    let contact_id = *contact_id;
    tokio::spawn(async move {
        let contact_cs = {
            let mut state = pw.state_mut().await;
            let Some(managed) = state.identity_manager.managed_identity_mut(&owner_id) else {
                tracing::debug!(
                    identity = %owner_id,
                    "platform-wallet cache: identity not in IdentityManager"
                );
                return;
            };
            managed.bump_contact_highest_receive_index(&contact_id, index)
        };
        pw.queue_persist(PlatformWalletChangeSet {
            contacts: Some(contact_cs),
            ..Default::default()
        });
    });
}
