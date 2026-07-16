//! Test helper utilities for database testing.
//!
//! This module provides utilities for creating test databases that can be used
//! in unit and integration tests throughout the codebase.

use crate::database::Database;
use crate::model::qualified_identity::IdentityStatus;
use dash_sdk::dpp::dashcore::Network;
use rusqlite::Connection;
use tempfile::TempDir;

/// Creates an in-memory SQLite database for testing.
///
/// This is the fastest option for tests that don't need to persist data
/// or test file-based functionality.
///
/// # Example
/// ```
/// use dash_evo_tool::database::test_helpers::create_test_database;
///
/// let db = create_test_database().unwrap();
/// // Use db for testing...
/// ```
pub fn create_test_database() -> rusqlite::Result<Database> {
    let db = Database::new(":memory:")?;
    // Force-create the legacy wallet-family schema so domain tests
    // (`database::utxo`, `database::wallet`, …) keep working after
    // T-DEV-01 gated those tables out of fresh installs. We bypass
    // `initialize` because that path now skips them for truly-fresh DBs.
    db.create_tables(true)?;
    db.set_default_version()?;
    Ok(db)
}

/// Creates a file-based temporary database for testing.
///
/// Use this when you need to test file-based operations like:
/// - Database migrations
/// - Backup functionality
/// - Persistence across connections
///
/// The returned `TempDir` must be kept alive for the duration of the test,
/// as dropping it will delete the temporary directory.
///
/// # Example
/// ```
/// use dash_evo_tool::database::test_helpers::create_temp_database;
///
/// let (db, _temp_dir) = create_temp_database().unwrap();
/// // Use db for testing...
/// // _temp_dir is dropped at the end, cleaning up the test files
/// ```
pub fn create_temp_database() -> rusqlite::Result<(Database, TempDir)> {
    let temp_dir = tempfile::tempdir().map_err(|e| {
        rusqlite::Error::ToSqlConversionFailure(format!("Failed to create temp dir: {}", e).into())
    })?;
    let db_path = temp_dir.path().join("test_data.db");
    let db = Database::new(&db_path)?;
    // Same rationale as `create_test_database`: force the full legacy
    // schema so file-backed tests still see wallet-family tables.
    db.create_tables(true)?;
    db.set_default_version()?;
    Ok((db, temp_dir))
}

/// Creates a test database with a specific file path.
///
/// Useful when you need to control the exact location of the database file.
pub fn create_database_at_path(path: &std::path::Path) -> rusqlite::Result<Database> {
    let db = Database::new(path)?;
    db.create_tables(true)?;
    db.set_default_version()?;
    Ok(db)
}

/// Insert an unprotected HD wallet row into the legacy `wallet` table, the
/// pre-PR-#860 on-disk shape the `FinishUnwire` migration drains. Lets tests
/// (including the network e2e suite, which cannot depend on `rusqlite`) stage
/// a "migrated-on-disk" wallet without raw SQL.
///
/// `encrypted_seed` carries the verbatim 64-byte seed (salt/nonce stay empty
/// for an unprotected wallet); `epk_encoded` is the BIP44 ECDSA
/// account-0 extended-public-key bytes the W2 fund-routing gate matches.
pub fn seed_legacy_unprotected_hd_wallet_row(
    db: &Database,
    seed_hash: &[u8; 32],
    encrypted_seed: &[u8; 64],
    epk_encoded: &[u8],
    alias: &str,
    network: dash_sdk::dpp::dashcore::Network,
) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO wallet (
            seed_hash, encrypted_seed, salt, nonce,
            master_ecdsa_bip44_account_0_epk, alias, is_main,
            uses_password, password_hint, network, core_wallet_name
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, 0, NULL, ?7, NULL)",
        rusqlite::params![
            seed_hash.as_slice(),
            encrypted_seed.as_slice(),
            Vec::<u8>::new(),
            Vec::<u8>::new(),
            epk_encoded,
            alias,
            network.to_string(),
        ],
    )?;
    Ok(())
}

/// Insert a password-protected HD wallet row into the legacy `wallet` table —
/// the `uses_password=1` sibling of [`seed_legacy_unprotected_hd_wallet_row`].
/// Lets tests stage a "migrated-on-disk" protected wallet whose seed stays
/// encrypted until the user unlocks it, so the cold-start migration's deferral
/// of upstream registration for still-locked protected wallets can be covered.
///
/// `encrypted_seed`/`salt`/`nonce` are the AES-GCM envelope quartet produced by
/// [`encrypt_message`](crate::model::wallet::encryption::encrypt_message) for
/// the seed under the user's password: a 16-byte Argon2 salt and a 12-byte GCM
/// nonce, as the migration's `crypto_field_lengths_ok` gate requires
/// for a protected row. `epk_encoded` is the BIP44 ECDSA account-0
/// extended-public-key bytes the W2 fund-routing gate matches.
#[allow(clippy::too_many_arguments)]
pub fn seed_legacy_protected_hd_wallet_row(
    db: &Database,
    seed_hash: &[u8; 32],
    encrypted_seed: &[u8],
    salt: &[u8],
    nonce: &[u8],
    epk_encoded: &[u8],
    alias: &str,
    password_hint: Option<&str>,
    network: dash_sdk::dpp::dashcore::Network,
) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO wallet (
            seed_hash, encrypted_seed, salt, nonce,
            master_ecdsa_bip44_account_0_epk, alias, is_main,
            uses_password, password_hint, network, core_wallet_name
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, 1, ?7, ?8, NULL)",
        rusqlite::params![
            seed_hash.as_slice(),
            encrypted_seed,
            salt,
            nonce,
            epk_encoded,
            alias,
            password_hint,
            network.to_string(),
        ],
    )?;
    Ok(())
}

/// BIP44 ECDSA account-0 extended-public-key bytes for `seed`, the value a
/// legacy `wallet` row carries in `master_ecdsa_bip44_account_0_epk`. The
/// migration copies it verbatim and the W2 fund-routing gate matches on it, so a
/// staged legacy row needs an xpub that genuinely derives from its seed.
pub fn legacy_master_epk_bytes(
    seed: &[u8; 64],
    network: dash_sdk::dpp::dashcore::Network,
) -> Vec<u8> {
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::dashcore::secp256k1::Secp256k1;
    use dash_sdk::dpp::key_wallet::bip32::{
        ChildNumber, DerivationPath, ExtendedPrivKey, ExtendedPubKey,
    };

    let coin_type = if network == Network::Mainnet { 5 } else { 1 };
    let secp = Secp256k1::new();
    let master = ExtendedPrivKey::new_master(network, seed).expect("master key");
    let path = DerivationPath::from(vec![
        ChildNumber::Hardened { index: 44 },
        ChildNumber::Hardened { index: coin_type },
        ChildNumber::Hardened { index: 0 },
    ]);
    let account = master.derive_priv(&secp, &path).expect("derive account");
    ExtendedPubKey::from_priv(&secp, &account).encode().to_vec()
}

/// Create the legacy `scheduled_votes` table in `data.db`. Fresh installs no
/// longer create it (the unwire dropped it from `create_tables`), so a test that
/// stages a v0.10-dev vote queue has to put it back exactly as the old schema
/// had it.
pub fn create_legacy_scheduled_votes_table(db: &Database) -> rusqlite::Result<()> {
    db.execute(
        "CREATE TABLE IF NOT EXISTS scheduled_votes (
            identity_id BLOB NOT NULL,
            contested_name TEXT NOT NULL,
            vote_choice TEXT NOT NULL,
            time INTEGER NOT NULL,
            executed INTEGER NOT NULL DEFAULT 0,
            network TEXT NOT NULL,
            PRIMARY KEY (identity_id, contested_name)
        )",
        rusqlite::params![],
    )?;
    Ok(())
}

/// Insert one row into the legacy `scheduled_votes` table. `vote_choice` is the
/// `Display` form of `ResourceVoteChoice` (`Abstain`, `Lock`,
/// `TowardsIdentity(<base58>)`); pass an unparseable string to stage the corrupt
/// row a migration must survive.
pub fn seed_legacy_scheduled_vote_row(
    db: &Database,
    voter_id: &[u8; 32],
    contested_name: &str,
    vote_choice: &str,
    network: dash_sdk::dpp::dashcore::Network,
) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO scheduled_votes
         (identity_id, contested_name, vote_choice, time, executed, network)
         VALUES (?1, ?2, ?3, 1700000000, 0, ?4)",
        rusqlite::params![
            voter_id.as_slice(),
            contested_name,
            vote_choice,
            network.to_string(),
        ],
    )?;
    Ok(())
}

/// Create the legacy `identity` table in v0.9.3's column shape — what every
/// upgrading user's `data.db` actually holds.
///
/// The v0.9.3 original also carries a `CHECK` tying `wallet` to `wallet_index`.
/// It is omitted here on purpose: the import must survive a half-filled wallet
/// link, and a test cannot stage that row if SQLite rejects it on insert. Tests
/// that assert the genuine v0.9.3 schema (constraint included) build their own
/// DDL instead.
pub fn create_legacy_identity_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE identity (
            id BLOB PRIMARY KEY,
            data BLOB,
            status INTEGER NOT NULL DEFAULT 0,
            is_local INTEGER NOT NULL,
            alias TEXT,
            info TEXT,
            wallet BLOB,
            wallet_index INTEGER,
            identity_type TEXT,
            network TEXT NOT NULL
        );",
    )
}

/// A genuinely-decodable legacy `data` blob for a keyless identity, in the
/// `QualifiedIdentity::to_bytes()` shape v0.9.3 wrote.
///
/// `alias` seeds the blob's own copy of the alias — pass `None` to stage a blob
/// whose alias is absent, which is what exercises the `alias` column fallback.
/// Tests that need real key material in the blob build their own.
pub fn basic_legacy_identity_blob(id: [u8; 32], alias: Option<&str>, network: Network) -> Vec<u8> {
    use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::Identifier;

    let identity = dash_sdk::dpp::identity::Identity::create_basic_identity(
        Identifier::from(id),
        PlatformVersion::latest(),
    )
    .expect("basic identity");
    QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
        alias: alias.map(str::to_string),
        private_keys: Default::default(),
        dpns_names: vec![],
        associated_wallets: std::collections::BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: Default::default(),
        status: Default::default(),
        network,
    }
    .to_bytes()
}

/// One row of the legacy `identity` table, as v0.9.3 stored it. Defaults to the
/// common case — a local, `Active` identity with no alias, wallet link, or
/// identity type — so a test states only what it varies.
///
/// Rows with a *malformed* column (a 300 in `status`, a BLOB in `alias`) are not
/// expressible here by design: the typed fields cannot hold them. Stage those
/// with raw SQL, which is the point of those tests.
pub struct LegacyIdentityFixture {
    id: [u8; 32],
    data: Option<Vec<u8>>,
    status: u8,
    is_local: bool,
    alias: Option<String>,
    wallet: Option<(Vec<u8>, u32)>,
    identity_type: Option<String>,
    network: String,
}

impl LegacyIdentityFixture {
    /// A local, `Active` identity on `network`. `data` is the encoded blob —
    /// `None` stages the NULL-blob row the import must skip, and an undecodable
    /// blob stages the corrupt row it must count.
    ///
    /// `network` is a string, not a [`Network`], so a test can write the pre-v29
    /// mainnet spelling (`dash`) that a real legacy database still carries.
    pub fn new(id: [u8; 32], data: Option<Vec<u8>>, network: impl Into<String>) -> Self {
        Self {
            id,
            data,
            status: u8::from(IdentityStatus::Active),
            is_local: true,
            alias: None,
            wallet: None,
            identity_type: None,
            network: network.into(),
        }
    }

    /// Set the raw `status` column. v0.9.3 stored [`IdentityStatus`] as its
    /// discriminant, and this column is the only source of an imported
    /// identity's status — the blob never carried it.
    pub fn with_status(mut self, status: IdentityStatus) -> Self {
        self.status = u8::from(status);
        self
    }

    /// `false` stages an *observed* identity — v0.9.3's lookup cache, which every
    /// one of its read paths filtered out and so must the import.
    pub fn with_is_local(mut self, is_local: bool) -> Self {
        self.is_local = is_local;
        self
    }

    /// Set the authoritative `alias` column. In v0.9.3 this always won over the
    /// blob's own (often stale) copy.
    pub fn with_alias(mut self, alias: impl Into<String>) -> Self {
        self.alias = Some(alias.into());
        self
    }

    /// Link the identity to the wallet holding its keys. `seed_hash` is a plain
    /// byte vector so a test can stage a corrupt, non-32-byte link.
    pub fn with_wallet(mut self, seed_hash: Vec<u8>, wallet_index: u32) -> Self {
        self.wallet = Some((seed_hash, wallet_index));
        self
    }

    /// Set the `identity_type` column (`User`, `Masternode`, `Evonode`).
    pub fn with_identity_type(mut self, identity_type: impl Into<String>) -> Self {
        self.identity_type = Some(identity_type.into());
        self
    }

    /// Write the row.
    pub fn insert(&self, conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "INSERT INTO identity
                (id, data, status, is_local, alias, wallet, wallet_index, identity_type, network)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                self.id.as_slice(),
                self.data,
                self.status,
                i64::from(self.is_local),
                self.alias,
                self.wallet.as_ref().map(|(seed_hash, _)| seed_hash.clone()),
                self.wallet.as_ref().map(|(_, index)| *index),
                self.identity_type,
                self.network,
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The staged `identity` row, read straight back out of SQLite.
    #[derive(Debug, PartialEq)]
    struct StagedRow {
        id: Vec<u8>,
        data: Option<Vec<u8>>,
        status: i64,
        is_local: i64,
        alias: Option<String>,
        wallet: Option<Vec<u8>>,
        wallet_index: Option<i64>,
        identity_type: Option<String>,
        network: String,
    }

    fn read_back(conn: &Connection) -> StagedRow {
        conn.query_row(
            "SELECT id, data, status, is_local, alias, wallet, wallet_index, identity_type, \
             network FROM identity",
            [],
            |row| {
                Ok(StagedRow {
                    id: row.get(0)?,
                    data: row.get(1)?,
                    status: row.get(2)?,
                    is_local: row.get(3)?,
                    alias: row.get(4)?,
                    wallet: row.get(5)?,
                    wallet_index: row.get(6)?,
                    identity_type: row.get(7)?,
                    network: row.get(8)?,
                })
            },
        )
        .expect("read back the staged row")
    }

    /// The fixture's defaults are the ones every call site relies on: a local,
    /// `Active` row whose optional columns are genuinely NULL. A drifted default
    /// would silently re-target every test that omits the setter.
    #[test]
    fn legacy_identity_fixture_defaults_to_a_local_active_row() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        create_legacy_identity_table(&conn).expect("create identity table");
        let id = [0xAA; 32];
        let blob = basic_legacy_identity_blob(id, Some("alias"), Network::Testnet);

        LegacyIdentityFixture::new(id, Some(blob.clone()), "testnet")
            .insert(&conn)
            .expect("insert identity");

        assert_eq!(
            read_back(&conn),
            StagedRow {
                id: id.to_vec(),
                data: Some(blob),
                status: i64::from(u8::from(IdentityStatus::Active)),
                is_local: 1,
                alias: None,
                wallet: None,
                wallet_index: None,
                identity_type: None,
                network: "testnet".to_string(),
            },
            "the default row is a local, Active identity whose optional columns are NULL",
        );
    }

    /// Every setter must reach its own column — a fixture that silently drops the
    /// wallet link would turn the import's wallet-reattachment tests green for the
    /// wrong reason.
    #[test]
    fn legacy_identity_fixture_writes_every_column_it_is_given() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        create_legacy_identity_table(&conn).expect("create identity table");
        let id = [0xBB; 32];
        let seed_hash = vec![0x77; 32];

        LegacyIdentityFixture::new(id, None, "dash")
            .with_status(IdentityStatus::NotFound)
            .with_is_local(false)
            .with_alias("my-evonode")
            .with_wallet(seed_hash.clone(), 3)
            .with_identity_type("Evonode")
            .insert(&conn)
            .expect("insert identity");

        assert_eq!(
            read_back(&conn),
            StagedRow {
                id: id.to_vec(),
                data: None,
                status: i64::from(u8::from(IdentityStatus::NotFound)),
                is_local: 0,
                alias: Some("my-evonode".to_string()),
                wallet: Some(seed_hash),
                wallet_index: Some(3),
                identity_type: Some("Evonode".to_string()),
                network: "dash".to_string(),
            },
            "every setter reaches its own column, and the pre-v29 mainnet spelling \
             survives verbatim",
        );
    }

    #[test]
    fn test_create_test_database() {
        let db = create_test_database();
        assert!(db.is_ok(), "Should create in-memory database successfully");
    }

    #[test]
    fn test_create_temp_database() {
        let result = create_temp_database();
        assert!(
            result.is_ok(),
            "Should create temporary database successfully"
        );

        let (_db, temp_dir) = result.unwrap();
        let db_path = temp_dir.path().join("test_data.db");
        assert!(db_path.exists(), "Database file should exist");
    }
}
