//! End-to-end upgrade regression from a genuine **v0.9.3** `data.db`.
//!
//! v0.9.3 is the newest released build, so its on-disk shape is what every
//! upgrading user actually hands to v1.0. The three subsystems that carry that
//! data across each have their own unit tests, but each starts from an
//! already-normalised fixture — the schema ladder from v5 or v27, the settings
//! import from a v0.10-dev `settings` table. Nothing proved they **compose**
//! from real v0.9.3 raw data, in the order `AppState` actually runs them:
//!
//! 1. [`Database::initialize`] — the schema ladder, v11 → current.
//! 2. [`import_legacy_settings`] — user preferences, at boot, **before** the
//!    active network is chosen (`AppState::new_inner`).
//! 3. [`finish_unwire::run`] — the wallet drain plus the scheduled-vote and
//!    top-up import, on the network step 2 selected.
//!
//! The fixture is the v0.9.3 schema verbatim (`git show v0.9.3:src/database/`),
//! not the modern one: `database_version = 11`, no `single_key_wallet` table,
//! no `core_wallet_name` column, no `onboarding_completed` column, and seeds
//! stored raw with empty salt/nonce.

use std::sync::Arc;

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use rusqlite::{Connection, params};

use crate::backend_task::migration::finish_unwire::{self, MigrationCompletion, sentinel_key_for};
use crate::backend_task::migration::legacy_settings::{SettingsImport, import_legacy_settings};
use crate::context::AppContext;
use crate::database::Database;
use crate::database::test_helpers::{create_database_at_path, legacy_master_epk_bytes};
use crate::model::settings::{AppSettings, RootScreenType, ThemeMode};
use crate::model::wallet::encryption::encrypt_message;
use crate::model::wallet::{ClosedKeyItem, WalletSeedHash};
use crate::wallet_backend::secret_seam::SecretScheme;
use crate::wallet_backend::{DetScope, WalletBackend};

/// `DEFAULT_DB_VERSION` as shipped by v0.9.3. Every upgrading user's `data.db`
/// enters the ladder here.
const V093_DB_VERSION: u16 = 11;

/// The network the fixture user runs on. The regression this file locks: a
/// v0.9.3 testnet user must not be relaunched on mainnet.
const USER_NETWORK: Network = Network::Testnet;

const UNPROTECTED_SEED: [u8; 64] = [0xA9; 64];
const PROTECTED_SEED: [u8; 64] = [0xC7; 64];
const PROTECTED_PASSWORD: &str = "correct horse battery staple";
const IDENTITY_ID: [u8; 32] = [0xBB; 32];
const TOP_UP_AMOUNT: u64 = 123_456;
const CONTESTED_NAME: &str = "quantum";

/// The seed vault + metadata sidecar state of the two migrated wallets, plus
/// the envelope bytes the fixture wrote, so a test can assert the protected
/// envelope travelled byte-for-byte.
struct Fixture {
    unprotected: WalletSeedHash,
    protected: WalletSeedHash,
    /// Exactly what the v0.9.3 `wallet` row holds for the protected wallet:
    /// AES-256-GCM ciphertext, 16-byte Argon2 salt, 12-byte GCM nonce.
    protected_ciphertext: Vec<u8>,
    protected_salt: Vec<u8>,
    protected_nonce: Vec<u8>,
}

/// Write a `data.db` in the exact shape v0.9.3 left on disk, then hand back the
/// keys the assertions need.
///
/// The DDL is copied from `git show v0.9.3:src/database/{initialization,
/// scheduled_votes,top_ups,tokens,proof_log}.rs` — a v0.9.3 `create_tables()`
/// builds every one of these, so a real install has them all even if it never
/// walked a migration. Deliberately absent: `single_key_wallet` (introduced by
/// ladder arm 18 — the feature did not exist in v0.9.3) and
/// `wallet.core_wallet_name` (arm 33).
fn write_v093_database(dir: &std::path::Path) -> Fixture {
    let conn = Connection::open(dir.join("data.db")).expect("create legacy data.db");
    conn.execute_batch(
        "CREATE TABLE settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            password_check BLOB,
            main_password_salt BLOB,
            main_password_nonce BLOB,
            network TEXT NOT NULL,
            start_root_screen INTEGER NOT NULL,
            custom_dash_qt_path TEXT,
            overwrite_dash_conf INTEGER,
            theme_preference TEXT DEFAULT 'System',
            database_version INTEGER NOT NULL
        );

        CREATE TABLE wallet (
            seed_hash BLOB NOT NULL PRIMARY KEY,
            encrypted_seed BLOB NOT NULL,
            salt BLOB NOT NULL,
            nonce BLOB NOT NULL,
            master_ecdsa_bip44_account_0_epk BLOB NOT NULL,
            alias TEXT,
            is_main INTEGER,
            uses_password INTEGER NOT NULL,
            password_hint TEXT,
            network TEXT NOT NULL
        );

        CREATE TABLE wallet_addresses (
            seed_hash BLOB NOT NULL,
            address TEXT NOT NULL,
            derivation_path TEXT NOT NULL,
            balance INTEGER,
            path_reference INTEGER NOT NULL,
            path_type INTEGER NOT NULL,
            PRIMARY KEY (seed_hash, address),
            FOREIGN KEY (seed_hash) REFERENCES wallet(seed_hash) ON DELETE CASCADE
        );

        CREATE TABLE utxos (
            txid BLOB NOT NULL,
            vout INTEGER NOT NULL,
            address TEXT NOT NULL,
            value INTEGER NOT NULL,
            script_pubkey BLOB NOT NULL,
            network TEXT NOT NULL,
            PRIMARY KEY (txid, vout, network)
        );

        CREATE TABLE asset_lock_transaction (
            tx_id BLOB PRIMARY KEY,
            transaction_data BLOB NOT NULL,
            amount INTEGER,
            instant_lock_data BLOB,
            chain_locked_height INTEGER,
            identity_id BLOB,
            identity_id_potentially_in_creation BLOB,
            wallet BLOB NOT NULL,
            network TEXT NOT NULL,
            FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE SET NULL,
            FOREIGN KEY (identity_id_potentially_in_creation) REFERENCES identity(id) ON DELETE SET NULL,
            FOREIGN KEY (wallet) REFERENCES wallet(seed_hash) ON DELETE CASCADE
        );

        CREATE TABLE identity (
            id BLOB PRIMARY KEY,
            data BLOB,
            status INTEGER NOT NULL DEFAULT 0,
            is_local INTEGER NOT NULL,
            alias TEXT,
            info TEXT,
            wallet BLOB,
            wallet_index INTEGER,
            identity_type TEXT,
            network TEXT NOT NULL,
            CHECK ((wallet IS NOT NULL AND wallet_index IS NOT NULL)
                OR (wallet IS NULL AND wallet_index IS NULL)),
            FOREIGN KEY (wallet) REFERENCES wallet(seed_hash) ON DELETE CASCADE
        );

        CREATE TABLE contested_name (
            normalized_contested_name TEXT NOT NULL,
            locked_votes INTEGER,
            abstain_votes INTEGER,
            awarded_to BLOB,
            end_time INTEGER,
            locked INTEGER NOT NULL DEFAULT 0,
            last_updated INTEGER,
            network TEXT NOT NULL,
            PRIMARY KEY (normalized_contested_name, network)
        );

        CREATE TABLE contestant (
            normalized_contested_name TEXT NOT NULL,
            identity_id BLOB NOT NULL,
            name TEXT,
            votes INTEGER,
            created_at INTEGER,
            created_at_block_height INTEGER,
            created_at_core_block_height INTEGER,
            document_id BLOB,
            network TEXT NOT NULL,
            PRIMARY KEY (normalized_contested_name, identity_id, network),
            FOREIGN KEY (normalized_contested_name, network)
                REFERENCES contested_name(normalized_contested_name, network) ON DELETE CASCADE
        );

        CREATE TABLE contract (
            contract_id BLOB,
            contract BLOB,
            alias TEXT,
            network TEXT NOT NULL,
            PRIMARY KEY (contract_id, network)
        );

        CREATE TABLE proof_log (
            proof_id INTEGER PRIMARY KEY AUTOINCREMENT,
            request_type INTEGER NOT NULL,
            request_bytes BLOB NOT NULL,
            path_query_bytes BLOB NOT NULL,
            height INTEGER NOT NULL,
            time_ms INTEGER NOT NULL,
            proof_bytes BLOB NOT NULL,
            error TEXT
        );

        CREATE TABLE top_up (
            identity_id BLOB NOT NULL,
            top_up_index INTEGER NOT NULL,
            amount INTEGER NOT NULL,
            PRIMARY KEY (identity_id, top_up_index),
            FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE CASCADE
        );

        CREATE TABLE scheduled_votes (
            identity_id BLOB NOT NULL,
            contested_name TEXT NOT NULL,
            vote_choice TEXT NOT NULL,
            time INTEGER NOT NULL,
            executed INTEGER NOT NULL DEFAULT 0,
            network TEXT NOT NULL,
            PRIMARY KEY (identity_id, contested_name),
            FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE CASCADE
        );

        CREATE TABLE token (
            id BLOB PRIMARY KEY,
            token_alias TEXT NOT NULL,
            token_config BLOB NOT NULL,
            data_contract_id BLOB NOT NULL,
            token_position INTEGER NOT NULL,
            network TEXT NOT NULL,
            FOREIGN KEY (data_contract_id, network)
                REFERENCES contract(contract_id, network) ON DELETE CASCADE
        );

        CREATE TABLE identity_token_balances (
            token_id BLOB NOT NULL,
            identity_id BLOB NOT NULL,
            balance INTEGER NOT NULL,
            network TEXT NOT NULL,
            PRIMARY KEY(token_id, identity_id, network),
            FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE CASCADE,
            FOREIGN KEY (token_id) REFERENCES token(id) ON DELETE CASCADE
        );

        CREATE TABLE identity_order (
            pos INTEGER NOT NULL,
            identity_id BLOB NOT NULL,
            PRIMARY KEY(pos),
            FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE CASCADE
        );

        CREATE TABLE token_order (
            pos INTEGER NOT NULL,
            token_id BLOB NOT NULL,
            identity_id BLOB NOT NULL,
            PRIMARY KEY(pos, token_id),
            FOREIGN KEY (token_id) REFERENCES token(id) ON DELETE CASCADE,
            FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE CASCADE
        );",
    )
    .expect("create v0.9.3 schema");

    // A testnet user with a dark theme who parked on the scheduled-votes screen.
    // `start_root_screen = 10` means the same screen in v0.9.3 and today, so it
    // is a value that genuinely round-trips rather than a coincidence.
    conn.execute(
        "INSERT INTO settings
            (id, network, start_root_screen, custom_dash_qt_path, overwrite_dash_conf,
             theme_preference, database_version)
         VALUES (1, ?1, ?2, '/opt/dash-qt', 0, 'Dark', ?3)",
        params![
            USER_NETWORK.to_string(),
            RootScreenType::RootScreenDPNSScheduledVotes.to_int(),
            V093_DB_VERSION,
        ],
    )
    .expect("insert settings row");

    // Unprotected wallet: v0.9.3 stores the raw 64-byte seed with EMPTY salt and
    // nonce (`add_new_wallet_screen.rs`: `(seed.to_vec(), vec![], vec![], false)`).
    let unprotected = ClosedKeyItem::compute_seed_hash(&UNPROTECTED_SEED);
    conn.execute(
        "INSERT INTO wallet
            (seed_hash, encrypted_seed, salt, nonce, master_ecdsa_bip44_account_0_epk,
             alias, is_main, uses_password, password_hint, network)
         VALUES (?1, ?2, ?3, ?4, ?5, 'Masternode Owner Wallet', 1, 0, NULL, ?6)",
        params![
            unprotected.as_slice(),
            UNPROTECTED_SEED.as_slice(),
            Vec::<u8>::new(),
            Vec::<u8>::new(),
            legacy_master_epk_bytes(&UNPROTECTED_SEED, USER_NETWORK),
            USER_NETWORK.to_string(),
        ],
    )
    .expect("insert unprotected wallet row");

    // Protected wallet: the legacy Argon2 + AES-256-GCM envelope, produced by the
    // very function v0.9.3 used, so the bytes under test are real ciphertext.
    let envelope = encrypt_message(&PROTECTED_SEED, PROTECTED_PASSWORD).expect("encrypt seed");
    let protected = ClosedKeyItem::compute_seed_hash(&PROTECTED_SEED);
    conn.execute(
        "INSERT INTO wallet
            (seed_hash, encrypted_seed, salt, nonce, master_ecdsa_bip44_account_0_epk,
             alias, is_main, uses_password, password_hint, network)
         VALUES (?1, ?2, ?3, ?4, ?5, 'Cold Storage', 0, 1, 'the usual', ?6)",
        params![
            protected.as_slice(),
            envelope.ciphertext.as_slice(),
            envelope.salt.as_slice(),
            envelope.nonce.as_slice(),
            legacy_master_epk_bytes(&PROTECTED_SEED, USER_NETWORK),
            USER_NETWORK.to_string(),
        ],
    )
    .expect("insert protected wallet row");

    // A masternode identity owned by the unprotected wallet. `data` is the opaque
    // bincode blob v0.9.3 wrote (`QualifiedIdentity::to_bytes`); no current code
    // path decodes it — see the module note on the identity-import gap.
    conn.execute(
        "INSERT INTO identity
            (id, data, status, is_local, alias, wallet, wallet_index, identity_type, network)
         VALUES (?1, ?2, 0, 1, 'my-masternode', ?3, 0, 'Masternode', ?4)",
        params![
            IDENTITY_ID.as_slice(),
            vec![0u8; 16],
            unprotected.as_slice(),
            USER_NETWORK.to_string(),
        ],
    )
    .expect("insert identity row");

    conn.execute(
        "INSERT INTO scheduled_votes
            (identity_id, contested_name, vote_choice, time, executed, network)
         VALUES (?1, ?2, 'Lock', 1700000000, 0, ?3)",
        params![
            IDENTITY_ID.as_slice(),
            CONTESTED_NAME,
            USER_NETWORK.to_string()
        ],
    )
    .expect("insert scheduled vote row");

    conn.execute(
        "INSERT INTO top_up (identity_id, top_up_index, amount) VALUES (?1, 0, ?2)",
        params![IDENTITY_ID.as_slice(), TOP_UP_AMOUNT],
    )
    .expect("insert top-up row");

    Fixture {
        unprotected,
        protected,
        protected_ciphertext: envelope.ciphertext,
        protected_salt: envelope.salt,
        protected_nonce: envelope.nonce,
    }
}

/// Boot over `dir` exactly as `AppState` does: run the ladder, import the legacy
/// preferences, then build the `AppContext` **on the network those preferences
/// named**. Returns the context and the imported settings blob.
///
/// Taking the network from the import (rather than hard-coding testnet) is the
/// point: it is what makes this a composition test. If the import lost the
/// network, every downstream `WHERE network = ?1` filter in the wallet drain
/// would silently target mainnet and find nothing.
fn boot(dir: &std::path::Path) -> (Arc<AppContext>, AppSettings) {
    crate::app_dir::ensure_env_file(dir);
    let db_file = dir.join("data.db");

    let db = Arc::new(Database::new(&db_file).expect("open data.db"));
    db.initialize(&db_file)
        .expect("schema ladder v11 -> current");

    let app_kv = AppContext::open_app_kv(dir).expect("open app k/v");
    let outcome = import_legacy_settings(&app_kv, &db).expect("import legacy settings");
    assert_eq!(
        outcome,
        SettingsImport::Imported {
            network: USER_NETWORK
        },
        "the boot import must report the network it restored",
    );

    let settings = app_kv
        .get::<AppSettings>(DetScope::Global, AppSettings::KV_KEY)
        .expect("read settings blob")
        .expect("the import must write a settings blob");

    let secret_store = AppContext::open_secret_store(dir).expect("open secret store");
    let ctx = AppContext::new(
        dir.to_path_buf(),
        settings.network,
        db,
        Default::default(),
        Default::default(),
        egui::Context::default(),
        app_kv,
        secret_store,
        crate::model::user_role::UserRoleCell::default(),
    )
    .expect("AppContext");

    (ctx, settings)
}

/// Wire the real wallet seam offline — the backend builds and hydrates its
/// sidecars without touching the network.
async fn wire_backend(ctx: &Arc<AppContext>) -> Arc<WalletBackend> {
    let (tx, _rx) = tokio::sync::mpsc::channel::<crate::app::TaskResult>(32);
    let sender = crate::utils::egui_mpsc::SenderAsync::new(tx, ctx.egui_ctx().clone());
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("wallet backend must wire offline");
    ctx.wallet_backend().expect("backend wired")
}

fn schema_version_at(db_file: &std::path::Path) -> u16 {
    Connection::open(db_file)
        .expect("open database file")
        .query_row(
            "SELECT database_version FROM settings WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .expect("read schema version")
}

fn schema_version(dir: &std::path::Path) -> u16 {
    schema_version_at(&dir.join("data.db"))
}

/// The schema version a **fresh** install lands on. Asserting the upgraded DB
/// matches this — rather than a hard-coded number — states the real contract
/// ("an upgraded v0.9.3 install is schema-identical to a new one") and cannot
/// drift when the ladder grows another arm.
fn fresh_install_schema_version() -> u16 {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_file = dir.path().join("fresh.db");
    create_database_at_path(&db_file).expect("fresh install database");
    schema_version_at(&db_file)
}

fn top_up_history(ctx: &Arc<AppContext>) -> Option<std::collections::BTreeMap<u32, u64>> {
    // `det:top_ups:v1` is `context::identity_db::TOP_UPS_KEY`, which is private to
    // that module. A key change surfaces here as a missing entry, not a silent pass.
    ctx.det_kv()
        .expect("per-network k/v")
        .get::<std::collections::BTreeMap<u32, u64>>(
            DetScope::Identity(&IDENTITY_ID),
            "det:top_ups:v1",
        )
        .expect("read top-up history")
}

/// The full upgrade: a v0.9.3 install boots on v1.0 and finds everything where
/// it left it — funds, wallet names, the network it runs on, its queued vote and
/// its top-up history.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v093_install_upgrades_with_wallets_settings_votes_and_history_intact() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let fixture = write_v093_database(tmp.path());
    assert_eq!(
        schema_version(tmp.path()),
        V093_DB_VERSION,
        "precondition: the fixture is a v0.9.3-shaped database",
    );

    let (ctx, settings) = boot(tmp.path());
    let backend = wire_backend(&ctx).await;

    assert!(
        finish_unwire::run(&ctx).await.expect("migration"),
        "a v0.9.3 install has data to move, so the launch must report work done",
    );

    // ── Schema ladder ────────────────────────────────────────────────
    assert_eq!(
        schema_version(tmp.path()),
        fresh_install_schema_version(),
        "the ladder must walk v11 all the way to the current version",
    );

    // ── Settings: the safety-critical field ──────────────────────────
    assert_eq!(
        settings.network,
        Network::Testnet,
        "a v0.9.3 testnet user must not be silently relaunched on mainnet",
    );
    assert_eq!(
        settings.theme_mode,
        ThemeMode::Dark,
        "the chosen theme must survive"
    );
    assert_eq!(
        settings.root_screen_type,
        RootScreenType::RootScreenDPNSScheduledVotes,
        "the start screen must survive",
    );
    assert_eq!(
        settings.dash_qt_path,
        Some(std::path::PathBuf::from("/opt/dash-qt")),
        "the configured Dash-Qt path must survive",
    );
    assert!(
        !settings.overwrite_dash_conf,
        "an explicit `false` must not be flipped back to the default `true`",
    );
    // v0.9.3's `settings` table has NO `onboarding_completed` column and no ladder
    // arm adds one, so it can only fall back to the default — the documented
    // contract of `read_app_settings` for columns an older schema predates.
    assert_eq!(
        settings.onboarding_completed,
        AppSettings::default().onboarding_completed,
        "a column v0.9.3 never had must fall back to its default, not fail the read",
    );

    // ── Wallet seeds: the funds path ─────────────────────────────────
    // The drain copies each legacy envelope into the vault; hydration then
    // promotes the unprotected one to the raw seam (`seed.raw.v1`) and drops the
    // legacy row. So the seed is asserted where it actually ends up — as the raw
    // 64 bytes the wallet is made of.
    let seeds = backend.wallet_seeds();
    let raw_seed = seeds
        .get_raw(&fixture.unprotected)
        .expect("read unprotected seed")
        .expect("the unprotected seed must be readable from the vault after the upgrade");
    assert_eq!(
        raw_seed.as_slice(),
        UNPROTECTED_SEED.as_slice(),
        "the seed bytes are the wallet — they must arrive verbatim",
    );
    assert_eq!(
        seeds.scheme(&fixture.unprotected).expect("scheme"),
        SecretScheme::Unprotected,
        "a wallet the user never password-protected must not gain a password it cannot supply",
    );
    assert!(
        seeds
            .legacy_envelope_get(&fixture.unprotected)
            .expect("read legacy envelope")
            .is_none(),
        "the promoted legacy envelope must be dropped, not left as a second at-rest copy",
    );

    // The protected wallet cannot be promoted — that needs the user's password —
    // so its legacy envelope stays put and must be byte-identical to what v0.9.3
    // wrote. Re-encrypting or truncating it would lock the user out permanently.
    let protected = seeds
        .legacy_envelope_get(&fixture.protected)
        .expect("read protected envelope")
        .expect("the protected seed must reach the vault");
    assert_eq!(
        (
            protected.encrypted_seed,
            protected.salt,
            protected.nonce,
            protected.uses_password,
            protected.password_hint
        ),
        (
            fixture.protected_ciphertext,
            fixture.protected_salt,
            fixture.protected_nonce,
            true,
            Some("the usual".to_string())
        ),
        "the legacy AES-GCM envelope must be copied byte-for-byte",
    );
    assert_eq!(
        seeds.scheme(&fixture.protected).expect("scheme"),
        SecretScheme::Absent,
        "a locked wallet must stay locked — no silent unseal of a protected seed",
    );

    // ── Wallet metadata + registration ───────────────────────────────
    let meta_view = backend.wallet_meta();
    let meta = meta_view
        .get(USER_NETWORK, &fixture.unprotected)
        .expect("the migrated wallet must have a metadata entry");
    assert_eq!(
        meta.alias, "Masternode Owner Wallet",
        "the name the user chose must survive the upgrade",
    );
    assert!(meta.is_main, "the main-wallet flag must survive");
    assert!(!meta.uses_password);
    assert_eq!(
        meta.xpub_encoded,
        legacy_master_epk_bytes(&UNPROTECTED_SEED, USER_NETWORK),
        "the master xpub must survive — the cold-boot picker renders addresses from it",
    );
    assert!(
        backend.is_wallet_registered(&fixture.unprotected),
        "the open wallet must be reachable upstream once the migration completes",
    );

    let protected_meta = meta_view
        .get(USER_NETWORK, &fixture.protected)
        .expect("the protected wallet must have a metadata entry");
    assert_eq!(protected_meta.alias, "Cold Storage");
    assert!(
        protected_meta.uses_password,
        "the protected wallet must stay marked protected, or the unlock prompt never appears",
    );
    assert_eq!(protected_meta.password_hint.as_deref(), Some("the usual"));

    // ── Single keys: a feature v0.9.3 never had ──────────────────────
    // Ladder arm 18 creates the table, so post-upgrade it exists and is empty. The
    // drain must read zero rows and report no error — `run()` returning `Ok` above
    // is that proof; nothing may be conjured into the modern index either.
    assert!(
        backend.single_key().list().is_empty(),
        "a v0.9.3 install has no single keys, so none may appear after the upgrade",
    );

    // ── Scheduled votes ──────────────────────────────────────────────
    let votes = ctx.get_scheduled_votes().expect("read scheduled votes");
    assert_eq!(votes.len(), 1, "the queued vote must come across");
    assert_eq!(votes[0].contested_name, CONTESTED_NAME);
    assert_eq!(votes[0].choice, ResourceVoteChoice::Lock);
    assert_eq!(votes[0].voter_id, Identifier::from(IDENTITY_ID));
    assert!(
        !votes[0].executed_successfully,
        "an uncast vote must not arrive marked as cast — that would skip the vote window",
    );

    // ── Top-up history ───────────────────────────────────────────────
    assert_eq!(
        top_up_history(&ctx),
        Some(std::collections::BTreeMap::from([(0, TOP_UP_AMOUNT)])),
        "the top-up audit trail must come across, scoped by its identity's network",
    );

    // ── Identity ─────────────────────────────────────────────────────
    // The identity row survives the ladder with its wallet link intact. It is the
    // join the top-up import scopes on, and the source a future identity importer
    // reads (see the module note): the ladder must not drop or orphan it.
    let conn = Connection::open(tmp.path().join("data.db")).expect("open data.db");
    let (alias, wallet, wallet_index, identity_type, network): (
        String,
        Vec<u8>,
        u32,
        String,
        String,
    ) = conn
        .query_row(
            "SELECT alias, wallet, wallet_index, identity_type, network
             FROM identity WHERE id = ?1",
            params![IDENTITY_ID.as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .expect("the identity row must survive the ladder");
    assert_eq!(alias, "my-masternode");
    assert_eq!(
        wallet,
        fixture.unprotected.to_vec(),
        "the identity must stay linked to the wallet that owns it",
    );
    assert_eq!(wallet_index, 0);
    assert_eq!(identity_type, "Masternode");
    assert_eq!(
        network, "testnet",
        "a testnet identity must not be swept into mainnet by the v33 network rename",
    );

    backend.shutdown().await;
}

/// The second launch after an upgrade. Every step must short-circuit on its
/// sentinel: no duplicated votes, no resurrected history, no rewritten sentinel,
/// no clobbered preferences — and the legacy rows still in `data.db`, because a
/// migration that deletes its source can never be retried.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn second_launch_after_a_v093_upgrade_changes_nothing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let fixture = write_v093_database(tmp.path());

    let (ctx, _) = boot(tmp.path());
    let backend = wire_backend(&ctx).await;
    finish_unwire::run(&ctx).await.expect("first migration");

    let sentinel_after_first = ctx
        .app_kv()
        .get::<MigrationCompletion>(DetScope::Global, &sentinel_key_for(USER_NETWORK))
        .expect("read sentinel")
        .expect("the first launch must record completion");

    // The user switches to mainnet and casts the queued vote. Both are choices a
    // re-import would undo.
    let app_kv = ctx.app_kv();
    let mut chosen = app_kv
        .get::<AppSettings>(DetScope::Global, AppSettings::KV_KEY)
        .expect("read settings")
        .expect("settings blob");
    chosen.network = Network::Mainnet;
    app_kv
        .put(DetScope::Global, AppSettings::KV_KEY, &chosen)
        .expect("user switches network");
    ctx.clear_all_scheduled_votes()
        .expect("user casts the vote");

    // Second launch: the boot import runs again, then the migration.
    assert_eq!(
        import_legacy_settings(&app_kv, &ctx.db).expect("second settings import"),
        SettingsImport::AlreadyDone,
        "the settings sentinel must stop the import from running twice",
    );
    assert!(
        !finish_unwire::run(&ctx).await.expect("second migration"),
        "a second launch must move no data",
    );

    assert_eq!(
        app_kv
            .get::<AppSettings>(DetScope::Global, AppSettings::KV_KEY)
            .expect("read settings")
            .expect("settings blob")
            .network,
        Network::Mainnet,
        "a re-import must not resurrect the legacy network over the user's choice",
    );
    assert!(
        ctx.get_scheduled_votes().expect("read votes").is_empty(),
        "a re-run must not requeue a vote the user has already cast",
    );
    assert_eq!(
        ctx.app_kv()
            .get::<MigrationCompletion>(DetScope::Global, &sentinel_key_for(USER_NETWORK))
            .expect("read sentinel")
            .expect("sentinel still present"),
        sentinel_after_first,
        "a no-op launch must not rewrite the completion sentinel",
    );
    assert!(
        backend.is_wallet_registered(&fixture.unprotected),
        "the migrated wallet must stay reachable across launches",
    );

    // The migration never deletes its source, so a later build can re-read it.
    let conn = Connection::open(tmp.path().join("data.db")).expect("open data.db");
    let wallets: i64 = conn
        .query_row("SELECT COUNT(*) FROM wallet", [], |r| r.get(0))
        .expect("count wallet rows");
    let votes: i64 = conn
        .query_row("SELECT COUNT(*) FROM scheduled_votes", [], |r| r.get(0))
        .expect("count vote rows");
    assert_eq!(
        (wallets, votes),
        (2, 1),
        "legacy rows must survive untouched"
    );

    backend.shutdown().await;
}
