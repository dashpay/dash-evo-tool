use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};

const OLD_SCHEMA: &str = include_str!("fixtures/67d4ef3.sql");
const TARGET_SCHEMA: &str = include_str!("fixtures/e3cd7cf.sql");
const HISTORY: &str = "refinery_schema_history";
const IDENTITY_INDEX_KEY: &str = "det:identity_index:v1";

/// A compatibility upgrade stopped before committing changes to the original database.
#[derive(Debug, thiserror::Error)]
pub enum UpgradeError {
    #[error(
        "Could not upgrade wallet data. Check available disk space and restart the application."
    )]
    Sqlite(#[from] rusqlite::Error),
    #[error(
        "Could not back up wallet data. Check available disk space and restart the application."
    )]
    Io(#[from] std::io::Error),
    #[error(
        "Wallet data does not match a supported upgrade. Keep your data folder and reopen the previous application version."
    )]
    Unrecognized,
    #[error(
        "The saved identity list could not be read. Keep your data folder and reopen the previous application version."
    )]
    IdentityRoster,
    #[error(
        "The saved identity list could not be read. Keep your data folder and reopen the previous application version."
    )]
    IdentityRosterDecode(#[from] bincode::error::DecodeError),
    #[error(
        "Wallet data verification failed. Keep your data folder and reopen the previous application version."
    )]
    Verification,
    #[error(
        "The updated application could not read your wallet data. Keep your data folder and reopen the previous application version."
    )]
    TypedValidation(#[source] Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Debug, PartialEq, Eq)]
struct Object {
    kind: String,
    name: String,
    sql: String,
}

fn objects(conn: &Connection) -> Result<Vec<Object>, UpgradeError> {
    Ok(conn
        .prepare(
            "SELECT type, name, sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY type, name",
        )?
        .query_map([], |r| {
            Ok(Object {
                kind: r.get(0)?,
                name: r.get(1)?,
                sql: r.get(2)?,
            })
        })?
        .collect::<Result<_, _>>()?)
}

fn history(conn: &Connection) -> Result<Vec<(i64, String, String)>, UpgradeError> {
    Ok(conn
        .prepare("SELECT version, name, checksum FROM refinery_schema_history ORDER BY version")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?)
}

fn quote(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn columns(conn: &Connection, table: &str) -> Result<Vec<String>, UpgradeError> {
    Ok(conn
        .prepare(&format!("PRAGMA table_info({})", quote(table)))?
        .query_map([], |r| r.get(1))?
        .collect::<Result<_, _>>()?)
}

fn verify(conn: &Connection) -> Result<(), UpgradeError> {
    let result: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if result != "ok" || conn.prepare("PRAGMA foreign_key_check")?.exists([])? {
        return Err(UpgradeError::Verification);
    }
    Ok(())
}

fn old_reference() -> Result<Connection, UpgradeError> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(OLD_SCHEMA)?;
    Ok(conn)
}

fn target_reference() -> Result<Connection, UpgradeError> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(TARGET_SCHEMA)?;
    Ok(conn)
}

fn active_identities(conn: &Connection) -> Result<BTreeSet<Vec<u8>>, UpgradeError> {
    let value: Option<Vec<u8>> = conn
        .query_row(
            "SELECT value FROM meta_global WHERE key = ?1",
            [IDENTITY_INDEX_KEY],
            |r| r.get(0),
        )
        .optional()?;
    let Some(value) = value else {
        if conn.prepare("SELECT 1 FROM identities i JOIN meta_identity m ON i.identity_id = m.identity_id WHERE i.tombstoned = 1 AND m.key = 'det:identity:v1'")?.exists([])? {
            return Err(UpgradeError::IdentityRoster);
        }
        return Ok(BTreeSet::new());
    };
    let Some((&1, body)) = value.split_first() else {
        return Err(UpgradeError::IdentityRoster);
    };
    let (ids, consumed): (Vec<[u8; 32]>, usize) = bincode::serde::decode_from_slice(
        body,
        bincode::config::standard().with_limit::<16777216>(),
    )?;
    if consumed != body.len() {
        return Err(UpgradeError::IdentityRoster);
    }
    Ok(ids.into_iter().map(Vec::from).collect())
}

fn retired_identities(conn: &Connection) -> Result<Vec<Vec<u8>>, UpgradeError> {
    let active = active_identities(conn)?;
    Ok(conn
        .prepare("SELECT identity_id FROM identities WHERE tombstoned = 1")?
        .query_map([], |r| r.get::<_, Vec<u8>>(0))?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|id| !active.contains(id))
        .collect())
}

fn retirement_column(table: &str) -> Option<&'static str> {
    match table {
        "identities"
        | "identity_keys"
        | "token_balances"
        | "dashpay_profiles"
        | "dashpay_payments_overlay" => Some("identity_id"),
        "contacts" | "ignored_senders" => Some("owner_id"),
        "pending_contact_crypto" => Some("owner_identity_id"),
        _ => None,
    }
}

fn selected_rows(table: &str) -> String {
    retirement_column(table)
        .map(|c| {
            format!(
                " WHERE {} NOT IN (SELECT id FROM temp.retired_identities)",
                quote(c)
            )
        })
        .unwrap_or_default()
}

fn backup(path: &Path) -> Result<PathBuf, UpgradeError> {
    let parent = path.parent().ok_or(UpgradeError::Unrecognized)?;
    let filename = path.file_name().ok_or(UpgradeError::Unrecognized)?;
    let prefix = format!("{}.platform-67d4ef3-backup-", filename.to_string_lossy());
    let file = tempfile::Builder::new()
        .prefix(&prefix)
        .suffix(".sqlite")
        .tempfile_in(parent)?;
    let source = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    let mut dest = Connection::open(file.path())?;
    // A separate reader sees the committed snapshot while the writer lock excludes changes.
    let copier = rusqlite::backup::Backup::new(&source, &mut dest)?;
    if !matches!(copier.step(-1)?, rusqlite::backup::StepResult::Done) {
        return Err(UpgradeError::Verification);
    }
    drop(copier);
    verify(&dest)?;
    drop(dest);
    file.as_file().sync_all()?;
    let (_, path) = file.keep().map_err(|e| e.error)?;
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(path)
}

fn copy_rows(
    source: &Connection,
    target: &Connection,
    table: &str,
    cols: &[String],
    filter: &str,
) -> Result<(), UpgradeError> {
    let names = cols.iter().map(|c| quote(c)).collect::<Vec<_>>().join(",");
    let placeholders = vec!["?"; cols.len()].join(",");
    let mut input = source.prepare(&format!("SELECT {names} FROM {}{filter}", quote(table)))?;
    let mut output = target.prepare(&format!(
        "INSERT INTO {} ({names}) VALUES ({placeholders})",
        quote(table)
    ))?;
    let mut rows = input.query([])?;
    while let Some(row) = rows.next()? {
        let values = (0..cols.len())
            .map(|i| row.get::<_, rusqlite::types::Value>(i))
            .collect::<Result<Vec<_>, _>>()?;
        output.execute(rusqlite::params_from_iter(values))?;
    }
    Ok(())
}

fn equal_rows(
    source: &Connection,
    target: &Connection,
    table: &str,
    cols: &[String],
    filter: &str,
) -> Result<(), UpgradeError> {
    let names = cols.iter().map(|c| quote(c)).collect::<Vec<_>>().join(",");
    let sql = format!("SELECT {names} FROM {}", quote(table));
    let order = format!(" ORDER BY {names}");
    let mut a = source.prepare(&format!("{sql}{filter}{order}"))?;
    let mut b = target.prepare(&format!("{sql}{order}"))?;
    let mut a = a.query([])?;
    let mut b = b.query([])?;
    loop {
        match (a.next()?, b.next()?) {
            (None, None) => return Ok(()),
            (Some(a), Some(b)) => {
                for i in 0..cols.len() {
                    if a.get_ref(i)? != b.get_ref(i)? {
                        return Err(UpgradeError::Verification);
                    }
                }
            }
            _ => return Err(UpgradeError::Verification),
        }
    }
}

fn allowed_columns(
    table: &str,
    old: &[String],
    new: &[String],
) -> Result<Vec<String>, UpgradeError> {
    let old_set = old.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let new_set = new.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let added = new_set.difference(&old_set).copied().collect::<Vec<_>>();
    let removed = old_set.difference(&new_set).copied().collect::<Vec<_>>();
    let expected_added: &[&str] = match table {
        "core_utxos" => &[
            "is_sweep_placeholder",
            "spent_in_txid",
            "winner_mined_height",
        ],
        "core_sync_state" => &["chainlock_height"],
        _ => &[],
    };
    let expected_removed: &[&str] = if table == "identities" {
        &["tombstoned"]
    } else {
        &[]
    };
    if added != expected_added || removed != expected_removed {
        return Err(UpgradeError::Unrecognized);
    }
    Ok(old
        .iter()
        .filter(|c| new_set.contains(c.as_str()))
        .cloned()
        .collect())
}

/// Translate only the exact pinned PR schema, preserving the original in a durable backup.
pub(super) fn upgrade(
    path: &Path,
    target_path: &Path,
    validate: impl FnOnce(&Path) -> Result<(), UpgradeError>,
) -> Result<Option<PathBuf>, UpgradeError> {
    upgrade_with_hook(path, target_path, validate, || Ok(()))
}

fn upgrade_with_hook(
    path: &Path,
    target_path: &Path,
    validate: impl FnOnce(&Path) -> Result<(), UpgradeError>,
    before_commit: impl FnOnce() -> Result<(), UpgradeError>,
) -> Result<Option<PathBuf>, UpgradeError> {
    let old = old_reference()?;
    let reference = target_reference()?;
    let mut source = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    source.busy_timeout(std::time::Duration::from_secs(5))?;
    source.pragma_update(None, "foreign_keys", false)?;
    source.pragma_update(None, "synchronous", "FULL")?;
    let source = source.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing = objects(&source)?;
    if !existing.iter().any(|o| o.name == HISTORY) {
        return Ok(None);
    }
    if history(&source)? != history(&old)? {
        return Ok(None);
    }
    if existing != objects(&old)?
        || source.query_row("PRAGMA application_id", [], |r| r.get::<_, i64>(0))? != 0x504c5754
    {
        return Err(UpgradeError::Unrecognized);
    }
    verify(&source)?;
    let retired = retired_identities(&source)?;
    let backup = backup(path)?;
    let mut target = Connection::open_with_flags(
        target_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    if objects(&target)? != objects(&reference)? || history(&target)? != history(&reference)? {
        return Err(UpgradeError::Unrecognized);
    }
    let target_objects = objects(&target)?;
    target.pragma_update(None, "foreign_keys", false)?;
    let target_tx = target.transaction()?;
    source.execute_batch("CREATE TEMP TABLE retired_identities (id BLOB PRIMARY KEY)")?;
    for id in retired {
        source.execute("INSERT INTO temp.retired_identities VALUES (?1)", [id])?;
    }
    for o in target_objects.iter().filter(|o| o.kind == "trigger") {
        target_tx.execute_batch(&format!("DROP TRIGGER {}", quote(&o.name)))?;
    }
    if target_tx.query_row(
        "SELECT count(*) FROM meta_store_generation WHERE id = 0 AND length(generation) = 16",
        [],
        |r| r.get::<_, i64>(0),
    )? != 1
    {
        return Err(UpgradeError::Unrecognized);
    }
    target_tx.execute("DELETE FROM meta_store_generation", [])?;
    let old_tables = existing
        .iter()
        .filter(|o| o.kind == "table")
        .map(|o| o.name.as_str())
        .collect::<BTreeSet<_>>();
    let new_tables = target_objects
        .iter()
        .filter(|o| o.kind == "table")
        .map(|o| o.name.as_str())
        .collect::<BTreeSet<_>>();
    let added = new_tables
        .difference(&old_tables)
        .copied()
        .collect::<Vec<_>>();
    if !old_tables.is_subset(&new_tables)
        || added != ["identity_scan_failed_indices", "identity_scan_states"]
    {
        return Err(UpgradeError::Unrecognized);
    }
    for table in new_tables.iter().filter(|t| **t != HISTORY) {
        if target_tx.query_row(&format!("SELECT count(*) FROM {}", quote(table)), [], |r| {
            r.get::<_, i64>(0)
        })? != 0
        {
            return Err(UpgradeError::Unrecognized);
        }
    }
    for table in old_tables.iter().filter(|t| **t != HISTORY) {
        let cols = allowed_columns(
            table,
            &columns(&source, table)?,
            &columns(&target_tx, table)?,
        )?;
        let filter = selected_rows(table);
        copy_rows(&source, &target_tx, table, &cols, &filter)?;
        equal_rows(&source, &target_tx, table, &cols, &filter)?;
    }
    for (old_domain, new_domain) in [
        ("wallet_metadata", "wallets"),
        ("account_address_pools", "core_address_pool"),
    ] {
        target_tx.execute(
            "INSERT INTO meta_data_versions (wallet_id, domain, seq) SELECT wallet_id, ?2, seq FROM meta_data_versions WHERE domain = ?1 ON CONFLICT(wallet_id, domain) DO UPDATE SET seq = MAX(seq, excluded.seq)",
            [old_domain, new_domain],
        )?;
    }
    for o in target_objects.iter().filter(|o| o.kind == "trigger") {
        target_tx.execute_batch(&o.sql)?;
    }
    verify(&target_tx)?;
    target_tx.commit()?;
    drop(target);
    validate(target_path)?;
    let target = Connection::open_with_flags(
        target_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    // Rebuild under one SQLite transaction; power loss exposes either complete schema.
    for o in existing.iter().filter(|o| o.kind == "trigger") {
        source.execute_batch(&format!("DROP TRIGGER {}", quote(&o.name)))?;
    }
    for table in &old_tables {
        source.execute_batch(&format!("DROP TABLE {}", quote(table)))?;
    }
    for o in target_objects.iter().filter(|o| o.kind == "table") {
        source.execute_batch(&o.sql)?;
    }
    for table in &new_tables {
        let cols = columns(&target, table)?;
        copy_rows(&target, &source, table, &cols, "")?;
        equal_rows(&target, &source, table, &cols, "")?;
    }
    for o in target_objects.iter().filter(|o| o.kind != "table") {
        source.execute_batch(&o.sql)?;
    }
    if objects(&source)? != target_objects {
        return Err(UpgradeError::Verification);
    }
    verify(&source)?;
    before_commit()?;
    source.commit()?;
    Ok(Some(backup))
}

#[cfg(test)]
#[path = "engine/tests.rs"]
mod tests;
