use crate::backend_task::error::TaskError;
use crate::backend_task::{
    BackendTaskSuccessResult, DashPayPaymentHistoryEntry, DashPayPaymentHistoryResult,
};
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
use dash_sdk::platform::{Identifier, IdentityPublicKey};

const PARTIAL_PAYMENT_HISTORY_WARNING: &str = "Some older DashPay payments may be missing. Unlock all associated wallets and refresh payment history to complete the backfill.";

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
            DashPayTask::LoadContactRequests { identity } => {
                Ok(contact_requests::load_contact_requests(self, sdk, identity).await?)
            }
            DashPayTask::FetchContactProfile {
                identity,
                contact_id,
            } => Ok(profile::fetch_contact_profile(self, sdk, identity, contact_id).await?),
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
            } => Ok(
                contact_requests::accept_contact_request(self, sdk, identity, request_id).await?,
            ),
            DashPayTask::RejectContactRequest {
                identity,
                request_id,
            } => Ok(
                contact_requests::reject_contact_request(self, sdk, identity, request_id).await?,
            ),
            DashPayTask::LoadPaymentHistory { identity } => {
                let identity_id = identity.identity.id();
                let network_str = self.network.to_string();
                let mut payment_history_warning = None;

                // Retroactively scan wallet transactions for DashPay payments
                // that may not yet be in the dashpay_payments table. Once a
                // wallet has completed its full backfill scan we rely on the
                // lifecycle reconcile path to handle deltas.
                for (seed_hash, wallet_arc) in &identity.associated_wallets {
                    let has_scan_marker = self.db.has_dashpay_wallet_tx_scan_marker(
                        seed_hash,
                        &identity_id,
                        &network_str,
                    )?;
                    if has_scan_marker {
                        continue;
                    }

                    let (wallet_txs, seed) = match wallet_arc.read() {
                        Ok(guard) => {
                            if guard.transactions.is_empty() {
                                continue;
                            }

                            if !guard.is_open() {
                                payment_history_warning =
                                    Some(PARTIAL_PAYMENT_HISTORY_WARNING.to_string());
                                continue;
                            }

                            let Ok(seed) = guard.seed_bytes() else {
                                payment_history_warning =
                                    Some(PARTIAL_PAYMENT_HISTORY_WARNING.to_string());
                                continue;
                            };

                            (guard.transactions.clone(), *seed)
                        }
                        Err(_) => {
                            return Err(TaskError::DashPayScan {
                                source: incoming_payments::DashPayScanError::WalletLockPoisoned,
                            });
                        }
                    };

                    let scan_mode = incoming_payments::DashPayWalletScanMode::Full {
                        active_seed_bytes: &seed,
                        active_wallet: wallet_arc,
                    };

                    let scan_result =
                        incoming_payments::scan_wallet_transactions_for_dashpay_payments(
                            self,
                            &identity_id,
                            &wallet_txs,
                            scan_mode,
                        )
                        .map_err(|source| TaskError::DashPayScan { source })?;

                    if scan_result.mappings_available {
                        self.db.mark_dashpay_wallet_tx_scan_complete(
                            seed_hash,
                            &identity_id,
                            &network_str,
                        )?;
                    }
                    if scan_result.saved_payments > 0 {
                        tracing::info!(
                            "Retroactively discovered {} DashPay payment(s) from wallet transactions",
                            scan_result.saved_payments
                        );
                    }
                }

                let records = payments::load_payment_history(self, &identity_id, None)
                    .await
                    .map_err(
                        |e| crate::backend_task::dashpay::errors::DashPayError::Internal {
                            message: e,
                        },
                    )?;

                let contacts = self
                    .db
                    .load_dashpay_contacts(&identity_id, &network_str)
                    .map_err(|source| TaskError::Database { source })?;

                let results: Vec<_> = records
                    .into_iter()
                    .map(|rec| {
                        let is_incoming = rec.to_identity == identity_id;
                        let contact_id = if is_incoming {
                            rec.from_identity
                        } else {
                            rec.to_identity
                        };

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

                        DashPayPaymentHistoryEntry {
                            tx_id: rec.tx_id.unwrap_or_default(),
                            contact_name,
                            amount: rec.amount,
                            is_incoming,
                            memo: rec.memo.unwrap_or_default(),
                            timestamp: rec.timestamp,
                        }
                    })
                    .collect();

                Ok(BackendTaskSuccessResult::DashPayPaymentHistory(
                    DashPayPaymentHistoryResult {
                        payments: results,
                        warning: payment_history_warning,
                    },
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
            } => Ok(contact_info::create_or_update_contact_info(
                self,
                sdk,
                identity,
                contact_id,
                nickname,
                note,
                is_hidden,
                accepted_accounts,
            )
            .await?),
            DashPayTask::RegisterDashPayAddresses { identity } => {
                let result =
                    incoming_payments::register_dashpay_addresses_for_identity(self, &identity)
                        .await
                        .map_err(|source| TaskError::DashPayScan { source })?;

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
