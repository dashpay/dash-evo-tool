"""Required checks remain present without compiling documentation-only PRs."""

import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import yaml

import pr_scope

ROOT = Path(__file__).resolve().parents[2]


def git(repo, *args):
    return subprocess.check_output(["git", "-C", str(repo), *args], text=True).strip()


def pr_diff(repo, files, change):
    """Commit `files`, apply `change`, and return the PR diff between them."""
    git(repo, "init", "-q")
    git(repo, "config", "user.email", "test@example.invalid")
    git(repo, "config", "user.name", "Test")
    for name in files:
        (repo / name).parent.mkdir(parents=True, exist_ok=True)
        (repo / name).write_text("// test\n")
    git(repo, "add", "--", *files)
    git(repo, "commit", "-qm", "initial")
    base = git(repo, "rev-parse", "HEAD")
    change()
    git(repo, "commit", "-qm", "change")
    event = repo / "event.json"
    event.write_text(
        json.dumps(
            {
                "pull_request": {
                    "base": {"sha": base},
                    "head": {"sha": git(repo, "rev-parse", "HEAD")},
                }
            }
        )
    )
    return pr_scope.changed_paths(repo, event)


# Git variables inherited from a hook or worktree would redirect the temporary
# repositories' commands to the developer's checkout.
GIT_LOCATION_VARIABLES = (
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
)


class PrScopeTests(unittest.TestCase):
    def setUp(self):
        environment = mock.patch.dict(os.environ)
        environment.start()
        self.addCleanup(environment.stop)
        for name in GIT_LOCATION_VARIABLES:
            os.environ.pop(name, None)

    def test_everything_outside_documentation_requires_both_suites(self):
        for path in (
            "build.rs",
            "src/main.rs",
            "Cargo.toml",
            "nested/Cargo.toml",
            "Cargo.lock",
            ".cargo/config.toml",
            "rust-toolchain.toml",
            "clippy.toml",
            ".env.example",
            "dash_core_configs/mainnet.conf",
            "assets/DET_LOGO.png",
            "assets/Fonts/NotoSans-Regular.ttf",
            "src/wallet_backend/platform_compatibility/fixtures/67d4ef3.sql",
            "tests/migration-fixtures/manifest.json",
            "scripts/ci/pr_scope.py",
            "scripts/ci/run-tests.sh",
            "scripts/migration-fixtures/capture.sh",
            ".github/workflows/tests.yml",
            ".github/workflows/clippy.yml",
            ".github/workflows/weekly-build.yml",
            "flatpak/org.dash.DashEvoTool.yml",
            "an/unknown/new-input.bin",
        ):
            for suite in ("tests", "clippy"):
                with self.subTest(path=path, suite=suite):
                    self.assertTrue(pr_scope.is_relevant([path], suite))

    def test_documentation_only_skips_compilation(self):
        paths = [
            "README.md",
            "docs/example.md",
            "docs/ai-design/2026-01-01-example/wireframe.html",
            "docs/ai-design/2026-01-01-example/report.json",
            "tests/migration-fixtures/README.md",
            ".claude/skills/example/SKILL.md",
        ]
        for suite in ("tests", "clippy"):
            with self.subTest(suite=suite):
                self.assertFalse(pr_scope.is_relevant(paths, suite))
                self.assertTrue(pr_scope.is_relevant([*paths, "clippy.toml"], suite))

    def test_pr_diff_includes_deleted_and_renamed_rust_files(self):
        with tempfile.TemporaryDirectory() as scratch:
            repo = Path(scratch)
            paths = pr_diff(
                repo, ["old.rs"], lambda: git(repo, "mv", "old.rs", "README.md")
            )
            self.assertIn("old.rs", paths)
            self.assertTrue(pr_scope.is_relevant(paths, "clippy"))

    def test_deleted_embedded_input_requires_both_suites(self):
        for name in (".env.example", "assets/DET_LOGO.png", "clippy.toml"):
            with tempfile.TemporaryDirectory() as scratch, self.subTest(path=name):
                repo = Path(scratch)
                paths = pr_diff(repo, [name], lambda: git(repo, "rm", "-q", name))
                self.assertEqual(paths, [name])
                for suite in ("tests", "clippy"):
                    self.assertTrue(pr_scope.is_relevant(paths, suite))

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

    def test_pushes_skip_only_documentation_like_the_pr_classifier(self):
        for filename in ("tests.yml", "clippy.yml"):
            with self.subTest(workflow=filename):
                document = yaml.safe_load(
                    (ROOT / ".github/workflows" / filename).read_text()
                )
                push = document.get("on", document.get(True))["push"]
                self.assertNotIn("paths", push)
                self.assertEqual(push["paths-ignore"], ["docs/**", "**/*.md"])

    def test_linting_has_no_repository_or_check_write_credentials(self):
        document = yaml.safe_load((ROOT / ".github/workflows/clippy.yml").read_text())
        job = document["jobs"]["clippy"]
        self.assertNotIn(
            "write", job.get("permissions", document["permissions"]).values()
        )
        self.assertNotIn("secrets.GITHUB_TOKEN", str(job))


if __name__ == "__main__":
    unittest.main()
