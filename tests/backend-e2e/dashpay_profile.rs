//! Profile writes through platform-wallet, independent of the contact-flow suite.

use crate::framework::{harness, identity_helpers, task_runner::run_task};
use dash_evo_tool::backend_task::dashpay::DashPayTask;
use dash_evo_tool::backend_task::identity::{IdentityKeyEntry, IdentityKeySpecs, IdentityTask};
use dash_evo_tool::backend_task::wallet::WalletTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::signer::Signer;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};

#[ignore = "requires a funded E2E_WALLET_MNEMONIC and live testnet"]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn profile_create_and_replace_with_high_derived_key() {
    profile_create_and_replace(KeyType::ECDSA_SECP256K1, false).await;
}

#[ignore = "requires a funded E2E_WALLET_MNEMONIC and live testnet"]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn profile_create_and_replace_with_high_hash160_derived_key() {
    profile_create_and_replace(KeyType::ECDSA_HASH160, false).await;
}

#[ignore = "requires a funded E2E_WALLET_MNEMONIC and live testnet"]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn profile_create_and_replace_skips_unavailable_hash160_key() {
    profile_create_and_replace(KeyType::ECDSA_SECP256K1, true).await;
}

async fn profile_create_and_replace(key_type: KeyType, unavailable_hash160: bool) {
    let ctx = harness::ctx().await;
    let (seed_hash, wallet) = ctx.create_funded_test_wallet(30_000_000).await;
    let mut registration =
        identity_helpers::build_identity_registration(&ctx.app_context, &wallet, seed_hash).await;
    run_task(
        &ctx.app_context,
        BackendTask::WalletTask(WalletTask::WarmIdentityAuthPubkeys {
            seed_hash,
            identity_index: 0,
            key_count: if unavailable_hash160 { 3 } else { 2 },
        }),
    )
    .await
    .expect("derive public authentication keys");
    let backend = ctx.app_context.wallet_backend().expect("wallet backend");
    let network = ctx.app_context.network();
    let cache = backend.auth_pubkey_cache().get(network, &seed_hash);
    let entry = |index, security_level, key_type| {
        IdentityKeyEntry::from_cached_public_key(
            cache.get(network, 0, index).expect("derived public key"),
            network,
            0,
            index,
            key_type,
            Purpose::AUTHENTICATION,
            security_level,
            None,
        )
    };
    let signing_keys = if unavailable_hash160 {
        vec![
            entry(1, SecurityLevel::HIGH, KeyType::ECDSA_HASH160),
            entry(2, SecurityLevel::HIGH, key_type),
        ]
    } else {
        vec![entry(1, SecurityLevel::HIGH, key_type)]
    };
    registration.keys = IdentityKeySpecs::new(
        Some(entry(0, SecurityLevel::MASTER, key_type)),
        signing_keys,
    );

    let result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::RegisterIdentity(registration)),
    )
    .await
    .expect("register a wallet-derived HIGH authentication key");
    let BackendTaskSuccessResult::RegisteredIdentity(mut identity, _) = result else {
        panic!("expected a registered identity, got {result:?}");
    };
    let owner = identity.identity.id();
    if unavailable_hash160 {
        // Keep the registered public key but simulate a partial private-key import.
        identity.private_keys = identity
            .private_keys
            .into_entries()
            .filter(|(_, (key, _))| key.identity_public_key.id() != 1)
            .collect::<std::collections::BTreeMap<_, _>>()
            .into();
        let unavailable_key = identity.identity.public_keys().get(&1).unwrap();
        assert_eq!(unavailable_key.key_type(), KeyType::ECDSA_HASH160);
        assert!(!identity.can_sign_with(unavailable_key));
    }
    let signing_key = identity
        .identity
        .get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            [SecurityLevel::HIGH].into(),
            [key_type].into(),
            false,
        )
        .expect("registered identity must retain its requested signing-key type");
    assert_eq!(signing_key.data().len(), key_type.default_size());
    assert!(identity.can_sign_with(signing_key));

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
