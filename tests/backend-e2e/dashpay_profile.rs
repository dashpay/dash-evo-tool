//! Profile writes through platform-wallet, independent of the contact-flow suite.

use crate::framework::{harness, identity_helpers, task_runner::run_task};
use dash_evo_tool::backend_task::dashpay::DashPayTask;
use dash_evo_tool::backend_task::identity::{IdentityKeyEntry, IdentityKeySpecs, IdentityTask};
use dash_evo_tool::backend_task::wallet::WalletTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};

#[ignore = "requires a funded E2E_WALLET_MNEMONIC and live testnet"]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn profile_create_and_replace_with_high_derived_key() {
    let ctx = harness::ctx().await;
    let (seed_hash, wallet) = ctx.create_funded_test_wallet(30_000_000).await;
    let mut registration =
        identity_helpers::build_identity_registration(&ctx.app_context, &wallet, seed_hash).await;
    run_task(
        &ctx.app_context,
        BackendTask::WalletTask(WalletTask::WarmIdentityAuthPubkeys {
            seed_hash,
            identity_index: 0,
            key_count: 2,
        }),
    )
    .await
    .expect("derive public authentication keys");
    let backend = ctx.app_context.wallet_backend().expect("wallet backend");
    let network = ctx.app_context.network();
    let cache = backend.auth_pubkey_cache().get(network, &seed_hash);
    let entry = |index, security_level| {
        IdentityKeyEntry::from_cached_public_key(
            cache.get(network, 0, index).expect("derived public key"),
            network,
            0,
            index,
            KeyType::ECDSA_SECP256K1,
            Purpose::AUTHENTICATION,
            security_level,
            None,
        )
    };
    registration.keys = IdentityKeySpecs::new(
        Some(entry(0, SecurityLevel::MASTER)),
        vec![entry(1, SecurityLevel::HIGH)],
    );

    let result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::RegisterIdentity(registration)),
    )
    .await
    .expect("register a wallet-derived HIGH authentication key");
    let BackendTaskSuccessResult::RegisteredIdentity(identity, _) = result else {
        panic!("expected a registered identity, got {result:?}");
    };
    let owner = identity.identity.id();

    for (name, bio) in [
        ("Profile creation", "First bio"),
        ("Profile replacement", "Updated bio"),
    ] {
        let result = run_task(
            &ctx.app_context,
            BackendTask::DashPayTask(Box::new(DashPayTask::UpdateProfile {
                identity: identity.clone(),
                display_name: Some(name.into()),
                bio: Some(bio.into()),
                avatar_url: None,
            })),
        )
        .await
        .expect("write the profile using its HIGH key");
        assert!(
            matches!(result, BackendTaskSuccessResult::DashPayProfileUpdated(id) if id == owner)
        );

        let backend = ctx.app_context.wallet_backend().expect("wallet backend");
        let profile = backend
            .dashpay_view()
            .profile(&owner)
            .await
            .expect("profile cache");
        assert_eq!(profile.display_name.as_deref(), Some(name));
        assert_eq!(profile.bio.as_deref(), Some(bio));

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            let result = run_task(
                &ctx.app_context,
                BackendTask::DashPayTask(Box::new(DashPayTask::LoadProfile {
                    identity: identity.clone(),
                })),
            )
            .await;
            if matches!(&result,
                Ok(BackendTaskSuccessResult::DashPayProfile(Some((actual_name, actual_bio, _))))
                    if actual_name == name && actual_bio == bio
            ) {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "published profile did not propagate: {result:?}"
            );
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }
}
