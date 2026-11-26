"""Workflow environments must satisfy the scripts invoked by each step."""

import ast
import json
import os
import re
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import prepare
from test_security_followup import evaluate, workflow
from test_process_and_export import SOURCE


RUNNER_KEYS = {
    "PATH",
    "GITHUB_REPOSITORY",
    "GITHUB_RUN_ID",
    "GITHUB_OUTPUT",
    "GITHUB_STEP_SUMMARY",
}


def required_keys(source, entry):
    tree = ast.parse(source)
    functions = {n.name: n for n in tree.body if isinstance(n, ast.FunctionDef)}
    pending = [n for n in tree.body if not isinstance(n, ast.FunctionDef)]
    pending.extend([functions[entry]] if entry in functions else functions.values())
    visited, required = set(), set()
    while pending:
        node = pending.pop()
        for child in ast.walk(node):
            if (
                isinstance(child, ast.Subscript)
                and ast.unparse(child.value) == "os.environ"
                and isinstance(child.ctx, ast.Load)
            ):
                required.add(ast.literal_eval(child.slice))
            if isinstance(child, ast.Call) and isinstance(child.func, ast.Name):
                name = child.func.id
                if name in functions and name not in visited:
                    visited.add(name)
                    pending.append(functions[name])
    return required


class WorkflowEnvironmentTests(unittest.TestCase):
    def test_every_python_step_declares_required_environment(self):
        config = workflow()
        for job_name, job in config["jobs"].items():
            for step in job["steps"]:
                command = step.get("run", "")
                if not command.startswith("python3 "):
                    continue
                if command.startswith("python3 - <<'PY'\n"):
                    source = command.split("\n", 1)[1].rsplit("\nPY", 1)[0]
                    entry = ""
                else:
                    args = command.split()
                    path = Path(args[1])
                    if path.is_absolute():
                        path = Path("scripts/ci/release-qa") / path.name
                    source = (SOURCE / path).read_text()
                    entry = args[2] if len(args) > 2 else "main"
                available = RUNNER_KEYS | config.get("env", {}).keys()
                available |= job.get("env", {}).keys() | step.get("env", {}).keys()
                with self.subTest(job=job_name, step=step.get("name")):
                    self.assertEqual(required_keys(source, entry) - available, set())

    def test_prepare_entry_point_with_workflow_environment(self):
        config = workflow()
        for job_name in ("prepare", "prepare_pr"):
            job = config["jobs"][job_name]
            step = next(
                s for s in job["steps"] if s.get("run", "").endswith("/prepare.py")
            )
            for omit_tag in (False, True) if job_name == "prepare_pr" else (False,):
                with (
                    self.subTest(job=job_name, omit_tag=omit_tag),
                    tempfile.TemporaryDirectory() as temp,
                ):
                    root = Path(temp)
                    values = {
                        "github.workspace": str(root / "checkout"),
                        "runner.temp": str(root / "runner"),
                        "github.token": "test-token",
                        "inputs.tag": "v1.0.0-weekly.20261117",
                        "inputs.baseline": "",
                    }
                    env = {"GITHUB_REPOSITORY": "owner/repo"}
                    for name, value in (
                        config.get("env", {}) | job.get("env", {}) | step.get("env", {})
                    ).items():
                        env[name] = re.sub(
                            r"\$\{\{\s*(.*?)\s*\}\}", lambda m: values[m[1]], value
                        )
                    if omit_tag:
                        env.pop("NEW_TAG", None)
                    (root / "checkout").mkdir()
                    staging = Path(env["QA_DOWNLOAD"])
                    staging.mkdir(parents=True)
                    binary = staging / "dash-evo-tool-linux-x86_64.zip"
                    binary.write_bytes(b"verified zip")
                    releases = [
                        dict(
                            tag_name=f"v1.0.0-weekly.202611{day}",
                            prerelease=True,
                            draft=False,
                            created_at=day,
                            published_at=day,
                        )
                        for day in ("10", "24", "17")
                    ]

                    def execute(*args):
                        if args[:2] == ("gh", "api"):
                            return json.dumps([releases])
                        if args[:2] == ("git", "rev-parse"):
                            return args[2].removesuffix("^{commit}")
                        return ""

                    with (
                        patch.dict(os.environ, env, clear=True),
                        patch.object(prepare, "run", side_effect=execute),
                    ):
                        prepare.main()
                    inputs = json.loads(
                        (Path(env["QA_ROOT"]) / "context/inputs.json").read_text()
                    )
                    self.assertEqual(
                        inputs["new_tag"],
                        releases[1 if job_name == "prepare_pr" else 2]["tag_name"],
                    )
                    self.assertEqual(
                        inputs["baseline"],
                        releases[2 if job_name == "prepare_pr" else 0]["tag_name"],
                    )
                    self.assertEqual(
                        (Path(env["QA_ROOT"]) / binary.name).read_bytes(),
                        binary.read_bytes(),
                    )

    def test_report_download_is_bound_to_successful_review_upload(self):
        jobs = workflow()["jobs"]
        review, report = jobs["review"], jobs["report"]
        upload = next(
            s
            for s in review["steps"]
            if s.get("with", {}).get("name") == "release-qa-report"
        )
        self.assertEqual(upload.get("id"), "upload")
        self.assertEqual(
            review["outputs"].get("report_artifact_id"),
            "${{ steps.upload.outcome == 'success' && steps.upload.outputs.artifact-id || '' }}",
        )
        self.assertIn("review", report["needs"])
        downloads = [
            s
            for s in report["steps"]
            if s.get("uses", "").startswith("actions/download-artifact@")
        ]
        self.assertEqual(len(downloads), 1)
        download = downloads[0]
        self.assertNotIn("name", download["with"])
        self.assertNotIn("pattern", download["with"])
        self.assertEqual(
            download["with"].get("artifact-ids"),
            "${{ needs.review.outputs.report_artifact_id }}",
        )
        self.assertEqual(download["with"].get("merge-multiple"), "true")
        self.assertEqual(download["with"]["path"], "report")
        for artifact_id, expected in (("", False), ("12345", True)):
            with self.subTest(artifact_id=artifact_id):
                self.assertEqual(
                    bool(
                        evaluate(
                            download["if"],
                            **{
                                "needs.review.outputs.report_artifact_id": artifact_id,
                            },
                        )
                    ),
                    expected,
                )
        result = next(s for s in report["steps"] if s.get("id") == "result")
        self.assertEqual(
            result["env"].get("REPORT_ARTIFACT_ID"),
            "${{ needs.review.outputs.report_artifact_id }}",
        )
