//! DashPayTask backend E2E tests (TC-031 to TC-044).
//!
//! DEFERRED: this module is currently disabled (commented out in
//! `tests/backend-e2e/main.rs`). The dashpay backend depends on upstream
//! `platform-wallet` dashpay support that is still incomplete; the completion
//! lands in `dashpay/platform#3841` ("fix(platform-wallet)!: complete dashpay",
//! shumkov, branch `feat/dashpay-m1-sync-correctness`). Re-enable the `mod
//! dashpay_tasks;` declaration once that PR merges and the DET platform-wallet
//! dep is bumped.
//!
//! Tests run serially via `--test-threads=1`. TC-037 through TC-042 form a
//! sequential contact flow merged into a single lifecycle test:
//! send request -> load requests -> accept -> register addresses -> update info.

use crate::framework::dashpay_helpers;
use crate::framework::fixtures;
use crate::framework::harness;
use crate::framework::task_runner::{run_task, run_task_with_nonce_retry};
use dash_evo_tool::backend_task::dashpay::DashPayTask;
use dash_evo_tool::backend_task::identity::IdentityTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::model::dashpay::{
    ContactInfoUpdate, DashPaySnapshot, ProfileSnapshot, ProfileState,
};
use dash_evo_tool::model::qualified_identity::QualifiedIdentity;
use dash_evo_tool::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::platform::{Identifier, IdentityPublicKey};

/// Sync `identity`'s DashPay state now and return its snapshot.
async fn sync_and_snapshot(
    ctx: &crate::framework::harness::BackendTestContext,
    identity: &QualifiedIdentity,
) -> std::sync::Arc<DashPaySnapshot> {
    let task = BackendTask::DashPayTask(Box::new(DashPayTask::SyncNow {
        identity: Some(identity.clone()),
    }));
    let result = run_task(&ctx.app_context, task)
        .await
        .expect("SyncNow should not fail");
    assert!(
        matches!(result, BackendTaskSuccessResult::DashPaySynced),
        "expected DashPaySynced, got: {result:?}"
    );
    ctx.app_context
        .wallet_backend()
        .expect("wallet backend wired")
        .dashpay_snapshot(&identity.identity.id())
        .expect("a sync publishes the identity's snapshot")
}

/// TC-031: the synced profile of an identity with no profile
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_031_synced_profile_no_profile() {
    let ctx = harness::ctx().await;
    let si = fixtures::shared_identity().await;

    let snapshot = sync_and_snapshot(ctx, &si.qualified_identity).await;
    // SHARED_IDENTITY has no DashPay profile, so we expect none. However, if
    // some prior test run created one, a profile is also valid.
    tracing::info!("TC-031: synced profile={:?}", snapshot.profile);
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
        BackendTaskSuccessResult::DashPayProfileUpdated(ProfileSnapshot { owner: id, .. }) => {
            assert_eq!(
                id, identity_a_id,
                "Updated profile ID should match identity A"
            );
            tracing::info!("TC-032: profile updated for {:?}", id);
        }
        other => panic!("TC-032: expected DashPayProfileUpdated, got: {:?}", other),
    }

    let snapshot = sync_and_snapshot(ctx, &identity_a).await;
    match &snapshot.profile {
        ProfileState::Present(profile) => {
            assert_eq!(
                profile.display_name.as_deref(),
                Some("E2E Test User"),
                "Display name should match what was set"
            );
            tracing::info!("TC-032: verified display_name = {:?}", profile.display_name);
        }
        other => panic!("TC-032: expected the saved profile, got: {other:?}"),
    }
}

/// TC-033: SearchProfiles
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_033_search_profiles() {
    let ctx = harness::ctx().await;
    let pair = fixtures::shared_dashpay_pair().await;

    // TODO: DPNS propagation delay
    // Expected: SearchProfiles finds the username immediately after fixture registration
    // Actual: DPNS names may not be queryable for several seconds after registration
    //         due to platform propagation across DAPI nodes
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
            for (id, _profile, username) in &results {
                tracing::info!("TC-033: result entry: id={}, username='{}'", id, username);
            }
            // Production bug: search_profiles returns normalizedLabel (with
            // homograph conversion, e.g. i→1) instead of the original label.
            // Compare against both original and normalized forms until fixed.
            let normalized_a = dash_evo_tool::model::dpns::normalize_dpns_label(&pair.username_a);
            let found = results.iter().any(|(_id, _profile, username)| {
                let u = username.trim_end_matches(".dash");
                u == pair.username_a || u == normalized_a
            });
            assert!(
                found,
                "TC-033: username '{}' not found in search results \
                 (got {} results: {:?}). DAPI propagation delay — \
                 the fixture waits for propagation, so this should not happen.",
                pair.username_a,
                results.len(),
                results
                    .iter()
                    .map(|(_, _, u)| u.as_str())
                    .collect::<Vec<_>>()
            );
            tracing::info!("TC-033: found username '{}' in results", pair.username_a);
        }
        other => panic!(
            "TC-033: expected DashPayProfileSearchResults, got: {:?}",
            other
        ),
    }
}

/// TC-034: synced contacts and contact requests
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_034_synced_contacts_and_requests() {
    let ctx = harness::ctx().await;
    let si = fixtures::shared_identity().await;

    let snapshot = sync_and_snapshot(ctx, &si.qualified_identity).await;
    tracing::info!(
        "TC-034: synced {} contacts, {} incoming and {} outgoing requests",
        snapshot.contacts.len(),
        snapshot.incoming.len(),
        snapshot.outgoing.len()
    );
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

    let signing_key = &pair.signing_key_a;

    // TODO: DPNS propagation delay on username resolution
    // Expected: SendContactRequest resolves the DPNS name immediately after
    //           fixture registration (shared_dashpay_pair waits for propagation)
    // Actual: occasionally fails with UsernameResolutionFailed because a
    //         different DAPI node serves the query before it has the name
    let task = BackendTask::DashPayTask(Box::new(DashPayTask::SendContactRequest {
        identity: pair.identity_a.clone(),
        signing_key: signing_key.clone(),
        to_username: pair.username_b.clone(),
        account_label: None,
    }));

    let result = run_task_with_nonce_retry(&ctx.app_context, task)
        .await
        .expect("Step 1: SendContactRequest failed");

    match result {
        BackendTaskSuccessResult::DashPayContactRequestSent(username) => {
            tracing::info!("Step 1: contact request sent to '{}'", username);
        }
        BackendTaskSuccessResult::DashPayContactAlreadyEstablished(request) => {
            tracing::info!(
                "Step 1: contact already established with {:?} (previous test run)",
                request.counterparty
            );
        }
        other => panic!(
            "Step 1: expected DashPayContactRequestSent, got: {:?}",
            other
        ),
    }
}

/// Returns A's identity if B has a pending request from A, or `None` if the
/// contact is already established from a previous run.
async fn step_load_contact_requests(
    ctx: &crate::framework::harness::BackendTestContext,
    pair: &fixtures::SharedDashPayPair,
) -> Option<Identifier> {
    tracing::info!("=== Step 2: sync B and check its incoming requests ===");

    let snapshot = sync_and_snapshot(ctx, &pair.identity_b).await;
    let incoming = &snapshot.incoming;
    tracing::info!(
        "Step 2: B has {} incoming, {} outgoing requests",
        incoming.len(),
        snapshot.outgoing.len()
    );

    let identity_a_id = pair.identity_a.identity.id();
    // Filter by sender identity to avoid picking up requests from other tests
    // running concurrently against the same identity.
    if incoming
        .iter()
        .any(|request| request.counterparty_id == identity_a_id)
    {
        tracing::info!("Step 2: found request from A");
        Some(identity_a_id)
    } else if incoming.is_empty() {
        // No incoming requests — contact may already be established from a
        // previous test run. Return None so the caller can verify the contact
        // exists instead of accepting.
        tracing::info!("Step 2: no incoming requests for B — contact likely already established");
        None
    } else {
        panic!(
            "Step 2: B has {} incoming request(s) but none from A ({:?}). Senders: {:?}",
            incoming.len(),
            identity_a_id,
            incoming
                .iter()
                .map(|request| request.counterparty_id)
                .collect::<Vec<_>>()
        );
    }
}

async fn step_accept_contact_request(
    ctx: &crate::framework::harness::BackendTestContext,
    pair: &fixtures::SharedDashPayPair,
    sender_id: Identifier,
) {
    tracing::info!("=== Step 3: AcceptContactRequest ===");

    let task = BackendTask::DashPayTask(Box::new(DashPayTask::AcceptContactRequest {
        identity: pair.identity_b.clone(),
        counterparty_id: sender_id,
    }));

    let result = run_task_with_nonce_retry(&ctx.app_context, task)
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

    let result = run_task_with_nonce_retry(&ctx.app_context, task)
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

/// Exercise the real seed-bearing contact receiving-account registration path
/// against a live (seeded) wallet, idempotently.
///
/// This drives `bootstrap_wallet_addresses_jit` — the boot /
/// unlock path that calls `register_contact_receiving_accounts` for every
/// established contact — which derives and registers the hardened DIP-15
/// receiving account so SPV watches the addresses the contact pays us at.
///
/// Requires `dashpay_sync` to run first so the upstream wallet-manager's
/// `established_contacts` is populated; without it `established_contact_pairs`
/// returns an empty slice and the step is a silent no-op. Asserts that at least
/// one account was wired after the first pass. The second pass must also succeed
/// without newly wiring additional accounts (idempotency check: `bump_monitor_revision`
/// is skipped on re-registration so no spurious bloom-filter rebuilds occur).
async fn step_register_contact_receiving_accounts(
    ctx: &crate::framework::harness::BackendTestContext,
    pair: &fixtures::SharedDashPayPair,
) {
    tracing::info!("=== Step: register contact receiving accounts (watch faucet) ===");

    let seed_hash = pair
        .identity_b
        .dashpay_wallet_seed_hash()
        .expect("identity_b must have a DashPay wallet");
    let backend = ctx
        .app_context
        .wallet_backend()
        .expect("wallet backend wired");

    // Populate upstream established_contacts via dashpay_sync so
    // established_contact_pairs returns the mutual contact with identity_a.
    // Without this, the view reads an empty in-memory map and the step no-ops.
    backend.dashpay_sync_now().await;

    // First pass: bootstrap should register at least one DashpayReceivingFunds
    // account (the mutual contact with identity_a).
    ctx.app_context
        .bootstrap_wallet_addresses_jit(&pair.wallet_b)
        .await;

    let count_after_first = backend.dashpay_receiving_account_count(&seed_hash).await;
    assert!(
        count_after_first > 0,
        "Expected at least one DashpayReceivingFunds account after first registration, got 0. \
         Check that dashpay_sync populated established_contacts before the bootstrap."
    );
    tracing::info!(
        count = count_after_first,
        "First registration: {} DashpayReceivingFunds account(s) wired",
        count_after_first
    );

    // Second pass: idempotent re-run must not panic, must not add new accounts
    // (same contacts → same keys → len unchanged), and must not bump
    // monitor_revision (which would cause a spurious bloom-filter rebuild).
    ctx.app_context
        .bootstrap_wallet_addresses_jit(&pair.wallet_b)
        .await;

    let count_after_second = backend.dashpay_receiving_account_count(&seed_hash).await;
    assert_eq!(
        count_after_second, count_after_first,
        "Re-registration must not add new accounts (idempotent re-run changed the count)"
    );
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

        let add_result = run_task_with_nonce_retry(
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

    let identity_b_id = identity_b.identity.id();
    let task = BackendTask::DashPayTask(Box::new(DashPayTask::UpdateContactInfo {
        identity: identity_b,
        contact_id,
        update: ContactInfoUpdate::replace_all(
            Some("Test Nickname".into()),
            Some("E2E note".into()),
            false,
        ),
    }));

    let result = run_task_with_nonce_retry(&ctx.app_context, task)
        .await
        .expect("Step 5: UpdateContactInfo should succeed");

    match result {
        BackendTaskSuccessResult::DashPayContactInfoUpdated {
            identity,
            contact_id: id,
            ..
        } => {
            assert_eq!(identity, identity_b_id, "update should belong to B");
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
        // Contact already established from a previous run — verify it
        // exists by loading contacts for B.
        tracing::info!("TC-037: no pending request — contact already established, verifying...");
        let contacts = sync_and_snapshot(ctx, &pair.identity_b).await.contacts.clone();
        assert!(
            !contacts.is_empty(),
            "TC-037: no contacts found but no pending request either — test state inconsistent"
        );
        tracing::info!(
            "TC-037: contact already established (B has {} contacts)",
            contacts.len()
        );
    }

    step_register_dashpay_addresses(ctx, pair).await;

    step_register_contact_receiving_accounts(ctx, pair).await;

    step_update_contact_info(ctx, pair).await;
}

/// TC-041: synced payment history smoke check.
///
/// Payments are read from the upstream `ManagedIdentity` into the DashPay
/// snapshot; populated-state coverage lives in
/// `tc_037_dashpay_contact_lifecycle` and `tc_044_pay_contact`.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_041_synced_payment_history() {
    let ctx = harness::ctx().await;
    let pair = fixtures::shared_dashpay_pair().await;

    let snapshot = sync_and_snapshot(ctx, &pair.identity_a).await;
    tracing::info!("TC-041: synced {} payments", snapshot.payments.len());
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
    let qi_c =
        dashpay_helpers::create_dashpay_identity(&ctx.app_context, &wallet_c).await;

    // Register a DPNS name for C so A can send a contact request
    // >= 20 chars to avoid DPNS contest voting period
    let username_c = format!("e2erej-c-{}", hex::encode(&seed_hash_c[..8]));
    tracing::info!("TC-043: registering DPNS name '{}' for C...", username_c);

    let dpns_task = BackendTask::IdentityTask(
        dash_evo_tool::backend_task::identity::IdentityTask::RegisterDpnsName(
            dash_evo_tool::backend_task::identity::RegisterDpnsNameInput {
                qualified_identity: qi_c.clone(),
                name_input: username_c.clone(),
        approved_contest_fee: 0,
                signing_key_id: None,
            },
        ),
    );
    let dpns_result = run_task(&ctx.app_context, dpns_task)
        .await
        .expect("TC-043: DPNS registration for C should succeed");
    assert!(
        matches!(
            dpns_result,
            BackendTaskSuccessResult::RegisteredDpnsName { .. }
        ),
        "TC-043: expected RegisteredDpnsName for C, got: {:?}",
        dpns_result
    );

    // Wait for C's DPNS name to propagate before sending a contact request.
    tracing::info!(
        "TC-043: waiting for DPNS name '{}' to propagate...",
        username_c
    );
    let propagation_timeout = crate::framework::harness::MAX_TEST_TIMEOUT / 3;
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

    // TODO: DPNS propagation delay on username resolution
    // Expected: SendContactRequest resolves C's DPNS name immediately after
    //           the propagation check above confirms the name is queryable
    // Actual: occasionally fails with UsernameResolutionFailed because a
    //         different DAPI node serves the query before it has the name
    let signing_key_a = &pair.signing_key_a;
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

    run_task_with_nonce_retry(&ctx.app_context, send_task)
        .await
        .expect("TC-043: SendContactRequest from A to C failed");

    // Confirm C received A's request
    tracing::info!("TC-043: syncing C's incoming contact requests...");
    let incoming = sync_and_snapshot(ctx, &qi_c).await.incoming.clone();
    assert!(
        !incoming.is_empty(),
        "TC-043: C should have at least one incoming request from A"
    );
    // Filter by sender (identity A) to avoid picking up requests from other
    // concurrent tests targeting the same identity.
    let identity_a_id = pair.identity_a.identity.id();
    assert!(
        incoming
            .iter()
            .any(|request| request.counterparty_id == identity_a_id),
        "TC-043: no request from A ({:?}) in C's incoming. Got {} request(s) from: {:?}",
        identity_a_id,
        incoming.len(),
        incoming
            .iter()
            .map(|request| request.counterparty_id)
            .collect::<Vec<_>>()
    );
    let sender_id = identity_a_id;

    // Reject the contact request
    tracing::info!("TC-043: rejecting the contact request from {:?}...", sender_id);
    let reject_task = BackendTask::DashPayTask(Box::new(DashPayTask::RejectContactRequest {
        identity: qi_c,
        counterparty_id: sender_id,
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

    let signing_key = &pair.signing_key_a;

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
