//! Measurement: what the network charges for each of the two documents of a
//! DPNS name registration, and what balance it wants before the second.
//!
//! Each run logs one line starting with `MEASURE` holding the identity balance
//! before, after the first document and after the second, in credits. The
//! tests call the SDK's registration directly, as the app's task does, so the
//! second document can be held back while the balance in between is read, and
//! so the app's own balance check does not stop an identity that is funded
//! too little on purpose. What is left on each identity goes back to the
//! platform address that funded it.
//!
//! Every run spends credits to learn a number, not to check the app. The
//! module stays in the suite because it is the only way to re-measure the
//! split between the two documents and the balance the network wants after a
//! Platform upgrade.

use crate::framework::harness::ctx;
use crate::framework::task_runner::run_on_large_stack;
use crate::register_dpns::{
    balance_below, network_balance, platform_funded_identity, refused_for_balance,
    returning_credits, vote_label,
};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::fee_estimation::contest_fee_credits;
use dash_evo_tool::model::identity_key_usability::SigningScope;
use dash_evo_tool::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
use dash_sdk::platform::dpns_usernames::{
    RegisterDpnsNameInput as SdkRegistrationInput, RegisterDpnsNameResult,
};
use rand::prelude::*;
use std::sync::Arc;
use std::time::Duration;

/// How long the second document is held back while the balance is read.
const SECOND_DOCUMENT_HOLD: Duration = Duration::from_secs(90);
/// Enough for the first document, too little for the network to accept the
/// second.
const UNDERFUNDED_CREDITS: u64 = 60_000_000;

/// A registration as the network billed it.
struct Measured {
    /// Balance once the first document was confirmed, when it could be read.
    after_first: Option<u64>,
    registered: Result<RegisterDpnsNameResult, dash_sdk::Error>,
}

/// A 20-character name: the shortest that never needs a community vote.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn measure_dpns_registration_20_chars() {
    let measured = measure(twenty_char_label(), false, None).await;
    measured
        .registered
        .expect("the registration should succeed");
}

/// A 63-character name: the longest a label can be.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn measure_dpns_registration_63_chars() {
    let mut rng = rand::rng();
    let label = format!(
        "e2e-m-{:016x}{:016x}{:016x}{}",
        rng.random::<u64>(),
        rng.random::<u64>(),
        rng.random::<u64>(),
        "k".repeat(9)
    );
    let measured = measure(label, false, None).await;
    measured
        .registered
        .expect("the registration should succeed");
}

/// A 19-letter name, which opens a community vote.
///
/// Costs the community vote fee (0.1 DASH from protocol 14, 0.2 DASH before),
/// which the network never returns, and leaves a vote open on the network for
/// a random name.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn measure_dpns_registration_contested() {
    let measured = measure(vote_label(), true, None).await;
    measured
        .registered
        .expect("the registration should succeed");
}

/// An identity that can pay for the first document but not the second: the
/// network's refusal names the balance it wanted, which is logged.
///
/// The first document's fee is spent for nothing; that is the point.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn measure_dpns_registration_balance_the_network_requires() {
    let measured = measure(twenty_char_label(), false, Some(UNDERFUNDED_CREDITS)).await;

    assert!(
        measured.after_first.is_some(),
        "the first document should have been paid for"
    );
    let error = measured
        .registered
        .expect_err("the second document should be refused");
    let (available, required) = refused_for_balance(error)
        .expect("the second document should be refused for lack of balance");
    tracing::info!(
        available,
        required_for_second_document = required,
        "MEASURE balance the network requires"
    );
}

fn twenty_char_label() -> String {
    let suffix: u64 = rand::rng().random();
    format!("e2e-m-{:014x}", suffix >> 8)
}

/// Fund a new identity from platform address credits, register `label` for it,
/// log the balances, and send what is left back.
///
/// `funding` replaces the balance the app would require, vote fee included.
async fn measure(label: String, needs_vote: bool, funding: Option<u64>) -> Measured {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;
    let vote_fee = if needs_vote {
        contest_fee_credits(app_context.sdk_platform_version())
    } else {
        0
    };
    let funding = funding.unwrap_or_else(|| {
        app_context
            .fee_estimator()
            .estimate_username_registration(label.len(), needs_vote)
            .saturating_add(vote_fee)
    });

    let funded = platform_funded_identity(ctx, funding).await;
    returning_credits(
        app_context,
        &funded,
        register_and_log(app_context, &funded.identity, label, needs_vote, vote_fee),
    )
    .await
}

/// Register `label` and log the identity balance around each document.
async fn register_and_log(
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    label: String,
    needs_vote: bool,
    vote_fee: u64,
) -> Measured {
    let id = identity.identity.id();
    let before = network_balance(app_context, id)
        .await
        .expect("the new identity's balance should be readable");
    let dpns =
        load_system_data_contract(SystemDataContract::DPNS, app_context.sdk_platform_version())
            .expect("the DPNS contract should load");
    let preorder = dpns
        .document_type_for_name("preorder")
        .expect("the DPNS contract should define preorder");
    let signing_key = identity
        .document_signing_key(
            SigningScope::ContractWide {
                contract_id: dpns.id(),
            },
            &preorder,
        )
        .cloned()
        .expect("the identity should hold a key that signs name documents");

    let (confirmed_tx, confirmed_rx) = tokio::sync::oneshot::channel::<()>();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel::<()>();
    let input = SdkRegistrationInput {
        // States the fund the app approves, as the app's task does: the vote
        // fee for a contested name, nothing otherwise.
        contest_fund: Some(vote_fee),
        label: label.clone(),
        identity: identity.identity.clone(),
        identity_public_key: signing_key,
        signer: identity.clone(),
        // The SDK calls this once the first document is confirmed. Blocking
        // here holds the second one back until the balance in between is read.
        preorder_callback: Some(Box::new(move |_| {
            let _ = confirmed_tx.send(());
            let _ = resume_rx.recv_timeout(SECOND_DOCUMENT_HOLD);
        })),
    };
    let registration = run_on_large_stack({
        let sdk = app_context.sdk();
        move || async move { sdk.register_dpns_name(input).await }
    });
    let balance_between = async {
        confirmed_rx.await.ok()?;
        let after_first = balance_below(app_context, id, before).await;
        let _ = resume_tx.send(());
        after_first
    };
    let (registered, after_first) = tokio::join!(registration, balance_between);

    let after_second = if registered.is_ok() {
        balance_below(app_context, id, after_first.unwrap_or(before)).await
    } else {
        network_balance(app_context, id).await
    };
    tracing::info!(
        label,
        label_chars = label.len(),
        needs_vote,
        vote_fee,
        balance_before = before,
        balance_after_first = ?after_first,
        balance_after_second = ?after_second,
        first_document = ?after_first.and_then(|after| before.checked_sub(after)),
        second_document = ?after_first.zip(after_second).and_then(|(first, second)| first.checked_sub(second)),
        total = ?after_second.and_then(|after| before.checked_sub(after)),
        error = ?registered.as_ref().err(),
        "MEASURE dpns registration"
    );
    Measured {
        after_first,
        registered,
    }
}
