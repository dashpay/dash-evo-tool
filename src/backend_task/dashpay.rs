use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use dash_sdk::Sdk;
use std::sync::Arc;

pub mod auto_accept_handler;
pub mod auto_accept_proof;
pub mod avatar_processing;
pub mod contact_info;
pub mod contact_requests;
pub mod contacts;
pub mod dip14_derivation;
pub mod encryption;
pub mod encryption_tests;
pub mod errors;
pub mod hd_derivation;
pub mod incoming_payments;
pub mod payments;
pub mod profile;
pub mod validation;

pub use contacts::ContactData;

use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::{Document, Identifier, IdentityPublicKey};

#[derive(Debug, Clone, PartialEq)]
pub enum DashPayResult {
    Profile(Option<(String, String, String)>), // (display_name, bio, avatar_url)
    ContactProfile(Option<Document>),          // Contact's public profile document
    ProfileSearchResults(Vec<(Identifier, Option<Document>, String)>), // (identity_id, profile_document, username)
    ContactRequests {
        incoming: Vec<(Identifier, Document)>,
        outgoing: Vec<(Identifier, Document)>,
    },
    Contacts(Vec<Identifier>),          // List of contact identity IDs
    ContactsWithInfo(Vec<ContactData>), // List of contacts with metadata
    PaymentHistory(Vec<(String, String, u64, bool, String)>), // (tx_id, contact_name, amount, is_incoming, memo)
    ProfileUpdated(Identifier),                               // Identity ID of updated profile
    ContactRequestSent(String),                               // Username or ID of recipient
    ContactRequestAccepted(Identifier),                       // Request ID that was accepted
    ContactRequestRejected(Identifier),                       // Request ID that was rejected
    ContactAlreadyEstablished(Identifier),                    // Contact ID that already exists
    ContactInfoUpdated(Identifier),                           // Contact ID whose info was updated
    PaymentSent(String, String, f64),                         // (recipient, address, amount)
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
    LoadContactRequests {
        identity: QualifiedIdentity,
    },
    FetchContactProfile {
        identity: QualifiedIdentity,
        contact_id: Identifier,
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
    },
    LoadPaymentHistory {
        identity: QualifiedIdentity,
    },
    SendPaymentToContact {
        identity: QualifiedIdentity,
        contact_id: Identifier,
        amount_dash: f64,
        memo: Option<String>,
    },
    UpdateContactInfo {
        identity: QualifiedIdentity,
        contact_id: Identifier,
        nickname: Option<String>,
        note: Option<String>,
        is_hidden: bool,
        accepted_accounts: Vec<u32>,
    },
    /// Register DashPay receiving addresses for incoming payment detection
    RegisterDashPayAddresses {
        identity: QualifiedIdentity,
    },
}

impl AppContext {
    pub async fn run_dashpay_task(
        self: &Arc<Self>,
        task: DashPayTask,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, String> {
        match task {
            DashPayTask::LoadProfile { identity } => {
                profile::load_profile(self, sdk, identity).await
            }
            DashPayTask::UpdateProfile {
                identity,
                display_name,
                bio,
                avatar_url,
            } => profile::update_profile(self, sdk, identity, display_name, bio, avatar_url).await,
            DashPayTask::LoadContacts { identity } => {
                contacts::load_contacts(self, sdk, identity).await
            }
            DashPayTask::LoadContactRequests { identity } => {
                contact_requests::load_contact_requests(self, sdk, identity).await
            }
            DashPayTask::FetchContactProfile {
                identity,
                contact_id,
            } => profile::fetch_contact_profile(self, sdk, identity, contact_id).await,
            DashPayTask::SearchProfiles { search_query } => {
                profile::search_profiles(self, sdk, search_query).await
            }
            DashPayTask::SendContactRequest {
                identity,
                signing_key,
                to_username,
                account_label,
            } => {
                contact_requests::send_contact_request(
                    self,
                    sdk,
                    identity,
                    signing_key,
                    to_username,
                    account_label,
                )
                .await
            }
            DashPayTask::SendContactRequestWithProof {
                identity,
                signing_key,
                to_identity_id,
                account_label,
                qr_auto_accept,
            } => {
                contact_requests::send_contact_request_with_proof(
                    self,
                    sdk,
                    identity,
                    signing_key,
                    to_identity_id.to_string(
                        dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58,
                    ),
                    account_label,
                    Some(qr_auto_accept),
                )
                .await
            }
            DashPayTask::AcceptContactRequest {
                identity,
                request_id,
            } => contact_requests::accept_contact_request(self, sdk, identity, request_id).await,
            DashPayTask::RejectContactRequest {
                identity,
                request_id,
            } => contact_requests::reject_contact_request(self, sdk, identity, request_id).await,
            DashPayTask::LoadPaymentHistory { identity } => {
                // Load locally stored payment records from database.
                // Full blockchain-based history (scanning DIP-15 addresses via SPV)
                // remains deferred until SPV support is available.
                let identity_id = identity.identity.id();
                let stored = self
                    .db
                    .load_payment_history(&identity_id, 100)
                    .map_err(|e| format!("Failed to load payment history: {}", e))?;

                let network_str = self.network.to_string();
                let contacts = self
                    .db
                    .load_dashpay_contacts(&identity_id, &network_str)
                    .unwrap_or_default();

                let mut results = Vec::new();
                for sp in stored {
                    let is_incoming = sp.to_identity_id == identity_id.to_buffer().to_vec();
                    let contact_bytes = if is_incoming {
                        &sp.from_identity_id
                    } else {
                        &sp.to_identity_id
                    };

                    let contact_name = contacts
                        .iter()
                        .find(|c| c.contact_identity_id == *contact_bytes)
                        .and_then(|c| c.username.clone().or(c.display_name.clone()))
                        .unwrap_or_else(|| {
                            if let Ok(cid) = Identifier::from_bytes(contact_bytes) {
                                let s = cid.to_string(Encoding::Base58);
                                format!("Unknown ({})", &s[..s.len().min(8)])
                            } else {
                                "Unknown".to_string()
                            }
                        });

                    results.push((
                        sp.tx_id,
                        contact_name,
                        sp.amount as u64,
                        is_incoming,
                        sp.memo.unwrap_or_default(),
                    ));
                }

                Ok(BackendTaskSuccessResult::DashPay(
                    DashPayResult::PaymentHistory(results),
                ))
            }
            DashPayTask::SendPaymentToContact {
                identity,
                contact_id,
                amount_dash,
                memo,
            } => {
                payments::send_payment_to_contact_impl(
                    self,
                    sdk,
                    identity,
                    contact_id,
                    amount_dash,
                    memo,
                )
                .await
            }
            DashPayTask::UpdateContactInfo {
                identity,
                contact_id,
                nickname,
                note,
                is_hidden,
                accepted_accounts,
            } => {
                contact_info::create_or_update_contact_info(
                    self,
                    sdk,
                    identity,
                    contact_id,
                    nickname,
                    note,
                    is_hidden,
                    accepted_accounts,
                )
                .await
            }
            DashPayTask::RegisterDashPayAddresses { identity } => {
                let result =
                    incoming_payments::register_dashpay_addresses_for_identity(self, &identity)
                        .await?;

                Ok(BackendTaskSuccessResult::Message(format!(
                    "Registered {} DashPay addresses for {} contacts{}",
                    result.addresses_registered,
                    result.contacts_processed,
                    if result.errors.is_empty() {
                        String::new()
                    } else {
                        format!(" ({} errors)", result.errors.len())
                    }
                )))
            }
        }
    }
}
