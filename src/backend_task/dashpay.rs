use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use dash_sdk::Sdk;
use std::sync::Arc;

pub mod auto_accept_handler;
pub mod auto_accept_proof;
pub mod avatar_processing;
pub mod contact_info;
pub mod contact_requests;
pub mod contacts;
pub mod encryption;
pub mod errors;
pub mod incoming_payments;
pub mod payments;
pub mod profile;
pub mod sync;
pub mod validation;

pub use crate::model::dashpay::ContactData;

use crate::model::dashpay::{ContactInfoUpdate, ContactRequestKey};
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::{DocumentQuery, Identifier, IdentityPublicKey};
use errors::DashPayError;

/// A fresh `contactRequest` [`DocumentQuery`] against the cached DashPay
/// contract. Callers add their own `where` / `order by` clauses and limit.
///
/// Every `contactRequest` read in this module goes through here, so the
/// contract handle and the error attribution are stated once.
fn contact_request_query(app_context: &AppContext) -> Result<DocumentQuery, DashPayError> {
    DocumentQuery::new(app_context.dashpay_contract(), "contactRequest").map_err(|e| {
        DashPayError::QueryCreation {
            query_target: "DashPay contactRequest",
            source: Box::new(e.into()),
        }
    })
}

// TODO(dashpay-invitations): creating and claiming DashPay invitations, and
// withdrawing credits to a contact, are not offered yet. See
// https://github.com/dashpay/dash-evo-tool/issues/1104
#[derive(Debug, Clone, PartialEq)]
pub enum DashPayTask {
    UpdateProfile {
        identity: QualifiedIdentity,
        display_name: Option<String>,
        bio: Option<String>,
        avatar_url: Option<String>,
    },
    FetchContactProfile {
        identity: QualifiedIdentity,
        contact_id: Identifier,
    },
    /// Fetch an avatar image for a URL, consulting and populating the DET avatar
    /// disk cache. Off the egui frame loop; the result flows back to the
    /// screen's [`AvatarCache`](crate::ui::state::avatar_cache::AvatarCache).
    FetchAvatar {
        url: String,
    },
    SearchProfiles {
        search_query: String,
    },
    SendContactRequest {
        identity: QualifiedIdentity,
        signing_key: IdentityPublicKey,
        to_username: String,
        account_label: Option<String>,
    },
    SendContactRequestWithProof {
        identity: QualifiedIdentity,
        signing_key: IdentityPublicKey,
        to_identity_id: Identifier,
        account_label: Option<String>,
        qr_auto_accept: crate::backend_task::dashpay::auto_accept_proof::AutoAcceptProofData,
    },
    /// Accept the contact request `counterparty_id` sent this identity.
    AcceptContactRequest {
        identity: QualifiedIdentity,
        counterparty_id: Identifier,
    },
    /// Decline the contact request `counterparty_id` sent this identity.
    /// Local only: the sender is ignored on this device and nothing is
    /// published.
    RejectContactRequest {
        identity: QualifiedIdentity,
        counterparty_id: Identifier,
    },
    /// Withdraw this identity's still-pending contact request to
    /// `counterparty_id`.
    ///
    /// Platform cannot delete a `contactRequest` document, so the handler
    /// records the withdrawal on this device only — see
    /// [`contact_requests::cancel_contact_request`].
    CancelContactRequest {
        identity: QualifiedIdentity,
        counterparty_id: Identifier,
    },
    SendPaymentToContact {
        identity: QualifiedIdentity,
        contact_id: Identifier,
        amount_duffs: u64,
        memo: Option<String>,
    },
    UpdateContactInfo {
        identity: QualifiedIdentity,
        contact_id: Identifier,
        /// Explicit intent for every field in the whole encrypted payload.
        update: ContactInfoUpdate,
    },
    /// Register DashPay receiving addresses for incoming payment detection
    RegisterDashPayAddresses {
        identity: QualifiedIdentity,
    },
    /// Run a DashPay sync pass now, then republish the DashPay snapshots.
    /// With an identity, also read the contact details its `contactInfo`
    /// documents hold with the wallet seed, which may ask for the wallet
    /// password.
    SyncNow {
        identity: Option<QualifiedIdentity>,
    },
    /// Republish the DashPay snapshots from platform-wallet's state, after
    /// looking up what it does not hold when a sync pass completed. The frame
    /// loop dispatches this; it never prompts and never fails.
    RefreshSnapshots,
    /// Build an auto-accept contact QR payload. Resolves the ENCRYPTION key and
    /// derives the auto-accept key through the JIT chokepoint in the backend, so
    /// the UI never reads a seed.
    GenerateAutoAcceptQrCode {
        identity: QualifiedIdentity,
        account_reference: u32,
        validity_hours: u32,
    },
}

impl AppContext {
    pub async fn run_dashpay_task(
        self: &Arc<Self>,
        task: DashPayTask,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        if matches!(task, DashPayTask::RefreshSnapshots) {
            return sync::refresh(self, sdk).await;
        }
        // DashPay resolves identities through their wallet; one stored after
        // that wallet bootstrapped is not registered with it yet.
        self.reconcile_managed_identities_for_loaded_wallets().await;
        let changes_state = task.changes_dashpay_state();
        let result = match task {
            DashPayTask::UpdateProfile {
                identity,
                display_name,
                bio,
                avatar_url,
            } => Ok(
                profile::update_profile(self, sdk, identity, display_name, bio, avatar_url).await?,
            ),
            DashPayTask::FetchContactProfile {
                identity,
                contact_id,
            } => Ok(profile::fetch_contact_profile(self, sdk, identity, contact_id).await?),
            DashPayTask::FetchAvatar { url } => {
                let bytes = avatar_processing::fetch_avatar_cached(self, &url).await;
                Ok(BackendTaskSuccessResult::DashPayAvatar { url, bytes })
            }
            DashPayTask::SearchProfiles { search_query } => {
                Ok(profile::search_profiles(self, sdk, search_query).await?)
            }
            DashPayTask::SendContactRequest {
                identity,
                signing_key,
                to_username,
                account_label,
            } => Ok(contact_requests::send_contact_request(
                self,
                sdk,
                identity,
                signing_key,
                to_username,
                account_label,
            )
            .await?),
            DashPayTask::SendContactRequestWithProof {
                identity,
                signing_key,
                to_identity_id,
                account_label,
                qr_auto_accept,
            } => Ok(contact_requests::send_contact_request_with_proof(
                self,
                sdk,
                identity,
                signing_key,
                to_identity_id
                    .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58),
                account_label,
                Some(qr_auto_accept),
            )
            .await?),
            DashPayTask::AcceptContactRequest {
                identity,
                counterparty_id,
            } => {
                let request = ContactRequestKey {
                    owner: identity.identity.id(),
                    counterparty: counterparty_id,
                };
                contact_requests::accept_contact_request(self, sdk, identity, counterparty_id)
                    .await
                    .map_err(|source| TaskError::DashPayContactRequestActionFailed {
                        request,
                        source: Box::new(source),
                    })
            }
            DashPayTask::RejectContactRequest {
                identity,
                counterparty_id,
            } => {
                let request = ContactRequestKey {
                    owner: identity.identity.id(),
                    counterparty: counterparty_id,
                };
                contact_requests::reject_contact_request(self, sdk, identity, counterparty_id)
                    .await
                    .map_err(|source| TaskError::DashPayContactRequestActionFailed {
                        request,
                        source: Box::new(source),
                    })
            }
            DashPayTask::CancelContactRequest {
                identity,
                counterparty_id,
            } => {
                let request = ContactRequestKey {
                    owner: identity.identity.id(),
                    counterparty: counterparty_id,
                };
                contact_requests::cancel_contact_request(self, sdk, identity, counterparty_id)
                    .await
                    .map_err(|source| TaskError::DashPayContactRequestActionFailed {
                        request,
                        source: Box::new(source),
                    })
            }
            DashPayTask::SendPaymentToContact {
                identity,
                contact_id,
                amount_duffs,
                memo,
            } => {
                payments::send_payment_to_contact(
                    self,
                    sdk,
                    identity,
                    contact_id,
                    amount_duffs,
                    memo,
                )
                .await
            }
            DashPayTask::UpdateContactInfo {
                identity,
                contact_id,
                update,
            } => {
                let identity_id = identity.identity.id();
                contact_info::create_or_update_contact_info(self, sdk, identity, contact_id, update)
                    .await
                    .map_err(|source| TaskError::DashPayContactInfoActionFailed {
                        identity_id,
                        contact_id,
                        source: Box::new(source),
                    })
            }
            DashPayTask::RegisterDashPayAddresses { identity } => {
                let result =
                    incoming_payments::register_dashpay_addresses_for_identity(self, &identity)
                        .await?;

                Ok(BackendTaskSuccessResult::DashPayAddressesRegistered {
                    addresses: result.addresses_registered,
                    contacts: result.contacts_processed,
                    errors: result.errors.len(),
                })
            }
            DashPayTask::SyncNow { identity } => sync::sync_now(self, sdk, identity).await,
            DashPayTask::RefreshSnapshots => sync::refresh(self, sdk).await,
            DashPayTask::GenerateAutoAcceptQrCode {
                identity,
                account_reference,
                validity_hours,
            } => {
                let proof = auto_accept_proof::generate_auto_accept_proof(
                    &identity,
                    account_reference,
                    validity_hours,
                )
                .await?;
                Ok(BackendTaskSuccessResult::DashPayAutoAcceptQrCode(
                    proof.to_qr_string(),
                ))
            }
        };
        // A write is shown as soon as its result lands, not after the next
        // periodic refresh.
        if changes_state && let Ok(backend) = self.wallet_backend() {
            backend.refresh_dashpay_snapshots().await;
        }
        result
    }
}

impl DashPayTask {
    /// Whether the task can change what the DashPay snapshots show.
    fn changes_dashpay_state(&self) -> bool {
        match self {
            Self::UpdateProfile { .. }
            | Self::SendContactRequest { .. }
            | Self::SendContactRequestWithProof { .. }
            | Self::AcceptContactRequest { .. }
            | Self::RejectContactRequest { .. }
            | Self::CancelContactRequest { .. }
            | Self::SendPaymentToContact { .. }
            | Self::UpdateContactInfo { .. } => true,
            Self::FetchContactProfile { .. }
            | Self::FetchAvatar { .. }
            | Self::SearchProfiles { .. }
            | Self::RegisterDashPayAddresses { .. }
            | Self::GenerateAutoAcceptQrCode { .. }
            | Self::SyncNow { .. }
            | Self::RefreshSnapshots => false,
        }
    }
}
