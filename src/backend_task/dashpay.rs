use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use dash_sdk::Sdk;
use std::sync::Arc;

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
    /// Send a contact request to the owner of a scanned contact QR code,
    /// carrying the auto-accept proof its key signs.
    SendContactRequestWithProof {
        identity: QualifiedIdentity,
        signing_key: IdentityPublicKey,
        qr: crate::model::dashpay::ContactQr,
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
    /// Build the identity's contact QR code through upstream platform-wallet;
    /// the wallet seed is read through the secret seam in the backend, so the
    /// UI never reads a seed.
    GenerateAutoAcceptQrCode {
        identity: QualifiedIdentity,
    },
}

impl AppContext {
    pub async fn run_dashpay_task(
        self: &Arc<Self>,
        task: DashPayTask,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // DashPay resolves identities through their wallet; one stored after
        // that wallet bootstrapped is not registered with it yet.
        self.reconcile_managed_identities_for_loaded_wallets().await;
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
                qr,
            } => Ok(contact_requests::send_contact_request_with_proof(
                self,
                sdk,
                identity,
                signing_key,
                qr.dpns_name(),
                None,
                Some(qr),
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
                    Ok(BackendTaskSuccessResult::CacheUpdated(
                        crate::backend_task::CacheUpdate {
                            network: self.network,
                            domain: crate::backend_task::CacheDomain::ContactPayments,
                        },
                    ))
                }
            }
            DashPayTask::GenerateAutoAcceptQrCode { identity } => {
                let seed_hash = self.contact_qr_wallet_seed_hash(&identity)?;
                let uri = self
                    .wallet_backend()?
                    .dashpay_build_auto_accept_qr(&identity, seed_hash)
                    .await?;
                Ok(BackendTaskSuccessResult::DashPayAutoAcceptQrCode(uri))
            }
        }
    }
}

#[cfg(test)]
mod qr_generation_tests {
    use super::*;
    use crate::context::test_support::{bare_user_identity, test_app_context};
    use crate::model::dashpay::ContactQr;
    use crate::model::qualified_identity::DPNSNameInfo;
    use crate::model::wallet::Wallet;
    use crate::model::wallet::birth_height::WalletOrigin;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::key_wallet::wallet::Wallet as UpstreamWallet;
    use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;
    use platform_wallet::wallet::identity::crypto::auto_accept::verify_auto_accept_proof;

    /// An offline SDK on which Platform proves that `owner` owns `alice.dash`.
    async fn sdk_proving_owner_name(ctx: &AppContext, owner: Identifier) -> dash_sdk::Sdk {
        use crate::context::test_support::dpns_name_document;
        use dash_sdk::dpp::document::Document;
        use dash_sdk::dpp::platform_value::Value;
        use dash_sdk::drive::query::{SelectProjection, WhereClause, WhereOperator};
        use dash_sdk::platform::DocumentQuery;
        use dash_sdk::query_types::Documents;

        let mut sdk = crate::context::test_support::mock_network_sdk_for(ctx);
        let query = DocumentQuery {
            select: SelectProjection::documents(),
            data_contract: ctx.dpns_contract(),
            document_type_name: "domain".into(),
            where_clauses: vec![WhereClause {
                field: "records.identity".into(),
                operator: WhereOperator::Equal,
                value: Value::Identifier(owner.to_buffer()),
            }],
            time_range_clauses: vec![],
            integer_range_clauses: vec![],
            sub_queries: vec![],
            group_by: vec![],
            having: vec![],
            order_by_clauses: vec![],
            limit: 10,
            offset: None,
            start: None,
        };
        let domain = dpns_name_document(owner, "alice", Value::Identifier(owner.to_buffer()));
        let documents: Documents = [(Identifier::new([0x5D; 32]), Some(domain))]
            .into_iter()
            .collect();
        sdk.mock()
            .expect_fetch_many::<Identifier, Document, _, Documents>(query, Some(documents))
            .await
            .expect("domain expectation");
        sdk
    }

    /// The code names the username Platform proves the identity owns, never
    /// a cached one: a name can be transferred, so a cached name may now
    /// send paid contact requests to someone else.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn generated_contact_qr_uses_the_linked_wallet_through_the_secret_session() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        let owner_id = Identifier::new([2; 32]);
        ctx.sdk
            .store(Arc::new(sdk_proving_owner_name(&ctx, owner_id).await));
        let (tx, _rx) = tokio::sync::mpsc::channel(32);
        ctx.ensure_wallet_backend(crate::utils::egui_mpsc::SenderAsync::new(
            tx,
            ctx.egui_ctx().clone(),
        ))
        .await
        .expect("wire wallet backend");
        let mut wallets = Vec::new();
        for label in ["Unrelated", "Owner"] {
            let seed = crate::test_support::keys::seed();
            let wallet = Wallet::new_from_seed(seed, Network::Testnet, Some(label.into()), None)
                .expect("wallet");
            let (hash, wallet) = ctx
                .register_wallet(wallet, &seed, WalletOrigin::Imported)
                .expect("register wallet");
            ctx.wallet_backend()
                .expect("wallet backend")
                .register_wallet_from_seed(&hash, &seed, Some(0))
                .await
                .expect("register upstream wallet");
            wallets.push((hash, wallet, seed));
        }
        wallets.sort_by_key(|wallet| wallet.0);
        let unrelated = &wallets[0];
        let owner = &wallets[1];
        let mut identity = bare_user_identity(owner_id, Network::Testnet);
        identity.dpns_names.push(DPNSNameInfo {
            name: "transferred.dash".into(),
            acquired_at: 0,
        });
        for (hash, wallet, _) in &wallets {
            identity.associated_wallets.insert(*hash, wallet.clone());
        }
        ctx.insert_local_qualified_identity(&identity, &Some((owner.0, 0)))
            .expect("store owner link");
        assert_ne!(identity.dashpay_wallet_seed_hash(), Some(owner.0));

        let sdk = crate::context::test_support::mock_sdk_for(&ctx);
        let result = ctx
            .run_dashpay_task(DashPayTask::GenerateAutoAcceptQrCode { identity }, &sdk)
            .await
            .expect("generate QR through the production task");
        let BackendTaskSuccessResult::DashPayAutoAcceptQrCode(uri) = result else {
            panic!("expected generated contact QR");
        };
        assert!(uri.starts_with("dash:?du=alice&"), "{uri}");
        let now = u64::try_from(chrono::Utc::now().timestamp()).expect("current time");
        let sender = Identifier::new([3; 32]);
        let proof = ContactQr::parse(&uri, now)
            .expect("parse upstream QR")
            .sign_proof(&sender, &owner_id, 3, now)
            .expect("sign contact proof");
        for (wallet, expected) in [(owner, true), (unrelated, false)] {
            let upstream = UpstreamWallet::from_seed_bytes(
                wallet.2,
                Network::Testnet,
                WalletAccountCreationOptions::None,
            )
            .expect("upstream wallet");
            assert_eq!(
                verify_auto_accept_proof(
                    &upstream,
                    Network::Testnet,
                    &proof,
                    &sender,
                    &owner_id,
                    3
                )
                .expect("verify proof"),
                expected
            );
        }
    }
}
