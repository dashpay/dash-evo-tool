//! U1: the offer to restore keys stranded in the previous version's saved data
//! (issue #889) shows exactly what it can restore, and only when it can.

use std::sync::{Arc, Mutex};

use dash_evo_tool::model::legacy_recovery::{
    ExclusionReason, RecoveryItem, RecoveryItemDescriptor, RecoveryPlan,
};
use dash_evo_tool::model::qualified_identity::PrivateKeyTarget;
use dash_evo_tool::ui::components::legacy_recovery_section::LegacyRecoverySection;
use dash_evo_tool::ui::components::{Component, ComponentResponse};
use dash_sdk::dpp::identity::Purpose;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use crate::support;

const INTRO: &str = "Some keys for this identity from your previous Dash Evo Tool version haven't been brought \
     across.";
const RESTORE: &str = "Restore keys…";

const MAIN: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnMainIdentity;
const VOTER: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnVoterIdentity;

fn key(target: PrivateKeyTarget, key_id: u32, purpose: Purpose) -> RecoveryItemDescriptor {
    RecoveryItemDescriptor {
        item: RecoveryItem::Key { target, key_id },
        purpose: Some(purpose),
    }
}

fn association(item: RecoveryItem) -> RecoveryItemDescriptor {
    RecoveryItemDescriptor {
        item,
        purpose: None,
    }
}

/// Render `plan` once, returning the harness and the items the section
/// approved (only non-`None` if the caller clicks Restore before running).
fn render(
    plan: RecoveryPlan,
    restoring: bool,
) -> (Harness<'static>, Arc<Mutex<Option<Vec<RecoveryItem>>>>) {
    let approved = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&approved);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(move |ui| {
            let response = LegacyRecoverySection::new(&plan)
                .restoring(restoring)
                .show(ui);
            if let Some(items) = response.inner.changed_value() {
                *sink.lock().expect("approval sink") = Some(items.clone());
            }
        });
    // Stepped rather than run to settle: the in-flight spinner repaints
    // forever by design, which `run` treats as a failure to converge.
    harness.run_steps(3);
    (harness, approved)
}

/// An empty plan renders nothing at all, so the offer retires itself once
/// there is nothing left stranded rather than lingering as an empty box.
#[test]
fn an_empty_plan_renders_no_section() {
    let (harness, _) = render(RecoveryPlan::default(), false);

    assert!(harness.query_by_label(INTRO).is_none());
    assert!(harness.query_by_label(RESTORE).is_none());
}

/// Every restorable item is named in the operator's own role words, with the
/// voter-identity link folded into the voting key it belongs to — a voting key
/// without its link cannot vote, so the two are one decision.
#[test]
fn restorable_items_are_listed_by_role_with_the_voter_link_folded_in() {
    let plan = RecoveryPlan {
        items: vec![
            key(MAIN.clone(), 1, Purpose::OWNER),
            key(MAIN.clone(), 2, Purpose::TRANSFER),
            key(VOTER.clone(), 3, Purpose::VOTING),
            association(RecoveryItem::VoterAssociation),
        ],
        excluded: vec![],
    };

    let (harness, _) = render(plan, false);

    assert!(harness.query_by_label(INTRO).is_some());
    assert!(harness.query_by_label("Owner key").is_some());
    assert!(harness.query_by_label("Payout key").is_some());
    assert!(harness.query_by_label("Voting key").is_some());
    assert!(
        harness.query_by_label("Voting identity link").is_none(),
        "the voter link is previewed as part of its voting key, not as its own row",
    );
    assert!(harness.query_by_label(RESTORE).is_some());
}

/// Pressing Restore approves exactly the previewed set — including the voter
/// link the preview folded away, which travels with its voting key.
#[test]
fn pressing_restore_approves_the_previewed_items() {
    let plan = RecoveryPlan {
        items: vec![
            key(VOTER.clone(), 3, Purpose::VOTING),
            association(RecoveryItem::VoterAssociation),
        ],
        excluded: vec![],
    };

    let (mut harness, approved) = render(plan, false);
    harness.get_by_label(RESTORE).click();
    harness.run_steps(3);

    let approved = approved.lock().expect("approval sink").clone();
    assert_eq!(
        approved,
        Some(vec![
            RecoveryItem::Key {
                target: VOTER,
                key_id: 3,
            },
            RecoveryItem::VoterAssociation,
        ]),
    );
}

/// While a restore is in flight the button is replaced by a progress line, so
/// the same restore cannot be dispatched twice.
#[test]
fn a_restore_in_flight_replaces_the_button_with_progress() {
    let plan = RecoveryPlan {
        items: vec![key(MAIN.clone(), 1, Purpose::OWNER)],
        excluded: vec![],
    };

    let (harness, _) = render(plan, true);

    assert!(harness.query_by_label("Restoring…").is_some());
    assert!(harness.query_by_label(RESTORE).is_none());
}

/// A key this flow cannot restore is listed with its own explanation rather
/// than dropped from a list the user reads as complete — and it never gets a
/// Restore button of its own.
#[test]
fn unrestorable_keys_are_listed_separately_with_no_restore_offered() {
    let plan = RecoveryPlan {
        items: vec![],
        excluded: vec![(
            key(MAIN.clone(), 1, Purpose::OWNER),
            ExclusionReason::LegacyEncryptedFormat,
        )],
    };

    let (harness, _) = render(plan, false);

    assert!(harness.query_by_label(INTRO).is_some());
    assert!(
        harness
            .query_by_label("These keys cannot be brought back automatically:")
            .is_some()
    );
    assert!(harness.query_by_label("Owner key").is_some());
    assert!(
        harness.query_by_label(RESTORE).is_none(),
        "nothing here is restorable, so there is nothing to press",
    );
}

/// An install with no previous-version database never even asks: detection is
/// not dispatched, so the offer can never appear on a fresh install.
#[test]
fn an_install_without_previous_version_data_never_checks() {
    support::with_isolated_data_dir(|| {
        let (_rt, app_context) = support::fresh_app_context();
        let mut state = dash_evo_tool::ui::state::legacy_recovery::LegacyRecoveryState::new(
            &app_context,
            dash_sdk::platform::Identifier::from([7u8; 32]),
        );

        assert!(
            state.ensure_checked().is_none(),
            "with no previous-version database there is nothing to detect",
        );
        assert!(state.plan().is_none());
    });
}
