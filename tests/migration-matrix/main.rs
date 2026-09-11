//! Cross-version data-migration matrix.
//!
//! Boots profiles captured by older DET releases with the binary built from
//! this tree, and asserts the upgrade lands them intact. The fixtures are
//! multi-megabyte captures of real profiles, stored as CI build artifacts
//! rather than in the repository, so the matrix only runs when a workflow
//! points `MIGRATION_FIXTURES_DIR` at an unpacked fixture set. Unset — the
//! normal `cargo test --all-features --workspace` case — it reports the skip
//! and passes, while the module-level unit tests below still exercise the
//! harness's own logic.
//!
//! What is deliberately *not* done here: booting `AppState` in-process. Under
//! the `testing` feature `AppState::boot_inputs` substitutes an in-memory
//! database, which would leave every migration assertion below testing
//! nothing. The harness spawns the real `det-cli` binary instead; see
//! [`cli`] for the boot path it drives.
//!
//! Each fixture boots once without a password and once per `password_runs`
//! manifest entry, every time from a freshly staged copy.
//!
//! Environment:
//!
//! | Variable | Effect |
//! |---|---|
//! | `MIGRATION_FIXTURES_DIR` | Unpacked fixtures + `manifest.json`. Unset ⇒ skip. |
//! | `MIGRATION_FIXTURES_MANIFEST` | Manifest path, when it lives elsewhere. |
//! | `MIGRATION_MATRIX_ONLY` | Comma-separated fixture ids to run. |
//! | `MIGRATION_MATRIX_SKIP_NETWORK` | Skip the checks needing a synced chain. |
//! | `MIGRATION_MATRIX_BOOT_TIMEOUT_SECS` | Per-boot wall clock (default 300). |
//! | `MIGRATION_MATRIX_NETWORK_TIMEOUT_SECS` | Chain-gated calls (default 900). |
//! | `DET_CLI_BIN` | Binary under test, instead of the one Cargo just built. |

mod assertions;
mod cli;
mod manifest;
mod stage;

use std::path::{Path, PathBuf};
use std::time::Duration;

use assertions::{APP_DB, DATA_DB};
use dash_evo_tool::database::DEFAULT_DB_VERSION;
use manifest::{ExpectedWallet, Fixture, Scenario};

const FIXTURES_DIR_ENV: &str = "MIGRATION_FIXTURES_DIR";
const ONLY_ENV: &str = "MIGRATION_MATRIX_ONLY";
const SKIP_NETWORK_ENV: &str = "MIGRATION_MATRIX_SKIP_NETWORK";
const BOOT_TIMEOUT_ENV: &str = "MIGRATION_MATRIX_BOOT_TIMEOUT_SECS";
const NETWORK_TIMEOUT_ENV: &str = "MIGRATION_MATRIX_NETWORK_TIMEOUT_SECS";

/// Generous next to the 60s the MCP layer waits for a cold-start migration,
/// tight enough that a wedged boot fails instead of hanging the job.
const DEFAULT_BOOT_TIMEOUT: Duration = Duration::from_secs(300);

/// Covers the 600s SPV gate plus the work that follows it.
const DEFAULT_NETWORK_TIMEOUT: Duration = Duration::from_secs(900);

/// Knobs resolved once per run.
struct Options {
    boot_timeout: Duration,
    network_timeout: Duration,
    skip_network: bool,
}

impl Options {
    fn from_env() -> Self {
        Self {
            boot_timeout: duration_from_env(BOOT_TIMEOUT_ENV, DEFAULT_BOOT_TIMEOUT),
            network_timeout: duration_from_env(NETWORK_TIMEOUT_ENV, DEFAULT_NETWORK_TIMEOUT),
            skip_network: flag(SKIP_NETWORK_ENV),
        }
    }
}

fn duration_from_env(name: &str, fallback: Duration) -> Duration {
    match std::env::var(name).ok().and_then(|v| v.trim().parse().ok()) {
        Some(seconds) => Duration::from_secs(seconds),
        None => fallback,
    }
}

fn flag(name: &str) -> bool {
    matches!(
        std::env::var(name).unwrap_or_default().trim(),
        "1" | "true" | "yes"
    )
}

/// Runs every fixture in the manifest, reporting all failures at once rather
/// than stopping at the first — one broken fixture should not hide the state
/// of the rest of the matrix.
#[test]
fn migration_matrix() {
    let Some(fixtures_dir) = fixtures_dir() else {
        println!(
            "{FIXTURES_DIR_ENV} is not set — skipping the cross-version migration matrix. \
             Set it to a directory holding {} plus the unpacked fixtures to run it.",
            manifest::MANIFEST_FILE
        );
        return;
    };

    let manifest = manifest::load(&fixtures_dir).unwrap_or_else(|error| panic!("{error}"));
    let selected = select(&manifest.fixtures);
    assert!(
        !selected.is_empty(),
        "no fixtures to run: {} lists {} fixture(s) and {ONLY_ENV} selects {:?}",
        manifest::manifest_path(&fixtures_dir).display(),
        manifest.fixtures.len(),
        std::env::var(ONLY_ENV).unwrap_or_default(),
    );

    let options = Options::from_env();
    println!(
        "Running {} migration fixture(s) from {} (manifest schema {}) against DEFAULT_DB_VERSION \
         {DEFAULT_DB_VERSION}",
        selected.len(),
        fixtures_dir.display(),
        manifest.schema_version,
    );

    let mut failures = Vec::new();
    for fixture in selected {
        println!("--- fixture {} ---", fixture.describe());
        match run_fixture(&fixtures_dir, fixture, &options) {
            Ok(()) => println!("--- fixture {} ok ---", fixture.id),
            Err(error) => {
                println!("--- fixture {} FAILED ---", fixture.id);
                failures.push(format!("[{}] {error}", fixture.id));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} of the migration fixtures failed:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

fn fixtures_dir() -> Option<PathBuf> {
    let dir = std::env::var(FIXTURES_DIR_ENV).ok()?;
    match dir.trim().is_empty() {
        true => None,
        false => Some(PathBuf::from(dir.trim())),
    }
}

/// Applies the `MIGRATION_MATRIX_ONLY` filter, if any.
fn select(fixtures: &[Fixture]) -> Vec<&Fixture> {
    let only = std::env::var(ONLY_ENV).unwrap_or_default();
    let wanted: Vec<&str> = only
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .collect();
    fixtures
        .iter()
        .filter(|fixture| wanted.is_empty() || wanted.contains(&fixture.id.as_str()))
        .collect()
}

/// Runs every scenario of one fixture — always the boot without a password,
/// then one per password run — each on a freshly staged copy, and reports
/// every failing scenario rather than stopping at the first.
fn run_fixture(fixtures_dir: &Path, fixture: &Fixture, options: &Options) -> Result<(), String> {
    let failures: Vec<String> = fixture
        .scenarios()?
        .iter()
        .filter_map(|scenario| {
            println!("  scenario: {}", scenario.label);
            run_scenario(fixtures_dir, fixture, scenario, options)
                .err()
                .map(|error| format!("scenario `{}`: {error}", scenario.label))
        })
        .collect();
    match failures.is_empty() {
        true => Ok(()),
        false => Err(failures.join("\n\n")),
    }
}

/// Stages one fixture, boots it twice, and checks what the upgrade produced.
/// A scenario with a password first finishes the storage update through
/// `app-storage-update --password-file`, as an operator automating an upgrade
/// would.
fn run_scenario(
    fixtures_dir: &Path,
    fixture: &Fixture,
    scenario: &Scenario,
    options: &Options,
) -> Result<(), String> {
    let network = fixture.network()?;
    let staged = stage::stage(fixtures_dir, fixture)?;
    let scratch = staged.scratch()?;
    let data_dir = staged.data_dir().to_path_buf();
    let data_db = data_dir.join(DATA_DB);
    let app_db = data_dir.join(APP_DB);
    let network_db = data_dir.join(assertions::network_db_name(network));

    let before = assertions::schema_snapshot(&data_db, &scratch, "before")?;
    let starting_version = before.as_ref().and_then(|snapshot| snapshot.version);
    if let (Some(declared), Some(found)) = (fixture.expect.starting_db_version, starting_version)
        && declared != found
    {
        return Err(format!(
            "manifest records a starting {DATA_DB} version of {declared}, the staged file is at {found}"
        ));
    }
    // Every boot path opens a pre-update data.db read-only, so its bytes are
    // part of the contract for every fixture that has one, not an opt-in.
    let before_bytes = assertions::file_bytes(&data_db)?;

    let cli = cli::DetCli::new(&staged)?;

    // Without its password, a fixture holding a password-protected wallet
    // cannot finish its storage update headless — det-cli never prompts — so
    // every wallet-gated boot must stop at exactly that error, not merely fail.
    let needs_desktop = scenario.needs_desktop();
    let check_wallet_boot = |run: &cli::CliRun, label: &str| match needs_desktop {
        true => assertions::check_needs_desktop(run, label),
        false => assertions::check_boot(run, label),
    };
    // Every run of a password scenario is checked for the password first, so
    // a leak is reported even when the run also fails for another reason.
    let check_not_echoed = |run: &cli::CliRun| match &scenario.password {
        Some(password) => assertions::check_password_not_echoed(run, password),
        None => Ok(()),
    };

    if let Some(password) = &scenario.password {
        let password_file = staged.write_password_file(password)?;
        let update = cli.storage_update(&password_file, options.boot_timeout)?;
        check_not_echoed(&update)?;
        assertions::check_boot(&update, "app-storage-update")?;
    }

    let first = cli.wallets_list(options.boot_timeout)?;
    check_not_echoed(&first)?;
    check_wallet_boot(&first, "the first boot")?;

    let after = assertions::schema_snapshot(&data_db, &scratch, "after")?;
    assertions::check_schema_outcome(before.as_ref(), after.as_ref())?;
    if before_bytes.is_some() {
        let after_bytes = assertions::file_bytes(&data_db)?;
        assertions::check_bytes_unchanged(before_bytes.as_ref(), after_bytes.as_ref(), DATA_DB)?;
    }

    let info = cli.network_info(options.boot_timeout)?;
    check_not_echoed(&info)?;
    assertions::check_boot(&info, "network-info")?;
    assertions::check_network(network, &info)?;

    let wallet_ids = match needs_desktop {
        // Wallet tools stay gated until the desktop app finishes the update,
        // so nothing is listed; the per-wallet check below reads storage.
        true => Vec::new(),
        false => assertions::check_wallets(&scenario.listed_aliases, &first)?,
    };
    check_wallet_outcomes(&scenario.wallets, &data_db, &network_db, &scratch, "after")?;
    assertions::check_identities(
        &fixture.expect.identity_ids,
        &assertions::identity_ids(&network_db, &scratch, "after")?,
    )?;

    let sentinels = assertions::migration_sentinels(&app_db, &scratch, "after")?;
    if needs_desktop {
        // An unfinished storage update must not claim completion.
        assertions::check_sentinel_absent(&sentinels, network)?;
    } else if fixture.expect.finish_unwire_sentinel {
        assertions::check_sentinel_recorded(&sentinels, network)?;
    }
    let backups = assertions::backup_files(&data_dir)?;

    // The second boot never gets the password: once the update finished, a
    // plain boot must need nothing more.
    let second = cli.wallets_list(options.boot_timeout)?;
    check_not_echoed(&second)?;
    check_wallet_boot(&second, "the second boot")?;
    if !needs_desktop {
        assertions::check_wallets(&scenario.listed_aliases, &second)?;
    }
    check_wallet_outcomes(
        &scenario.wallets,
        &data_db,
        &network_db,
        &scratch,
        "idempotent",
    )?;
    assertions::check_idempotent(
        (&backups, &sentinels),
        (
            &assertions::backup_files(&data_dir)?,
            &assertions::migration_sentinels(&app_db, &scratch, "idempotent")?,
        ),
    )?;

    // Last, because it is the only step that needs a reachable chain: a
    // failure here should not mask the offline evidence collected above.
    if needs_desktop {
        println!(
            "    address derivation skipped (wallet tools wait for the desktop app to finish the storage update)"
        );
    } else if fixture.expect.derive_address && !options.skip_network {
        for wallet_id in &wallet_ids {
            let label = format!("core-address-create for `{wallet_id}`");
            let run = cli.address_create(wallet_id, options.network_timeout)?;
            assertions::check_boot(&run, &label)?;
            let address = assertions::check_address(&run)?;
            println!("    {wallet_id}: derived {address}");
        }
    } else {
        println!("    address derivation skipped (needs a synced chain)");
    }

    Ok(())
}

/// Checks every wallet the manifest lists against the per-network store. Only
/// possible for a v0.9.3-era fixture, where `data.db` names each wallet's
/// account xpub; a fixture without one relies on the boot-level checks.
fn check_wallet_outcomes(
    wallets: &[ExpectedWallet],
    data_db: &Path,
    network_db: &Path,
    scratch: &Path,
    label: &str,
) -> Result<(), String> {
    if wallets.is_empty() {
        return Ok(());
    }
    match assertions::legacy_wallet_registrations(data_db, network_db, scratch, label)? {
        Some(registered) => assertions::check_wallet_outcomes(wallets, &registered),
        None => {
            println!("    per-wallet storage check skipped (no {DATA_DB} to name the wallets)");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(id: &str) -> Fixture {
        serde_json::from_str(&format!(r#"{{ "id": "{id}", "network": "testnet" }}"#))
            .expect("fixture json")
    }

    #[test]
    fn an_unset_filter_selects_every_fixture() {
        let fixtures = vec![fixture("v0.9.3-testnet"), fixture("v1.0.0-weekly")];
        assert_eq!(select(&fixtures).len(), 2);
    }

    #[test]
    fn timeouts_fall_back_to_the_defaults_on_junk_input() {
        assert_eq!(
            duration_from_env(
                "MIGRATION_MATRIX_TIMEOUT_THAT_IS_UNSET",
                DEFAULT_BOOT_TIMEOUT
            ),
            DEFAULT_BOOT_TIMEOUT
        );
    }

    #[test]
    fn an_absent_flag_is_off() {
        assert!(!flag("MIGRATION_MATRIX_FLAG_THAT_IS_UNSET"));
    }
}
