//! End-to-end upgrade regression from a genuine **v0.9.3** `data.db`.
//!
//! v0.9.3 is the newest released build, so its on-disk shape is what every
//! upgrading user actually hands to v1.0. The three subsystems that carry that
//! data across each have their own unit tests, but each starts from an
//! already-normalised fixture. Nothing proved they **compose** from real v0.9.3
//! raw data, in the order `AppState` actually runs them:
//!
//! 1. [`Database::open_legacy_read_only`] — preserve the v11 source verbatim.
//! 2. [`import_legacy_settings`] — user preferences, at boot, **before** the
//!    active network is chosen (`AppState::new_inner`).
//! 3. [`finish_unwire::run`] — the wallet drain plus the scheduled-vote and
//!    top-up import, on the network step 2 selected.
//!
//! The fixture is the v0.9.3 schema verbatim (`git show v0.9.3:src/database/`),
//! not the modern one: `database_version = 11`, no `single_key_wallet` table,
//! no `core_wallet_name` column, no `onboarding_completed` column, and seeds
//! stored raw with empty salt/nonce.

use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dash_sdk::dpp::identity::v0::IdentityV0;
use dash_sdk::dpp::identity::{
    Identity, IdentityPublicKey, KeyID, KeyType, Purpose, SecurityLevel,
};
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use dash_sdk::dpp::platform_value::BinaryData;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use rusqlite::{Connection, params};

use crate::backend_task::migration::finish_unwire::{
    self, MigrationCompletion, identities_sentinel_key_for, sentinel_key_for,
};
use crate::backend_task::migration::legacy_settings::{SettingsImport, import_legacy_settings};
use crate::context::AppContext;
use crate::database::Database;
use crate::database::test_helpers::{LegacyIdentityFixture, legacy_master_epk_bytes};
use crate::model::qualified_identity::encrypted_key_storage::{
    KeyStorage, PrivateKeyData, WalletDerivationPath,
};
use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use crate::model::qualified_identity::{
    IdentityStatus, IdentityType, PrivateKeyTarget, QualifiedIdentity,
};
use crate::model::settings::{AppSettings, RootScreenType, ThemeMode};
use crate::model::wallet::encryption::encrypt_message;
use crate::model::wallet::{ClosedKeyItem, WalletSeedHash};
use crate::wallet_backend::secret_seam::SecretScheme;
use crate::wallet_backend::{DetScope, IdentityKeyView, WalletBackend};

/// `DEFAULT_DB_VERSION` as shipped by v0.9.3. Every upgrading user's `data.db`
/// enters the ladder here.
const V093_DB_VERSION: u16 = 11;

/// The network the fixture user runs on. The regression this file locks: a
/// v0.9.3 testnet user must not be relaunched on mainnet.
const USER_NETWORK: Network = Network::Testnet;

const UNPROTECTED_SEED: [u8; 64] = [0xA9; 64];
const PROTECTED_SEED: [u8; 64] = [0xC7; 64];
const PROTECTED_PASSWORD: &str = "correct horse battery staple";
const TOP_UP_AMOUNT: u64 = 123_456;
const CONTESTED_NAME: &str = "quantum";

/// Row A — the masternode identity whose owner + voting keys are the whole
/// point of the identity import. Its `data` blob is [`V093_MASTERNODE_BLOB_HEX`].
const IDENTITY_ID: [u8; 32] = [0xBB; 32];
/// The voter identity associated with row A (`associated_voter_identity`).
const VOTER_IDENTITY_ID: [u8; 32] = [0xBC; 32];
/// Row B — a wallet-derived `User` identity on the unprotected wallet.
const USER_IDENTITY_ID: [u8; 32] = [0xCC; 32];
/// Row C — an `Evonode` identity with no wallet at all (loaded by ProTxHash).
const EVONODE_IDENTITY_ID: [u8; 32] = [0xDD; 32];
/// Row D — an observed (`is_local = 0`) identity: lookup cache, not user data.
const OBSERVED_IDENTITY_ID: [u8; 32] = [0xEE; 32];
/// Row E — a `User` identity on the password-protected (locked) wallet.
const PROTECTED_IDENTITY_ID: [u8; 32] = [0xCD; 32];
/// Row F — a local row whose `data` blob is NULL (v0.9.3 allowed it).
const NULL_BLOB_IDENTITY_ID: [u8; 32] = [0xEF; 32];

/// The masternode owner key held `Clear` in row A's legacy blob. After the
/// import it must exist only in the vault — never in `det-app.sqlite`.
const OWNER_PRIVATE_KEY: [u8; 32] = [0x11; 32];
/// The masternode voting key held `Clear` in row A's legacy blob.
const VOTING_PRIVATE_KEY: [u8; 32] = [0x22; 32];
/// The `Clear` key held by the wallet-less evonode identity (row C).
const EVONODE_PRIVATE_KEY: [u8; 32] = [0x33; 32];

/// Legacy `identity.status` column values. The bincode blob does **not** carry
/// status, so these are what prove the column is restored on import rather than
/// every identity reading back as `Unknown`.
const STATUS_ACTIVE: u8 = 2;
const STATUS_PENDING_CREATION: u8 = 1;
const STATUS_NOT_FOUND: u8 = 3;

/// A **genuine v0.9.3** `QualifiedIdentity::to_bytes()` — the masternode
/// identity of row A, carrying `Clear` owner and voting keys.
///
/// This is the wire-format contract between the two builds. v0.9.3 encodes with
/// bincode `2.0.0-rc.3`; this tree decodes with `2.0.1`. Everything else about
/// the import is reasoning about struct layout — this constant is the only proof
/// that the actual bytes a real user has on disk still decode here.
///
/// To regenerate: build a throwaway crate (not a workspace member) depending on
/// `dash-evo-tool` tag `v0.9.3`, construct the identity below — id
/// [`IDENTITY_ID`], voter id [`VOTER_IDENTITY_ID`], type `Masternode`, alias
/// `my-masternode`, `associated_owner_key_id = Some(0)`, an `OWNER` key (id 0,
/// `Clear`([`OWNER_PRIVATE_KEY`]), `ECDSA_HASH160`) on
/// `PrivateKeyOnMainIdentity` and a `VOTING` key (id 1,
/// `Clear`([`VOTING_PRIVATE_KEY`]), `ECDSA_HASH160`) on
/// `PrivateKeyOnVoterIdentity` — and print `hex::encode(qi.to_bytes())`.
const V093_MASTERNODE_BLOB_HEX: &str = "00bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb02000000060000020014fc7250a211deddc70ee5a2738de5f07817351cef00010001050000020014531260aa2a199e228c537dfa42c82bea2c7c1f4d00fc40420f00010100bcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbc01010001050000020014531260aa2a199e228c537dfa42c82bea2c7c1f4d00fc40420f00010001050000020014531260aa2a199e228c537dfa42c82bea2c7c1f4d0000010001010d6d792d6d61737465726e6f64650200000000060000020014fc7250a211deddc70ee5a2738de5f07817351cef000001111111111111111111111111111111111111111111111111111111111111111101010001050000020014531260aa2a199e228c537dfa42c82bea2c7c1f4d000001222222222222222222222222222222222222222222222222222222222222222200";

/// The legacy blob of row A, exactly as a v0.9.3 install holds it on disk.
fn v093_masternode_blob() -> Vec<u8> {
    hex::decode(V093_MASTERNODE_BLOB_HEX).expect("the golden blob is valid hex")
}

fn identity_public_key(id: KeyID, purpose: Purpose) -> IdentityPublicKey {
    IdentityPublicKey::V0(IdentityPublicKeyV0 {
        id,
        purpose,
        security_level: SecurityLevel::MASTER,
        contract_bounds: None,
        key_type: KeyType::ECDSA_HASH160,
        read_only: false,
        // 20 bytes: the shape of a hash160 key. The import never checks a key
        // against its public data, so a stand-in value is enough here — row A's
        // keys, the ones that matter, come from the real v0.9.3 blob instead.
        data: BinaryData::new(vec![id as u8; 20]),
        disabled_at: None,
    })
}

/// Encode a `QualifiedIdentity` the way v0.9.3 did — same manual `Encode`, same
/// bincode config. Used for the fixture rows whose exact bytes do not matter;
/// row A instead carries the real v0.9.3 blob, which is what pins the wire format.
fn legacy_identity_blob(
    id: [u8; 32],
    identity_type: IdentityType,
    alias: &str,
    keys: Vec<(PrivateKeyTarget, KeyID, Purpose, PrivateKeyData)>,
) -> Vec<u8> {
    let mut private_keys = BTreeMap::new();
    let mut public_keys = BTreeMap::new();
    for (target, key_id, purpose, data) in keys {
        let public_key = identity_public_key(key_id, purpose);
        public_keys.insert(key_id, public_key.clone());
        private_keys.insert(
            (target, key_id),
            (QualifiedIdentityPublicKey::from(public_key), data),
        );
    }

    QualifiedIdentity {
        identity: Identity::V0(IdentityV0 {
            id: Identifier::from(id),
            public_keys,
            balance: 1_000_000,
            revision: 1,
        }),
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type,
        alias: Some(alias.to_string()),
        private_keys: KeyStorage::from(private_keys),
        dpns_names: vec![],
        associated_wallets: BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: BTreeMap::new(),
        // Never encoded — the legacy `status` column is the only source, which is
        // exactly what the import has to restore.
        status: IdentityStatus::Unknown,
        network: USER_NETWORK,
    }
    .to_bytes()
}

/// A key the wallet derives on demand — no secret bytes in the blob at all.
fn wallet_derived_key(seed_hash: WalletSeedHash) -> PrivateKeyData {
    PrivateKeyData::AtWalletDerivationPath(WalletDerivationPath {
        wallet_seed_hash: seed_hash,
        derivation_path: DerivationPath::from_str("m/9'/1'/5'/0'/0'").expect("derivation path"),
    })
}

/// The seed hashes of the two migrated wallets.
struct Fixture {
    unprotected: WalletSeedHash,
    protected: WalletSeedHash,
}

/// Insert one row into the legacy `identity` table, in v0.9.3's column shape.
#[allow(clippy::too_many_arguments)]
fn insert_identity(
    conn: &Connection,
    id: [u8; 32],
    data: Option<Vec<u8>>,
    status: IdentityStatus,
    is_local: bool,
    alias: &str,
    wallet: Option<(WalletSeedHash, u32)>,
    identity_type: &str,
) {
    let mut row = LegacyIdentityFixture::new(id, data, USER_NETWORK.to_string())
        .with_status(status)
        .with_is_local(is_local)
        .with_alias(alias)
        .with_identity_type(identity_type);
    if let Some((seed_hash, index)) = wallet {
        row = row.with_wallet(seed_hash.to_vec(), index);
    }
    row.insert(conn).expect("insert identity row");
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

    // Row A — the masternode identity owned by the unprotected wallet, holding
    // the user's owner + voting keys `Clear`. `data` is a REAL v0.9.3 blob, so
    // this row also pins the cross-version bincode wire format.
    insert_identity(
        &conn,
        IDENTITY_ID,
        Some(v093_masternode_blob()),
        IdentityStatus::Active,
        true,
        "my-masternode",
        Some((unprotected, 0)),
        "Masternode",
    );

    // Row B — a `User` identity whose key is wallet-derived: nothing secret in
    // the blob, but the wallet link must survive or the key cannot be derived.
    insert_identity(
        &conn,
        USER_IDENTITY_ID,
        Some(legacy_identity_blob(
            USER_IDENTITY_ID,
            IdentityType::User,
            "my-username",
            vec![(
                PrivateKeyTarget::PrivateKeyOnMainIdentity,
                0,
                Purpose::AUTHENTICATION,
                wallet_derived_key(unprotected),
            )],
        )),
        IdentityStatus::PendingCreation,
        true,
        "my-username",
        Some((unprotected, 1)),
        "User",
    );

    // Row C — an evonode identity with no wallet at all (loaded by ProTxHash),
    // holding a `Clear` key. A wallet-less identity must still import.
    insert_identity(
        &conn,
        EVONODE_IDENTITY_ID,
        Some(legacy_identity_blob(
            EVONODE_IDENTITY_ID,
            IdentityType::Evonode,
            "my-evonode",
            vec![(
                PrivateKeyTarget::PrivateKeyOnMainIdentity,
                0,
                Purpose::OWNER,
                PrivateKeyData::Clear(EVONODE_PRIVATE_KEY),
            )],
        )),
        IdentityStatus::NotFound,
        true,
        "my-evonode",
        None,
        "Evonode",
    );

    // Row D — an observed identity: v0.9.3's lookup cache, not the user's own.
    // Importing it would put a stranger's identity on the Identities screen.
    insert_identity(
        &conn,
        OBSERVED_IDENTITY_ID,
        Some(legacy_identity_blob(
            OBSERVED_IDENTITY_ID,
            IdentityType::User,
            "someone-else",
            vec![],
        )),
        IdentityStatus::Active,
        false,
        "someone-else",
        None,
        "User",
    );

    // Row E — a `User` identity on the password-protected wallet. That wallet is
    // still locked after the drain, so this is the row that proves a locked
    // wallet does not cost the user the identity, nor its link to that wallet.
    insert_identity(
        &conn,
        PROTECTED_IDENTITY_ID,
        Some(legacy_identity_blob(
            PROTECTED_IDENTITY_ID,
            IdentityType::User,
            "cold-username",
            vec![(
                PrivateKeyTarget::PrivateKeyOnMainIdentity,
                0,
                Purpose::AUTHENTICATION,
                wallet_derived_key(protected),
            )],
        )),
        IdentityStatus::Active,
        true,
        "cold-username",
        Some((protected, 0)),
        "User",
    );

    // Row F — v0.9.3 allowed a local row with no blob. Nothing to import; it must
    // be skipped silently, not counted as a failure.
    insert_identity(
        &conn,
        NULL_BLOB_IDENTITY_ID,
        None,
        IdentityStatus::Active,
        true,
        "no-data",
        None,
        "User",
    );

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
    }
}

/// Boot over `dir` exactly as `AppState` does: open the pre-update database
/// read-only, import its preferences, then build the `AppContext` **on the
/// network those preferences named**. Returns the context and imported settings.
///
/// Taking the network from the import (rather than hard-coding testnet) is the
/// point: it is what makes this a composition test. If the import lost the
/// network, every downstream `WHERE network = ?1` filter in the wallet drain
/// would silently target mainnet and find nothing.
fn boot(dir: &std::path::Path) -> (Arc<AppContext>, AppSettings) {
    crate::app_dir::ensure_env_file(dir);
    let db_file = dir.join("data.db");

    let db = Arc::new(Database::open_legacy_read_only(&db_file).expect("open data.db read-only"));

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

/// Run the migration while acting as its blocking password-entry UI.
async fn run_migration_with_wallet_passwords(
    ctx: &Arc<AppContext>,
) -> Result<bool, crate::backend_task::error::TaskError> {
    ctx.install_secret_prompt(Arc::new(
        crate::wallet_backend::secret_prompt::test_support::TestPrompt::never(),
    ));
    let migration_context = Arc::clone(ctx);
    let migration = tokio::spawn(async move { finish_unwire::run(&migration_context).await });
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);

    while !migration.is_finished() {
        if let crate::context::migration_status::MigrationState::AwaitingWalletPasswords {
            wallets,
        } = ctx.migration_status().state().as_ref()
        {
            for seed_hash in wallets {
                let wallet = ctx.wallet_arc(seed_hash).expect("migrated wallet");
                let needs_password = {
                    let wallet = wallet.read().expect("wallet lock");
                    wallet.requires_password_unlock()
                };
                if needs_password {
                    wallet
                        .write()
                        .expect("wallet lock")
                        .wallet_seed
                        .open(PROTECTED_PASSWORD)
                        .expect("fixture password opens protected wallet");
                    ctx.handle_wallet_unlocked(
                        &wallet,
                        PROTECTED_PASSWORD,
                        crate::context::WalletUnlockRetention::UntilAppClose,
                    )
                    .expect("the protected seed must land in the current vault");
                    ctx.migration_status().notify_wallet_password_submitted();
                }
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "migration must finish after every fixture password is submitted",
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    migration.await.expect("migration task must not panic")
}

fn schema_version_at(db_file: &std::path::Path) -> u16 {
    Connection::open_with_flags(db_file, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
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

/// The persisted shape of an identity entry, mirroring the private
/// `context::identity_db::StoredQualifiedIdentity`. Lets the test read what
/// actually landed **on disk** rather than trusting the in-memory struct — the
/// only way to prove no plaintext key survived the import. Field order is the
/// bincode contract; a drift in the real struct surfaces here as a decode error.
#[derive(serde::Deserialize)]
struct StoredIdentityOnDisk {
    qi_bytes: Vec<u8>,
    status: u8,
    identity_type: String,
    wallet_hash: Option<[u8; 32]>,
    wallet_index: Option<u32>,
}

/// Read the raw stored entry for one identity out of the per-network k/v store.
fn stored_identity(ctx: &Arc<AppContext>, id: [u8; 32]) -> StoredIdentityOnDisk {
    ctx.det_kv()
        .expect("per-network k/v")
        .get::<StoredIdentityOnDisk>(DetScope::Identity(&id), "det:identity:v1")
        .expect("read stored identity")
        .expect("the identity must be stored after the import")
}

/// Every private key in a stored blob, as it sits on disk.
fn stored_key_data(stored: &StoredIdentityOnDisk) -> Vec<PrivateKeyData> {
    QualifiedIdentity::from_bytes(&stored.qi_bytes)
        .expect("the stored blob must decode")
        .private_keys
        .private_keys
        .values()
        .map(|(_, data)| data.clone())
        .collect()
}

/// `true` if `needle` appears anywhere in `haystack`.
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
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
    let legacy_before = std::fs::read(tmp.path().join("data.db"))
        .expect("snapshot the v0.9.3 database before boot");

    let (ctx, settings) = boot(tmp.path());
    let backend = wire_backend(&ctx).await;

    assert!(
        run_migration_with_wallet_passwords(&ctx)
            .await
            .expect("migration"),
        "a v0.9.3 install has data to move, so the launch must report work done",
    );

    // ── Read-only legacy source ─────────────────────────────────────
    assert_eq!(
        schema_version(tmp.path()),
        V093_DB_VERSION,
        "the storage update must not change the legacy schema version",
    );
    assert_eq!(
        std::fs::read(tmp.path().join("data.db"))
            .expect("snapshot the v0.9.3 database after the storage update"),
        legacy_before,
        "boot and the complete storage update must leave data.db byte-unchanged",
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
    // promotes the unprotected one to the raw seam (`seed.raw.v1`) and collects
    // the redundant copy. So the seed is asserted where it is used — as the raw
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
        "the unprotected seed must have exactly one vault copy after migration",
    );

    // The password gate opens the protected wallet through the normal unlock
    // path, which re-encrypts the seed into the current Tier-2 envelope and
    // removes the redundant legacy copy.
    assert!(
        seeds
            .legacy_envelope_get(&fixture.protected)
            .expect("read protected envelope")
            .is_none(),
        "the protected seed must have exactly one vault copy after migration",
    );
    assert_eq!(
        seeds.scheme(&fixture.protected).expect("scheme"),
        SecretScheme::Protected,
        "the migrated seed must stay password-protected under the current envelope",
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

    // ── Identity: the legacy row survives the ladder ─────────────────
    // The precondition the import consumes: the ladder must not drop or orphan
    // the row it reads from.
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

    // ── Identity import: the user's identities and their keys ────────
    let identities = ctx
        .load_local_qualified_identities()
        .expect("load imported identities");
    let ids: Vec<[u8; 32]> = identities
        .iter()
        .map(|qi| qi.identity.id().to_buffer())
        .collect();
    // Sorted by identity id — the documented order of `load_identities_filtered`.
    assert_eq!(
        ids,
        vec![
            IDENTITY_ID,
            USER_IDENTITY_ID,
            PROTECTED_IDENTITY_ID,
            EVONODE_IDENTITY_ID
        ],
        "every local identity must come across — and only those: an observed \
         (is_local = 0) row is a lookup cache, and a NULL-blob row has nothing to import",
    );

    let masternode = identities
        .iter()
        .find(|qi| qi.identity.id().to_buffer() == IDENTITY_ID)
        .expect("the masternode identity must be imported");
    assert_eq!(masternode.alias.as_deref(), Some("my-masternode"));
    assert_eq!(masternode.identity_type, IdentityType::Masternode);
    assert_eq!(masternode.wallet_index, Some(0));
    assert_eq!(
        masternode.status,
        IdentityStatus::Active,
        "status lives in a column, not the blob — dropping it would relabel every \
         imported identity as `Unknown, refresh required`",
    );

    // The whole point of the import: the masternode owner still controls the node.
    let presence = masternode.masternode_key_presence();
    assert!(
        presence.owner && presence.voting,
        "the owner and voting keys the user had loaded must come across, not be lost \
         to a manual re-import: got {presence:?}",
    );

    // Type-filtered views — the Masternodes and Identities screens read these.
    let masternode_ids: Vec<[u8; 32]> = ctx
        .load_local_masternode_identities()
        .expect("load masternode identities")
        .iter()
        .map(|qi| qi.identity.id().to_buffer())
        .collect();
    assert_eq!(
        masternode_ids,
        vec![IDENTITY_ID, EVONODE_IDENTITY_ID],
        "the Masternodes screen must show the masternode and the evonode",
    );
    let user_ids: Vec<[u8; 32]> = ctx
        .load_local_user_identities()
        .expect("load user identities")
        .iter()
        .map(|qi| qi.identity.id().to_buffer())
        .collect();
    assert_eq!(
        user_ids,
        vec![USER_IDENTITY_ID, PROTECTED_IDENTITY_ID],
        "the Identities screen must show both user identities",
    );

    // ── Wallet links survive, including to a still-locked wallet ─────
    let stored_masternode = stored_identity(&ctx, IDENTITY_ID);
    let stored_protected = stored_identity(&ctx, PROTECTED_IDENTITY_ID);
    assert_eq!(
        (stored_protected.wallet_hash, stored_protected.wallet_index),
        (Some(fixture.protected), Some(0)),
        "an identity on a locked wallet keeps its link — that link is what re-attaches \
         it when the user unlocks the wallet",
    );
    assert_eq!(
        (
            stored_masternode.wallet_hash,
            stored_masternode.wallet_index
        ),
        (Some(fixture.unprotected), Some(0)),
    );
    assert_eq!(
        stored_masternode.identity_type, "Masternode",
        "the type tag backs the filtered loads without a full blob decode",
    );
    let stored_evonode = stored_identity(&ctx, EVONODE_IDENTITY_ID);
    assert_eq!(
        (stored_evonode.wallet_hash, stored_evonode.wallet_index),
        (None, None),
        "a wallet-less identity must not acquire a wallet link",
    );
    assert_eq!(
        stored_evonode.status, STATUS_NOT_FOUND,
        "each identity keeps its own status",
    );
    assert_eq!(
        stored_identity(&ctx, USER_IDENTITY_ID).status,
        STATUS_PENDING_CREATION,
    );
    assert_eq!(
        stored_masternode.status, STATUS_ACTIVE,
        "the status column reaches disk as the same discriminant v0.9.3 stored",
    );

    // The top-up history already imported under the same identity scope must
    // still resolve — the two entries share a scope and must not collide.
    assert_eq!(
        top_up_history(&ctx),
        Some(std::collections::BTreeMap::from([(0, TOP_UP_AMOUNT)])),
        "the identity blob must not displace the top-up entry in the same scope",
    );

    backend.shutdown().await;
}

/// The cross-version wire contract, in isolation: a `QualifiedIdentity` encoded
/// by the **real v0.9.3 binary** (bincode `2.0.0-rc.3`) still decodes on this
/// tree (bincode `2.0.1`).
///
/// Everything else about the identity import is reasoning about struct layout.
/// This is the one test that reads bytes a real user actually has on disk. If
/// bincode's wire format had drifted between the two versions, this fails — and
/// the import would be dropping masternode keys on the floor.
#[test]
fn a_real_v093_identity_blob_still_decodes() {
    let qi = QualifiedIdentity::from_bytes(&v093_masternode_blob())
        .expect("a genuine v0.9.3 identity blob must decode on the current bincode");

    assert_eq!(qi.identity.id().to_buffer(), IDENTITY_ID);
    assert_eq!(qi.identity_type, IdentityType::Masternode);
    assert_eq!(qi.alias.as_deref(), Some("my-masternode"));
    assert_eq!(qi.associated_owner_key_id, Some(0));
    assert_eq!(
        qi.associated_voter_identity
            .as_ref()
            .map(|(identity, _)| identity.id().to_buffer()),
        Some(VOTER_IDENTITY_ID),
        "the associated voter identity must survive the decode",
    );

    // The keys are the payload. v0.9.3 held them `Clear`, and they must decode
    // to exactly the bytes it wrote — a shifted field or a changed varint would
    // corrupt them silently.
    let keys = &qi.private_keys.private_keys;
    assert_eq!(keys.len(), 2, "both private keys must decode");
    assert_eq!(
        keys.get(&(PrivateKeyTarget::PrivateKeyOnMainIdentity, 0))
            .map(|(_, data)| data.clone()),
        Some(PrivateKeyData::Clear(OWNER_PRIVATE_KEY)),
        "the masternode owner key must decode byte-for-byte",
    );
    assert_eq!(
        keys.get(&(PrivateKeyTarget::PrivateKeyOnVoterIdentity, 1))
            .map(|(_, data)| data.clone()),
        Some(PrivateKeyData::Clear(VOTING_PRIVATE_KEY)),
        "the masternode voting key must decode byte-for-byte",
    );

    // Status is not in the blob — the legacy column is its only source. This is
    // the default the import has to overwrite.
    assert_eq!(
        qi.status,
        IdentityStatus::Unknown,
        "status is not encoded; a decoded blob must come back `Unknown` so the \
         importer's column restore is load-bearing",
    );
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
    run_migration_with_wallet_passwords(&ctx)
        .await
        .expect("first migration");

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

    // …and renames the imported masternode identity. The identity sentinel is what
    // must protect the edit; the same guarantee under an undecodable row is covered
    // by `a_second_launch_after_an_unreadable_identity_preserves_user_edits_and_deletions`.
    ctx.set_identity_alias(&Identifier::from(IDENTITY_ID), Some("my-renamed-node"))
        .expect("user renames the identity");

    // Second launch: the boot import runs again, then the migration.
    assert_eq!(
        import_legacy_settings(&app_kv, &ctx.db).expect("second settings import"),
        SettingsImport::AlreadyDone,
        "the settings sentinel must stop the import from running twice",
    );
    assert!(
        !run_migration_with_wallet_passwords(&ctx)
            .await
            .expect("second migration"),
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

    // The skip-if-present rule: a re-run must not push the legacy blob back over
    // an identity the user has since edited.
    assert_eq!(
        ctx.get_identity_alias(&Identifier::from(IDENTITY_ID))
            .expect("read alias")
            .as_deref(),
        Some("my-renamed-node"),
        "a re-run must not overwrite the user's edit with the stale legacy identity",
    );
    assert_eq!(
        ctx.load_local_qualified_identities()
            .expect("load identities")
            .len(),
        4,
        "a second launch must not duplicate or drop an identity",
    );

    // The migration never deletes its source, so a later build can re-read it.
    let conn = Connection::open(tmp.path().join("data.db")).expect("open data.db");
    let wallets: i64 = conn
        .query_row("SELECT COUNT(*) FROM wallet", [], |r| r.get(0))
        .expect("count wallet rows");
    let votes: i64 = conn
        .query_row("SELECT COUNT(*) FROM scheduled_votes", [], |r| r.get(0))
        .expect("count vote rows");
    let identities: i64 = conn
        .query_row("SELECT COUNT(*) FROM identity", [], |r| r.get(0))
        .expect("count identity rows");
    assert_eq!(
        (wallets, votes, identities),
        (2, 1, 6),
        "legacy rows must survive untouched"
    );

    backend.shutdown().await;
}

/// The security contract: the import itself must never write a plaintext private
/// key to disk.
///
/// v0.9.3 stored masternode owner / voting keys as `Clear` — plaintext, inside
/// the identity blob. The import has to route them through the vault seam, so
/// the at-rest blob keeps only `InVault` placeholders.
///
/// This reads the stored bytes **immediately after the migration, before any
/// load path runs**, and that ordering is the whole point: reading them after a
/// `load_local_qualified_identities()` would prove nothing, because the eager
/// load-path repair (`migrate_identity_keys_to_vault`) vaults resident plaintext
/// on read and would mask an importer that had written it. "Repaired on next
/// read" is not a security property — the bytes must never hit the disk at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_import_never_writes_a_plaintext_key_to_disk() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_v093_database(tmp.path());

    let (ctx, _) = boot(tmp.path());
    let backend = wire_backend(&ctx).await;
    run_migration_with_wallet_passwords(&ctx)
        .await
        .expect("migration");

    // Deliberately no `load_*` call before these reads.
    for (id, label) in [
        (IDENTITY_ID, "the masternode's owner + voting keys"),
        (EVONODE_IDENTITY_ID, "the wallet-less evonode's key"),
    ] {
        let stored = stored_identity(&ctx, id);
        let key_data = stored_key_data(&stored);
        assert!(
            !key_data.is_empty(),
            "{label}: the keys must be stored, not dropped",
        );
        assert!(
            key_data
                .iter()
                .all(|data| matches!(data, PrivateKeyData::InVault)),
            "{label}: a `Clear`/`AlwaysClear` key must never survive the import to disk — \
             it must be vaulted: got {key_data:?}",
        );
        for key in [OWNER_PRIVATE_KEY, VOTING_PRIVATE_KEY, EVONODE_PRIVATE_KEY] {
            assert!(
                !contains_bytes(&stored.qi_bytes, &key),
                "{label}: raw private-key bytes must appear nowhere in the stored blob",
            );
        }
    }

    // Vaulted, not destroyed: the keys are readable back, byte-for-byte, so the
    // masternode owner can still sign. Silently losing them would be as bad as
    // leaking them.
    let secret_store = ctx.secret_store();
    let vault = IdentityKeyView::new(&secret_store, IDENTITY_ID);
    assert_eq!(
        vault
            .get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 0)
            .expect("read owner key from the vault")
            .expect("the owner key must be in the vault")
            .as_slice(),
        OWNER_PRIVATE_KEY.as_slice(),
        "the owner key must arrive in the vault intact",
    );
    assert_eq!(
        vault
            .get(&PrivateKeyTarget::PrivateKeyOnVoterIdentity, 1)
            .expect("read voting key from the vault")
            .expect("the voting key must be in the vault")
            .as_slice(),
        VOTING_PRIVATE_KEY.as_slice(),
        "the voting key must arrive in the vault intact",
    );

    backend.shutdown().await;
}

/// A corrupt vote queue must never cost the user their identity keys.
///
/// The app-data pass (scheduled votes, top-up history) can fail hard — one
/// malformed `det:scheduled_vote_voters:v1` blob is enough. That failure is
/// deterministic: it recurs on every launch, and the app-data sentinel is never
/// written. So if the identity import waited on the app-data result, a masternode
/// owner's owner/voting keys would never reach the vault — not on this launch,
/// not on any retry — because of a broken vote queue they cannot even see.
///
/// The two DET-owned passes are independent; neither may gate the other.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_corrupt_vote_index_never_strands_the_identity_keys() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_v093_database(tmp.path());

    let (ctx, _) = boot(tmp.path());
    let backend = wire_backend(&ctx).await;

    // Poison the scheduled-vote roster with a blob that cannot decode into the
    // voter list, so the app-data pass fails hard the way a corrupted entry would.
    // (`det:scheduled_vote_voters:v1` is private to `context::identity_db`; a key
    // rename surfaces here as an un-poisoned index, not a silent pass.)
    ctx.det_kv()
        .expect("per-network k/v")
        .put(
            DetScope::Global,
            "det:scheduled_vote_voters:v1",
            &vec![0xFFu8; 3],
        )
        .expect("poison the vote index");

    // The failure still reaches the user — it is not swallowed…
    assert!(
        run_migration_with_wallet_passwords(&ctx).await.is_err(),
        "a hard app-data failure must still surface, so the user gets a retry",
    );

    // …but it must not have taken the identities down with it.
    assert_eq!(
        ctx.load_local_qualified_identities()
            .expect("load identities")
            .len(),
        4,
        "a corrupt vote queue must not strand the user's identities",
    );

    let secret_store = ctx.secret_store();
    let vault = IdentityKeyView::new(&secret_store, IDENTITY_ID);
    assert_eq!(
        vault
            .get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 0)
            .expect("read owner key from the vault")
            .expect("the owner key must be in the vault")
            .as_slice(),
        OWNER_PRIVATE_KEY.as_slice(),
        "the masternode owner key must reach the vault even when the vote import fails — \
         otherwise a broken vote queue permanently costs the user control of their node",
    );

    backend.shutdown().await;
}

/// A corrupt legacy row must not make the import re-run forever, on a real
/// v0.9.3 database.
///
/// The import used to withhold its sentinel while any row was undecodable, so it
/// ran again on **every** launch. Skip-if-present made that harmless for an
/// identity the user had *edited* — and did nothing for one the user had
/// *deleted*: the next launch re-imported it, restored the alias the user had
/// cleared and re-wrote its legacy plaintext keys into the vault, forever. The
/// import now completes even with an undecodable row (the row is reported by a
/// durable warning instead), so a second launch changes nothing the user did.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_launch_after_an_unreadable_identity_preserves_user_edits_and_deletions() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_v093_database(tmp.path());

    // One corrupt blob used to be enough to re-run the whole pass on every launch.
    let corrupt_id = [0x0C; 32];
    let conn = Connection::open(tmp.path().join("data.db")).expect("open data.db");
    insert_identity(
        &conn,
        corrupt_id,
        Some(vec![0xFF; 16]),
        IdentityStatus::Active,
        true,
        "corrupt",
        None,
        "User",
    );
    drop(conn);

    let (ctx, _) = boot(tmp.path());
    let backend = wire_backend(&ctx).await;
    run_migration_with_wallet_passwords(&ctx)
        .await
        .expect("first migration");

    // The readable identities landed regardless of the corrupt row…
    assert_eq!(
        ctx.load_local_qualified_identities()
            .expect("load identities")
            .len(),
        4,
        "an undecodable blob must not block the identities that do decode",
    );
    // …the user is told…
    assert_eq!(
        *ctx.migration_status().state(),
        crate::context::migration_status::MigrationState::SucceededWithUnreadableData {
            identities: 1,
            votes: 0,
            top_ups: 0,
        },
        "the user must learn that an identity did not come across",
    );
    // …and the import is nonetheless COMPLETE: the corrupt row rides on the
    // durable warning, not on an import that re-runs until it decodes.
    assert!(
        ctx.app_kv()
            .get::<MigrationCompletion>(
                DetScope::Global,
                &identities_sentinel_key_for(USER_NETWORK)
            )
            .expect("read identity sentinel")
            .is_some(),
        "an unreadable row must not withhold the sentinel: a re-running import resurrects \
         identities the user has deleted",
    );

    // Between launches the user renames the imported masternode and deletes one
    // identity outright — the security gesture whose whole point is that those
    // keys stop living in this install.
    ctx.set_identity_alias(&Identifier::from(IDENTITY_ID), Some("my-renamed-node"))
        .expect("rename identity");
    // The first migration's best-effort DAPI refresh is detached onto a spawned
    // task that queues for `migration_run` right behind it (see
    // `finish_unwire::spawn_dapi_refresh`). `delete_local_qualified_identity`
    // claims that same guard via `try_lock`, so calling it immediately here
    // races the detached refresh — flaky under load, not a real contention
    // failure. Wait for the refresh to release the guard first.
    finish_unwire::wait_for_dapi_refresh(&ctx).await;
    let deleted = Identifier::from(USER_IDENTITY_ID);
    ctx.delete_local_qualified_identity(&deleted)
        .expect("delete identity");

    // Second launch.
    run_migration_with_wallet_passwords(&ctx)
        .await
        .expect("second migration");

    assert_eq!(
        ctx.get_identity_alias(&Identifier::from(IDENTITY_ID))
            .expect("read alias")
            .as_deref(),
        Some("my-renamed-node"),
        "a second launch must not overwrite the user's edit with the stale legacy blob",
    );
    assert!(
        !ctx.has_local_qualified_identity(&deleted)
            .expect("read identity store"),
        "a deleted identity must STAY deleted: re-importing it would restore the alias the \
         user cleared and re-write their legacy plaintext keys into the vault, on every \
         launch, with no way to stop it",
    );
    assert_eq!(
        ctx.load_local_qualified_identities()
            .expect("load identities")
            .len(),
        3,
        "the second launch must neither duplicate the identities it imported nor resurrect \
         the one the user removed",
    );

    backend.shutdown().await;
}

/// The identity import carries its own sentinel. Reusing the wallet drain's
/// would silently skip the import for every install that already drained its
/// wallets under a build that had no identity importer — i.e. exactly the
/// installs this feature exists for.
#[test]
fn the_identity_sentinel_is_per_network_and_distinct_from_the_wallet_sentinel() {
    let testnet = identities_sentinel_key_for(USER_NETWORK);
    assert_ne!(testnet, identities_sentinel_key_for(Network::Mainnet));
    assert_ne!(testnet, sentinel_key_for(USER_NETWORK));
}
