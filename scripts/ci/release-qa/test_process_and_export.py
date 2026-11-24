"""Offline verdict, export and process regression contracts."""

import copy
import hashlib
import json
import os
import signal
import subprocess
import sys
import time
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

os.environ.setdefault("QA_ROOT", "/tmp/release-qa-tests-unused")
import logic
import run
import wallet
import yaml
import prepare

SOURCE = Path(__file__).resolve().parents[3]
REPORT = {
    "summary": {"verdict": "PASS", "complete": True},
    "scenarios": [{"id": "GUI-1", "status": "PASS"}],
    "findings": [],
    "not_reached": [],
}


class ProcessAndExportTests(unittest.TestCase):
    def test_build_cleanup_deletes_new_broken_draft_and_tag(self):
        jobs = yaml.load(
            (SOURCE / ".github/workflows/weekly-build.yml").read_text(),
            Loader=yaml.BaseLoader,
        )["jobs"]
        cleanup = jobs["cleanup"]
        for outcome in ("qa", "seal", "finalize"):
            self.assertNotIn(outcome, cleanup["needs"])
            self.assertNotIn(f"needs.{outcome}.", cleanup["if"])
        self.assertIn("needs.tag.outputs.created == 'true'", cleanup["if"])
        script = cleanup["steps"][0]["run"]
        for exists in (True, False):
            with self.subTest(exists=exists), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                stub = 'gh() { printf "%s\\n" "$*" >> calls; if [[ "$1 $2" == "release view" ]]; then [[ "$EXISTS" == true ]]; else return 0; fi; }; '
                subprocess.run(
                    ["bash", "-euc", stub + script],
                    cwd=root,
                    check=True,
                    env=dict(
                        os.environ,
                        EXISTS=str(exists).lower(),
                        TAG="weekly",
                        VERSION="1.0",
                        GITHUB_REPOSITORY="test/repo",
                        GITHUB_OUTPUT=str(root / "output"),
                    ),
                )
                calls = (root / "calls").read_text()
                if exists:
                    self.assertIn("release delete weekly --yes --cleanup-tag", calls)
                    self.assertNotIn("api --method DELETE", calls)
                else:
                    self.assertIn(
                        "api --method DELETE repos/test/repo/git/refs/tags/weekly",
                        calls,
                    )
                    self.assertNotIn("release delete", calls)

    def test_all_scope_limited_scenarios_cannot_pass(self):
        report = copy.deepcopy(REPORT)
        report["scenarios"] = [{"id": "G1", "status": "NOT_REACHED"}]
        report["not_reached"] = [
            {"id": "G1", "category": "ci_no_identity", "reason": "No identity."}
        ]
        self.assertEqual(logic.verdict(report, smoke=True, agent_ok=True), "NOTE")
        report["scenarios"].append({"id": "navigation", "status": "PASS"})
        self.assertEqual(logic.verdict(report, smoke=True, agent_ok=True), "PASS")
        report["scenarios"][1]["status"] = "FAIL"
        self.assertEqual(logic.verdict(report, smoke=True, agent_ok=True), "FAIL")

    def test_controlled_stop_waits_for_monitor_receipt(self):
        for code in (0, 1):
            with self.subTest(code=code), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                app = root / "app/dash-evo-tool/dash-evo-tool"
                app.parent.mkdir(parents=True)
                app.write_text(
                    "#!/usr/bin/python3\nimport signal, sys\nfrom pathlib import Path\n"
                    f"signal.signal(signal.SIGUSR1, lambda *_: sys.exit({code}))\n"
                    "Path('ready').touch()\nsignal.pause()\n"
                )
                app.chmod(0o700)
                with subprocess.Popen(
                    [sys.executable, run.__file__, "monitor", temp],
                    env=logic.isolated_env(root),
                ) as monitor:
                    try:
                        for _ in range(200):
                            if (root / "ready").exists():
                                break
                            time.sleep(0.01)
                        self.assertTrue((root / "ready").exists())
                        (root / "window").write_text("42")

                        def close(args, **kwargs):
                            self.assertEqual(
                                args,
                                [
                                    "xdotool",
                                    "windowactivate",
                                    "--sync",
                                    "42",
                                    "key",
                                    "--clearmodifiers",
                                    "alt+F4",
                                ],
                            )
                            os.kill(int((root / "gui.pid").read_text()), signal.SIGUSR1)

                        with (
                            patch.object(run, "ROOT", root),
                            patch.object(run.subprocess, "run", side_effect=close),
                        ):
                            run.receipt(smoke=True)
                            self.assertEqual(run.stop() is False, code != 0)
                            state = run.read(root / "receipt.json")
                            self.assertIs(state["gui_stopped"], True)
                            self.assertEqual(state["smoke"], code == 0)
                            self.assertEqual(
                                logic.verdict(
                                    REPORT, smoke=state["smoke"], agent_ok=True
                                ),
                                "FAIL" if code else "PASS",
                            )
                            if code == 0:
                                self.assertIsNot(
                                    run.stop(), False
                                )  # pause after smoke is idempotent
                        self.assertEqual(monitor.wait(timeout=5), 0)
                    finally:
                        if monitor.poll() is None:
                            monitor.kill()

    def test_baseline_skips_newer_non_ancestor(self):
        candidates = [
            {"tag_name": name} for name in ("test-master", "v1.0.1", "v1.0.0")
        ]

        def execute(*args):
            if args[1] == "rev-parse":
                return args[2].split("^")[0]
            if args[1] == "merge-base" and args[3] == "v1.0.1":
                raise subprocess.CalledProcessError(1, args)
            return ""

        with patch.object(prepare, "run", side_effect=execute):
            self.assertEqual(
                prepare.select_baseline(candidates, "", "new"), ("v1.0.0", "v1.0.0")
            )
            with self.assertRaises(subprocess.CalledProcessError):
                prepare.select_baseline(candidates, "v1.0.1", "new")
            with self.assertRaises(ValueError):
                prepare.select_baseline(candidates[:2], "", "new")

    def test_monitor_records_native_signal_without_log_marker(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            app = root / "app/dash-evo-tool/dash-evo-tool"
            app.parent.mkdir(parents=True)
            app.write_text("#!/bin/sh\nkill -SEGV $$\n")
            app.chmod(0o700)
            with patch.object(run, "ROOT", root):
                run.receipt(smoke=True)
                run.monitor()
                self.assertEqual(run.read(root / "gui-exit.json")["code"], -11)
                self.assertIs(run.stop(), False)
                self.assertIs(run.read(root / "receipt.json")["smoke"], False)

    def test_wallet_sync_failure_preserves_safe_unfunded_state(self):
        with tempfile.TemporaryDirectory() as temp:
            state = Path(temp) / "wallet.json"
            with (
                patch.object(wallet, "STATE", state),
                patch.object(
                    wallet, "det", side_effect=[{}, {"seed_hash": "id"}, TimeoutError]
                ),
            ):
                with self.assertRaises(TimeoutError):
                    wallet.provision()
            saved = json.loads(state.read_text())
            self.assertEqual(
                (saved["funding"], saved["refund"]), ("not_reached", "not_needed")
            )
            self.assertNotIn("address", saved)

    def test_copy_failure_purges_staged_export(self):
        with tempfile.TemporaryDirectory() as temp:
            root, out = Path(temp), Path(temp) / "report"
            out.mkdir()
            (out / "first.md").write_text("Public evidence.")
            original = Path.write_bytes

            def write(path, data):
                if "export.tmp" in path.parts and path.name == "result.json":
                    raise OSError("disk full")
                return original(path, data)

            with (
                patch.object(run, "ROOT", root),
                patch.object(run, "OUT", out),
                patch.object(Path, "write_bytes", write),
            ):
                self.assertFalse(run.finish())
            self.assertFalse((root / "export.tmp").exists())
            self.assertFalse((root / "export/first.md").exists())
            self.assertEqual(
                run.read(root / "export/result.json")["verdict"], "INCOMPLETE"
            )

    def test_commit_contract_and_stable_artifacts(self):
        for name in (
            "release-review/ci-prompt.md",
            "release-review/SKILL.md",
            "visual-diff/SKILL.md",
        ):
            text = (SOURCE / ".claude/skills" / name).read_text()
            self.assertIn("context/commits.txt", text)
            self.assertIn("non-merge candidates", text)
        self.assertIn(
            '"--no-merges"', (SOURCE / "scripts/ci/release-qa/prepare.py").read_text()
        )
        text = (SOURCE / ".github/workflows/release-qa.yml").read_text()
        self.assertNotIn("github.run_attempt", text)
        self.assertIn("finish_result: ${{ steps.finish.outcome }}", text)

    def test_scope_limits_and_actual_failures(self):
        for category, expected in (
            ("ci_no_identity", "PASS"),
            ("ci_no_broadcast", "PASS"),
            ("ci_no_secrets", "PASS"),
            ("ci_new_only", "PASS"),
            ("ci_fixed_viewport", "PASS"),
            ("funding", "NOTE"),
            ("network", "NOTE"),
            ("app_error", "NOTE"),
            ("navigation", "NOTE"),
            ("made_up", "INCOMPLETE"),
        ):
            with self.subTest(category=category):
                report = copy.deepcopy(REPORT)
                gap = {"id": "G1", "category": category, "reason": "Explicit limit."}
                report["not_reached"] = [gap]
                report["scenarios"].append({"id": "G1", "status": "NOT_REACHED"})
                self.assertEqual(
                    logic.verdict(report, smoke=True, agent_ok=True), expected
                )
                report["scenarios"][0]["status"] = "FAIL"
                if expected != "INCOMPLETE":
                    self.assertEqual(
                        logic.verdict(report, smoke=True, agent_ok=True), "FAIL"
                    )
        report = copy.deepcopy(REPORT)
        report["scenarios"].append({"id": "unlinked", "status": "NOT_REACHED"})
        self.assertEqual(logic.verdict(report, smoke=True, agent_ok=True), "INCOMPLETE")

    def test_smoke_infrastructure_is_incomplete(self):
        with (
            tempfile.TemporaryDirectory() as temp,
            patch.object(run, "ROOT", Path(temp)),
        ):
            with patch.object(wallet, "det", side_effect=TimeoutError):
                with self.assertRaises(TimeoutError):
                    run.smoke()
            state = run.read(Path(temp) / "receipt.json", {})
            self.assertEqual(
                logic.verdict(REPORT, smoke=state.get("smoke"), agent_ok=True),
                "INCOMPLETE",
            )

    def test_native_crash_markers(self):
        with (
            tempfile.TemporaryDirectory() as temp,
            patch.object(run, "ROOT", Path(temp)),
        ):
            for marker in ("FATAL: SIGSEGV", "FATAL: SIGABRT", "panicked at"):
                (Path(temp) / "launch.log").write_text(marker)
                self.assertTrue(run.panicked(), marker)

    def test_disappeared_gui_fails_even_without_log(self):
        with (
            tempfile.TemporaryDirectory() as temp,
            patch.object(run, "ROOT", Path(temp)),
        ):
            root = Path(temp)
            (root / "receipt.json").write_text('{"smoke":true}')
            (root / "gui.pid").write_text("999999999")
            (root / "launch.log.pid").write_text("999999999")
            run.stop()
            self.assertIs(run.read(root / "receipt.json")["smoke"], False)

    def test_scan_exception_never_leaves_public_files(self):
        with tempfile.TemporaryDirectory() as temp:
            root, out = Path(temp), Path(temp) / "report"
            out.mkdir()
            (out / "first.md").write_text("Safe text")
            (out / "second.md").write_text("Other text")
            with (
                patch.object(run, "ROOT", root),
                patch.object(run, "OUT", out),
                patch.object(
                    run, "has_secret", side_effect=[False, RuntimeError("scan")]
                ) as scanner,
            ):
                self.assertIs(run.finish(), False)
                self.assertEqual(scanner.call_count, 2)
            self.assertFalse((root / "export/first.md").exists())
            self.assertFalse((root / "export/second.md").exists())
            self.assertEqual(
                run.read(root / "export/result.json")["verdict"], "INCOMPLETE"
            )

    def test_shared_env_disables_external_browser(self):
        self.assertEqual(run.env()["BROWSER"], "/bin/true")
        with patch.object(wallet.subprocess, "run") as execute:
            execute.return_value.returncode = 0
            execute.return_value.stdout = "{}"
            wallet.det("network-info")
        self.assertEqual(execute.call_args.kwargs["env"]["BROWSER"], "/bin/true")

    def test_report_rejects_partial_export_and_failed_finish(self):
        jobs = yaml.load(
            (SOURCE / ".github/workflows/release-qa.yml").read_text(),
            Loader=yaml.BaseLoader,
        )["jobs"]
        script = next(
            s["run"] for s in jobs["report"]["steps"] if s.get("id") == "result"
        )
        for finish, problem in (
            ("failure", ""),
            ("success", "absent"),
            ("success", "missing"),
            ("success", "corrupt"),
        ):
            with (
                self.subTest(finish=finish, problem=problem),
                tempfile.TemporaryDirectory() as temp,
            ):
                root = Path(temp)
                (root / "report").mkdir()
                (root / "report/result.json").write_text('{"verdict":"PASS"}')
                for name in (
                    "report.json",
                    "report.md",
                    "verified-changelog.md",
                    "scenarios.md",
                    "story-checklist.md",
                    "meta.json",
                    "spec.json",
                    "index.html",
                    "visual-changelog.html",
                ):
                    (root / "report" / name).write_text("Public evidence.")
                files = {
                    p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                    for p in (root / "report").iterdir()
                }
                if problem != "absent":
                    (root / "report/export-manifest.json").write_text(
                        json.dumps({"complete": True, "files": files})
                    )
                if problem == "missing":
                    (root / "report/visual-changelog.html").unlink()
                if problem == "corrupt":
                    (root / "report/report.md").write_text("Changed after export.")
                subprocess.run(
                    ["bash", "-euc", script],
                    cwd=root,
                    check=True,
                    env=dict(
                        os.environ,
                        REVIEW_RESULT="success",
                        FINISH_RESULT=finish,
                        GITHUB_REPOSITORY="test/repo",
                        GITHUB_RUN_ID="1",
                        GITHUB_OUTPUT=str(root / "output"),
                        GITHUB_STEP_SUMMARY=str(root / "summary"),
                    ),
                )
                self.assertIn("verdict=INCOMPLETE", (root / "output").read_text())

    def test_smoke_precedes_wallet_and_does_not_require_it(self):
        jobs = yaml.load(
            (SOURCE / ".github/workflows/release-qa.yml").read_text(),
            Loader=yaml.BaseLoader,
        )["jobs"]
        steps = jobs["review"]["steps"]
        ids = [s.get("id") for s in steps]
        self.assertLess(ids.index("smoke"), ids.index("wallet"))
        self.assertNotIn("wallet", steps[ids.index("smoke")]["if"])
