//! Keeping the DashPay snapshots current.
//!
//! platform-wallet's `DashPaySyncManager` syncs DashPay state in the
//! background; these tasks publish it to the UI and fill in what
//! platform-wallet does not hold.

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::dashpay::errors::DashPayError;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::dashpay::StoredProfile;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::wallet_backend::WalletBackend;
use dash_sdk::Sdk;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::drive::query::{WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, FetchMany, Identifier};
use futures::future::join_all;
use std::sync::Arc;

/// Usernames looked up at once after a sync pass.
const USERNAME_LOOKUP_CHUNK: usize = 10;

/// Republish the DashPay snapshots. After a new sync pass, first look up what
/// platform-wallet does not hold: counterparties' usernames, the profiles of
/// identities it does not sync, and the time this device first saw each
/// payment and profile.
pub(super) async fn refresh(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
) -> Result<BackendTaskSuccessResult, TaskError> {
    let backend = app_context.wallet_backend()?;
    let mut changed = false;
    if backend.take_dashpay_pass() {
        // TODO(upstream): on_dashpay_sync_completed — the frame loop polls the
        // pass time instead of hearing about each pass.
        changed |= backend.refresh_dashpay_snapshots().await;
        look_up_usernames(&backend, sdk).await;
        read_unsynced_profiles(app_context, &backend, sdk).await;
        let now_ms = chrono::Utc::now().timestamp_millis().max(0);
        backend.stamp_unseen_dashpay_payments(now_ms);
        backend.stamp_unseen_dashpay_profiles(now_ms);
    }
    changed |= backend.refresh_dashpay_snapshots().await;
    Ok(if changed {
        BackendTaskSuccessResult::DashPaySynced
    } else {
        BackendTaskSuccessResult::None
    })
}

/// Run a sync pass now, unless the network is not ready for one, and read
/// `identity`'s stored contact details, then republish the snapshots. A
/// failed contact details read leaves the details already held in place.
pub(super) async fn sync_now(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: Option<QualifiedIdentity>,
) -> Result<BackendTaskSuccessResult, TaskError> {
    let backend = app_context.wallet_backend()?;
    backend.dashpay_sync_now().await;
    if backend.dashpay_network_ready()
        && let Some(identity) = identity
        && identity.dashpay_wallet_seed_hash().is_some()
        && let Err(error) =
            super::contact_info::refresh_contact_details(app_context, sdk, &identity).await
    {
        tracing::debug!(
            ?error,
            "Stored contact details not read; showing the ones held"
        );
    }
    backend.refresh_dashpay_snapshots().await;
    Ok(BackendTaskSuccessResult::DashPaySynced)
}

/// Look up the DPNS username of every counterparty not looked up this
/// session. A failed lookup keeps the stored username.
async fn look_up_usernames(backend: &WalletBackend, sdk: &Sdk) {
    let ids = backend.take_usernames_to_look_up();
    let usernames = backend.contact_usernames();
    for chunk in ids.chunks(USERNAME_LOOKUP_CHUNK) {
        let lookups = chunk.iter().map(|id| async move {
            let names = sdk.get_dpns_usernames_by_identity(*id, Some(1)).await;
            (*id, names)
        });
        for (id, names) in join_all(lookups).await {
            let username = match names {
                Ok(names) => names.into_iter().next().map(|name| name.label),
                Err(error) => {
                    tracing::debug!(?error, "DashPay counterparty username not looked up");
                    continue;
                }
            };
            if let Err(error) = usernames.put(&id, username.as_deref()) {
                tracing::debug!(?error, "DashPay counterparty username not stored");
            }
        }
    }
}

/// Read, once per session, the profile of each local identity platform-wallet
/// does not sync: one no wallet manages, or one whose wallet is not loaded.
// TODO(upstream): wallet-less identities are not synced by
// `DashPaySyncManager`; remove once it syncs the unowned scope.
async fn read_unsynced_profiles(app_context: &Arc<AppContext>, backend: &WalletBackend, sdk: &Sdk) {
    let identities = match app_context.load_local_user_identities() {
        Ok(identities) => identities,
        Err(error) => {
            tracing::debug!(?error, "Local identities not listed for their profiles");
            return;
        }
    };
    for id in identities.iter().map(|identity| identity.identity.id()) {
        if backend.unsynced_profile_known(&id) || backend.dashpay_snapshot(&id).is_some() {
            continue;
        }
        match fetch_profile(app_context, sdk, id).await {
            Ok(profile) => backend.record_unsynced_profile(id, profile),
            Err(error) => tracing::debug!(?error, "Unsynced identity profile not read"),
        }
    }
}

/// The DashPay profile `owner` published, if any.
async fn fetch_profile(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    owner: Identifier,
) -> Result<Option<StoredProfile>, TaskError> {
    let mut query =
        DocumentQuery::new(app_context.dashpay_contract.clone(), "profile").map_err(|e| {
            DashPayError::QueryCreation {
                query_target: "DashPay profile",
                source: Box::new(e.into()),
            }
        })?;
    query = query.with_where(WhereClause {
        field: "$ownerId".to_string(),
        operator: WhereOperator::Equal,
        value: Value::Identifier(owner.to_buffer()),
    });
    query.limit = 1;
    let documents = Document::fetch_many(sdk, query).await?;
    Ok(documents
        .into_iter()
        .find_map(|(_, document)| document)
        .map(|document| {
            let text = |field: &str| {
                document
                    .get(field)
                    .and_then(|value| value.as_text())
                    .filter(|text| !text.is_empty())
                    .map(str::to_owned)
            };
            let bytes = |field: &str| {
                document
                    .get(field)
                    .and_then(|value| value.as_bytes_slice().ok())
                    .map(<[u8]>::to_vec)
            };
            StoredProfile {
                identity_id: owner.to_buffer().to_vec(),
                display_name: text("displayName"),
                // The contract's "publicMessage" field is the bio.
                bio: text("publicMessage"),
                avatar_url: text("avatarUrl"),
                avatar_hash: bytes("avatarHash"),
                avatar_fingerprint: bytes("avatarFingerprint"),
                avatar_bytes: None,
                public_message: text("publicMessage"),
                created_at: document.created_at().map_or(0, |ms| ms as i64),
                updated_at: document.updated_at().map_or(0, |ms| ms as i64),
            }
        }))
}
