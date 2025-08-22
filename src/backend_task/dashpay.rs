use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::utils::egui_mpsc::SenderAsync;
use dash_sdk::Sdk;
use std::sync::Arc;

pub mod auto_accept_handler;
pub mod auto_accept_proof;
pub mod avatar_processing;
pub mod contact_info;
pub mod contact_requests;
pub mod contacts;
pub mod encryption;
pub mod encryption_tests;
pub mod errors;
pub mod hd_derivation;
pub mod payments;
pub mod profile;
pub mod validation;

pub use contacts::ContactData;

use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::platform::{Identifier, IdentityPublicKey};

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
        identity: QualifiedIdentity,
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
        auto_accept_proof: Vec<u8>,
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
    UpdateContactInfo {
        identity: QualifiedIdentity,
        contact_id: Identifier,
        nickname: Option<String>,
        note: Option<String>,
        is_hidden: bool,
        accepted_accounts: Vec<u32>,
    },
}

impl AppContext {
    pub async fn run_dashpay_task(
        self: &Arc<Self>,
        task: DashPayTask,
        sdk: &Sdk,
        _sender: SenderAsync<TaskResult>,
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
            DashPayTask::SearchProfiles {
                identity,
                search_query,
            } => profile::search_profiles(self, sdk, identity, search_query).await,
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
                auto_accept_proof,
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
                    Some(auto_accept_proof),
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
            DashPayTask::LoadPaymentHistory { identity: _ } => {
                // TODO: Implement payment history loading according to DIP-0015
                // This requires:
                // 1. Get all established contacts (bidirectional contact requests)
                // 2. For each contact, derive payment addresses from their encrypted extended public key
                // 3. Query blockchain for transactions to/from those addresses
                // 4. Build payment history records with amount, timestamp, memo, etc.
                // 5. Store in local database for faster access
                //
                // The derivation path for DashPay addresses is:
                // m/9'/5'/15'/account'/(our_identity_id)/(contact_identity_id)/index
                //
                // For now, return empty payment history
                Ok(BackendTaskSuccessResult::DashPayPaymentHistory(Vec::new()))
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
        }
    }
}
