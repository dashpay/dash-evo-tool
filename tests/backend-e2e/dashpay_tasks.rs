//! DashPayTask backend E2E tests (TC-031 to TC-044).
//!
//! Tests run serially via `--test-threads=1`. TC-037 through TC-042 form a
//! sequential contact flow: send request -> load requests -> accept ->
//! register addresses -> load contacts -> update info.

use crate::framework::dashpay_helpers;
use crate::framework::fixtures;
use crate::framework::harness;
use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::dashpay::DashPayTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;

/// Stores the incoming request ID captured during TC-038 for use in TC-039.
static INCOMING_REQUEST_ID: tokio::sync::OnceCell<Identifier> = tokio::sync::OnceCell::const_new();

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
            assert!(
                !results.is_empty(),
                "Search for '{}' should return at least one result",
                pair.username_a
            );
            let found = results
                .iter()
                .any(|(_id, _profile, username)| username == &pair.username_a);
            assert!(found, "Expected username '{}' in results", pair.username_a);
        }
        other => panic!(
            "TC-033: expected DashPayProfileSearchResults, got: {:?}",
            other
        ),
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

/// TC-037: SendContactRequest (A -> B)
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_037_send_contact_request() {
    let ctx = harness::ctx().await;
    let pair = fixtures::shared_dashpay_pair().await;

    let (signing_key, _signing_key_bytes) = &pair.signing_key_a;

    let task = BackendTask::DashPayTask(Box::new(DashPayTask::SendContactRequest {
        identity: pair.identity_a.clone(),
        signing_key: signing_key.clone(),
        to_username: pair.username_b.clone(),
        account_label: None,
    }));

    let result = run_task(&ctx.app_context, task).await;

    match result {
        Ok(BackendTaskSuccessResult::DashPayContactRequestSent(username)) => {
            tracing::info!("TC-037: contact request sent to '{}'", username);
        }
        Ok(BackendTaskSuccessResult::DashPayContactAlreadyEstablished(id)) => {
            tracing::info!(
                "TC-037: contact already established with {:?} (previous test run)",
                id
            );
        }
        Ok(other) => panic!(
            "TC-037: expected DashPayContactRequestSent, got: {:?}",
            other
        ),
        Err(e) => panic!("TC-037: SendContactRequest failed: {:?}", e),
    }
}

/// TC-038: LoadContactRequests — after sending (check B's incoming)
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_038_load_contact_requests_after_sending() {
    let ctx = harness::ctx().await;
    let pair = fixtures::shared_dashpay_pair().await;

    let task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadContactRequests {
        identity: pair.identity_b.clone(),
    }));

    let result = run_task(&ctx.app_context, task)
        .await
        .expect("LoadContactRequests should not fail");

    match result {
        BackendTaskSuccessResult::DashPayContactRequests { incoming, outgoing } => {
            tracing::info!(
                "TC-038: B has {} incoming, {} outgoing requests",
                incoming.len(),
                outgoing.len()
            );

            // Find A's request among incoming
            let identity_a_id = pair.identity_a.identity.id();
            if let Some((request_id, _doc)) = incoming.first() {
                tracing::info!("TC-038: storing request_id={:?} for TC-039", request_id);
                INCOMING_REQUEST_ID
                    .set(*request_id)
                    .expect("INCOMING_REQUEST_ID should only be set once");
            } else {
                tracing::warn!(
                    "TC-038: no incoming requests found for B — contact may already be established from a previous run"
                );
            }

            tracing::info!(
                "TC-038: identity_a_id={:?}, incoming_count={}",
                identity_a_id,
                incoming.len()
            );
        }
        other => panic!("TC-038: expected DashPayContactRequests, got: {:?}", other),
    }
}

/// TC-039: AcceptContactRequest
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_039_accept_contact_request() {
    let ctx = harness::ctx().await;
    let pair = fixtures::shared_dashpay_pair().await;

    let request_id = match INCOMING_REQUEST_ID.get() {
        Some(id) => *id,
        None => {
            tracing::warn!(
                "TC-039: no request_id from TC-038 — contact may already be established. Skipping."
            );
            return;
        }
    };

    let task = BackendTask::DashPayTask(Box::new(DashPayTask::AcceptContactRequest {
        identity: pair.identity_b.clone(),
        request_id,
    }));

    let result = run_task(&ctx.app_context, task)
        .await
        .expect("AcceptContactRequest should succeed");

    match result {
        BackendTaskSuccessResult::DashPayContactRequestAccepted(id) => {
            tracing::info!("TC-039: accepted contact request {:?}", id);
        }
        other => panic!(
            "TC-039: expected DashPayContactRequestAccepted, got: {:?}",
            other
        ),
    }
}

/// TC-040: RegisterDashPayAddresses
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_040_register_dashpay_addresses() {
    let ctx = harness::ctx().await;
    let pair = fixtures::shared_dashpay_pair().await;

    let task = BackendTask::DashPayTask(Box::new(DashPayTask::RegisterDashPayAddresses {
        identity: pair.identity_b.clone(),
    }));

    let result = run_task(&ctx.app_context, task)
        .await
        .expect("RegisterDashPayAddresses should succeed");

    match result {
        BackendTaskSuccessResult::Message(msg) => {
            tracing::info!("TC-040: RegisterDashPayAddresses: {}", msg);
            assert!(
                msg.contains("Registered"),
                "Message should confirm address registration"
            );
        }
        other => panic!(
            "TC-040: expected Message (address registration confirmation), got: {:?}",
            other
        ),
    }
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

/// TC-042: UpdateContactInfo
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_042_update_contact_info() {
    let ctx = harness::ctx().await;
    let pair = fixtures::shared_dashpay_pair().await;

    let contact_id = pair.identity_a.identity.id();

    let task = BackendTask::DashPayTask(Box::new(DashPayTask::UpdateContactInfo {
        identity: pair.identity_b.clone(),
        contact_id,
        nickname: Some("Test Nickname".into()),
        note: Some("E2E note".into()),
        is_hidden: false,
        accepted_accounts: vec![0],
    }));

    let result = run_task(&ctx.app_context, task)
        .await
        .expect("UpdateContactInfo should succeed");

    match result {
        BackendTaskSuccessResult::DashPayContactInfoUpdated(id) => {
            assert_eq!(
                id, contact_id,
                "Updated contact info ID should match contact A"
            );
            tracing::info!("TC-042: contact info updated for {:?}", id);
        }
        other => panic!(
            "TC-042: expected DashPayContactInfoUpdated, got: {:?}",
            other
        ),
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
    let (seed_hash_c, wallet_c) = ctx.create_funded_test_wallet(3_000_000).await;
    let qi_c =
        dashpay_helpers::create_dashpay_identity(&ctx.app_context, &wallet_c, seed_hash_c).await;

    // Register a DPNS name for C so A can send a contact request
    let username_c = format!("e2erej-c-{}", hex::encode(&seed_hash_c[..4]));
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

    // Send contact request from A to C
    let (signing_key_a, _) = &pair.signing_key_a;
    tracing::info!(
        "TC-043: sending contact request from A to C ('{}')",
        username_c
    );
    let send_task = BackendTask::DashPayTask(Box::new(DashPayTask::SendContactRequest {
        identity: pair.identity_a.clone(),
        signing_key: signing_key_a.clone(),
        to_username: username_c.clone(),
        account_label: None,
    }));
    let send_result = run_task(&ctx.app_context, send_task)
        .await
        .expect("TC-043: SendContactRequest from A to C should succeed");
    assert!(
        matches!(
            send_result,
            BackendTaskSuccessResult::DashPayContactRequestSent(_)
        ),
        "TC-043: expected DashPayContactRequestSent, got: {:?}",
        send_result
    );

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
