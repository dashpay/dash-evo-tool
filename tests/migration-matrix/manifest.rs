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
    /// Demand that an already-current `data.db` is byte-identical after the
    /// boot. Off by default: a normal boot legitimately writes rows, so the
    /// default no-migration assertion is "schema unchanged" instead.
    pub data_db_byte_identical: bool,
}

impl Default for Expectations {
    fn default() -> Self {
        Self {
            wallet_aliases: Vec::new(),
            identity_ids: Vec::new(),
            starting_db_version: None,
            finish_unwire_sentinel: true,
            derive_address: true,
            data_db_byte_identical: false,
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
        assert!(
            !fixture.expect.data_db_byte_identical,
            "strict byte identity is opt-in"
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
