//! Serde model for `tests/migration-fixtures/manifest.json`.
//!
//! The manifest is the only permanent pointer to a fixture: the archives
//! themselves live as GitHub Actions build artifacts with a finite retention,
//! so the committed manifest records which run produced which fixture.
//!
//! Every field except `id` and `network` is optional and defaulted. The
//! capture tooling and this harness ship in separate changes, so an unknown
//! field must never fail the parse — `deny_unknown_fields` is deliberately
//! absent and new keys are additive.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use dash_sdk::dpp::dashcore::Network;
use serde::Deserialize;

/// Manifest file name inside the fixtures directory.
pub const MANIFEST_FILE: &str = "manifest.json";

/// Overrides the manifest path when the file lives outside the fixtures dir.
pub const MANIFEST_PATH_ENV: &str = "MIGRATION_FIXTURES_MANIFEST";

#[derive(Debug, Deserialize)]
pub struct Manifest {
    /// Manifest schema revision. Informational — the parse is
    /// version-tolerant by construction.
    #[serde(default)]
    pub schema_version: u32,
    pub fixtures: Vec<Fixture>,
}

/// One captured data directory: which DET build wrote it, on which network,
/// and what the harness should be able to observe after booting it.
#[derive(Debug, Deserialize)]
pub struct Fixture {
    /// Stable identifier, also the default archive/directory name.
    pub id: String,
    /// Network the profile was captured on (`mainnet` / `testnet` / `devnet`
    /// / `local`). Drives the sentinel key and the `network-info` assertion.
    pub network: String,
    #[serde(default)]
    pub git_tag: String,
    #[serde(default)]
    pub det_version: String,
    /// How the profile was produced (`gui`, `det-cli`, …). Informational.
    #[serde(default)]
    pub capture_method: String,
    /// Profile flavour (`basic`, `password-protected`, …). Informational.
    #[serde(default)]
    pub profile: String,
    #[serde(default)]
    pub captured_at: String,
    #[serde(default, alias = "artifacts")]
    pub artifact: Artifact,
    #[serde(default, alias = "expectations")]
    pub expect: Expectations,
    #[serde(default)]
    pub contents: Contents,
    /// Extra boots of the same fixture with the wallet password supplied
    /// non-interactively, each on a freshly staged copy.
    #[serde(default)]
    pub password_runs: Vec<PasswordRun>,
}

/// What the capture put into the data dir, as far as the harness asserts on
/// it.
#[derive(Debug, Default, Deserialize)]
pub struct Contents {
    #[serde(default)]
    pub wallets: Vec<FixtureWallet>,
}

/// One wallet in the captured `data.db`, found there by its alias.
#[derive(Deserialize)]
pub struct FixtureWallet {
    pub alias: String,
    /// What a boot without a supplied password must do with the wallet.
    #[serde(default)]
    pub expected_outcome: WalletOutcome,
    /// The wallet's password: a public, testnet-only fixture password that a
    /// [`PasswordRun`] hands det-cli through `--password-file`.
    #[serde(default)]
    pub password: Option<String>,
}

// Hand-written so the fixture password never lands in a failure report: the
// matrix asserts det-cli never prints it, and the harness keeps the same rule.
impl std::fmt::Debug for FixtureWallet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FixtureWallet")
            .field("alias", &self.alias)
            .field("expected_outcome", &self.expected_outcome)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// What a headless boot must do with one wallet. An unrecognised value fails
/// the parse: guessing an outcome would make the matrix assert the wrong thing.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WalletOutcome {
    /// Registered in the per-network wallet store by the boot.
    #[default]
    Migrated,
    /// Password-protected, and no password is supplied. det-cli never prompts,
    /// so the boot fails with `StorageUpdateNeedsDesktop` and the wallet stays
    /// unregistered until its password is supplied or the desktop app finishes
    /// the storage update.
    NeedsDesktop,
}

/// A boot of the fixture with the wallet password supplied non-interactively.
#[derive(Debug, Deserialize)]
pub struct PasswordRun {
    /// How det-cli receives the password.
    pub source: PasswordSource,
    /// Per-alias outcomes that differ from the wallet's `expected_outcome`.
    #[serde(default)]
    pub expected_outcomes: BTreeMap<String, WalletOutcome>,
}

/// How a [`PasswordRun`] hands det-cli the password. An unrecognised value
/// fails the parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasswordSource {
    /// `app-storage-update --password-file <owner-only file>`.
    File,
}

/// What one wallet must end up as in one scenario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedWallet {
    pub alias: String,
    pub outcome: WalletOutcome,
}

/// One staged boot sequence of a fixture. Deliberately not `Debug`: it holds
/// the fixture password.
pub struct Scenario {
    pub label: String,
    /// Supplied through `--password-file`; `None` for the boot without one.
    pub password: Option<String>,
    pub wallets: Vec<ExpectedWallet>,
    /// Aliases a completed boot must list.
    pub listed_aliases: Vec<String>,
}

impl Scenario {
    /// Whether this boot must stop at `StorageUpdateNeedsDesktop` rather than
    /// complete.
    pub fn needs_desktop(&self) -> bool {
        self.wallets
            .iter()
            .any(|wallet| wallet.outcome == WalletOutcome::NeedsDesktop)
    }
}

/// Where the fixture bytes came from. `archive` is the only field the harness
/// reads; the rest exist so a human (or a re-fetch script) can find the
/// artifact again after the local copy is gone.
#[derive(Debug, Default, Deserialize)]
pub struct Artifact {
    /// Archive (or directory) name inside the fixtures directory.
    /// `archive_filename` is the key tests/migration-fixtures/README.md
    /// documents and the capture tooling writes.
    #[serde(default, alias = "archive_filename", alias = "file", alias = "path")]
    pub archive: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub artifact_name: Option<String>,
    #[serde(default)]
    pub workflow_run_id: Option<u64>,
    #[serde(default)]
    pub retention_days: Option<u32>,
}

/// What the harness must observe after booting this fixture.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Expectations {
    /// Wallet aliases that must appear in `core-wallets-list`.
    pub wallet_aliases: Vec<String>,
    /// Lowercase hex identity ids that must survive the migration.
    pub identity_ids: Vec<String>,
    /// `data.db` schema version at capture time. Cross-checked against the
    /// staged file; the file itself is authoritative when the two disagree
    /// only in that the manifest is silent.
    pub starting_db_version: Option<u16>,
    /// Whether the boot must record the `finish_unwire` completion sentinel.
    pub finish_unwire_sentinel: bool,
    /// Whether to derive a receive address. Needs a synced SPV chain, so a
    /// fixture captured for an offline-only check can opt out.
    pub derive_address: bool,
}

impl Default for Expectations {
    fn default() -> Self {
        Self {
            wallet_aliases: Vec::new(),
            identity_ids: Vec::new(),
            starting_db_version: None,
            finish_unwire_sentinel: true,
            derive_address: true,
        }
    }
}

impl Artifact {
    /// Where the bytes came from, for a failure report: a local archive name
    /// is useless once the CI artifact has expired, the run id is not.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(name) = &self.artifact_name {
            parts.push(format!("artifact {name}"));
        }
        if let Some(run) = self.workflow_run_id {
            parts.push(format!("run {run}"));
        }
        if let Some(days) = self.retention_days {
            parts.push(format!("retention {days}d"));
        }
        match parts.is_empty() {
            true => "no artifact pointer".to_string(),
            false => parts.join(", "),
        }
    }
}

impl Fixture {
    /// One-line provenance, printed before each fixture runs so a failing CI
    /// log says which capture it was looking at.
    pub fn describe(&self) -> String {
        let field = |label: &str, value: &str| match value.is_empty() {
            true => String::new(),
            false => format!(" {label}={value}"),
        };
        format!(
            "{} network={}{}{}{}{}{} [{}]",
            self.id,
            self.network,
            field("tag", &self.git_tag),
            field("version", &self.det_version),
            field("capture", &self.capture_method),
            field("profile", &self.profile),
            field("captured", &self.captured_at),
            self.artifact.describe(),
        )
    }

    /// The boots the harness runs for this fixture: always one without a
    /// password, then one per [`PasswordRun`].
    ///
    /// # Errors
    ///
    /// A password run the fixture cannot honour: no wallet declares a
    /// password, the wallets declare different ones (one supplied password
    /// cannot open them all), an override names an unknown alias, or an
    /// override expects `needs_desktop` — with the password supplied, the
    /// update either opens every protected wallet or fails.
    pub fn scenarios(&self) -> Result<Vec<Scenario>, String> {
        let wallets = &self.contents.wallets;
        let mut scenarios = vec![
            self.scenario(
                "no password",
                None,
                wallets
                    .iter()
                    .map(|wallet| ExpectedWallet {
                        alias: wallet.alias.clone(),
                        outcome: wallet.expected_outcome,
                    })
                    .collect(),
            ),
        ];

        for run in &self.password_runs {
            let label = match run.source {
                PasswordSource::File => "password file",
            };
            let password = self.shared_password(label)?;
            if let Some(alias) = run
                .expected_outcomes
                .keys()
                .find(|alias| !wallets.iter().any(|wallet| &wallet.alias == *alias))
            {
                return Err(format!(
                    "fixture '{}': the {label} run names `{alias}`, which is not in contents.wallets",
                    self.id
                ));
            }
            let expected: Vec<ExpectedWallet> = wallets
                .iter()
                .map(|wallet| ExpectedWallet {
                    alias: wallet.alias.clone(),
                    outcome: run
                        .expected_outcomes
                        .get(&wallet.alias)
                        .copied()
                        .unwrap_or(wallet.expected_outcome),
                })
                .collect();
            if let Some(wallet) = expected
                .iter()
                .find(|wallet| wallet.outcome == WalletOutcome::NeedsDesktop)
            {
                return Err(format!(
                    "fixture '{}': the {label} run expects `{}` to need the desktop app, but with \
                     the password supplied the update either opens every protected wallet or fails",
                    self.id, wallet.alias
                ));
            }
            scenarios.push(self.scenario(label, Some(password), expected));
        }
        Ok(scenarios)
    }

    fn scenario(
        &self,
        label: &str,
        password: Option<String>,
        wallets: Vec<ExpectedWallet>,
    ) -> Scenario {
        let mut listed_aliases = self.expect.wallet_aliases.clone();
        for wallet in &wallets {
            if wallet.outcome == WalletOutcome::Migrated && !listed_aliases.contains(&wallet.alias)
            {
                listed_aliases.push(wallet.alias.clone());
            }
        }
        Scenario {
            label: label.to_owned(),
            password,
            wallets,
            listed_aliases,
        }
    }

    /// The one password every password-protected wallet shares. Errors never
    /// quote it.
    fn shared_password(&self, label: &str) -> Result<String, String> {
        let passwords: BTreeSet<&str> = self
            .contents
            .wallets
            .iter()
            .filter_map(|wallet| wallet.password.as_deref())
            .collect();
        let mut distinct = passwords.into_iter();
        match (distinct.next(), distinct.next()) {
            (Some(password), None) if !password.is_empty() => Ok(password.to_owned()),
            (Some(_), None) => Err(format!(
                "fixture '{}': the {label} run needs a non-empty wallet `password`",
                self.id
            )),
            (None, _) => Err(format!(
                "fixture '{}': the {label} run needs a wallet `password` in contents.wallets",
                self.id
            )),
            (Some(_), Some(_)) => Err(format!(
                "fixture '{}': the {label} run supplies one password, but the wallets declare different ones",
                self.id
            )),
        }
    }

    /// Parsed network, matching the spelling `network_info` reports
    /// (`local` for regtest).
    pub fn network(&self) -> Result<Network, String> {
        match self.network.to_ascii_lowercase().as_str() {
            "mainnet" | "dash" => Ok(Network::Mainnet),
            "testnet" => Ok(Network::Testnet),
            "devnet" => Ok(Network::Devnet),
            "local" | "regtest" => Ok(Network::Regtest),
            other => Err(format!(
                "fixture '{}' declares unknown network '{other}' (expected mainnet, testnet, devnet or local)",
                self.id
            )),
        }
    }
}

/// Resolves the manifest path for `fixtures_dir`, honoring
/// [`MANIFEST_PATH_ENV`].
pub fn manifest_path(fixtures_dir: &Path) -> PathBuf {
    match std::env::var(MANIFEST_PATH_ENV) {
        Ok(path) if !path.trim().is_empty() => PathBuf::from(path),
        _ => fixtures_dir.join(MANIFEST_FILE),
    }
}

/// Reads and parses the fixture manifest. A missing or malformed manifest is
/// a hard error: the matrix only runs when a workflow explicitly points it at
/// fixtures, and silently testing nothing is exactly the failure mode this
/// harness exists to prevent.
pub fn load(fixtures_dir: &Path) -> Result<Manifest, String> {
    let path = manifest_path(fixtures_dir);
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("could not read fixture manifest {}: {e}", path.display()))?;
    serde_json::from_str(&raw)
        .map_err(|e| format!("could not parse fixture manifest {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_fields_and_absent_blocks_stay_parseable() {
        let manifest: Manifest = serde_json::from_str(
            r#"{
                "schema_version": 1,
                "generated_by": "a field this harness has never heard of",
                "fixtures": [
                    { "id": "v0.9.3-testnet", "network": "testnet", "future_key": [1, 2] }
                ]
            }"#,
        )
        .expect("a forward-compatible manifest must parse");

        let fixture = &manifest.fixtures[0];
        assert_eq!(fixture.network().expect("known network"), Network::Testnet);
        assert!(fixture.expect.wallet_aliases.is_empty());
        assert!(
            fixture.expect.finish_unwire_sentinel && fixture.expect.derive_address,
            "the sentinel and address checks default to on"
        );
        assert!(fixture.artifact.archive.is_none());
    }

    #[test]
    fn expectations_and_artifact_pointers_are_read() {
        let manifest: Manifest = serde_json::from_str(
            r#"{
                "fixtures": [{
                    "id": "v0.9.3-testnet",
                    "network": "testnet",
                    "git_tag": "v0.9.3",
                    "det_version": "0.9.3",
                    "capture_method": "gui",
                    "profile": "basic",
                    "captured_at": "2026-09-10T00:00:00Z",
                    "artifacts": { "file": "v0.9.3-testnet.tar.zst", "workflow_run_id": 42 },
                    "expectations": {
                        "wallet_aliases": ["public-test-wallet"],
                        "starting_db_version": 11,
                        "derive_address": false
                    }
                }]
            }"#,
        )
        .expect("manifest with aliases must parse");

        let fixture = &manifest.fixtures[0];
        assert_eq!(
            fixture.artifact.archive.as_deref(),
            Some("v0.9.3-testnet.tar.zst")
        );
        assert_eq!(fixture.artifact.workflow_run_id, Some(42));
        assert_eq!(fixture.expect.starting_db_version, Some(11));
        assert_eq!(fixture.expect.wallet_aliases, ["public-test-wallet"]);
        assert!(!fixture.expect.derive_address);
        assert!(
            fixture.expect.finish_unwire_sentinel,
            "an unlisted expectation keeps its default"
        );
    }

    /// Reads the committed manifest rather than a hand-written sample, so the
    /// key names the capture tooling writes and the ones this reader expects
    /// cannot drift apart unnoticed. A pointer that fails to deserialize is
    /// silently `None` (every field is defaulted), which would send the
    /// stager hunting for `<id>.tar.zst` instead of the archive CI downloaded.
    #[test]
    fn the_committed_manifest_resolves_every_artifact_pointer() {
        let manifest: Manifest =
            serde_json::from_str(include_str!("../migration-fixtures/manifest.json"))
                .expect("the committed manifest must parse");
        assert!(
            !manifest.fixtures.is_empty(),
            "the committed manifest lists no fixtures"
        );
        for fixture in &manifest.fixtures {
            let artifact = &fixture.artifact;
            assert!(
                artifact.archive.is_some(),
                "fixture '{}' has no archive file name the stager can resolve",
                fixture.id
            );
            assert!(
                artifact.sha256.is_some(),
                "fixture '{}' has no sha256 to verify the download against",
                fixture.id
            );
            assert!(
                artifact.artifact_name.is_some(),
                "fixture '{}' has no artifact name",
                fixture.id
            );
        }
    }

    #[test]
    fn wallet_outcomes_default_to_migrated_and_reject_unknown_values() {
        let fixture: Fixture = serde_json::from_str(
            r#"{ "id": "f", "network": "testnet", "contents": { "wallets": [
                { "alias": "plain" },
                { "alias": "locked", "expected_outcome": "needs_desktop" }
            ] } }"#,
        )
        .expect("parse");
        assert_eq!(
            fixture.contents.wallets[0].expected_outcome,
            WalletOutcome::Migrated
        );
        let scenarios = fixture.scenarios().expect("no password runs to validate");
        assert_eq!(scenarios.len(), 1, "only the boot without a password");
        assert!(scenarios[0].needs_desktop());
        assert!(scenarios[0].password.is_none());
        assert_eq!(scenarios[0].listed_aliases, ["plain"]);

        let unknown = serde_json::from_str::<Fixture>(
            r#"{ "id": "f", "network": "testnet", "contents": { "wallets": [
                { "alias": "x", "expected_outcome": "maybe" }
            ] } }"#,
        );
        assert!(unknown.is_err(), "an unknown outcome must not parse");
    }

    fn outcomes(scenario: &Scenario) -> Vec<(&str, WalletOutcome)> {
        scenario
            .wallets
            .iter()
            .map(|wallet| (wallet.alias.as_str(), wallet.outcome))
            .collect()
    }

    /// The committed v0.9.3 entry runs twice. Without a password the plain
    /// wallet migrates and the protected one needs the desktop app; with the
    /// password supplied through `--password-file`, both migrate.
    #[test]
    fn the_committed_v093_fixture_runs_without_and_with_the_password() {
        let manifest: Manifest =
            serde_json::from_str(include_str!("../migration-fixtures/manifest.json"))
                .expect("the committed manifest must parse");
        let fixture = manifest
            .fixtures
            .iter()
            .find(|fixture| fixture.id == "v0.9.3-wallet-only")
            .expect("the v0.9.3 baseline entry");
        let scenarios = fixture.scenarios().expect("the committed runs are valid");
        assert_eq!(scenarios.len(), 2);

        let [without, with] = &scenarios[..] else {
            unreachable!("length checked above");
        };
        assert!(without.password.is_none() && without.needs_desktop());
        assert_eq!(
            outcomes(without),
            [
                ("migration-fixture-v093", WalletOutcome::Migrated),
                (
                    "migration-fixture-v093-protected",
                    WalletOutcome::NeedsDesktop
                ),
            ]
        );

        assert!(with.password.is_some() && !with.needs_desktop());
        assert_eq!(
            outcomes(with),
            [
                ("migration-fixture-v093", WalletOutcome::Migrated),
                ("migration-fixture-v093-protected", WalletOutcome::Migrated),
            ]
        );
        assert_eq!(
            with.listed_aliases,
            ["migration-fixture-v093", "migration-fixture-v093-protected"]
        );
    }

    fn fixture_with_runs(wallets: &str, runs: &str) -> Fixture {
        serde_json::from_str(&format!(
            r#"{{ "id": "f", "network": "testnet",
                  "contents": {{ "wallets": [{wallets}] }},
                  "password_runs": [{runs}] }}"#
        ))
        .expect("parse")
    }

    #[test]
    fn a_password_run_the_fixture_cannot_honour_is_rejected() {
        let file_run = r#"{ "source": "file", "expected_outcomes": { "locked": "migrated" } }"#;

        let no_password = fixture_with_runs(
            r#"{ "alias": "locked", "expected_outcome": "needs_desktop" }"#,
            file_run,
        );
        let error = no_password.scenarios().err().expect("no password declared");
        assert!(error.contains("needs a wallet `password`"), "{error}");

        let different = fixture_with_runs(
            r#"{ "alias": "locked", "password": "one-password" },
               { "alias": "other", "password": "another-password" }"#,
            file_run,
        );
        let error = different.scenarios().err().expect("different passwords");
        assert!(error.contains("declare different ones"), "{error}");
        assert!(
            !error.contains("one-password") && !error.contains("another-password"),
            "an error must never quote a password: {error}"
        );

        let unknown_alias = fixture_with_runs(
            r#"{ "alias": "other", "password": "one-password" }"#,
            file_run,
        );
        let error = unknown_alias.scenarios().err().expect("unknown alias");
        assert!(error.contains("`locked`"), "{error}");

        let still_locked = fixture_with_runs(
            r#"{ "alias": "locked", "expected_outcome": "needs_desktop", "password": "one-password" }"#,
            r#"{ "source": "file" }"#,
        );
        let error = still_locked
            .scenarios()
            .err()
            .expect("needs_desktop with a password");
        assert!(
            error.contains("either opens every protected wallet"),
            "{error}"
        );

        let unknown_source = serde_json::from_str::<Fixture>(
            r#"{ "id": "f", "network": "testnet", "password_runs": [{ "source": "env" }] }"#,
        );
        assert!(unknown_source.is_err(), "an unknown source must not parse");
    }

    #[test]
    fn a_fixture_wallet_never_prints_its_password() {
        let fixture = fixture_with_runs(
            r#"{ "alias": "locked", "password": "fixture-password-canary" }"#,
            "",
        );
        let debug = format!("{fixture:?}");
        assert!(!debug.contains("fixture-password-canary"), "{debug}");
        assert!(debug.contains("locked"), "{debug}");
    }

    #[test]
    fn an_unknown_network_names_the_fixture() {
        let fixture: Fixture =
            serde_json::from_str(r#"{ "id": "odd", "network": "mainnnet" }"#).expect("parse");
        let error = fixture
            .network()
            .expect_err("unknown network must be rejected");
        assert!(
            error.contains("odd") && error.contains("mainnnet"),
            "{error}"
        );
    }
}
