"""Security and workflow contracts; no network, Claude, display or payouts."""

import json
import hashlib
import os
import re
import random
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from guard import allowed


class DriverTests(unittest.TestCase):
    def test_no_arbitrary_shell_or_secret_file_access(self):
        with tempfile.TemporaryDirectory() as temp:
            root, source = Path(temp) / "runtime", Path(temp) / "source"
            (root / "report").mkdir(parents=True)
            source.mkdir()
            for command in (
                "env",
                "/usr/local/bin/qa-ui tree; env",
                "/usr/local/bin/qa-ui type $(env)",
                "/usr/local/bin/qa-ui tree\ncat /proc/self/environ",
            ):
                self.assertFalse(
                    allowed(
                        {"tool_name": "Bash", "tool_input": {"command": command}},
                        root,
                        source,
                    )
                )
            self.assertTrue(
                allowed(
                    {
                        "tool_name": "Bash",
                        "tool_input": {"command": "/usr/local/bin/qa-ui tree"},
                    },
                    root,
                    source,
                )
            )
            for tool, path in (
                ("Read", root / "wallet.json"),
                ("Read", source / ".env"),
                ("Edit", source / "app.py"),
                ("Write", root / "receipt.json"),
            ):
                self.assertFalse(
                    allowed(
                        {"tool_name": tool, "tool_input": {"file_path": str(path)}},
                        root,
                        source,
                    )
                )
            link = root / "report/escape"
            link.symlink_to(root)
            self.assertFalse(
                allowed(
                    {
                        "tool_name": "Write",
                        "tool_input": {"file_path": str(link / "receipt.json")},
                    },
                    root,
                    source,
                )
            )

    def test_reserved_evidence_and_background_calls_are_denied(self):
        root, source = Path("/tmp/qa-test"), Path("/tmp/source-test")
        for name in (
            "result.json",
            "wallet-status.json",
            "app-launch.log",
            "new/00-smoke.png",
            "index.html",
            "visual-changelog.html",
        ):
            self.assertFalse(
                allowed(
                    {
                        "tool_name": "Write",
                        "tool_input": {"file_path": str(root / "report" / name)},
                    },
                    root,
                    source,
                )
            )
        self.assertFalse(
            allowed(
                {
                    "tool_name": "Bash",
                    "tool_input": {
                        "command": "/usr/local/bin/qa-ui tree",
                        "run_in_background": True,
                    },
                },
                root,
                source,
            )
        )

    def test_agent_uses_oauth_and_fail_closed_hook_without_bare_mode(self):
        with (
            tempfile.TemporaryDirectory() as temp,
            patch.dict(os.environ, QA_ROOT=temp),
        ):
            import run

            root, source = Path(temp), Path(__file__).resolve().parents[3]
            out = root / "report"
            out.mkdir()
            (root / "data").mkdir()
            (root / "data/.env").write_text("PRIVATE=mock")
            with (
                patch.object(run, "ROOT", root),
                patch.object(run, "OUT", out),
                patch.dict(
                    os.environ,
                    QA_SOURCE=str(source),
                    CLAUDE_CODE_OAUTH_TOKEN="mock-oauth",
                ),
                patch.object(run.subprocess, "run") as execute,
            ):
                execute.return_value.returncode = 0
                run.agent()
            args, options = execute.call_args.args[0], execute.call_args.kwargs
            self.assertEqual(args[args.index("--max-turns") + 1], "600")
            self.assertNotIn("--bare", args)
            self.assertIn("--restricted", args)
            self.assertIn("--strict-mcp-config", args)
            self.assertIn("--add-dir", args)
            self.assertIn(str(out), args)
            self.assertEqual(options["env"]["CLAUDE_CODE_OAUTH_TOKEN"], "mock-oauth")
            self.assertNotIn("GH_TOKEN", options["env"])
            self.assertNotEqual(options["cwd"], source)
            hook = json.loads((root / "settings.json").read_text())["hooks"][
                "PreToolUse"
            ][0]["hooks"][0]
            self.assertEqual(hook["onFailure"], "block")
            self.assertFalse((root / "data/.env").exists())
            self.assertFalse(
                json.loads((root / "receipt.json").read_text())["agent_ok"]
            )

    def test_payout_timeout_persists_uncertainty_without_retry(self):
        with (
            tempfile.TemporaryDirectory() as temp,
            patch.dict(os.environ, QA_ROOT=temp),
        ):
            import wallet

            status = {
                "network": "testnet",
                "depositAddress": "yPrynRN6ViX2rCcRL5SoSi8ntcWgdHSEaS",
                "coreFaucetAmount": 1,
                "capEndpoint": "https://faucet.thepasta.org/cap/v1/",
            }
            with (
                patch.object(wallet, "ROOT", Path(temp)),
                patch.object(wallet, "STATE", Path(temp) / "wallet.json"),
                patch.object(wallet, "token", return_value="cap"),
                patch.object(
                    wallet,
                    "det",
                    side_effect=[
                        {},
                        {"seed_hash": "id"},
                        {"address": status["depositAddress"]},
                    ],
                ) as det,
                patch.object(
                    wallet, "request", side_effect=[(200, status), TimeoutError]
                ) as request,
            ):
                with self.assertRaises(TimeoutError):
                    wallet.provision()
                    wallet.fund()
                self.assertEqual(request.call_count, 2)
                state = json.loads((Path(temp) / "wallet.json").read_text())
                self.assertEqual(
                    (state["funding"], state["refund"]), ("unknown", "pending")
                )
                phrase = det.call_args_list[1].kwargs["phrase"].strip()
                from mnemonic import Mnemonic

                self.assertTrue(Mnemonic("english").check(phrase))
                self.assertNotIn(phrase, (Path(temp) / "wallet.json").read_text())
                self.assertNotIn("txid", state)

                def send(*args, **kwargs):
                    saved = json.loads(wallet.STATE.read_text())
                    self.assertEqual(saved["refund"], "unknown")
                    self.assertEqual(saved["txid"], "a" * 64)
                    self.assertEqual(args[0], "core-funds-send")
                    self.assertIn("amount_duffs=99999774", args)
                    return {"txid": "b" * 64}

                with patch.object(
                    wallet.insight,
                    "query",
                    return_value=[
                        {
                            "address": state["address"],
                            "satoshis": state["payout_duffs"],
                            "confirmations": 1,
                            "vout": 0,
                            "txid": "a" * 64,
                        }
                    ],
                ):
                    det.reset_mock(side_effect=True)
                    det.side_effect = send
                    wallet.refund()
                    self.assertEqual(
                        json.loads(wallet.STATE.read_text())["refund"], "sent"
                    )
                    wallet.refund()
                    det.assert_called_once()
                    self.assertEqual(request.call_count, 2)


class WorkflowTests(unittest.TestCase):
    def test_qa_never_deletes_and_only_pass_publishes(self):
        import yaml

        workflow = (
            Path(__file__).resolve().parents[3] / ".github/workflows/weekly-build.yml"
        )
        jobs = yaml.load(workflow.read_text(), Loader=yaml.BaseLoader)["jobs"]
        publish = next(
            s["if"] for s in jobs["finalize"]["steps"] if s.get("id") == "publish"
        )

        def evaluate(
            expression, qa, verdict, event="schedule", created="true", fixture="success"
        ):
            values = {
                "gate.outputs.should_release": "true",
                "tag.result": "success",
                "tag.outputs.created": created,
                "release.result": "success",
                "flatpak.result": "success",
                "fixture.result": fixture,
                "seal.result": "success",
                "qa.result": qa,
                "qa.outputs.verdict": verdict,
            }
            expression = re.sub(
                r"needs\.([\w.]+)", lambda m: repr(values[m[1]]), expression
            )
            expression = (
                expression.replace("github.event_name", repr(event))
                .replace("&&", " and ")
                .replace("||", " or ")
            )
            expression = expression.replace("!cancelled()", "True").replace(
                "always()", "True"
            )
            return eval(
                expression,
                {
                    "__builtins__": {},
                    "contains": lambda a, b: b in a,
                    "fromJSON": json.loads,
                },
            )

        for qa in ("success", "failure", "cancelled", "skipped"):
            for result in ("PASS", "NOTE", "FAIL", "INCOMPLETE", ""):
                self.assertFalse(evaluate(jobs["cleanup"]["if"], qa, result))
                self.assertTrue(evaluate(jobs["finalize"]["if"], qa, result))
                self.assertEqual(
                    evaluate(publish, qa, result),
                    qa == "success" and result == "PASS",
                )
                self.assertFalse(
                    evaluate(publish, qa, result, event="workflow_dispatch")
                )
        self.assertTrue(
            evaluate(jobs["cleanup"]["if"], "skipped", "", fixture="failure")
        )
        self.assertFalse(
            evaluate(
                jobs["cleanup"]["if"], "skipped", "", fixture="failure", created=""
            )
        )
        self.assertNotIn("Delete stale weekly drafts", workflow.read_text())
        for name in ("qa", "seal", "finalize"):
            self.assertNotIn("gh release delete", str(jobs[name]))

    def test_summary_fails_closed_and_reports_missing_refund_address(self):
        import yaml

        path = Path(__file__).resolve().parents[3] / ".github/workflows/release-qa.yml"
        jobs = yaml.load(path.read_text(), Loader=yaml.BaseLoader)["jobs"]
        script = next(
            s["run"] for s in jobs["report"]["steps"] if s.get("id") == "result"
        )
        for content, result, expected, warning in (
            (None, "success", "INCOMPLETE", False),
            ("{bad", "success", "INCOMPLETE", False),
            ("[]", "success", "INCOMPLETE", False),
            ('{"verdict":"approved"}', "success", "INCOMPLETE", False),
            ('{"verdict":"PASS"}', "failure", "INCOMPLETE", False),
            (
                '{"verdict":"PASS","wallet":{"warning":"missing"}}',
                "success",
                "PASS",
                True,
            ),
        ):
            with (
                self.subTest(content=content, result=result),
                tempfile.TemporaryDirectory() as temp,
            ):
                root = Path(temp)
                (root / "report").mkdir()
                if content is not None:
                    (root / "report/result.json").write_text(content)
                files = {}
                if expected == "PASS":
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
                        (root / "report" / name).write_text("test evidence")
                    files = {
                        p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                        for p in (root / "report").iterdir()
                    }
                (root / "report/export-manifest.json").write_text(
                    json.dumps({"complete": True, "files": files})
                )
                env = dict(
                    os.environ,
                    REPORT_ARTIFACT_ID="12345",
                    FINISH_RESULT="success",
                    REVIEW_RESULT=result,
                    GITHUB_REPOSITORY="test/repo",
                    GITHUB_RUN_ID="1",
                    GITHUB_OUTPUT=str(root / "output"),
                    GITHUB_STEP_SUMMARY=str(root / "summary"),
                )
                subprocess.run(["bash", "-euc", script], cwd=root, env=env, check=True)
                self.assertIn(f"verdict={expected}\n", (root / "output").read_text())
                self.assertEqual(
                    "WARNING: Refund skipped" in (root / "summary").read_text(), warning
                )

    def test_agent_permissions_and_post_survive_failure(self):
        import yaml

        path = Path(__file__).resolve().parents[3] / ".github/workflows/release-qa.yml"
        jobs = yaml.load(path.read_text(), Loader=yaml.BaseLoader)["jobs"]
        self.assertEqual(jobs["review"]["permissions"], {"contents": "read"})
        steps = jobs["review"]["steps"]
        post = next(
            s for s in steps if s.get("name") == "Return remaining funds to the faucet"
        )
        self.assertIn("always()", post["if"])
        self.assertNotIn("success", post["if"].split("&&")[0])
        # The two prepare jobs are mutually exclusive.
        for prepare in ("prepare_pr", "prepare"):
            self.assertLessEqual(
                sum(
                    int(jobs[name]["timeout-minutes"])
                    for name in (prepare, "review", "report")
                ),
                240,
            )
        review = jobs["review"]
        step_minutes = sum(int(s.get("timeout-minutes", 0)) for s in steps)
        self.assertGreaterEqual(int(review["timeout-minutes"]) - step_minutes, 10)

    def test_skill_changes_only_trigger_qa_script_workflow(self):
        import fnmatch
        import yaml

        workflows = Path(__file__).resolve().parents[3] / ".github/workflows"
        rust = yaml.load((workflows / "tests.yml").read_text(), Loader=yaml.BaseLoader)
        skills = (
            ".claude/skills/release-review/ci-prompt.md",
            ".claude/skills/release-review/SKILL.md",
            ".claude/skills/visual-diff/SKILL.md",
            ".claude/skills/visual-diff/scripts/gallery.py",
        )
        for event in ("push", "pull_request"):
            for path in skills:
                with self.subTest(event=event, path=path):
                    self.assertFalse(
                        any(
                            fnmatch.fnmatchcase(path, pattern)
                            for pattern in rust["on"][event]["paths"]
                        )
                    )
        qa = yaml.load(
            (workflows / "release-qa-tests.yml").read_text(), Loader=yaml.BaseLoader
        )
        self.assertNotIn("schedule", qa["on"])
        self.assertNotIn("release-qa-tests", rust["jobs"])
        for event in ("push", "pull_request"):
            for path in (
                *skills,
                "scripts/ci/release-qa/logic.py",
                ".github/workflows/release-qa.yml",
            ):
                with self.subTest(event=event, path=path):
                    self.assertTrue(
                        any(
                            fnmatch.fnmatchcase(path, pattern)
                            for pattern in qa["on"][event]["paths"]
                        )
                    )


class ReportTests(unittest.TestCase):
    def test_missing_evidence_and_unreturned_funds_hold_draft(self):
        with (
            tempfile.TemporaryDirectory() as temp,
            patch.dict(os.environ, QA_ROOT=temp),
        ):
            import run

            root, out = Path(temp), Path(temp) / "report"
            out.mkdir()
            (root / "receipt.json").write_text(
                json.dumps({"smoke": True, "agent_ok": True, "gui_stopped": True})
            )
            report = {
                "summary": {"verdict": "PASS", "complete": True},
                "scenarios": [{"id": "GUI-1", "status": "PASS"}],
                "findings": [],
                "not_reached": [],
            }
            with (
                patch.object(run, "ROOT", root),
                patch.object(run, "OUT", out),
            ):
                (out / "report.json").write_text(json.dumps(report))
                run.finish()
                self.assertEqual(
                    json.loads((out / "result.json").read_text())["verdict"],
                    "INCOMPLETE",
                )
                from PIL import Image

                for file in (
                    "report.md",
                    "verified-changelog.md",
                    "scenarios.md",
                    "story-checklist.md",
                ):
                    (out / file).write_text("Completed coverage.")
                (out / "index.html").write_text(
                    '<html><img src="annotated/screen.png"></html>'
                )
                (out / "meta.json").write_text(
                    json.dumps(
                        {
                            "images": [{"name": "screen", "title": "Screen"}],
                            "coverage": [
                                {
                                    "commit": "abcdef0",
                                    "subject": "Change",
                                    "images": ["screen"],
                                }
                            ],
                        }
                    )
                )
                (out / "spec.json").write_text('{"screen": []}')
                (root / "context").mkdir()
                (root / "context/commits.txt").write_text("abcdef0123456 test commit")
                for directory in ("new", "annotated", "annotated/clean"):
                    (out / directory).mkdir(parents=True, exist_ok=True)
                    Image.new("RGB", (2, 2)).save(out / directory / "screen.png")
                (out / "report.json").write_text(json.dumps(report))
                (root / "wallet.json").write_text(
                    '{"funding":"sent","refund":"unknown"}'
                )
                run.finish()
                self.assertEqual(
                    json.loads((out / "result.json").read_text())["verdict"],
                    "INCOMPLETE",
                )
                (out / "report.json").write_text(json.dumps(report))
                (root / "wallet.json").write_text(
                    '{"funding":"not_reached","refund":"not_needed"}'
                )
                run.finish()
                self.assertEqual(
                    json.loads((out / "result.json").read_text())["verdict"], "NOTE"
                )

                (root / "wallet.json").write_text(
                    json.dumps(
                        {
                            "funding": "sent",
                            "refund": "skipped_no_address",
                            "warning": "missing",
                        }
                    )
                )
                # A real multi-MB PNG exercises the embedded-gallery export path.
                Image.frombytes(
                    "RGB", (1600, 1200), random.Random(1).randbytes(1600 * 1200 * 3)
                ).save(out / "new/screen.png")
                (out / "report.json").write_text(json.dumps(report))
                run.finish()
                self.assertEqual(
                    json.loads((out / "result.json").read_text())["verdict"], "PASS"
                )
                self.assertEqual(
                    json.loads((root / "export/result.json").read_text())["verdict"],
                    "PASS",
                )
                self.assertGreater(
                    (root / "export/visual-changelog.html").stat().st_size, 7_000_000
                )

                html = (root / "export/visual-changelog.html").read_text()
                self.assertIn("<style>", html)
                self.assertIn('charset="utf-8"', html)
                self.assertIn("max-width:100%", html)
                self.assertIn("data:image/png;base64,", html)
                self.assertIs(
                    json.loads((root / "export/export-manifest.json").read_text())[
                        "complete"
                    ],
                    True,
                )
                write_text = Path.write_text

                def fail_manifest(path, text, *args, **kwargs):
                    if path == root / "export.tmp/export-manifest.json":
                        raise OSError("disk full after copying PASS")
                    return write_text(path, text, *args, **kwargs)

                with patch.object(Path, "write_text", fail_manifest):
                    (out / "report.json").write_text(json.dumps(report))
                    self.assertFalse(run.finish())
                self.assertEqual(
                    json.loads((root / "export/result.json").read_text())["verdict"],
                    "INCOMPLETE",
                )
                self.assertFalse((root / "export.tmp").exists())
                with patch.object(run, "embed_images", return_value="x" * 30_000_001):
                    (out / "report.json").write_text(json.dumps(report))
                    self.assertTrue(run.finish())
                self.assertEqual(
                    json.loads((root / "export/result.json").read_text())["verdict"],
                    "INCOMPLETE",
                )
                self.assertFalse((root / "export/visual-changelog.html").exists())
                self.assertTrue((root / "export/annotated/screen.png").exists())
                self.assertTrue((root / "export/report.md").exists())
                (root / "context/commits.txt").write_text(
                    "abcdef0123456 visible\n123456789abc internal"
                )
                self.assertFalse(run.evidence_complete())
                meta = json.loads((out / "meta.json").read_text())
                meta["coverage"].append(
                    {
                        "commit": "1234567",
                        "subject": "Internal",
                        "images": [],
                        "note": "no user-visible change",
                    }
                )
                (out / "meta.json").write_text(json.dumps(meta))
                self.assertTrue(run.evidence_complete())
                self.assertIn("Refund skipped", (out / "report.md").read_text())
                (out / "meta.json").write_text("{}")
                (out / "report.json").write_text(json.dumps(report))
                run.finish()
                self.assertEqual(
                    json.loads((out / "result.json").read_text())["verdict"],
                    "INCOMPLETE",
                )
                (out / "report.json").write_text("{invalid")
                run.finish()
                self.assertEqual(
                    json.loads((out / "result.json").read_text())["verdict"],
                    "INCOMPLETE",
                )
                (out / "result.json").write_text('{"verdict":"PASS"}')
                (out / "report.json").unlink()
                run.finish()
                self.assertEqual(
                    json.loads((root / "export/result.json").read_text())["verdict"],
                    "INCOMPLETE",
                )


if __name__ == "__main__":
    unittest.main()
