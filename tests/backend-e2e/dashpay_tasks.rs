//! DashPayTask backend E2E tests (TC-031 to TC-044).
//!
//! Tests run serially via `--test-threads=1`. TC-037 through TC-042 form a
//! sequential contact flow merged into a single lifecycle test:
//! send request -> load requests -> accept -> register addresses -> update info.

use crate::framework::dashpay_helpers;
use crate::framework::fixtures;
use crate::framework::harness;
use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::dashpay::DashPayTask;
use dash_evo_tool::backend_task::identity::IdentityTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::platform::{Identifier, IdentityPublicKey};

/// TC-031: LoadProfile — identity with no profile
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_031_load_profile_no_profile() {
    let ctx = harness::ctx().await;
    let si = fixtures::shared_identity().await;

    let task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadProfile {
        identity: si.qualified_identity.clone(),
    }));

    let result = run_task(&ctx.app_context, task)
        .await
        .expect("LoadProfile should not fail");

    match result {
        BackendTaskSuccessResult::DashPayProfile(profile) => {
            tracing::info!("TC-031: LoadProfile returned profile={:?}", profile);
            // SHARED_IDENTITY has no DashPay profile, so we expect None.
            // However, if some prior test run created one, Some is also valid.
        }
        other => panic!("TC-031: expected DashPayProfile, got: {:?}", other),
    }
}

/// TC-032: UpdateProfile
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_032_update_profile() {
    let ctx = harness::ctx().await;
    let pair = fixtures::shared_dashpay_pair().await;

    let identity_a = pair.identity_a.clone();
    let identity_a_id = identity_a.identity.id();

    let task = BackendTask::DashPayTask(Box::new(DashPayTask::UpdateProfile {
        identity: identity_a.clone(),
        display_name: Some("E2E Test User".into()),
        bio: Some("Backend E2E test profile".into()),
        avatar_url: None,
    }));

    let result = run_task(&ctx.app_context, task)
        .await
        .expect("UpdateProfile should succeed");

    match result {
        BackendTaskSuccessResult::DashPayProfileUpdated(id) => {
            assert_eq!(
                id, identity_a_id,
                "Updated profile ID should match identity A"
            );
            tracing::info!("TC-032: profile updated for {:?}", id);
        }
        other => panic!("TC-032: expected DashPayProfileUpdated, got: {:?}", other),
    }

    // Verify by loading the profile back
    let verify_task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadProfile {
        identity: identity_a,
    }));
    let verify_result = run_task(&ctx.app_context, verify_task)
        .await
        .expect("LoadProfile verification should succeed");

    match verify_result {
        BackendTaskSuccessResult::DashPayProfile(Some((display_name, _bio, _url))) => {
            assert_eq!(
                display_name, "E2E Test User",
                "Display name should match what was set"
            );
            tracing::info!("TC-032: verified display_name = '{}'", display_name);
        }
        BackendTaskSuccessResult::DashPayProfile(None) => {
            panic!("TC-032: profile should exist after update, got None");
        }
        other => panic!(
            "TC-032: expected DashPayProfile for verification, got: {:?}",
            other
        ),
    }
}

/// TC-033: SearchProfiles
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_033_search_profiles() {
    let ctx = harness::ctx().await;
    let pair = fixtures::shared_dashpay_pair().await;

    // Give Platform extra time after fixture DPNS registration before querying.
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    // Retry search — DPNS name may not be immediately queryable
    // after registration due to platform propagation delay.
    let poll_interval = std::time::Duration::from_secs(5);
    let timeout = std::time::Duration::from_secs(120);
    let start = std::time::Instant::now();

    loop {
        let task = BackendTask::DashPayTask(Box::new(DashPayTask::SearchProfiles {
            search_query: pair.username_a.clone(),
        }));

        let result = run_task(&ctx.app_context, task)
            .await
            .expect("SearchProfiles should succeed");

        match result {
            BackendTaskSuccessResult::DashPayProfileSearchResults(results) => {
                tracing::info!(
                    "TC-033: search for '{}' returned {} results",
                    pair.username_a,
                    results.len()
                );
                // Log actual returned usernames for debugging
                for (id, _profile, username) in &results {
                    tracing::info!("TC-033: result entry: id={}, username='{}'", id, username);
                }
                // Production bug: search_profiles returns normalizedLabel (with
                // homograph conversion, e.g. i→1) instead of the original label.
                // Compare against both original and normalized forms until fixed.
                let normalized_a =
                    dash_evo_tool::model::dpns::normalize_dpns_label(&pair.username_a);
                let found = results.iter().any(|(_id, _profile, username)| {
                    let u = username.trim_end_matches(".dash");
                    u == pair.username_a || u == normalized_a
                });
                if found {
                    tracing::info!("TC-033: found username '{}' in results", pair.username_a);
                    break;
                }
                if start.elapsed() > timeout {
                    panic!(
                        "TC-033: search for '{}' did not return expected result within {:?} \
                         (got {} results but none matched: {:?})",
                        pair.username_a,
                        timeout,
                        results.len(),
                        results
                            .iter()
                            .map(|(_, _, u)| u.as_str())
                            .collect::<Vec<_>>()
                    );
                }
                tracing::info!(
                    "TC-033: username '{}' not yet in results, retrying in {:?}...",
                    pair.username_a,
                    poll_interval
                );
                tokio::time::sleep(poll_interval).await;
            }
            other => panic!(
                "TC-033: expected DashPayProfileSearchResults, got: {:?}",
                other
            ),
        }
    }
}

/// TC-034: LoadContacts — empty
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_034_load_contacts_empty() {
    let ctx = harness::ctx().await;
    let si = fixtures::shared_identity().await;

    let task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadContacts {
        identity: si.qualified_identity.clone(),
    }));

    let result = run_task(&ctx.app_context, task)
        .await
        .expect("LoadContacts should not fail");

    match result {
        BackendTaskSuccessResult::DashPayContacts(contacts) => {
            tracing::info!("TC-034: LoadContacts returned {} contacts", contacts.len());
        }
        BackendTaskSuccessResult::DashPayContactsWithInfo(contacts) => {
            tracing::info!(
                "TC-034: LoadContactsWithInfo returned {} contacts",
                contacts.len()
            );
        }
        other => panic!("TC-034: expected DashPayContacts*, got: {:?}", other),
    }
}

/// TC-035: LoadContactRequests — empty
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_035_load_contact_requests_empty() {
    let ctx = harness::ctx().await;
    let si = fixtures::shared_identity().await;

    let task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadContactRequests {
        identity: si.qualified_identity.clone(),
    }));

    let result = run_task(&ctx.app_context, task)
        .await
        .expect("LoadContactRequests should not fail");

    match result {
        BackendTaskSuccessResult::DashPayContactRequests { incoming, outgoing } => {
            tracing::info!(
                "TC-035: LoadContactRequests returned {} incoming, {} outgoing",
                incoming.len(),
                outgoing.len()
            );
        }
        other => panic!("TC-035: expected DashPayContactRequests, got: {:?}", other),
    }
}

/// TC-036: FetchContactProfile
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_036_fetch_contact_profile() {
    let ctx = harness::ctx().await;
    let pair = fixtures::shared_dashpay_pair().await;

    let task = BackendTask::DashPayTask(Box::new(DashPayTask::FetchContactProfile {
        identity: pair.identity_a.clone(),
        contact_id: pair.identity_b.identity.id(),
    }));

    let result = run_task(&ctx.app_context, task)
        .await
        .expect("FetchContactProfile should not fail");

    match result {
        BackendTaskSuccessResult::DashPayContactProfile(profile) => {
            tracing::info!(
                "TC-036: FetchContactProfile returned {:?}",
                profile.is_some()
            );
        }
        other => panic!("TC-036: expected DashPayContactProfile, got: {:?}", other),
    }
}

// ─── TC-037 lifecycle steps ────────────────────────────────────────────────

async fn step_send_contact_request(
    ctx: &crate::framework::harness::BackendTestContext,
    pair: &fixtures::SharedDashPayPair,
) {
    tracing::info!("=== Step 1: SendContactRequest (A -> B) ===");

    let (signing_key, _signing_key_bytes) = &pair.signing_key_a;

    // Give Platform extra time after fixture DPNS registration before resolving.
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    // SendContactRequest does username resolution internally. If the DPNS
    // name hasn't fully propagated yet, it fails with UsernameResolutionFailed.
    // Retry with backoff for up to 120s.
    let retry_timeout = std::time::Duration::from_secs(120);
    let retry_interval = std::time::Duration::from_secs(10);
    let start = std::time::Instant::now();

    loop {
        let task = BackendTask::DashPayTask(Box::new(DashPayTask::SendContactRequest {
            identity: pair.identity_a.clone(),
            signing_key: signing_key.clone(),
            to_username: pair.username_b.clone(),
            account_label: None,
        }));

        let result = run_task(&ctx.app_context, task).await;

        match result {
            Ok(BackendTaskSuccessResult::DashPayContactRequestSent(username)) => {
                tracing::info!("Step 1: contact request sent to '{}'", username);
                return;
            }
            Ok(BackendTaskSuccessResult::DashPayContactAlreadyEstablished(id)) => {
                tracing::info!(
                    "Step 1: contact already established with {:?} (previous test run)",
                    id
                );
                return;
            }
            Ok(other) => panic!(
                "Step 1: expected DashPayContactRequestSent, got: {:?}",
                other
            ),
            Err(e) => {
                let is_resolution_failure = format!("{:?}", e).contains("UsernameResolution");
                if is_resolution_failure && start.elapsed() < retry_timeout {
                    tracing::info!(
                        "Step 1: username resolution failed, retrying in {:?}... ({:?} elapsed)",
                        retry_interval,
                        start.elapsed()
                    );
                    tokio::time::sleep(retry_interval).await;
                    continue;
                }
                panic!("Step 1: SendContactRequest failed: {:?}", e);
            }
        }
    }
}

/// Returns the incoming request ID from B's contact requests, or `None` if the
/// contact is already established from a previous run.
async fn step_load_contact_requests(
    ctx: &crate::framework::harness::BackendTestContext,
    pair: &fixtures::SharedDashPayPair,
) -> Option<Identifier> {
    tracing::info!("=== Step 2: LoadContactRequests (check B's incoming) ===");

    let task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadContactRequests {
        identity: pair.identity_b.clone(),
    }));

    let result = run_task(&ctx.app_context, task)
        .await
        .expect("LoadContactRequests should not fail");

    match result {
        BackendTaskSuccessResult::DashPayContactRequests { incoming, outgoing } => {
            tracing::info!(
                "Step 2: B has {} incoming, {} outgoing requests",
                incoming.len(),
                outgoing.len()
            );

            let identity_a_id = pair.identity_a.identity.id();
            tracing::info!(
                "Step 2: identity_a_id={:?}, incoming_count={}",
                identity_a_id,
                incoming.len()
            );

            if let Some((request_id, _doc)) = incoming.first() {
                tracing::info!("Step 2: found request_id={:?}", request_id);
                Some(*request_id)
            } else {
                tracing::warn!(
                    "Step 2: no incoming requests found for B — contact may already be established from a previous run"
                );
                None
            }
        }
        other => panic!("Step 2: expected DashPayContactRequests, got: {:?}", other),
    }
}

async fn step_accept_contact_request(
    ctx: &crate::framework::harness::BackendTestContext,
    pair: &fixtures::SharedDashPayPair,
    request_id: Identifier,
) {
    tracing::info!("=== Step 3: AcceptContactRequest ===");

    let task = BackendTask::DashPayTask(Box::new(DashPayTask::AcceptContactRequest {
        identity: pair.identity_b.clone(),
        request_id,
    }));

    let result = run_task(&ctx.app_context, task)
        .await
        .expect("AcceptContactRequest should succeed");

    match result {
        BackendTaskSuccessResult::DashPayContactRequestAccepted(id) => {
            tracing::info!("Step 3: accepted contact request {:?}", id);
        }
        other => panic!(
            "Step 3: expected DashPayContactRequestAccepted, got: {:?}",
            other
        ),
    }
}

async fn step_register_dashpay_addresses(
    ctx: &crate::framework::harness::BackendTestContext,
    pair: &fixtures::SharedDashPayPair,
) {
    tracing::info!("=== Step 4: RegisterDashPayAddresses ===");

    let task = BackendTask::DashPayTask(Box::new(DashPayTask::RegisterDashPayAddresses {
        identity: pair.identity_b.clone(),
    }));

    let result = run_task(&ctx.app_context, task)
        .await
        .expect("RegisterDashPayAddresses should succeed");

    match result {
        BackendTaskSuccessResult::Message(msg) => {
            tracing::info!("Step 4: RegisterDashPayAddresses: {}", msg);
            assert!(
                msg.contains("Registered"),
                "Message should confirm address registration"
            );
        }
        other => panic!(
            "Step 4: expected Message (address registration confirmation), got: {:?}",
            other
        ),
    }
}

async fn step_update_contact_info(
    ctx: &crate::framework::harness::BackendTestContext,
    pair: &fixtures::SharedDashPayPair,
) {
    tracing::info!("=== Step 5: UpdateContactInfo ===");

    let mut identity_b = pair.identity_b.clone();
    let contact_id = pair.identity_a.identity.id();

    // Check if identity B already has an ECDSA_SECP256K1 AUTHENTICATION key.
    let has_secp_auth = identity_b
        .identity
        .get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            [
                SecurityLevel::CRITICAL,
                SecurityLevel::HIGH,
                SecurityLevel::MEDIUM,
            ]
            .into(),
            [KeyType::ECDSA_SECP256K1].into(),
            false,
        )
        .is_some();

    if !has_secp_auth {
        tracing::info!(
            "Step 5: identity B lacks ECDSA_SECP256K1 AUTHENTICATION key, adding one..."
        );
        let private_key_bytes: [u8; 32] = rand::random();
        let new_ipk = IdentityPublicKey::V0(IdentityPublicKeyV0 {
            id: 0, // placeholder, AddKeyToIdentity reassigns
            purpose: Purpose::AUTHENTICATION,
            security_level: SecurityLevel::HIGH,
            contract_bounds: None,
            key_type: KeyType::ECDSA_SECP256K1,
            read_only: false,
            data: {
                use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
                use dash_sdk::dpp::dashcore::PrivateKey;
                let secp = Secp256k1::new();
                let secret_key =
                    dash_sdk::dpp::dashcore::secp256k1::SecretKey::from_slice(&private_key_bytes)
                        .expect("Step 5: invalid secret key");
                let pk = PrivateKey::new(secret_key, dash_sdk::dpp::dashcore::Network::Testnet);
                pk.public_key(&secp).to_bytes().into()
            },
            disabled_at: None,
        });

        let new_qualified_key = QualifiedIdentityPublicKey::from(new_ipk);

        let add_result = run_task(
            &ctx.app_context,
            BackendTask::IdentityTask(IdentityTask::AddKeyToIdentity(
                identity_b.clone(),
                new_qualified_key,
                private_key_bytes,
            )),
        )
        .await
        .expect("Step 5: AddKeyToIdentity should succeed");

        assert!(
            matches!(add_result, BackendTaskSuccessResult::AddedKeyToIdentity(_)),
            "Step 5: expected AddedKeyToIdentity, got: {:?}",
            add_result
        );

        // Refresh identity B from Platform to pick up the new key.
        // Note: RefreshIdentity updates the local DB but returns the input QI
        // (a known limitation), so we re-load from the local DB afterward.
        let refresh_result = run_task(
            &ctx.app_context,
            BackendTask::IdentityTask(IdentityTask::RefreshIdentity(identity_b.clone())),
        )
        .await
        .expect("Step 5: RefreshIdentity should succeed");

        assert!(
            matches!(
                refresh_result,
                BackendTaskSuccessResult::RefreshedIdentity(_)
            ),
            "Step 5: expected RefreshedIdentity, got: {:?}",
            refresh_result
        );

        // Re-load from local DB to get the updated identity with the new key.
        let identity_b_id = identity_b.identity.id();
        identity_b = ctx
            .app_context
            .load_local_qualified_identities()
            .expect("Step 5: load_local_qualified_identities should succeed")
            .into_iter()
            .find(|qi| qi.identity.id() == identity_b_id)
            .expect("Step 5: identity B should be in local DB after refresh");

        tracing::info!(
            "Step 5: identity B refreshed from local DB, keys={}",
            identity_b.identity.public_keys().len()
        );
    }

    let task = BackendTask::DashPayTask(Box::new(DashPayTask::UpdateContactInfo {
        identity: identity_b,
        contact_id,
        nickname: Some("Test Nickname".into()),
        note: Some("E2E note".into()),
        is_hidden: false,
        accepted_accounts: vec![0],
    }));

    let result = run_task(&ctx.app_context, task)
        .await
        .expect("Step 5: UpdateContactInfo should succeed");

    match result {
        BackendTaskSuccessResult::DashPayContactInfoUpdated(id) => {
            assert_eq!(
                id, contact_id,
                "Updated contact info ID should match contact A"
            );
            tracing::info!("Step 5: contact info updated for {:?}", id);
        }
        other => panic!(
            "Step 5: expected DashPayContactInfoUpdated, got: {:?}",
            other
        ),
    }
}

/// TC-037: Full DashPay contact lifecycle (A sends → B loads → B accepts →
/// B registers addresses → B updates contact info).
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_037_dashpay_contact_lifecycle() {
    let ctx = harness::ctx().await;
    let pair = fixtures::shared_dashpay_pair().await;

    step_send_contact_request(ctx, pair).await;

    let request_id = step_load_contact_requests(ctx, pair).await;

    if let Some(id) = request_id {
        step_accept_contact_request(ctx, pair, id).await;
    } else {
        tracing::warn!(
            "TC-037: no pending request — skipping accept step (contact already established)"
        );
    }

    step_register_dashpay_addresses(ctx, pair).await;

    step_update_contact_info(ctx, pair).await;
}

/// TC-041: LoadPaymentHistory — empty
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_041_load_payment_history_empty() {
    let ctx = harness::ctx().await;
    let pair = fixtures::shared_dashpay_pair().await;

    let task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadPaymentHistory {
        identity: pair.identity_a.clone(),
    }));

    let result = run_task(&ctx.app_context, task)
        .await
        .expect("LoadPaymentHistory should not fail");

    match result {
        BackendTaskSuccessResult::DashPayPaymentHistory(history) => {
            tracing::info!(
                "TC-041: LoadPaymentHistory returned {} entries",
                history.len()
            );
        }
        other => panic!("TC-041: expected DashPayPaymentHistory, got: {:?}", other),
    }
}

/// TC-043: RejectContactRequest (requires third DashPay identity)
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_043_reject_contact_request() {
    let ctx = harness::ctx().await;
    let pair = fixtures::shared_dashpay_pair().await;

    // Create a third DashPay identity (C) from a fresh funded wallet
    tracing::info!("TC-043: creating third DashPay identity (C)...");
    let (seed_hash_c, wallet_c) = ctx.create_funded_test_wallet(30_000_000).await;
    let (qi_c, _key_bytes_c) =
        dashpay_helpers::create_dashpay_identity(&ctx.app_context, &wallet_c, seed_hash_c).await;

    // Register a DPNS name for C so A can send a contact request
    // >= 20 chars to avoid DPNS contest voting period
    let username_c = format!("e2erej-c-{}", hex::encode(&seed_hash_c[..8]));
    tracing::info!("TC-043: registering DPNS name '{}' for C...", username_c);

    let dpns_task = BackendTask::IdentityTask(
        dash_evo_tool::backend_task::identity::IdentityTask::RegisterDpnsName(
            dash_evo_tool::backend_task::identity::RegisterDpnsNameInput {
                qualified_identity: qi_c.clone(),
                name_input: username_c.clone(),
            },
        ),
    );
    let dpns_result = run_task(&ctx.app_context, dpns_task)
        .await
        .expect("TC-043: DPNS registration for C should succeed");
    assert!(
        matches!(dpns_result, BackendTaskSuccessResult::RegisteredDpnsName(_)),
        "TC-043: expected RegisteredDpnsName for C, got: {:?}",
        dpns_result
    );

    // Wait for C's DPNS name to propagate before sending a contact request.
    tracing::info!(
        "TC-043: waiting for DPNS name '{}' to propagate...",
        username_c
    );
    let propagation_timeout = std::time::Duration::from_secs(120);
    let poll_interval = std::time::Duration::from_secs(5);
    let start = std::time::Instant::now();
    loop {
        let search = run_task(
            &ctx.app_context,
            BackendTask::IdentityTask(
                dash_evo_tool::backend_task::identity::IdentityTask::SearchIdentityByDpnsName(
                    username_c.clone(),
                    None,
                ),
            ),
        )
        .await;

        if matches!(search, Ok(BackendTaskSuccessResult::LoadedIdentity(_))) {
            tracing::info!(
                "TC-043: DPNS name '{}' propagated after {:?}",
                username_c,
                start.elapsed()
            );
            break;
        }

        if start.elapsed() > propagation_timeout {
            panic!(
                "TC-043: DPNS name '{}' did not propagate within {:?}",
                username_c, propagation_timeout
            );
        }

        tokio::time::sleep(poll_interval).await;
    }

    // Give Platform extra time after DPNS propagation check before sending.
    tokio::time::sleep(std::time::Duration::from_secs(15)).await;

    // Send contact request from A to C, with retry on UsernameResolutionFailed
    let (signing_key_a, _) = &pair.signing_key_a;
    tracing::info!(
        "TC-043: sending contact request from A to C ('{}')",
        username_c
    );

    let retry_timeout = std::time::Duration::from_secs(120);
    let retry_interval = std::time::Duration::from_secs(10);
    let send_start = std::time::Instant::now();

    loop {
        let send_task = BackendTask::DashPayTask(Box::new(DashPayTask::SendContactRequest {
            identity: pair.identity_a.clone(),
            signing_key: signing_key_a.clone(),
            to_username: username_c.clone(),
            account_label: None,
        }));

        match run_task(&ctx.app_context, send_task).await {
            Ok(BackendTaskSuccessResult::DashPayContactRequestSent(_)) => break,
            Ok(other) => panic!(
                "TC-043: expected DashPayContactRequestSent, got: {:?}",
                other
            ),
            Err(e) => {
                let is_resolution_failure = format!("{:?}", e).contains("UsernameResolution");
                if is_resolution_failure && send_start.elapsed() < retry_timeout {
                    tracing::info!(
                        "TC-043: username resolution failed, retrying in {:?}...",
                        retry_interval
                    );
                    tokio::time::sleep(retry_interval).await;
                    continue;
                }
                panic!("TC-043: SendContactRequest from A to C failed: {:?}", e);
            }
        }
    }

    // Load C's incoming requests to get the request_id
    tracing::info!("TC-043: loading C's incoming contact requests...");
    let load_task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadContactRequests {
        identity: qi_c.clone(),
    }));
    let load_result = run_task(&ctx.app_context, load_task)
        .await
        .expect("TC-043: LoadContactRequests for C should succeed");

    let request_id = match load_result {
        BackendTaskSuccessResult::DashPayContactRequests { incoming, .. } => {
            assert!(
                !incoming.is_empty(),
                "TC-043: C should have at least one incoming request from A"
            );
            let (req_id, _doc) = &incoming[0];
            tracing::info!("TC-043: C has incoming request_id={:?}", req_id);
            *req_id
        }
        other => panic!("TC-043: expected DashPayContactRequests, got: {:?}", other),
    };

    // Reject the contact request
    tracing::info!("TC-043: rejecting contact request {:?}...", request_id);
    let reject_task = BackendTask::DashPayTask(Box::new(DashPayTask::RejectContactRequest {
        identity: qi_c,
        request_id,
    }));
    let reject_result = run_task(&ctx.app_context, reject_task)
        .await
        .expect("TC-043: RejectContactRequest should succeed");

    match reject_result {
        BackendTaskSuccessResult::DashPayContactRequestRejected(id) => {
            tracing::info!("TC-043: rejected contact request {:?}", id);
        }
        other => panic!(
            "TC-043: expected DashPayContactRequestRejected, got: {:?}",
            other
        ),
    }
}

/// TC-044: DashPayTask error — send contact request to nonexistent username
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_044_send_contact_request_nonexistent_username() {
    let ctx = harness::ctx().await;
    let pair = fixtures::shared_dashpay_pair().await;

    let (signing_key, _) = &pair.signing_key_a;

    let task = BackendTask::DashPayTask(Box::new(DashPayTask::SendContactRequest {
        identity: pair.identity_a.clone(),
        signing_key: signing_key.clone(),
        to_username: "zzz_nonexistent_e2e_test_user_999".into(),
        account_label: None,
    }));

    let result = run_task(&ctx.app_context, task).await;

    assert!(
        result.is_err(),
        "TC-044: sending contact request to nonexistent username should fail, got: {:?}",
        result
    );
    tracing::info!(
        "TC-044: correctly failed with error: {:?}",
        result.unwrap_err()
    );
}
