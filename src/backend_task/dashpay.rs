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
pub mod validation;

pub use contacts::ContactData;

use crate::model::dashpay::{ContactInfoUpdate, UnreadableContactInfoPolicy};
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::{DocumentQuery, Identifier, IdentityPublicKey};
use errors::DashPayError;

/// A fresh `contactRequest` [`DocumentQuery`] against the cached DashPay
/// contract. Callers add their own `where` / `order by` clauses and limit.
///
/// Every `contactRequest` read in this module goes through here, so the
/// contract handle and the error attribution are stated once.
fn contact_request_query(app_context: &AppContext) -> Result<DocumentQuery, DashPayError> {
    DocumentQuery::new(app_context.dashpay_contract.clone(), "contactRequest").map_err(|e| {
        DashPayError::QueryCreation {
            query_target: "DashPay contactRequest",
            source: Box::new(e),
        }
    })
}

#[derive(Debug, Clone, PartialEq)]
pub enum DashPayTask {
    LoadProfile {
        identity: QualifiedIdentity,
    },
    UpdateProfile {
        identity: QualifiedIdentity,
        display_name: Option<String>,
        bio: Option<String>,
        avatar_url: Option<String>,
    },
    LoadContacts {
        identity: QualifiedIdentity,
    },
    /// Read the contact list from offline state only — rehydrated
    /// relationships + private memos plus the DET contact-profile cache. No
    /// network round-trip, so a view renders without connectivity.
    LoadContactsOffline {
        identity: QualifiedIdentity,
    },
    LoadContactRequests {
        identity: QualifiedIdentity,
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
    AcceptContactRequest {
        identity: QualifiedIdentity,
        request_id: Identifier,
    },
    RejectContactRequest {
        identity: QualifiedIdentity,
        request_id: Identifier,
        unreadable: UnreadableContactInfoPolicy,
    },
    /// Withdraw a still-pending contact request this identity sent.
    ///
    /// Platform cannot delete a `contactRequest` document, so the handler
    /// broadcasts a hidden `contactInfo` and records the withdrawal locally —
    /// see [`contact_requests::cancel_contact_request`].
    CancelContactRequest {
        identity: QualifiedIdentity,
        request_id: Identifier,
        unreadable: UnreadableContactInfoPolicy,
    },
    LoadPaymentHistory {
        identity: QualifiedIdentity,
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
    /// Resolve received outputs (surfaced by the `EventBridge` when a wallet
    /// transaction is first seen) against every local identity's DashPay
    /// address map, recording the ones that pay a known contact. Idempotent
    /// on re-scan — recording is keyed by `tx_id` with last-write-wins, and
    /// the receive cursor only advances.
    DetectIncomingContactPayments {
        outputs: Vec<crate::model::dashpay::DetectedIncomingOutput>,
    },
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
        match task {
            DashPayTask::LoadProfile { identity } => {
                Ok(profile::load_profile(self, sdk, identity).await?)
            }
            DashPayTask::UpdateProfile {
                identity,
                display_name,
                bio,
                avatar_url,
            } => Ok(
                profile::update_profile(self, sdk, identity, display_name, bio, avatar_url).await?,
            ),
            DashPayTask::LoadContacts { identity } => {
                Ok(contacts::load_contacts(self, sdk, identity).await?)
            }
            DashPayTask::LoadContactsOffline { identity } => {
                Ok(contacts::load_contacts_offline(self, identity).await?)
            }
            DashPayTask::LoadContactRequests { identity } => {
                Ok(contact_requests::load_contact_requests(self, sdk, identity).await?)
            }
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
                request_id,
            } => contact_requests::accept_contact_request(self, sdk, identity, request_id)
                .await
                .map_err(|source| TaskError::DashPayContactRequestActionFailed {
                    request_id,
                    source: Box::new(source),
                }),
            DashPayTask::RejectContactRequest {
                identity,
                request_id,
                unreadable,
            } => contact_requests::reject_contact_request(
                self, sdk, identity, request_id, unreadable,
            )
            .await
            .map_err(|source| TaskError::DashPayContactRequestActionFailed {
                request_id,
                source: Box::new(source),
            }),
            DashPayTask::CancelContactRequest {
                identity,
                request_id,
                unreadable,
            } => contact_requests::cancel_contact_request(
                self, sdk, identity, request_id, unreadable,
            )
            .await
            .map_err(|source| TaskError::DashPayContactRequestActionFailed {
                request_id,
                source: Box::new(source),
            }),
            DashPayTask::LoadPaymentHistory { identity } => {
                let identity_id = identity.identity.id();
                // Refresh-style action: kick upstream before reading so the
                // view sees the latest contact / payment state.
                if let Ok(backend) = self.wallet_backend()
                    && let Err(e) = backend.dashpay_sync(&identity_id).await
                {
                    tracing::debug!(
                        identity = %identity_id,
                        error = ?e,
                        "LoadPaymentHistory: dashpay_sync degraded; reading cached state"
                    );
                }

                let records = payments::load_payment_history(self, &identity_id, None).await?;

                // The WalletBackend DashPay adapter is the sole source of truth
                // for contacts. Before it is wired (e.g. cold start) we surface
                // an empty list — a missing backend means "not loaded yet".
                let contacts = match self.wallet_backend() {
                    Ok(backend) => backend.dashpay_view().contacts(&identity_id).await,
                    Err(_) => Vec::new(),
                };

                let results: Vec<_> = records
                    .into_iter()
                    .map(|rec| {
                        let is_incoming = rec.to_identity == identity_id;
                        let contact_id = if is_incoming {
                            rec.from_identity
                        } else {
                            rec.to_identity
                        };

                        // Resolve the counterparty against saved DashPay contacts
                        // only. A payment recipient who is not a mutual contact
                        // (e.g. paid by DPNS username via the Pay screen) is not in
                        // this list, so it falls back to "Unknown (<base58 prefix>)".
                        // TODO(NEW-004): resolve non-contact counterparties to their
                        // DPNS username. That needs a DPNS lookup by identity id
                        // (network) or persisting the name resolved at send time —
                        // deeper plumbing than this cached-read path, so deferred.
                        let contact_name = contacts
                            .iter()
                            .find(|c| {
                                Identifier::from_bytes(&c.contact_identity_id)
                                    .map(|id| id == contact_id)
                                    .unwrap_or(false)
                            })
                            .and_then(|c| c.username.clone().or(c.display_name.clone()))
                            .unwrap_or_else(|| {
                                let s = contact_id.to_string(Encoding::Base58);
                                format!("Unknown ({})", &s[..s.len().min(8)])
                            });

                        (
                            rec.tx_id.unwrap_or_default(),
                            contact_name,
                            rec.amount,
                            is_incoming,
                            rec.memo.unwrap_or_default(),
                        )
                    })
                    .collect();

                Ok(BackendTaskSuccessResult::DashPayPaymentHistory(results))
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
            DashPayTask::DetectIncomingContactPayments { outputs } => {
                let recorded =
                    incoming_payments::detect_incoming_contact_payments(self, &outputs).await?;
                if recorded == 0 {
                    Ok(BackendTaskSuccessResult::None)
                } else {
                    Ok(BackendTaskSuccessResult::Refresh)
                }
            }
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
        }
    }
}
