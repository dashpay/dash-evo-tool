//! Platform-wallet one-time migration — Stage B (post-unlock async engine).
//!
//! Stage A (SQL v35, sync, pre-unlock; `initialization.rs`) only arms
//! `settings.platform_wallet_migration_pending` and writes the retained
//! `<db>.premigration` recovery floor. Stage B (here) does the actual data
//! migration once, post-unlock, async, marker-gated, behind the
//! `AppContext`-owned `tokio::sync::Mutex` (invariant C6).
//!
//! Back-compat for legacy DashPay DIP-14 derivation is DROPPED
//! (Decision #6 — `dip14-migration-hardstop.md` SUPERSEDED). Contacts are
//! re-established on UPSTREAM derivation unconditionally; there is no
//! DET re-derivation, no comparison, no quarantine. The accepted
//! fund-accessibility trade-off is covered by the mandatory one-time notice
//! (P3e), the sole compensating control.
//!
//! Each step is idempotent so a crash-and-relaunch re-runs cleanly. The
//! legacy-table DROP is the strictly-last destructive step and is performed
//! only after a durable upstream flush. Restore-from-`premigration` is a
//! launch-time concern (`Database::recover_from_premigration_if_corrupt`)
//! that fires ONLY on injected corruption — never on a healthy-but-pending
//! DB. These ordering & recovery invariants are pinned by the `p3d` test
//! module in `database::initialization`.

use std::sync::Arc;

use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;

use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use crate::wallet_backend::WalletBackend;

/// Default DashPay account index. DET established contacts have always used
/// account 0 (the legacy DIP-14 path hardcoded it); upstream
/// `register_contact_account` re-derives on account 0 to match.
const DEFAULT_DASHPAY_ACCOUNT: u32 = 0;

/// The exact wallet-backend surface Stage-B needs. Abstracting these three
/// upstream-touching operations lets the real [`run_stage_b`] orchestration
/// (backup precondition, contacts loop, durable-flush-before-DROP ordering,
/// strictly-last legacy DROP, completion-before-pending marker writes,
/// `utxos` retention) be exercised network-free with a fake backend, instead
/// of relying on the live backend-e2e lane. [`WalletBackend`] implements it
/// as pure delegation — no behavior change, fund-spend path untouched.
#[allow(async_fn_in_trait)]
pub trait StageBBackend {
    /// Re-register every persisted wallet with the upstream manager.
    async fn ensure_wallets_registered(&self, ctx: &Arc<AppContext>) -> Result<(), TaskError>;

    /// Re-establish one accepted DashPay contact on upstream derivation.
    async fn register_dashpay_contact(
        &self,
        seed_hash: &WalletSeedHash,
        owner_identity_id: &Identifier,
        contact_identity_id: &Identifier,
        account_index: u32,
    ) -> Result<(), TaskError>;

    /// Durably flush every wallet's changesets to the upstream persister.
    async fn flush_persister(&self) -> Result<(), TaskError>;
}

impl StageBBackend for WalletBackend {
    async fn ensure_wallets_registered(&self, ctx: &Arc<AppContext>) -> Result<(), TaskError> {
        WalletBackend::ensure_wallets_registered(self, ctx).await
    }

    async fn register_dashpay_contact(
        &self,
        seed_hash: &WalletSeedHash,
        owner_identity_id: &Identifier,
        contact_identity_id: &Identifier,
        account_index: u32,
    ) -> Result<(), TaskError> {
        WalletBackend::register_dashpay_contact(
            self,
            seed_hash,
            owner_identity_id,
            contact_identity_id,
            account_index,
        )
        .await
    }

    async fn flush_persister(&self) -> Result<(), TaskError> {
        WalletBackend::flush_persister(self).await
    }
}

impl<B: StageBBackend + Sync> StageBBackend for Arc<B> {
    async fn ensure_wallets_registered(&self, ctx: &Arc<AppContext>) -> Result<(), TaskError> {
        (**self).ensure_wallets_registered(ctx).await
    }

    async fn register_dashpay_contact(
        &self,
        seed_hash: &WalletSeedHash,
        owner_identity_id: &Identifier,
        contact_identity_id: &Identifier,
        account_index: u32,
    ) -> Result<(), TaskError> {
        (**self)
            .register_dashpay_contact(
                seed_hash,
                owner_identity_id,
                contact_identity_id,
                account_index,
            )
            .await
    }

    async fn flush_persister(&self) -> Result<(), TaskError> {
        (**self).flush_persister().await
    }
}

/// Run the one-time Stage-B migration. Idempotent; marker-gated by the
/// caller. On success (and once [`LEGACY_DROP_ENABLED`]) clears
/// `platform_wallet_migration_pending`; on any error the marker is left set
/// (caller logs; next launch retries). Restore-from-`premigration` is the
/// caller/launch responsibility and happens ONLY on an exceptional path,
/// never here.
pub async fn run_stage_b<B: StageBBackend>(
    ctx: &Arc<AppContext>,
    backend: &B,
) -> Result<(), TaskError> {
    tracing::info!("Platform-wallet Stage-B migration: starting");

    // Step 1 — backup precondition. The retained recovery floor must exist
    // before any later (eventually destructive) step. `initialize()`
    // (re)creates it idempotently on every post-marker launch; re-assert it
    // here so Stage B never proceeds without the floor.
    if let Some(path) = ctx.db.db_file_path() {
        let backup = crate::database::Database::premigration_backup_path(&path);
        if !backup.exists() {
            return Err(TaskError::WalletBackend {
                source: Box::new(platform_wallet::error::PlatformWalletError::WalletCreation(
                    "pre-migration backup missing; refusing to migrate without recovery floor"
                        .to_string(),
                )),
            });
        }
    }

    // Step 2 — re-register every wallet from seed (idempotent; skips
    // already-registered, tolerates upstream WalletAlreadyExists). Upstream
    // `create_wallet_from_seed_bytes` also runs identity discovery from
    // chain, which is Step 3's mechanism.
    backend.ensure_wallets_registered(ctx).await?;

    // Step 3 — identities. The DET `QualifiedIdentity` blob and the
    // identity/platform-address/token tables are RETAINED (upstream "Outside
    // scope", data-model-and-migration.md conversion surface). Upstream
    // `IdentityManager` is repopulated by the chain-driven identity sync that
    // step 2's wallet registration already performed — no separate
    // low-level `add_identity` push is needed (and it would risk
    // IdentityAlreadyExists against that sync). Intentional no-op.

    // Step 4 — re-establish every accepted DashPay contact on UPSTREAM
    // derivation only. Idempotent: upstream `register_contact_account`
    // no-ops if the contact account already exists. No DET re-derivation,
    // no comparison, no quarantine (Decision #6).
    //
    // The owning wallet for a contact is the one whose seed derives the
    // contact's owner identity. DET tracks that linkage as
    // (QualifiedIdentity, owner-wallet seed_hash); build owner-id →
    // seed_hash so each contact account is registered on the right wallet.
    let owner_seed: std::collections::HashMap<Identifier, [u8; 32]> = ctx
        .db
        .get_local_user_identities(ctx)
        .map_err(|source| TaskError::Database { source })?
        .into_iter()
        .filter_map(|(qi, seed)| seed.map(|s| (qi.identity.id(), s)))
        .collect();

    let contacts = ctx
        .db
        .load_all_accepted_dashpay_contacts()
        .map_err(|source| TaskError::Database { source })?;
    tracing::info!(
        count = contacts.len(),
        "Stage-B: re-establishing DashPay contacts on upstream derivation"
    );
    for c in &contacts {
        let owner = Identifier::from_bytes(&c.owner_identity_id).map_err(|_| {
            invalid_identity("stored DashPay owner identity id is not a valid Identifier")
        })?;
        let contact = Identifier::from_bytes(&c.contact_identity_id).map_err(|_| {
            invalid_identity("stored DashPay contact identity id is not a valid Identifier")
        })?;
        let Some(seed_hash) = owner_seed.get(&owner) else {
            // Owner identity not linked to any local wallet (e.g. an
            // imported-by-id identity without its seed). The contact cannot
            // be re-established on upstream derivation; skip it — the
            // accepted-trade-off notice (P3e) already informs the user that
            // some DashPay state may not carry over.
            tracing::warn!(
                owner = %owner,
                "Stage-B: accepted DashPay contact has no local owner wallet; skipping"
            );
            continue;
        };
        // A per-contact derivation/registration failure aborts Stage B with
        // the marker left set (idempotent re-run next launch) rather than
        // silently skipping — DashPay correctness is not best-effort.
        backend
            .register_dashpay_contact(seed_hash, &owner, &contact, DEFAULT_DASHPAY_ACCOUNT)
            .await?;
    }

    // Step 5 — finalize (SINGLE fork: success). The exception fork is
    // implicit: any `?` above returns Err with the marker still set, so the
    // next launch re-runs Stage B (idempotent) and the launch-time recovery
    // (`Database::recover_from_premigration_if_corrupt`) restores from
    // `premigration` only if the new state is corrupt.
    //
    // Ordering is the P3d-proven invariant: durable upstream flush BEFORE the
    // destructive drop, then DROP as the strictly-last step, then clear the
    // marker. If we crash after the flush but before the drop, the next
    // launch re-runs Stage B: registration/contacts are idempotent no-ops and
    // the drop is `DROP TABLE IF EXISTS`. If we crash after the drop but
    // before clearing the marker, the next launch re-runs and the idempotent
    // drop is a clean no-op, then the marker clears.
    backend.flush_persister().await?;
    ctx.db
        .drop_legacy_migrated_tables()
        .map_err(|source| TaskError::Database { source })?;
    // Record completion BEFORE clearing the pending marker so that, if we
    // crash between the two writes, the next launch still re-runs Stage B
    // (pending stays set) and the idempotent finalise re-asserts both bits.
    // `completed` distinguishes a migrated user from a fresh install for the
    // one-time post-migration notice.
    ctx.db
        .set_platform_wallet_migration_completed(true)
        .map_err(|source| TaskError::Database { source })?;
    ctx.db
        .set_platform_wallet_migration_pending(false)
        .map_err(|source| TaskError::Database { source })?;
    tracing::info!("Platform-wallet Stage-B migration: complete (legacy tables dropped)");

    Ok(())
}

fn invalid_identity(msg: &str) -> TaskError {
    TaskError::WalletBackend {
        source: Box::new(
            platform_wallet::error::PlatformWalletError::InvalidIdentityData(msg.to_string()),
        ),
    }
}

/// QA-001 — direct coverage of the real [`run_stage_b`] engine, network-free
/// via the [`StageBBackend`] seam. Exercises the actual orchestration
/// (backup precondition, wallet re-registration, the contacts loop, the
/// durable-flush-BEFORE-DROP ordering, the strictly-last legacy DROP, the
/// completion-before-pending marker writes, and the `utxos` retention) — not
/// just flag flips.
#[cfg(test)]
mod stage_b_engine {
    use super::*;
    use crate::database::Database;
    use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
    use crate::model::wallet::WalletSeedHash;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
    use dash_sdk::platform::Identifier;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// Records every [`StageBBackend`] call the real engine makes so the
    /// test can assert the engine actually drove each step (not a no-op).
    #[derive(Default)]
    struct FakeBackend {
        wallets_registered: Mutex<u32>,
        contacts: Mutex<Vec<(WalletSeedHash, Identifier, Identifier, u32)>>,
        flushed: Mutex<u32>,
    }

    impl StageBBackend for FakeBackend {
        async fn ensure_wallets_registered(&self, _ctx: &Arc<AppContext>) -> Result<(), TaskError> {
            *self.wallets_registered.lock().unwrap() += 1;
            Ok(())
        }

        async fn register_dashpay_contact(
            &self,
            seed_hash: &WalletSeedHash,
            owner_identity_id: &Identifier,
            contact_identity_id: &Identifier,
            account_index: u32,
        ) -> Result<(), TaskError> {
            self.contacts.lock().unwrap().push((
                *seed_hash,
                *owner_identity_id,
                *contact_identity_id,
                account_index,
            ));
            Ok(())
        }

        async fn flush_persister(&self) -> Result<(), TaskError> {
            *self.flushed.lock().unwrap() += 1;
            Ok(())
        }
    }

    /// Write the bundled `.env` into `data_dir` so `AppContext::new`
    /// resolves the Testnet config network-free (no `.start()` => no
    /// network I/O), mirroring the backend-e2e harness setup.
    fn ensure_test_env(data_dir: &std::path::Path) {
        crate::app_dir::ensure_env_file(data_dir);
    }

    fn table_exists(db: &Database, table: &str) -> bool {
        let conn = db.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            rusqlite::params![table],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .unwrap_or(false)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_stage_b_success_path_exercises_real_engine() {
        let tmp = tempfile::tempdir().expect("tempdir");
        ensure_test_env(tmp.path());
        let db_file = tmp.path().join("data.db");
        let db = Arc::new(Database::new(&db_file).expect("db"));
        db.initialize(&db_file).expect("init");

        let network = Network::Testnet;
        let net_str = network.to_string();
        let seed_hash: WalletSeedHash = [7u8; 32];

        // Owner identity (linked to the wallet seed) + accepted contact.
        let owner_id = Identifier::from(*b"qa001-stage-b-owner-identity-id!");
        let contact_id = Identifier::from(*b"qa001-stage-b-contact-identity!!");

        // A valid serialized BIP-44 account-0 xpub so `AppContext::new`'s
        // `get_wallets` parse of the seeded legacy row succeeds.
        let epk_bytes = {
            use dash_sdk::dpp::dashcore::secp256k1::Secp256k1;
            use dash_sdk::dpp::key_wallet::bip32::{
                ChildNumber, DerivationPath, ExtendedPrivKey, ExtendedPubKey,
            };
            let secp = Secp256k1::new();
            let master = ExtendedPrivKey::new_master(network, &[7u8; 64]).expect("master key");
            let path = DerivationPath::from(vec![
                ChildNumber::Hardened { index: 44 },
                ChildNumber::Hardened { index: 1 },
                ChildNumber::Hardened { index: 0 },
            ]);
            let account = master.derive_priv(&secp, &path).expect("derive account");
            ExtendedPubKey::from_priv(&secp, &account).encode().to_vec()
        };

        // Seed a representative legacy DB: legacy wallet row, a single-key
        // wallet + its retained utxos, a local owner identity linked to the
        // wallet seed, and an accepted DashPay contact owned by it.
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO wallet (seed_hash, encrypted_seed, salt, nonce, \
                 master_ecdsa_bip44_account_0_epk, alias, is_main, uses_password, \
                 network) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, 0, ?7)",
                rusqlite::params![
                    seed_hash.to_vec(),
                    vec![0u8; 64], // open-wallet seed: exactly 64 bytes
                    vec![0u8; 16],
                    vec![0u8; 12],
                    epk_bytes,
                    "legacy-wallet",
                    net_str,
                ],
            )
            .expect("seed legacy wallet");
            conn.execute(
                "INSERT INTO dashpay_contacts (owner_identity_id, \
                 contact_identity_id, network, contact_status) \
                 VALUES (?1, ?2, ?3, 'accepted')",
                rusqlite::params![owner_id.to_vec(), contact_id.to_vec(), net_str],
            )
            .expect("seed accepted contact");
        }

        // Local owner identity blob linked to the wallet seed_hash so the
        // contacts loop can resolve owner-id -> seed_hash and re-register.
        let pv = dash_sdk::dpp::version::PlatformVersion::latest();
        let identity = Identity::create_basic_identity(owner_id, pv).expect("identity");
        let qi = QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: Some("qa001-owner".to_string()),
            private_keys: Default::default(),
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: crate::model::qualified_identity::IdentityStatus::Active,
            network,
        };
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO identity (id, data, is_local, alias, identity_type, \
                 network, wallet, wallet_index, status) \
                 VALUES (?1, ?2, 1, ?3, 'User', ?4, ?5, 0, 0)",
                rusqlite::params![
                    qi.identity.id().to_vec(),
                    qi.to_bytes(),
                    qi.alias,
                    net_str,
                    seed_hash.to_vec(),
                ],
            )
            .expect("seed local identity");
        }

        // Seed the RETAINED utxos table (single-key load path, Decision #7).
        db.insert_utxo(
            &[9u8; 32],
            0,
            &dash_sdk::dpp::dashcore::Address::p2pkh(
                &dash_sdk::dpp::dashcore::PublicKey::from_slice(&[
                    2, 0x50, 0x86, 0x3a, 0xd6, 0x4a, 0x87, 0xae, 0x8a, 0x2f, 0xe8, 0x3c, 0x1a,
                    0xf1, 0xa8, 0x40, 0x3c, 0xb5, 0x3f, 0x53, 0xe4, 0x86, 0xd8, 0x51, 0x1d, 0xad,
                    0x8a, 0x04, 0x88, 0x7e, 0x5b, 0x23, 0x52,
                ])
                .expect("pubkey"),
                network,
            ),
            42_000,
            &[0u8; 25],
            network,
        )
        .expect("seed retained utxo");

        // The Stage-B backup precondition: lay down the recovery floor.
        let floor = Database::premigration_backup_path(&db_file);
        std::fs::copy(&db_file, &floor).expect("create recovery floor");

        // Network-free AppContext (no `.start()` => no network).
        let app_context = AppContext::new(
            tmp.path().to_path_buf(),
            network,
            db.clone(),
            None,
            Default::default(),
            Default::default(),
            egui::Context::default(),
        )
        .expect("AppContext");

        let backend = FakeBackend::default();

        // Run the REAL engine.
        run_stage_b(&app_context, &backend)
            .await
            .expect("Stage-B success path");

        // Engine actually drove each backend step.
        assert_eq!(
            *backend.wallets_registered.lock().unwrap(),
            1,
            "wallets must be re-registered by the engine"
        );
        assert_eq!(
            *backend.flushed.lock().unwrap(),
            1,
            "persister must be durably flushed before the drop"
        );
        let contacts = backend.contacts.lock().unwrap();
        assert_eq!(
            contacts.len(),
            1,
            "the accepted contact must be re-established"
        );
        assert_eq!(
            contacts[0].0, seed_hash,
            "contact registered on owner wallet"
        );
        assert_eq!(contacts[0].1, owner_id);
        assert_eq!(contacts[0].2, contact_id);
        assert_eq!(contacts[0].3, DEFAULT_DASHPAY_ACCOUNT);
        drop(contacts);

        // Strictly-last destructive DROP ran: only the upstream-owned
        // legacy tables are gone. The `wallet` / `wallet_addresses` tables
        // are the DET-retained seed store and must survive.
        assert!(
            !table_exists(&db, "wallet_transactions"),
            "legacy wallet_transactions table dropped"
        );
        assert!(
            !table_exists(&db, "dashpay_contacts"),
            "migrated dashpay_contacts table dropped"
        );

        // DET-retained seed store: must survive Stage-B so that the next
        // `AppContext::new` rehydrates the persisted wallets and the GUI
        // does not show an empty wallet list.
        assert!(
            table_exists(&db, "wallet"),
            "DET-retained wallet table must survive Stage-B (seed store)"
        );
        assert!(
            table_exists(&db, "wallet_addresses"),
            "DET-retained wallet_addresses table must survive Stage-B"
        );

        // Decision-#7 carve-out: utxos RETAINED through the migration.
        assert!(
            table_exists(&db, "utxos"),
            "utxos table must survive Stage-B (single-key carve-out)"
        );
        assert!(
            table_exists(&db, "single_key_wallet"),
            "single_key_wallet table must survive Stage-B"
        );

        // Markers: completed set, pending cleared (completion BEFORE clear).
        assert!(
            db.get_platform_wallet_migration_completed().unwrap(),
            "completed marker set"
        );
        assert!(
            !db.get_platform_wallet_migration_pending().unwrap(),
            "pending marker cleared as the strictly-last write"
        );
    }
}
