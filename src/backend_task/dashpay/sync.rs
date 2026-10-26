//! Keeping the DashPay snapshots current.
//!
//! platform-wallet's `DashPaySyncManager` syncs DashPay state in the
//! background; these tasks publish it to the UI and fill in what
//! platform-wallet does not hold.

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::dashpay::StoredProfile;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::wallet_backend::WalletBackend;
use dash_sdk::Sdk;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::{Document, Identifier};
use futures::future::join_all;
use std::collections::{BTreeMap, BTreeSet};
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
        // TODO(upstream): dashpay/platform#5596 — no sync-completed event; the
        // frame loop polls the pass time instead of hearing about each pass.
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
// TODO(upstream): dashpay/platform#5600 — wallet-less identities are not
// synced by `DashPaySyncManager`; remove once it syncs the unowned scope.
async fn read_unsynced_profiles(app_context: &Arc<AppContext>, backend: &WalletBackend, sdk: &Sdk) {
    let identities = match app_context.load_local_user_identities() {
        Ok(identities) => identities,
        Err(error) => {
            tracing::debug!(?error, "Local identities not listed for their profiles");
            return;
        }
    };
    let owners: Vec<Identifier> = identities
        .iter()
        .map(|identity| identity.identity.id())
        .filter(|id| !backend.unsynced_profile_known(id) && backend.dashpay_snapshot(id).is_none())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if owners.is_empty() {
        return;
    }
    match sdk
        .get_profiles(&app_context.dashpay_contract(), &owners)
        .await
    {
        Ok(documents) => {
            for (owner, profile) in owner_profiles(&owners, &documents) {
                backend.record_unsynced_profile(owner, profile);
            }
        }
        Err(error) => tracing::debug!(?error, "Unsynced identity profiles not read"),
    }
}

/// Each of `owners` with the profile `documents` holds for it, `None` for an
/// owner that published none.
fn owner_profiles(
    owners: &[Identifier],
    documents: &BTreeMap<Identifier, Document>,
) -> Vec<(Identifier, Option<StoredProfile>)> {
    owners
        .iter()
        .map(|owner| {
            let profile = documents
                .get(owner)
                .map(|document| stored_profile(*owner, document));
            (*owner, profile)
        })
        .collect()
}

/// `owner`'s profile as `document` publishes it.
fn stored_profile(owner: Identifier, document: &Document) -> StoredProfile {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::document::DocumentV0;
    use dash_sdk::dpp::platform_value::Value;

    fn id(byte: u8) -> Identifier {
        Identifier::from([byte; 32])
    }

    /// Every owner asked about gets an answer: its published profile, or
    /// none when it published no profile.
    #[test]
    fn an_owner_without_a_profile_is_recorded_as_having_none() {
        let document = Document::V0(DocumentV0 {
            owner_id: id(1),
            properties: [
                ("displayName".to_string(), Value::Text("Alice".into())),
                ("publicMessage".to_string(), Value::Text(String::new())),
            ]
            .into(),
            created_at: Some(5),
            ..Default::default()
        });
        let documents = BTreeMap::from([(id(1), document)]);

        let profiles = owner_profiles(&[id(1), id(2)], &documents);

        let [(alice, Some(profile)), (bob, None)] = profiles.as_slice() else {
            panic!("expected Alice's profile and none for Bob, got {profiles:?}");
        };
        assert_eq!((*alice, *bob), (id(1), id(2)));
        assert_eq!(profile.display_name.as_deref(), Some("Alice"));
        assert_eq!(profile.bio, None, "an empty bio is no bio");
        assert_eq!(profile.created_at, 5);
    }
}
