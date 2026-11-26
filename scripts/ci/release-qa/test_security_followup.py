"""Offline authorization, scheduling and delayed-close regressions."""

import json
import os
import re
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import yaml

os.environ.setdefault("QA_ROOT", "/tmp/release-qa-tests-unused")
import logic
import run
import wallet
from test_process_and_export import REPORT, SOURCE


def workflow():
    return yaml.load(
        (SOURCE / ".github/workflows/release-qa.yml").read_text(),
        Loader=yaml.BaseLoader,
    )


def evaluate(expression, **overrides):
    values = {
        "github.event_name": "pull_request",
        "github.ref": "refs/pull/1/merge",
        "github.event.action": "labeled",
        "github.event.label.name": "release-qa",
        "github.event.pull_request.number": 1,
        "github.event.pull_request.head.repo.full_name": "owner/repo",
        "github.event.pull_request.base.repo.full_name": "owner/repo",
        "github.event.sender.login": "lklimek",
        "github.event.sender.type": "User",
        "github.run_id": 123,
        "vars.RELEASE_QA_LABEL_ACTORS": "",
        "inputs.tag": "v1.0.0-weekly.test",
    } | overrides
    expression = expression.removeprefix("${{").removesuffix("}}").strip()
    expression = re.sub(
        r"\b(?:github|vars|inputs|needs)\.[\w.]+",
        lambda match: repr(values[match[0]]),
        expression,
    )
    expression = re.sub(r"!(?!=)", " not ", expression)
    return eval(
        expression.replace("&&", " and ").replace("||", " or "),
        {"__builtins__": {}},
        {
            "contains": lambda text, part: part.lower() in text.lower(),
            "format": lambda template, *args: template.format(*args),
            "always": lambda: True,
            "cancelled": lambda: False,
        },
    )


def reachable(**event):
    results = {}
    reached = []
    for name, job in workflow()["jobs"].items():
        needs = job.get("needs", [])
        if isinstance(needs, str):
            needs = [needs]
        condition = job.get("if")
        active = (
            evaluate(condition, **(event | results))
            if condition
            else all(results[f"needs.{dep}.result"] == "success" for dep in needs)
        )
        results[f"needs.{name}.result"] = "success" if active else "skipped"
        if active:
            reached.append(name)
    return reached


class LabelAuthorizationTests(unittest.TestCase):
    def test_label_actor_allowlist_and_exact_membership(self):
        condition = next(iter(workflow()["jobs"].values()))["if"]
        for actors, sender, allowed in (
            ("", "lklimek", True),
            ("", "someone", False),
            ("", "Claudius-Maginificent", False),
            ("", "github-actions[bot]", False),
            ("", "", False),
            ("", "klime", False),
            ("other", "lklimek", False),
            ("lklimek,other", "other", True),
            ("other,lklimek", "other", True),
            ("one other two", "other", True),
            ("one, other,two", "other", True),
            ("one,other two", "other", True),
            ("  one,  other  ,two ", "other", True),
            ("other-person,another", "other", False),
            (" , ", "lklimek", False),
        ):
            with self.subTest(actors=actors, sender=sender):
                event = {
                    "vars.RELEASE_QA_LABEL_ACTORS": actors,
                    "github.event.sender.login": sender,
                }
                self.assertEqual(bool(evaluate(condition, **event)), allowed)
                self.assertEqual(bool(reachable(**event)), allowed)

    def test_invalid_events_never_reach_any_job(self):
        for change in (
            {"github.event.pull_request.head.repo.full_name": "fork/repo"},
            {"github.event.pull_request.head.repo.full_name": ""},
            {"github.event.action": "opened"},
            {"github.event.label.name": "other"},
            {"github.event.sender.type": "Bot"},
        ):
            with self.subTest(change=change):
                self.assertEqual(reachable(**change), [])

    def test_label_path_is_read_only_and_trusted_prepare_stays_exclusive(self):
        jobs = workflow()["jobs"]
        self.assertEqual(reachable(), ["prepare_pr", "review", "report"])
        for name in reachable():
            self.assertNotIn("write", jobs[name].get("permissions", {}).values())
        for event in ("workflow_dispatch", "workflow_call", "schedule"):
            self.assertEqual(
                reachable(
                    **{"github.event_name": event, "github.ref": "refs/heads/v1.0-dev"}
                ),
                ["prepare", "review", "report"],
            )
            self.assertEqual(
                reachable(
                    **{"github.event_name": event, "github.ref": "refs/heads/feature"}
                ),
                [],
            )

    def test_non_qa_labels_have_unique_concurrency_groups(self):
        group = workflow()["concurrency"]["group"]
        expression = group[group.index("${{") :]
        qa = evaluate(expression)
        other = evaluate(expression, **{"github.event.label.name": "other"})
        another = evaluate(
            expression, **{"github.event.label.name": "other", "github.run_id": 124}
        )
        self.assertNotEqual(qa, other)
        self.assertNotEqual(other, another)
        self.assertEqual(qa, evaluate(expression, **{"github.run_id": 125}))


class DelayedCloseTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.now = 0.0
        self.exit_after = None
        (self.root / "gui.pid").write_text(str(os.getpid()))
        (self.root / "window").write_text("42")
        for target, name, value in (
            (run, "ROOT", self.root),
            (run.time, "monotonic", lambda: self.now),
            (run.time, "sleep", self.tick),
            (run.subprocess, "run", lambda *a, **kw: SimpleNamespace(returncode=0)),
        ):
            p = patch.object(target, name, value)
            p.start()
            self.addCleanup(p.stop)
        self.kill = patch.object(run.os, "kill").start()
        self.addCleanup(patch.stopall)
        run.receipt(smoke=True)

    def tick(self, seconds):
        self.now += seconds
        if self.exit_after is not None and self.now >= self.exit_after:
            (self.root / "gui-exit.json").write_text(
                json.dumps({"code": 0, "controlled": True})
            )

    def test_clean_close_after_55_seconds_passes_without_signals(self):
        self.exit_after = 55
        self.assertIsNot(run.stop(), False)
        self.kill.assert_not_called()
        self.assertIs(run.read(self.root / "receipt.json")["smoke"], True)

    def test_close_timeout_is_incomplete_and_waits_beyond_app_deadline(self):
        self.assertIs(run.stop(), False)
        self.assertGreaterEqual(self.now, 70)
        state = run.read(self.root / "receipt.json")
        self.assertEqual(
            logic.verdict(REPORT, smoke=state.get("smoke"), agent_ok=True), "INCOMPLETE"
        )
        # A late exit after fallback must not turn a second stop into FAIL.
        (self.root / "gui-exit.json").write_text('{"code":-15,"controlled":true}')
        self.assertIs(run.stop(), False)
        state = run.read(self.root / "receipt.json")
        self.assertEqual(
            logic.verdict(REPORT, smoke=state.get("smoke"), agent_ok=True), "INCOMPLETE"
        )

    def test_timeout_without_monitor_receipt_stays_incomplete_on_retry(self):
        self.assertIs(run.stop(), False)
        exists = Path.exists
        with patch.object(
            Path,
            "exists",
            lambda path: False if str(path).startswith("/proc/") else exists(path),
        ):
            self.assertIs(run.stop(), False)
        state = run.read(self.root / "receipt.json")
        self.assertEqual(
            logic.verdict(REPORT, smoke=state.get("smoke"), agent_ok=True), "INCOMPLETE"
        )

    def test_timeout_does_not_erase_an_earlier_crash(self):
        run.receipt(smoke=False)
        self.assertIs(run.stop(), False)
        self.assertIs(run.read(self.root / "receipt.json")["smoke"], False)

    def test_smoke_preserves_incomplete_close(self):
        def incomplete():
            run.receipt(smoke=None)
            return False

        with (
            patch.object(wallet, "det", return_value={"active": "testnet"}),
            patch.object(run.subprocess, "check_output", return_value="bus\n42"),
            patch.object(run, "launch"),
            patch.object(run, "resume"),
            patch.object(run, "stop", side_effect=incomplete),
        ):
            with self.assertRaises(RuntimeError):
                run.smoke()
        self.assertEqual(
            logic.verdict(
                REPORT,
                smoke=run.read(self.root / "receipt.json").get("smoke"),
                agent_ok=True,
            ),
            "INCOMPLETE",
        )
