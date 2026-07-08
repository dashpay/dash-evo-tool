//! Kittest coverage for `IdentitySelector` — the app-scoped write-back keystone.
//!
//! The critical write-back path — a genuine ComboBox picker
//! change must call `AppContext::set_selected_identity`, keeping "who am I signing
//! as" in sync across the breadcrumb and all 12 SYNC screens. This is a
//! component-level lock: testing `IdentitySelector` directly proves the shared
//! mechanism for all 12 SYNC callsites without needing private keys or a full
//! screen fixture for each.
//!
//! Two phases:
//! - **Phase 1 (seeding-not-write-back):** with the buffer pre-seeded to Alice's
//!   Base58, an initial render must NOT invoke `set_selected_identity`. The
//!   `combo_changed` flag stays false until a user interaction occurs.
//! - **Phase 2 (genuine ComboBox change):** click the ComboBox header (Alice),
//!   click Bob's dropdown item, and assert
//!   `ctx.selected_identity_id() == Some(bob_id)`.
//!
//! ## Setup
//!
//! We use a `build_eframe` harness (`run_steps(5)`) to fully initialize the
//! `AppContext` — specifically to let `ensure_wallet_backend` run and complete
//! `restore_selected_identity_from_kv`. Identity selection is set AFTER that
//! initialization so the KV restore does not race against our seed.
//! A separate `build_ui` harness then hosts the standalone `IdentitySelector`.
//!
//! ## egui-kittest ComboBox quirk (documented by Marvin's QA run)
//!
//! The ComboBox header is exposed to accesskit via its `selected_text` as the
//! **value** property → use `harness.get_by_value("Alice")` to click it open.
//! Items inside the popup are `selectable_label` widgets exposed as Buttons via
//! their text **label** → use `harness.get_by_label("Bob")` to select them.
//!
//! Per-screen click-through tests (which DO need private keys so the screen gates
//! on `has_suitable_keys`) remain deferred per the `TODO(WalletFixture)` comments
//! in `contract_screen.rs`, `dashpay_screen.rs`, and `tokens_screen.rs`.

use crate::support::with_isolated_data_dir;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use dash_evo_tool::ui::components::identity_selector::IdentitySelector;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Identifier;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

/// Build a wallet-less `QualifiedIdentity` in-memory. No DB insertion, no
/// private keys — only `id()` + `display_string()` (= alias) are exercised.
fn make_qi(byte: u8, alias: &str) -> QualifiedIdentity {
    let pv = PlatformVersion::latest();
    let identity =
        Identity::create_basic_identity(Identifier::from([byte; 32]), pv).expect("basic identity");
    QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
        alias: Some(alias.to_string()),
        private_keys: KeyStorage::default(),
        dpns_names: vec![],
        associated_wallets: BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: BTreeMap::new(),
        status: IdentityStatus::PendingCreation,
        network: Network::Testnet,
    }
}

/// Keystone write-back lock for the app-scoped identity migration.
///
/// A genuine ComboBox picker change on an `IdentitySelector` configured with
/// `.syncing_global(ctx)` must propagate to `ctx.selected_identity_id()`.
///
/// No private keys, no wallet backend write-path, no full screen fixture needed:
/// `set_selected_identity` with wallet-less identities only updates in-memory
/// mutexes; the selector's `identities` slice is built in-memory. The component-
/// level test covers the write-back for all 12 SYNC screens in one assertion.
#[test]
fn combo_change_writes_selection_to_app_context() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        // Step 1: fully initialize AppContext via build_eframe + run_steps.
        //
        // `ensure_wallet_backend` calls `restore_selected_identity_from_kv` on
        // first wiring. Running 5 steps ensures that happens BEFORE we seed the
        // identity, so our seed is not overwritten by the async initialization.
        let mut setup = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("AppState builds")
                .with_animations(false)
        });
        setup.run_steps(5);
        let ctx = setup.state().current_app_context().clone();

        let alice = make_qi(0xAA, "Alice");
        let bob = make_qi(0xBB, "Bob");
        let alice_id = alice.identity.id();
        let bob_id = bob.identity.id();

        // Seed global AFTER KV restore; wallet_backend is now wired and
        // restore_selected_identity_from_kv will not run again (idempotent).
        ctx.set_selected_identity(Some(alice_id));

        // Step 2: build a standalone build_ui harness hosting the IdentitySelector.
        //
        // Rc<RefCell<>> shared state: the closure captures the inner Rc clones,
        // the outer clones remain accessible for post-harness assertions.
        let ids = vec![alice.clone(), bob.clone()];
        let buf = Rc::new(RefCell::new(alice_id.to_string(Encoding::Base58)));
        let sel: Rc<RefCell<Option<QualifiedIdentity>>> = Rc::new(RefCell::new(None));

        let buf_inner = Rc::clone(&buf);
        let sel_inner = Rc::clone(&sel);
        let ctx_inner = Arc::clone(&ctx);

        let mut harness = Harness::builder()
            .with_size(egui::vec2(400.0, 80.0))
            .build_ui(move |ui| {
                let mut b = buf_inner.borrow_mut();
                let mut s = sel_inner.borrow_mut();
                let _ = ui.add(
                    IdentitySelector::new("qa001_combo", &mut b, &ids)
                        .selected_identity(&mut s)
                        .expect("selected_identity")
                        .other_option(false)
                        .syncing_global(Arc::clone(&ctx_inner)),
                );
            });

        // ── Phase 1: initial render ───────────────────────────────────────────
        // The buffer is pre-seeded to Alice's Base58. No user interaction has
        // occurred, so combo_changed=false and sync_to_global is NOT called.
        // The global must remain Alice after the render.
        harness.run();
        assert_eq!(
            ctx.selected_identity_id(),
            Some(alice_id),
            "Phase 1: initial render must not invoke set_selected_identity (seeding ≠ write-back)"
        );

        // ── Phase 2: genuine ComboBox change to Bob ───────────────────────────
        // ComboBox header selected_text = "Alice" → accesskit value = "Alice".
        // selectable_label("Bob") popup item → accesskit label = "Bob".
        harness.get_by_value("Alice").click(); // open the ComboBox popup
        harness.run(); // render the open popup (Alice + Bob as selectable items)
        harness.get_by_label("Bob").click(); // select Bob's item
        harness.run(); // combo_changed=true → sync_to_global() → set_selected_identity(bob)

        assert_eq!(
            ctx.selected_identity_id(),
            Some(bob_id),
            "Phase 2: selecting Bob via ComboBox must propagate bob_id to AppContext"
        );
    });
}
