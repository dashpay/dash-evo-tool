"""Release jobs keep untrusted build execution outside signing credentials."""

import unittest
import subprocess
import tempfile
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]


def workflow(name):
    return yaml.safe_load((ROOT / ".github/workflows" / name).read_text())


class ReleaseWorkflowTests(unittest.TestCase):
    def bundle_check(self, mutate=None):
        signer = workflow("release.yml")["jobs"]["sign-macos"]
        script = next(
            step["run"]
            for step in signer["steps"]
            if step["name"] == "Check the unsigned bundle before loading credentials"
        )
        with tempfile.TemporaryDirectory() as scratch:
            app = Path(scratch) / "build/Dash Evo Tool.app"
            paths = (
                "Contents/Info.plist",
                "Contents/Resources/AppIcon.icns",
                "Contents/MacOS/dash-evo-tool",
                "Contents/MacOS/det-cli",
            )
            for path in paths:
                file = app / path
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text("unsigned test artifact")
            if mutate:
                mutate(app)
            return subprocess.run(
                ["bash", "-c", script], cwd=scratch, capture_output=True
            ).returncode

    def test_only_the_expected_app_files_can_reach_the_signer(self):
        self.assertEqual(self.bundle_check(), 0)
        self.assertNotEqual(
            self.bundle_check(
                lambda app: (app / "Contents/MacOS/helper.sh").write_text("unexpected")
            ),
            0,
        )

    def test_symlink_cannot_reach_the_signer(self):
        def replace_icon(app):
            icon = app / "Contents/Resources/AppIcon.icns"
            icon.unlink()
            icon.symlink_to("../Info.plist")

        self.assertNotEqual(self.bundle_check(replace_icon), 0)

    def test_jobs_executing_cargo_have_no_signing_or_repository_write_authority(self):
        for filename in ("release.yml", "flatpak.yml"):
            document = workflow(filename)
            for name, job in document["jobs"].items():
                steps = job.get("steps", [])
                builds = any(
                    "cargo " in str(s.get("run", ""))
                    or "flatpak-builder " in str(s.get("run", ""))
                    for s in steps
                )
                if not builds:
                    continue
                with self.subTest(workflow=filename, job=name):
                    self.assertNotIn("environment", job)
                    permissions = job.get(
                        "permissions", document.get("permissions", {})
                    )
                    self.assertNotIn("write", permissions.values())
                    self.assertNotIn("secrets.APPLE", str(job))
                    for step in steps:
                        if step.get("uses", "").startswith("actions/checkout@"):
                            self.assertIs(step["with"]["persist-credentials"], False)

    def test_signing_runs_on_a_fresh_runner_without_repository_checkout(self):
        jobs = workflow("release.yml")["jobs"]
        signers = [
            job for job in jobs.values() if job.get("environment") == "release-signing"
        ]
        self.assertTrue(signers)
        for job in signers:
            self.assertNotIn("actions/checkout@", str(job))
            self.assertNotIn("cargo ", str(job))
            self.assertNotIn("contents: write", str(job))
            self.assertTrue(
                any(
                    s.get("uses", "").startswith("actions/download-artifact@")
                    for s in job["steps"]
                )
            )

    def test_weekly_pins_all_builds_and_the_tag_to_one_source(self):
        jobs = workflow("weekly-build.yml")["jobs"]
        self.assertEqual(jobs["gate"]["if"], "github.ref == 'refs/heads/v1.0-dev'")
        for name in ("release", "flatpak"):
            self.assertEqual(
                jobs[name]["with"]["source_sha"], "${{ needs.gate.outputs.source_sha }}"
            )
        checkout = next(
            s
            for s in jobs["tag"]["steps"]
            if s.get("uses", "").startswith("actions/checkout@")
        )
        self.assertEqual(
            checkout["with"]["ref"], "${{ needs.gate.outputs.source_sha }}"
        )

    def test_release_builds_use_authorized_sha_and_authorization_code_is_trusted(self):
        jobs = workflow("release.yml")["jobs"]
        policy = jobs["authorize"]
        checkout = next(
            s
            for s in policy["steps"]
            if s.get("uses", "").startswith("actions/checkout@")
        )
        self.assertEqual(checkout["with"]["ref"], "refs/heads/v1.0-dev")
        for name in ("build-and-release", "build-macos-arm64", "build-macos-x86"):
            with self.subTest(job=name):
                self.assertIn("authorize", jobs[name]["needs"])
                checkout = next(
                    s
                    for s in jobs[name]["steps"]
                    if s.get("uses", "").startswith("actions/checkout@")
                )
                self.assertEqual(
                    checkout["with"]["ref"], "${{ needs.authorize.outputs.source_sha }}"
                )

    def test_called_workflows_do_not_exceed_the_weekly_permission_budget(self):
        weekly = workflow("weekly-build.yml")["jobs"]
        for name, filename in (("release", "release.yml"), ("flatpak", "flatpak.yml")):
            called = workflow(filename)
            granted = weekly[name]["permissions"]
            for job in called["jobs"].values():
                permissions = job.get("permissions", called.get("permissions", {}))
                for scope, level in permissions.items():
                    with self.subTest(workflow=filename, scope=scope):
                        self.assertIn(granted.get(scope), (level, "write"))


if __name__ == "__main__":
    unittest.main()
