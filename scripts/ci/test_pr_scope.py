"""Required checks remain present without compiling documentation-only PRs."""

import json
import subprocess
import tempfile
import unittest
from pathlib import Path

import yaml

import pr_scope

ROOT = Path(__file__).resolve().parents[2]


class PrScopeTests(unittest.TestCase):
    def test_rust_and_build_configuration_require_both_suites(self):
        for path in (
            "build.rs",
            "src/main.rs",
            "Cargo.toml",
            "nested/Cargo.toml",
            "Cargo.lock",
            ".cargo/config.toml",
            "rust-toolchain.toml",
            "scripts/ci/pr_scope.py",
        ):
            for suite in ("tests", "clippy"):
                with self.subTest(path=path, suite=suite):
                    self.assertTrue(pr_scope.is_relevant([path], suite))

    def test_fixture_and_weekly_tooling_require_tests_but_not_clippy(self):
        for path in (
            "tests/migration-fixtures/README.md",
            "tests/backend-e2e/README.md",
            "scripts/ci/run-tests.sh",
            "scripts/migration-fixtures/capture.sh",
            "src/wallet_backend/platform_compatibility/fixtures/manifest.json",
            ".github/workflows/weekly-build.yml",
            ".github/workflows/migration-fixture-capture.yml",
        ):
            with self.subTest(path=path):
                self.assertTrue(pr_scope.is_relevant([path], "tests"))
                self.assertFalse(pr_scope.is_relevant([path], "clippy"))

    def test_documentation_only_skips_compilation(self):
        for suite in ("tests", "clippy"):
            self.assertFalse(
                pr_scope.is_relevant(["README.md", "docs/example.md"], suite)
            )

    def test_each_workflow_tests_its_own_changes(self):
        self.assertTrue(pr_scope.is_relevant([".github/workflows/tests.yml"], "tests"))
        self.assertTrue(
            pr_scope.is_relevant([".github/workflows/clippy.yml"], "clippy")
        )

    def test_pr_diff_includes_deleted_and_renamed_rust_files(self):
        with tempfile.TemporaryDirectory() as scratch:
            repo = Path(scratch)

            def git(*args):
                return subprocess.check_output(
                    ["git", "-C", str(repo), *args], text=True
                ).strip()

            git("init", "-q")
            git("config", "user.email", "test@example.invalid")
            git("config", "user.name", "Test")
            (repo / "old.rs").write_text("// test\n")
            git("add", "old.rs")
            git("commit", "-qm", "initial")
            base = git("rev-parse", "HEAD")
            git("mv", "old.rs", "README.md")
            git("commit", "-qm", "rename")
            event = repo / "event.json"
            event.write_text(
                json.dumps(
                    {
                        "pull_request": {
                            "base": {"sha": base},
                            "head": {"sha": git("rev-parse", "HEAD")},
                        }
                    }
                )
            )
            paths = pr_scope.changed_paths(repo, event)
            self.assertIn("old.rs", paths)
            self.assertTrue(pr_scope.is_relevant(paths, "clippy"))

    def test_invalid_or_unavailable_history_does_not_report_a_safe_skip(self):
        with tempfile.TemporaryDirectory() as scratch:
            event = Path(scratch) / "event.json"
            event.write_text(
                json.dumps(
                    {
                        "pull_request": {
                            "base": {"sha": "--help"},
                            "head": {"sha": "a" * 40},
                        }
                    }
                )
            )
            with self.assertRaises(ValueError):
                pr_scope.changed_paths(Path(scratch), event)

    def test_required_checks_are_not_hidden_by_workflow_path_filters(self):
        for filename, job_name in (("tests.yml", "test"), ("clippy.yml", "clippy")):
            document = yaml.safe_load(
                (ROOT / ".github/workflows" / filename).read_text()
            )
            triggers = document.get("on", document.get(True))
            self.assertNotIn("paths", triggers["pull_request"])
            job = document["jobs"][job_name]
            self.assertIn("scope", job["needs"])
            self.assertIn("!cancelled()", job["if"])
            # Missing/failed classification must fall back to execution.
            self.assertIn("needs.scope.outputs.needed != 'false'", job["if"])
            self.assertIn("needs.scope.result != 'success'", job["if"])

    def test_linting_has_no_repository_or_check_write_credentials(self):
        document = yaml.safe_load((ROOT / ".github/workflows/clippy.yml").read_text())
        job = document["jobs"]["clippy"]
        self.assertNotIn(
            "write", job.get("permissions", document["permissions"]).values()
        )
        self.assertNotIn("secrets.GITHUB_TOKEN", str(job))


if __name__ == "__main__":
    unittest.main()
