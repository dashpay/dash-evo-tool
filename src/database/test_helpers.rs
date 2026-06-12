//! Test helper utilities for database testing.
//!
//! This module provides utilities for creating test databases that can be used
//! in unit and integration tests throughout the codebase.

use crate::database::Database;
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
/// for an unprotected wallet, per SEC-007); `epk_encoded` is the BIP44 ECDSA
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
/// nonce, as the migration's `crypto_field_lengths_ok` (SEC-007) gate requires
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

#[cfg(test)]
mod tests {
    use super::*;

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
