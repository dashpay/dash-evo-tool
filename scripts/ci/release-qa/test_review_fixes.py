"""Offline regressions for artifact trust, final evidence and manual triggering."""

import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from PIL import Image
import yaml

os.environ.setdefault("QA_ROOT", "/tmp/release-qa-tests-unused")
import run
import prepare
import wallet
from test_process_and_export import REPORT, SOURCE


class ReviewFixTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.out = self.root / "report"
        self.out.mkdir()
        for name, value in (("ROOT", self.root), ("OUT", self.out)):
            p = patch.object(run, name, value)
            p.start()
            self.addCleanup(p.stop)

    def evidence(self):
        (self.root / "context").mkdir()
        (self.root / "context/commits.txt").write_text("abcdef0123456 internal")
        for name in (
            "report.md",
            "verified-changelog.md",
            "scenarios.md",
            "story-checklist.md",
            "index.html",
        ):
            (self.out / name).write_text("Final evidence")
        (self.out / "meta.json").write_text(
            json.dumps(
                {
                    "images": [],
                    "coverage": [
                        {
                            "commit": "abcdef0",
                            "subject": "Internal",
                            "images": [],
                            "note": "no user-visible change",
                        }
                    ],
                }
            )
        )
        (self.out / "spec.json").write_text("{}")
        (self.out / "report.json").write_text(json.dumps(REPORT))
        (self.root / "wallet.json").write_text('{"funding":"sent","refund":"sent"}')
        run.receipt(smoke=True, agent_ok=True, gui_stopped=True)

    def test_display_allowlist_and_init_reject_planted_files(self):
        (self.root / "display.json").write_text(
            json.dumps(
                {"LD_PRELOAD": "/evil.so", "DBUS_SESSION_BUS_ADDRESS": "unix:path=/bus"}
            )
        )
        self.assertNotIn("LD_PRELOAD", run.env())
        self.assertEqual(run.env()["DBUS_SESSION_BUS_ADDRESS"], "unix:path=/bus")
        with patch.object(run.subprocess, "check_output") as execute:
            with self.assertRaisesRegex(ValueError, "Unexpected QA input"):
                run.init()
        execute.assert_not_called()
        (self.root / "display.json").unlink()
        (self.root / "extra.so").touch()
        with self.assertRaisesRegex(ValueError, "Unexpected QA input"):
            run.init()

    def test_notes_only_coverage_requires_a_list_and_all_commits(self):
        self.evidence()
        self.assertTrue(run.evidence_complete())
        (self.root / "context/commits.txt").write_text("123456789 uncovered")
        self.assertFalse(run.evidence_complete())
        for images in (None, {}, ""):
            (self.out / "meta.json").write_text(
                json.dumps({"images": images, "coverage": []})
            )
            self.assertFalse(run.evidence_complete())

    def test_finish_regenerates_final_gallery_and_omits_unlisted_captures(self):
        self.evidence()
        for directory in ("new", "annotated", "annotated/clean"):
            (self.out / directory).mkdir(parents=True, exist_ok=True)
            Image.new("RGB", (20, 20)).save(self.out / directory / "unlisted.png")
        Image.new("RGB", (20, 20)).save(self.out / "new/00-smoke.png")
        (self.out / "index.html").write_text("Stale gallery")
        self.assertTrue(run.finish())
        self.assertEqual(run.read(self.root / "export/result.json")["verdict"], "PASS")
        page = (self.root / "export/index.html").read_text()
        self.assertIn("no user-visible change", page)
        self.assertNotIn("Stale gallery", page)
        self.assertFalse((self.out / "annotated/unlisted.png").exists())
        self.assertTrue((self.root / "export/new/00-smoke.png").exists())
        self.assertFalse(list((self.root / "export").rglob("unlisted.png")))

    def test_render_failure_cannot_export_stale_gallery_or_pass(self):
        self.evidence()
        with patch.object(
            run.subprocess,
            "run",
            side_effect=subprocess.CalledProcessError(1, "render"),
        ):
            run.finish()
        self.assertEqual(
            run.read(self.root / "export/result.json")["verdict"], "INCOMPLETE"
        )
        self.assertFalse((self.root / "export/index.html").exists())

    def test_oversized_log_keeps_report_and_head_tail(self):
        self.evidence()
        (self.root / "launch.log").write_text("HEAD" + "x" * 30_000_000 + "TAIL")
        self.assertTrue(run.finish())
        exported = self.root / "export"
        self.assertEqual(run.read(exported / "result.json")["verdict"], "INCOMPLETE")
        text = (exported / "app-launch.log").read_text()
        self.assertTrue(text.startswith("HEAD") and text.endswith("TAIL"))
        self.assertIn("[TRUNCATED]", text)
        self.assertLessEqual(len(text.encode()), 30_000_000)
        self.assertTrue((exported / "report.md").exists())

    def test_resume_reports_missing_pid_timeout(self):
        with (
            patch.object(wallet, "det", return_value={"active": "testnet"}),
            patch.object(
                run, "launch", return_value=SimpleNamespace(poll=lambda: None)
            ),
            patch.object(run.time, "sleep"),
        ):
            with self.assertRaisesRegex(RuntimeError, "monitor.*PID"):
                run.resume()

    def test_same_repo_label_only_and_report_only(self):
        workflow = yaml.load(
            (SOURCE / ".github/workflows/release-qa.yml").read_text(),
            Loader=yaml.BaseLoader,
        )
        self.assertEqual(workflow["on"]["pull_request"]["types"], ["labeled"])
        jobs = workflow["jobs"]
        self.assertEqual(set(jobs), {"prepare_pr", "prepare", "review", "report"})
        self.assertIn("needs.prepare_pr.result != 'skipped'", jobs["report"]["if"])
        for name, job in jobs.items():
            if name != "prepare":
                self.assertNotEqual(job.get("permissions", {}).get("contents"), "write")
            for step in job["steps"]:
                self.assertNotRegex(
                    step.get("run", ""), r"gh release (edit|create|delete|upload)"
                )
                if "actions/checkout@" in step.get("uses", ""):
                    self.assertIn(
                        "github.event.pull_request.head.sha", step["with"]["ref"]
                    )
                if "CLAUDE_CODE_OAUTH_TOKEN" in step.get("env", {}):
                    self.assertEqual(step.get("id"), "agent")

    def test_latest_weekly_ignores_drafts_stable_and_non_weekly(self):
        releases = [
            dict(tag_name=tag, prerelease=pre, draft=draft, published_at=date)
            for tag, pre, draft, date in (
                ("v1.0.0-weekly.20261117", True, False, "2026-11-17"),
                ("v1.0.0-weekly.20261124", True, False, "2026-11-24"),
                ("v1.0.0-weekly.20261201", True, True, None),
                ("v2.0.0", False, False, "2026-12-02"),
                ("v2.0.1-rc.1", True, False, "2026-12-03"),
            )
        ]
        self.assertEqual(
            prepare.select_new_release(releases, "")["tag_name"],
            "v1.0.0-weekly.20261124",
        )
        self.assertEqual(
            prepare.select_new_release(releases, releases[0]["tag_name"]), releases[0]
        )

    def test_prepare_copies_only_verified_zip(self):
        staging, inputs = self.root / "download", self.root / "inputs"
        staging.mkdir()
        binary = staging / "dash-evo-tool-linux-x86_64.zip"
        binary.write_bytes(b"attested zip")
        (staging / "display.json").write_text('{"LD_PRELOAD":"/evil.so"}')
        (staging / "extra.so").touch()
        releases = [
            dict(
                tag_name=name,
                prerelease=True,
                draft=False,
                created_at=date,
                published_at=date,
            )
            for name, date in (("v1.0.1", "2"), ("v1.0.0", "1"))
        ]

        def execute(*args):
            if args[:2] == ("gh", "api"):
                return json.dumps([releases])
            if args[:3] == ("gh", "attestation", "verify"):
                self.assertFalse((inputs / binary.name).exists())
            return ""

        with (
            patch.dict(
                os.environ,
                QA_ROOT=str(inputs),
                QA_DOWNLOAD=str(staging),
                NEW_TAG="v1.0.1",
                BASELINE="",
                GITHUB_REPOSITORY="owner/repo",
            ),
            patch.object(prepare, "run", side_effect=execute),
        ):
            prepare.main()
        self.assertEqual({p.name for p in inputs.iterdir()}, {binary.name, "context"})
        self.assertEqual((inputs / binary.name).read_bytes(), binary.read_bytes())

    def test_atomic_state_preserves_old_file_on_replace_failure(self):
        import atomic

        path = self.root / "state.json"
        path.write_text("old")
        with patch.object(atomic.os, "replace", side_effect=OSError):
            with self.assertRaises(OSError):
                atomic.atomic_write(path, "new")
        self.assertEqual(path.read_text(), "old")
        self.assertEqual(
            {p.name for p in self.root.iterdir()}, {"state.json", "report"}
        )
