//! The checks a migrated profile has to satisfy.
//!
//! Every check is conditional on what the fixture actually started from: a
//! `data.db` it carried must come out byte-identical (every boot path opens it
//! read-only), a missing one must be created at [`DEFAULT_DB_VERSION`], and a
//! fixture holding a password-protected wallet must stop at the documented
//! `StorageUpdateNeedsDesktop` instead of completing.
//!
//! SQLite files are never opened in place. Each read copies the database (and
//! its `-wal` / `-shm` siblings) into a scratch directory first, so a read can
//! neither checkpoint the file under test nor disturb a byte-identity check.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use dash_evo_tool::backend_task::migration::finish_unwire::{
    dapi_refresh_sentinel_key_for, sentinel_key_for,
};
use dash_evo_tool::database::DEFAULT_DB_VERSION;
use dash_sdk::dpp::dashcore::{Network, base58};
use rusqlite::Connection;
use serde_json::Value;

use crate::cli::CliRun;
use crate::manifest::{ExpectedWallet, WalletOutcome};

/// Legacy DET database carrying the `settings.database_version` ladder.
pub const DATA_DB: &str = "data.db";

/// Cross-network app store holding the migration sentinels.
pub const APP_DB: &str = "det-app.sqlite";

/// Key every stored identity lives under in the per-network store
/// (`IDENTITY_KEY` in `src/context/identity_db.rs`).
const IDENTITY_KEY: &str = "det:identity:v1";

/// Namespace shared by every migration sentinel. Discovering keys by prefix
/// picks up the ones whose constants are private (the legacy-settings import)
/// without this harness guessing at their spelling.
const SENTINEL_PREFIX: &str = "det:migration:";

/// Text that must never appear in a boot's output. A panic or an
/// incompatibility verdict is a migration defect, not a condition to work
/// around — the matrix fails rather than falling back.
const FAILURE_MARKERS: &[&str] = &[
    "panicked at",
    "WalletDataIncompatible",
    "Your wallet data is not compatible with this version of the app",
];

/// On-disk token for a network, matching `wallet_backend::network_prefix`
/// (which is crate-private) — used for the per-network database file name.
fn storage_token(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "mainnet",
        Network::Testnet => "testnet",
        Network::Devnet => "devnet",
        Network::Regtest => "regtest",
    }
}

/// Network spelling `network_info` reports, matching
/// `mcp::server::network_display_name` — note `local`, not `regtest`.
pub fn display_name(network: Network) -> &'static str {
    match network {
        Network::Regtest => "local",
        other => storage_token(other),
    }
}

/// The durable per-network wallet database, sibling of [`APP_DB`].
pub fn network_db_name(network: Network) -> String {
    format!("det-{}.sqlite", storage_token(network))
}

/// A boot that ran to completion without panicking or declaring the data
/// incompatible.
pub fn check_boot(run: &CliRun, label: &str) -> Result<(), String> {
    if !run.succeeded() {
        return Err(format!("{label} did not complete\n{}", run.report()));
    }
    for marker in FAILURE_MARKERS {
        if run.stdout.contains(marker) || run.stderr.contains(marker) {
            return Err(format!("{label} reported `{marker}`\n{}", run.report()));
        }
    }
    Ok(())
}

/// How a wallet-gated boot ends when the fixture holds a wallet only the
/// desktop app can migrate: the typed error the MCP layer carries in the
/// error `data`, which rmcp logs to stderr at `warn` (inside the harness's
/// default `info` filter). Matching the variant rather than the user-facing
/// message keeps a copy edit from breaking the check; a log filter that hides
/// the line fails the check instead of passing it.
const NEEDS_DESKTOP_MARKER: &str =
    "StorageUpdateNeedsDesktop { source: InteractivePromptUnavailable }";

/// A wallet-gated boot over a fixture holding a password-protected wallet.
/// det-cli cannot prompt for the password by design
/// (`docs/ai-design/2026-07-14-migration-password-prompt/design.md`), so the
/// boot must fail with exactly [`NEEDS_DESKTOP_MARKER`]. Completing, or
/// failing in any other way, is a regression.
pub fn check_needs_desktop(run: &CliRun, label: &str) -> Result<(), String> {
    for marker in FAILURE_MARKERS {
        if run.stdout.contains(marker) || run.stderr.contains(marker) {
            return Err(format!("{label} reported `{marker}`\n{}", run.report()));
        }
    }
    if run.succeeded() {
        return Err(format!(
            "{label} completed, but the fixture holds a password-protected wallet that a headless \
             boot cannot migrate; it must stop at StorageUpdateNeedsDesktop\n{}",
            run.report()
        ));
    }
    if run.timed_out || !run.stderr.contains(NEEDS_DESKTOP_MARKER) {
        return Err(format!(
            "{label} failed, but not with `{NEEDS_DESKTOP_MARKER}`\n{}",
            run.report()
        ));
    }
    Ok(())
}

/// `data.db`'s schema as it can be compared across a boot: the version the
/// ladder records plus every object in `sqlite_master`.
#[derive(Debug, PartialEq, Eq)]
pub struct SchemaSnapshot {
    pub version: Option<u16>,
    objects: Vec<(String, String, String)>,
}

impl SchemaSnapshot {
    fn describe_version(&self) -> String {
        match self.version {
            Some(version) => version.to_string(),
            None => "unknown (no settings row)".to_string(),
        }
    }
}

/// Reads a `data.db` snapshot, or `None` when the fixture has no such file.
pub fn schema_snapshot(
    data_db: &Path,
    scratch: &Path,
    label: &str,
) -> Result<Option<SchemaSnapshot>, String> {
    let Some(conn) = open_copy(data_db, scratch, label)? else {
        return Ok(None);
    };
    let version = conn
        .query_row(
            "SELECT database_version FROM settings WHERE id = 1",
            [],
            |row| row.get::<_, u16>(0),
        )
        .ok();
    let mut statement = conn
        .prepare("SELECT type, name, IFNULL(sql, '') FROM sqlite_master ORDER BY type, name")
        .map_err(|e| format!("could not read the schema of {}: {e}", data_db.display()))?;
    let objects = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .and_then(|rows| rows.collect::<Result<Vec<(String, String, String)>, _>>())
        .map_err(|e| format!("could not read the schema of {}: {e}", data_db.display()))?;
    Ok(Some(SchemaSnapshot { version, objects }))
}

/// The legacy `data.db` verdict, conditional on where the fixture started.
///
/// Every boot path (GUI and det-cli) opens an existing `data.db` with SQLite
/// writes disabled: it is the recovery artifact the storage update reads
/// from, never a file it upgrades. The version ladder in
/// `Database::initialize` runs only when no `data.db` exists yet. So:
///
/// - present → version and schema exactly as captured;
/// - above [`DEFAULT_DB_VERSION`] → the fixture was captured by a newer build
///   than the one under test, which the matrix reports rather than silently
///   passing;
/// - absent → a fresh file must be created at [`DEFAULT_DB_VERSION`].
pub fn check_schema_outcome(
    before: Option<&SchemaSnapshot>,
    after: Option<&SchemaSnapshot>,
) -> Result<(), String> {
    let Some(after) = after else {
        return Err(format!("the boot left no {DATA_DB} behind"));
    };
    let Some(before) = before else {
        return match after.version {
            Some(DEFAULT_DB_VERSION) => Ok(()),
            _ => Err(format!(
                "a fixture without {DATA_DB} must get a fresh one at version {DEFAULT_DB_VERSION}, found {}",
                after.describe_version()
            )),
        };
    };

    if let Some(version) = before.version
        && version > DEFAULT_DB_VERSION
    {
        return Err(format!(
            "fixture {DATA_DB} is at version {version}, newer than this build's {DEFAULT_DB_VERSION} — \
             the fixture was captured by a newer DET than the binary under test"
        ));
    }
    if before == after {
        return Ok(());
    }
    Err(format!(
        "the boot changed the pre-update {DATA_DB} (version {} -> {}), which every boot path must \
         open read-only",
        before.describe_version(),
        after.describe_version()
    ))
}

/// Reads a file whole, or `None` when it does not exist.
pub fn file_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("could not read {}: {e}", path.display())),
    }
}

/// Byte identity of a pre-update `data.db` across a boot. Stronger than
/// [`check_schema_outcome`]: a read-only open cannot write a single page, so
/// any difference means something opened the file for writing.
pub fn check_bytes_unchanged(
    before: Option<&Vec<u8>>,
    after: Option<&Vec<u8>>,
    label: &str,
) -> Result<(), String> {
    match (before, after) {
        (Some(before), Some(after)) if before == after => Ok(()),
        (Some(before), Some(after)) if before.len() == after.len() => Err(format!(
            "{label} must be byte-identical across a boot, but its content changed ({} bytes)",
            after.len()
        )),
        (Some(before), Some(after)) => Err(format!(
            "{label} must be byte-identical across a boot, but its size went from {} to {} bytes",
            before.len(),
            after.len()
        )),
        (None, None) => Ok(()),
        (None, Some(_)) => Err(format!(
            "{label} did not exist before the boot but does now"
        )),
        (Some(_), None) => Err(format!("{label} existed before the boot but is gone")),
    }
}

/// The active network `network-info` reports, which is what proves the
/// migrated settings kept the profile's network.
pub fn check_network(expected: Network, run: &CliRun) -> Result<(), String> {
    let json = run.json()?;
    let active = json
        .get("active")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("network-info printed no `active` field\n{}", run.report()))?;
    if active != display_name(expected) {
        return Err(format!(
            "fixture declares network `{}` but the boot came up on `{active}`",
            display_name(expected)
        ));
    }
    Ok(())
}

/// Checks every expected alias survived and returns the identifiers usable
/// with `core-address-create` (alias when there is one, seed hash otherwise).
pub fn check_wallets(expected_aliases: &[String], run: &CliRun) -> Result<Vec<String>, String> {
    let json = run.json()?;
    let wallets = json
        .get("wallets")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            format!(
                "core-wallets-list printed no `wallets` array\n{}",
                run.report()
            )
        })?;

    let mut aliases = BTreeSet::new();
    let mut identifiers = Vec::new();
    for wallet in wallets {
        let alias = wallet.get("alias").and_then(Value::as_str);
        let seed_hash = wallet.get("seed_hash").and_then(Value::as_str);
        if let Some(alias) = alias {
            aliases.insert(alias.to_string());
        }
        match alias.or(seed_hash) {
            Some(id) => identifiers.push(id.to_string()),
            None => {
                return Err(format!(
                    "a listed wallet has neither alias nor seed hash\n{}",
                    run.report()
                ));
            }
        }
    }

    let missing: Vec<&String> = expected_aliases
        .iter()
        .filter(|alias| !aliases.contains(*alias))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "wallets missing after the migration: {missing:?} — the boot listed {aliases:?}"
        ));
    }
    Ok(identifiers)
}

/// A derived receive address, proving the migrated seed is usable and not
/// merely present.
pub fn check_address(run: &CliRun) -> Result<String, String> {
    let json = run.json()?;
    let address = json
        .get("address")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("core-address-create printed no `address`\n{}", run.report()))?;
    if address.trim().is_empty() {
        return Err(format!(
            "core-address-create returned an empty address\n{}",
            run.report()
        ));
    }
    Ok(address.to_string())
}

/// Every migration sentinel in the app store, keyed as written. Empty when the
/// store does not exist yet.
pub fn migration_sentinels(
    app_db: &Path,
    scratch: &Path,
    label: &str,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let Some(conn) = open_copy(app_db, scratch, label)? else {
        return Ok(BTreeMap::new());
    };
    let mut statement = conn
        .prepare("SELECT key, value FROM meta_global WHERE key LIKE ?1 ORDER BY key")
        .map_err(|e| {
            format!(
                "could not read migration sentinels from {}: {e} — the upstream `meta_global` \
                 table is where `DetKv` stores global-scope values",
                app_db.display()
            )
        })?;
    let rows = statement
        .query_map([format!("{SENTINEL_PREFIX}%")], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .and_then(|rows| rows.collect::<Result<BTreeMap<String, Vec<u8>>, _>>())
        .map_err(|e| {
            format!(
                "could not read migration sentinels from {}: {e}",
                app_db.display()
            )
        })?;
    Ok(rows)
}

/// The completion sentinel the legacy drain records once it finishes.
pub fn check_sentinel_recorded(
    sentinels: &BTreeMap<String, Vec<u8>>,
    network: Network,
) -> Result<(), String> {
    let key = sentinel_key_for(network);
    if sentinels.contains_key(&key) {
        return Ok(());
    }
    Err(format!(
        "the boot recorded no `{key}` completion sentinel in {APP_DB}; present migration keys: {:?}",
        sentinels.keys().collect::<Vec<_>>()
    ))
}

/// The completion sentinel must not exist while the storage update is
/// unfinished: a wallet still waiting for its password is not migrated.
pub fn check_sentinel_absent(
    sentinels: &BTreeMap<String, Vec<u8>>,
    network: Network,
) -> Result<(), String> {
    let key = sentinel_key_for(network);
    if sentinels.contains_key(&key) {
        return Err(format!(
            "the boot recorded the `{key}` completion sentinel although the storage update could \
             not finish"
        ));
    }
    Ok(())
}

/// Whether each aliased wallet of a v0.9.3-era `data.db` is registered in the
/// per-network store, keyed by alias. `None` when the fixture has no
/// `data.db`.
///
/// The two stores share no wallet id: `data.db` keys wallets by seed hash, the
/// upstream store by its own id. Both do keep the BIP44 account-0 xpub —
/// `data.db` as the 78-byte serialization, `account_registrations` as its
/// base58check text — so the match is a byte search for that text, with no
/// key derivation in the harness.
pub fn legacy_wallet_registrations(
    data_db: &Path,
    network_db: &Path,
    scratch: &Path,
    label: &str,
) -> Result<Option<BTreeMap<String, bool>>, String> {
    let Some(legacy) = open_copy(data_db, scratch, label)? else {
        return Ok(None);
    };
    let read_error = |e: rusqlite::Error| {
        format!(
            "could not read legacy wallets from {}: {e}",
            data_db.display()
        )
    };
    let mut statement = legacy
        .prepare("SELECT alias, master_ecdsa_bip44_account_0_epk FROM wallet")
        .map_err(read_error)?;
    let wallets = statement
        .query_map([], |row| {
            Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(read_error)?;

    let store = open_copy(network_db, scratch, label)?;
    let mut registered = BTreeMap::new();
    for (alias, account_xpub) in wallets {
        // The manifest names wallets by alias; an unnamed one cannot be asked about.
        let Some(alias) = alias else { continue };
        let found = match &store {
            None => false,
            Some(conn) => conn
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM account_registrations \
                     WHERE account_type = 'standard_bip44' AND account_index = 0 \
                     AND instr(account_xpub_bytes, CAST(?1 AS BLOB)) > 0)",
                    [base58::encode_check(&account_xpub)],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(|e| {
                    format!(
                        "could not read wallet registrations from {}: {e}",
                        network_db.display()
                    )
                })?,
        };
        registered.insert(alias, found);
    }
    Ok(Some(registered))
}

/// Every wallet the manifest lists must have landed where its expected outcome
/// says: registered when `migrated`, unregistered when it needs the desktop.
pub fn check_wallet_outcomes(
    expected: &[ExpectedWallet],
    registered: &BTreeMap<String, bool>,
) -> Result<(), String> {
    let problems: Vec<String> = expected
        .iter()
        .filter_map(|wallet| {
            let alias = &wallet.alias;
            match (wallet.outcome, registered.get(alias)) {
                (_, None) => Some(format!("`{alias}` is not a wallet in {DATA_DB}")),
                (WalletOutcome::Migrated, Some(false)) => Some(format!(
                    "`{alias}` was not registered in the per-network store"
                )),
                (WalletOutcome::NeedsDesktop, Some(true)) => Some(format!(
                    "`{alias}` is password-protected, yet was registered without its password"
                )),
                _ => None,
            }
        })
        .collect();
    match problems.is_empty() {
        true => Ok(()),
        false => Err(format!(
            "wallet outcomes differ from the manifest: {}",
            problems.join("; ")
        )),
    }
}

/// A supplied password must never be printed — not in a tool result, not in an
/// error, not in any log line at any level. The failure names the command and
/// the stream only, never the password.
pub fn check_password_not_echoed(run: &CliRun, password: &str) -> Result<(), String> {
    let streams: Vec<&str> = [("stdout", &run.stdout), ("stderr", &run.stderr)]
        .into_iter()
        .filter(|(_, text)| text.contains(password))
        .map(|(stream, _)| stream)
        .collect();
    match streams.is_empty() {
        true => Ok(()),
        false => Err(format!(
            "`{}` printed the supplied wallet password on {}",
            run.command,
            streams.join(" and ")
        )),
    }
}

/// Backup files a boot created, by path relative to the data dir. Covers the
/// `platform_compatibility` bridge's `*-backup-*.sqlite` copies and the
/// generic `.bak` convention.
pub fn backup_files(data_dir: &Path) -> Result<BTreeMap<String, u64>, String> {
    let mut found = BTreeMap::new();
    collect_backups(data_dir, data_dir, &mut found)?;
    Ok(found)
}

fn collect_backups(
    root: &Path,
    dir: &Path,
    found: &mut BTreeMap<String, u64>,
) -> Result<(), String> {
    let entries =
        fs::read_dir(dir).map_err(|e| format!("could not read {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("could not walk {}: {e}", dir.display()))?;
        let path = entry.path();
        let meta = entry
            .metadata()
            .map_err(|e| format!("could not stat {}: {e}", path.display()))?;
        if meta.is_dir() {
            collect_backups(root, &path, found)?;
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name.contains("backup") || name.ends_with(".bak") {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            found.insert(relative, meta.len());
        }
    }
    Ok(())
}

/// A second boot must be a no-op: no fresh backup copies, and the sentinels
/// still carrying the marker the first boot wrote.
pub fn check_idempotent(
    first: (&BTreeMap<String, u64>, &BTreeMap<String, Vec<u8>>),
    second: (&BTreeMap<String, u64>, &BTreeMap<String, Vec<u8>>),
) -> Result<(), String> {
    let (first_backups, first_sentinels) = first;
    let (second_backups, second_sentinels) = second;

    let new_backups: Vec<&String> = second_backups
        .keys()
        .filter(|name| !first_backups.contains_key(*name))
        .collect();
    if !new_backups.is_empty() {
        return Err(format!(
            "the second boot created new backup files: {new_backups:?}"
        ));
    }

    if first_sentinels != second_sentinels {
        let changed: Vec<&String> = second_sentinels
            .keys()
            .chain(first_sentinels.keys())
            .filter(|key| first_sentinels.get(*key) != second_sentinels.get(*key))
            .collect();
        return Err(format!(
            "the second boot rewrote migration sentinels: {changed:?} — a completed migration must \
             short-circuit on its sentinel"
        ));
    }
    Ok(())
}

/// Identity ids stored in the per-network database, lowercase hex.
///
// TODO: switch to identity_list/app_storage_status MCP tool once merged —
// reading the store directly couples this harness to the upstream kv table
// layout (`meta_identity`), which a read-only tool would hide.
pub fn identity_ids(
    network_db: &Path,
    scratch: &Path,
    label: &str,
) -> Result<BTreeSet<String>, String> {
    let Some(conn) = open_copy(network_db, scratch, label)? else {
        return Ok(BTreeSet::new());
    };
    let mut statement = conn
        .prepare("SELECT lower(hex(identity_id)) FROM meta_identity WHERE key = ?1")
        .map_err(|e| {
            format!(
                "could not read identities from {}: {e}",
                network_db.display()
            )
        })?;
    let ids = statement
        .query_map([IDENTITY_KEY], |row| row.get::<_, String>(0))
        .and_then(|rows| rows.collect::<Result<BTreeSet<String>, _>>())
        .map_err(|e| {
            format!(
                "could not read identities from {}: {e}",
                network_db.display()
            )
        })?;
    Ok(ids)
}

/// Every identity the fixture recorded must still be there afterwards.
pub fn check_identities(expected: &[String], found: &BTreeSet<String>) -> Result<(), String> {
    let missing: Vec<String> = expected
        .iter()
        .map(|id| id.to_ascii_lowercase())
        .filter(|id| !found.contains(id))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "identities missing after the migration: {missing:?} — the store holds {found:?}"
        ));
    }
    Ok(())
}

/// Opens a copy of a SQLite database, carrying its `-wal` / `-shm` siblings so
/// uncheckpointed commits are visible. Returns `None` when the database does
/// not exist.
fn open_copy(db: &Path, scratch: &Path, label: &str) -> Result<Option<Connection>, String> {
    if !db.exists() {
        return Ok(None);
    }
    let file_name = db
        .file_name()
        .ok_or_else(|| format!("{} has no file name", db.display()))?;
    let dir = scratch.join(label);
    fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;

    let copy: PathBuf = dir.join(file_name);
    fs::copy(db, &copy).map_err(|e| format!("could not copy {}: {e}", db.display()))?;
    for suffix in ["-wal", "-shm"] {
        let sibling = PathBuf::from(format!("{}{suffix}", db.display()));
        if sibling.exists() {
            let target = PathBuf::from(format!("{}{suffix}", copy.display()));
            fs::copy(&sibling, &target)
                .map_err(|e| format!("could not copy {}: {e}", sibling.display()))?;
        }
    }

    Connection::open(&copy)
        .map(Some)
        .map_err(|e| format!("could not open {}: {e}", copy.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(version: Option<u16>, extra_table: bool) -> SchemaSnapshot {
        let mut objects = vec![(
            "table".to_string(),
            "settings".to_string(),
            "CREATE TABLE settings(id INTEGER, database_version INTEGER)".to_string(),
        )];
        if extra_table {
            objects.push((
                "table".to_string(),
                "tokens".to_string(),
                "CREATE TABLE tokens(id INTEGER)".to_string(),
            ));
        }
        SchemaSnapshot { version, objects }
    }

    /// Every boot path opens an existing `data.db` read-only, so the legacy
    /// ladder never runs on it — whatever version it was left at.
    #[test]
    fn a_pre_update_data_db_must_be_left_untouched() {
        for version in [11, DEFAULT_DB_VERSION] {
            let before = snapshot(Some(version), false);
            check_schema_outcome(Some(&before), Some(&snapshot(Some(version), false)))
                .unwrap_or_else(|e| panic!("an untouched v{version} file must pass: {e}"));

            let migrated = snapshot(Some(DEFAULT_DB_VERSION), true);
            let error = check_schema_outcome(Some(&before), Some(&migrated))
                .expect_err("any schema change to a pre-update data.db must fail");
            assert!(error.contains("read-only"), "{error}");
        }
    }

    #[test]
    fn a_fixture_from_a_newer_build_is_named_as_such() {
        let before = snapshot(Some(DEFAULT_DB_VERSION + 1), false);
        let error = check_schema_outcome(
            Some(&before),
            Some(&snapshot(Some(DEFAULT_DB_VERSION + 1), false)),
        )
        .expect_err("a newer fixture must be rejected");
        assert!(error.contains("newer than this build"), "{error}");
    }

    #[test]
    fn a_missing_data_db_is_created_at_the_current_version() {
        check_schema_outcome(None, Some(&snapshot(Some(DEFAULT_DB_VERSION), false)))
            .expect("a fresh file at the current version passes");
        let error = check_schema_outcome(None, Some(&snapshot(Some(11), false)))
            .expect_err("a fresh file below the current version must fail");
        assert!(error.contains("fresh one at version"), "{error}");
    }

    #[test]
    fn a_second_boot_may_not_add_backups_or_rewrite_sentinels() {
        let key = sentinel_key_for(Network::Testnet);
        let backups =
            BTreeMap::from([("det-app.platform-67d4ef3-backup-1.sqlite".to_string(), 10)]);
        let sentinels = BTreeMap::from([(key.clone(), vec![1, 2, 3])]);

        check_idempotent((&backups, &sentinels), (&backups, &sentinels))
            .expect("an unchanged footprint is idempotent");

        let mut more_backups = backups.clone();
        more_backups.insert("det-app.platform-67d4ef3-backup-2.sqlite".to_string(), 10);
        let error = check_idempotent((&backups, &sentinels), (&more_backups, &sentinels))
            .expect_err("a new backup file must fail");
        assert!(error.contains("backup-2"), "{error}");

        let rewritten = BTreeMap::from([(key.clone(), vec![9])]);
        let error = check_idempotent((&backups, &sentinels), (&backups, &rewritten))
            .expect_err("a rewritten sentinel must fail");
        assert!(error.contains(&key), "{error}");
    }

    #[test]
    fn a_missing_completion_sentinel_names_the_key_it_wanted() {
        let error = check_sentinel_recorded(&BTreeMap::new(), Network::Testnet)
            .expect_err("an unrecorded migration must fail");
        assert!(
            error.contains(&sentinel_key_for(Network::Testnet)),
            "{error}"
        );
        assert_ne!(
            sentinel_key_for(Network::Testnet),
            dapi_refresh_sentinel_key_for(Network::Testnet),
            "the drain and the DAPI refresh must not share a sentinel"
        );
    }

    #[test]
    fn regtest_is_reported_as_local() {
        assert_eq!(display_name(Network::Regtest), "local");
        assert_eq!(network_db_name(Network::Regtest), "det-regtest.sqlite");
        assert_eq!(network_db_name(Network::Testnet), "det-testnet.sqlite");
    }

    #[test]
    fn a_panicking_boot_is_a_failure_even_on_exit_zero() {
        let run = CliRun {
            command: "det-cli --standalone core-wallets-list".to_string(),
            exit_code: Some(0),
            stdout: "{\"wallets\":[]}".to_string(),
            stderr: "thread 'main' panicked at src/lib.rs:1:1".to_string(),
            timed_out: false,
        };
        let error = check_boot(&run, "first boot").expect_err("a panic must fail the boot");
        assert!(error.contains("panicked at"), "{error}");
    }

    #[test]
    fn incompatible_wallet_data_fails_rather_than_falling_back() {
        let run = CliRun {
            command: "det-cli --standalone core-wallets-list".to_string(),
            exit_code: Some(1),
            stdout: String::new(),
            stderr: "Error: Your wallet data is not compatible with this version of the app"
                .to_string(),
            timed_out: false,
        };
        let error = check_boot(&run, "first boot").expect_err("an incompatibility must fail");
        assert!(error.contains("did not complete"), "{error}");
    }

    #[test]
    fn wallet_identifiers_fall_back_to_the_seed_hash() {
        let run = CliRun {
            command: "det-cli --standalone core-wallets-list".to_string(),
            exit_code: Some(0),
            stdout: r#"{"wallets":[{"seed_hash":"ab01","alias":"savings"},
                                   {"seed_hash":"cd02","alias":null}]}"#
                .to_string(),
            stderr: String::new(),
            timed_out: false,
        };

        let ids = check_wallets(&["savings".to_string()], &run).expect("expected alias present");
        assert_eq!(ids, ["savings", "cd02"]);

        let error =
            check_wallets(&["pension".to_string()], &run).expect_err("a missing wallet must fail");
        assert!(error.contains("pension"), "{error}");
    }

    #[test]
    fn the_reported_network_must_match_the_fixture() {
        let run = CliRun {
            command: "det-cli --standalone network-info".to_string(),
            exit_code: Some(0),
            stdout: r#"{"active":"testnet","available":["testnet"]}"#.to_string(),
            stderr: String::new(),
            timed_out: false,
        };

        check_network(Network::Testnet, &run).expect("matching network passes");
        let error = check_network(Network::Mainnet, &run).expect_err("a drifted network must fail");
        assert!(
            error.contains("mainnet") && error.contains("testnet"),
            "{error}"
        );
    }

    fn run(exit_code: Option<i32>, stderr: &str) -> CliRun {
        CliRun {
            command: "det-cli --standalone core-wallets-list".to_string(),
            exit_code,
            stdout: String::new(),
            stderr: stderr.to_string(),
            timed_out: false,
        }
    }

    #[test]
    fn a_needs_desktop_boot_must_stop_at_exactly_that_error() {
        let expected = format!("WARN rmcp::service: response error data: {NEEDS_DESKTOP_MARKER}");
        check_needs_desktop(&run(Some(1), &expected), "boot").expect("the documented outcome");

        let completed = check_needs_desktop(&run(Some(0), ""), "boot")
            .expect_err("completing must fail: the protected wallet cannot migrate headless");
        assert!(completed.contains("completed"), "{completed}");

        let other = check_needs_desktop(&run(Some(1), "StorageUpdateNeedsDesktop"), "boot")
            .expect_err("a different failure must not pass for the documented one");
        assert!(other.contains("not with"), "{other}");

        let panicked = check_needs_desktop(
            &run(Some(101), &format!("panicked at x\n{NEEDS_DESKTOP_MARKER}")),
            "boot",
        )
        .expect_err("a panic is never the expected outcome");
        assert!(panicked.contains("panicked at"), "{panicked}");
    }

    fn fixture_wallet(alias: &str, outcome: WalletOutcome) -> ExpectedWallet {
        ExpectedWallet {
            alias: alias.to_string(),
            outcome,
        }
    }

    #[test]
    fn a_printed_password_fails_without_being_quoted() {
        let password = "fixture-password-canary";
        let clean = CliRun {
            command: "det-cli --standalone app-storage-update --password-file /x".to_string(),
            exit_code: Some(0),
            stdout: "{\"migration\":{\"state\":\"success\"}}".to_string(),
            stderr: "INFO rmcp: serving".to_string(),
            timed_out: false,
        };
        check_password_not_echoed(&clean, password).expect("nothing printed");

        let leaked = CliRun {
            stderr: format!("DEBUG received request arguments={password}"),
            ..clean
        };
        let error = check_password_not_echoed(&leaked, password).expect_err("printed");
        assert!(error.contains("on stderr"), "{error}");
        assert!(
            !error.contains(password),
            "the failure must not quote it: {error}"
        );
    }

    #[test]
    fn wallet_outcomes_are_checked_per_wallet() {
        let expected = [
            fixture_wallet("plain", WalletOutcome::Migrated),
            fixture_wallet("locked", WalletOutcome::NeedsDesktop),
        ];
        let as_expected =
            BTreeMap::from([("plain".to_string(), true), ("locked".to_string(), false)]);
        check_wallet_outcomes(&expected, &as_expected).expect("both as the manifest says");

        let both = BTreeMap::from([("plain".to_string(), true), ("locked".to_string(), true)]);
        let error =
            check_wallet_outcomes(&expected, &both).expect_err("a protected wallet registered");
        assert!(error.contains("`locked` is password-protected"), "{error}");

        let neither = BTreeMap::from([("plain".to_string(), false), ("locked".to_string(), false)]);
        let error =
            check_wallet_outcomes(&expected, &neither).expect_err("the plain wallet missing");
        assert!(error.contains("`plain` was not registered"), "{error}");

        let error =
            check_wallet_outcomes(&expected, &BTreeMap::new()).expect_err("unknown aliases");
        assert!(error.contains("is not a wallet in"), "{error}");
    }

    #[test]
    fn an_unfinished_update_must_not_record_the_completion_sentinel() {
        check_sentinel_absent(&BTreeMap::new(), Network::Testnet).expect("absent is correct");
        let recorded = BTreeMap::from([(sentinel_key_for(Network::Testnet), vec![1])]);
        check_sentinel_absent(&recorded, Network::Testnet).expect_err("recorded is a regression");
    }

    /// BIP32 test vector 1, master key: the 78-byte serialization `data.db`
    /// stores must encode to the exact text the upstream store keeps.
    #[test]
    fn a_legacy_account_xpub_encodes_to_its_base58check_text() {
        let serialized = hex::decode(
            "0488b21e000000000000000000873dff81c02f525623fd1fe5167eac3a55a049de3d314bb42ee227ffed37d508\
             0339a36013301597daef41fbe593a02cc513d0b55527ec2df1050e2e8ff49c85c2",
        )
        .expect("test vector hex");
        assert_eq!(
            base58::encode_check(&serialized),
            "xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8YtGqsefD265TMg7usUDFdp6W1EGMcet8"
        );
    }

    /// The wallet the upstream store registered is found by its account xpub,
    /// under the length-prefixed text encoding the store actually uses.
    #[test]
    fn legacy_wallets_are_matched_to_registrations_by_account_xpub() {
        let dir = tempfile::tempdir().expect("tempdir");
        let scratch = dir.path().join("scratch");
        let data_db = dir.path().join(DATA_DB);
        let network_db = dir.path().join(network_db_name(Network::Testnet));
        let plain_xpub = vec![0x04, 0x35, 0x87, 0xcf, 1, 2, 3];
        let locked_xpub = vec![0x04, 0x35, 0x87, 0xcf, 9, 9, 9];

        let legacy = Connection::open(&data_db).expect("create data.db");
        legacy
            .execute_batch(
                "CREATE TABLE wallet (alias TEXT, master_ecdsa_bip44_account_0_epk BLOB NOT NULL);",
            )
            .expect("create wallet table");
        for (alias, xpub) in [("plain", &plain_xpub), ("locked", &locked_xpub)] {
            legacy
                .execute(
                    "INSERT INTO wallet (alias, master_ecdsa_bip44_account_0_epk) VALUES (?1, ?2)",
                    rusqlite::params![alias, xpub],
                )
                .expect("insert legacy wallet");
        }

        let store = Connection::open(&network_db).expect("create network db");
        store
            .execute_batch(
                "CREATE TABLE account_registrations (account_type TEXT, account_index INTEGER, \
                 account_xpub_bytes BLOB NOT NULL);",
            )
            .expect("create registrations");
        let text = base58::encode_check(&plain_xpub);
        let mut stored = (text.len() as u32).to_be_bytes().to_vec();
        stored.extend_from_slice(text.as_bytes());
        store
            .execute(
                "INSERT INTO account_registrations VALUES ('standard_bip44', 0, ?1)",
                [stored],
            )
            .expect("register the plain wallet");
        drop((legacy, store));

        let registered = legacy_wallet_registrations(&data_db, &network_db, &scratch, "t")
            .expect("read registrations")
            .expect("data.db exists");
        assert_eq!(
            registered,
            BTreeMap::from([("locked".to_string(), false), ("plain".to_string(), true)])
        );

        let absent =
            legacy_wallet_registrations(&dir.path().join("missing.db"), &network_db, &scratch, "t")
                .expect("no data.db is not an error");
        assert!(absent.is_none());
    }

    #[test]
    fn byte_identity_reports_the_size_change() {
        check_bytes_unchanged(Some(&vec![1, 2]), Some(&vec![1, 2]), DATA_DB).expect("identical");
        let error = check_bytes_unchanged(Some(&vec![1, 2]), Some(&vec![1, 2, 3]), DATA_DB)
            .expect_err("changed bytes must fail");
        assert!(error.contains("2 to 3 bytes"), "{error}");
        let error = check_bytes_unchanged(Some(&vec![1, 2]), Some(&vec![1, 3]), DATA_DB)
            .expect_err("a same-size rewrite must fail too");
        assert!(error.contains("content changed"), "{error}");
    }

    #[test]
    fn a_real_sqlite_file_round_trips_through_a_scratch_copy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join(DATA_DB);
        let conn = Connection::open(&db).expect("create db");
        conn.execute_batch(
            "CREATE TABLE settings (id INTEGER PRIMARY KEY, database_version INTEGER);
             INSERT INTO settings (id, database_version) VALUES (1, 11);",
        )
        .expect("seed schema");
        drop(conn);

        let scratch = dir.path().join("scratch");
        let snapshot = schema_snapshot(&db, &scratch, "before")
            .expect("snapshot")
            .expect("the file exists");
        assert_eq!(snapshot.version, Some(11));
        assert!(
            snapshot
                .objects
                .iter()
                .any(|(_, name, _)| name == "settings")
        );

        assert!(
            schema_snapshot(&dir.path().join("absent.db"), &scratch, "missing")
                .expect("a missing file is not an error")
                .is_none()
        );
    }

    #[test]
    fn sentinels_and_identities_are_read_from_the_upstream_kv_tables() {
        let dir = tempfile::tempdir().expect("tempdir");
        let app_db = dir.path().join(APP_DB);
        let conn = Connection::open(&app_db).expect("create app db");
        conn.execute_batch("CREATE TABLE meta_global (key TEXT PRIMARY KEY, value BLOB);")
            .expect("create meta_global");
        let key = sentinel_key_for(Network::Testnet);
        conn.execute(
            "INSERT INTO meta_global (key, value) VALUES (?1, X'0102')",
            [&key],
        )
        .expect("write sentinel");
        conn.execute(
            "INSERT INTO meta_global (key, value) VALUES ('det:settings:v1', X'03')",
            [],
        )
        .expect("write unrelated key");
        drop(conn);

        let scratch = dir.path().join("scratch");
        let sentinels = migration_sentinels(&app_db, &scratch, "after").expect("read sentinels");
        assert_eq!(sentinels.get(&key), Some(&vec![1u8, 2]));
        assert_eq!(sentinels.len(), 1, "only migration keys are collected");
        check_sentinel_recorded(&sentinels, Network::Testnet).expect("the sentinel is recorded");

        let network_db = dir.path().join(network_db_name(Network::Testnet));
        let conn = Connection::open(&network_db).expect("create network db");
        conn.execute_batch(
            "CREATE TABLE meta_identity (identity_id BLOB, key TEXT, value BLOB);
             INSERT INTO meta_identity (identity_id, key, value) VALUES (X'AABB', 'det:identity:v1', X'00');",
        )
        .expect("seed identity");
        drop(conn);

        let ids = identity_ids(&network_db, &scratch, "after").expect("read identities");
        check_identities(&["AABB".to_string()], &ids).expect("case-insensitive match");
        let error = check_identities(&["ccdd".to_string()], &ids)
            .expect_err("a dropped identity must fail");
        assert!(error.contains("ccdd"), "{error}");
    }

    #[test]
    fn backup_files_are_found_below_the_data_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("spv/testnet")).expect("create nested dir");
        fs::write(
            dir.path().join("det-app.platform-67d4ef3-backup-1.sqlite"),
            b"x",
        )
        .expect("write backup");
        fs::write(dir.path().join("spv/testnet/notes.bak"), b"yy").expect("write nested backup");
        fs::write(dir.path().join(DATA_DB), b"zzz").expect("write live db");

        let found = backup_files(dir.path()).expect("scan");
        assert_eq!(found.len(), 2, "found {found:?}");
        assert_eq!(found.get("spv/testnet/notes.bak"), Some(&2));
    }
}
