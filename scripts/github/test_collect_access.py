"""The administrator report is read-only and excludes credential values."""

import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import collect_access


class CollectAccessTests(unittest.TestCase):
    def test_every_api_request_is_explicitly_get(self):
        reply = subprocess.CompletedProcess(
            [], 0, stdout='{"login":"operator"}', stderr=""
        )
        with patch.object(collect_access.subprocess, "run", return_value=reply) as run:
            collect_access.api_get("user", "user")
        command = run.call_args.args[0]
        self.assertEqual(command[:4], ["gh", "api", "--method", "GET"])
        self.assertNotIn("--include", command)

    def test_reports_only_secret_names_and_dates(self):
        raw = {
            "total_count": 1,
            "secrets": [
                {
                    "name": "SIGNING_KEY",
                    "created_at": "yesterday",
                    "updated_at": "today",
                    "value": "MUST-NOT-APPEAR",
                    "encrypted_value": "ALSO-MUST-NOT-APPEAR",
                }
            ],
        }
        clean = collect_access.project("secrets", raw)
        self.assertEqual(clean["secrets"][0]["name"], "SIGNING_KEY")
        self.assertNotIn("MUST-NOT-APPEAR", json.dumps(clean))

    def test_repository_metadata_does_not_copy_clone_credentials(self):
        clean = collect_access.project(
            "repo",
            {
                "full_name": "dashpay/dash-evo-tool",
                "permissions": {"admin": True},
                "temp_clone_token": "MUST-NOT-APPEAR",
            },
        )
        self.assertNotIn("MUST-NOT-APPEAR", json.dumps(clean))

    def test_error_does_not_mean_protection_is_disabled(self):
        reply = subprocess.CompletedProcess(
            [],
            1,
            stdout='{"message":"Forbidden","status":"403"}',
            stderr="gh: Forbidden",
        )
        with patch.object(collect_access.subprocess, "run", return_value=reply):
            result = collect_access.api_get(
                "actions", "repos/owner/repo/actions/permissions"
            )
        self.assertFalse(result["ok"])
        self.assertEqual(result["error"]["status"], "403")
        self.assertNotIn("enabled", result)

    def test_all_pages_of_an_inventory_are_retained(self):
        payload = [{"secrets": [{"name": "FIRST"}]}, {"secrets": [{"name": "SECOND"}]}]
        self.assertEqual(
            [s["name"] for s in collect_access.pages(payload)["secrets"]],
            ["FIRST", "SECOND"],
        )

    def test_pull_request_sample_is_bounded_and_keeps_single_item_lists(self):
        reply = subprocess.CompletedProcess(
            [], 0, stdout='[{"number":1,"draft":false}]', stderr=""
        )
        with patch.object(collect_access.subprocess, "run", return_value=reply) as run:
            result = collect_access.api_get(
                "pulls", "repos/owner/repo/pulls?per_page=5", paginate=False
            )
        self.assertNotIn("--paginate", run.call_args.args[0])
        self.assertEqual(result["data"], [{"number": 1, "draft": False}])

    def test_timeout_keeps_partial_evidence_available(self):
        with patch.object(
            collect_access.subprocess,
            "run",
            side_effect=subprocess.TimeoutExpired("gh", 90),
        ):
            result = collect_access.api_get(
                "actions", "repos/owner/repo/actions/permissions"
            )
        self.assertFalse(result["ok"])
        self.assertEqual(result["error"]["message"], "TimeoutExpired")

    def test_graphql_partial_answer_is_not_accepted_as_complete(self):
        reply = subprocess.CompletedProcess(
            [],
            0,
            stdout=json.dumps(
                {"data": {"repository": None}, "errors": [{"message": "Unavailable"}]}
            ),
            stderr="",
        )
        with patch.object(collect_access.subprocess, "run", return_value=reply) as run:
            result = collect_access.api_get(
                "checks", "graphql", "query { viewer { login } }"
            )
        self.assertFalse(result["ok"])
        self.assertNotIn("--paginate", run.call_args.args[0])
        self.assertEqual(run.call_args.args[0][:4], ["gh", "api", "--method", "POST"])

    def test_graphql_mutations_are_refused_without_making_a_request(self):
        with patch.object(collect_access.subprocess, "run") as run:
            with self.assertRaises(ValueError):
                collect_access.api_get("checks", "graphql", "mutation { forbidden }")
        run.assert_not_called()

    def test_required_commit_status_is_not_confused_with_the_controller_job(self):
        pr = {
            "number": 1,
            "commits": {
                "nodes": [
                    {
                        "commit": {
                            "statusCheckRollup": {
                                "contexts": {
                                    "pageInfo": {"hasNextPage": False},
                                    "nodes": [
                                        {
                                            "__typename": "StatusContext",
                                            "context": "PR Hygiene",
                                            "isRequired": True,
                                        },
                                        {
                                            "__typename": "CheckRun",
                                            "name": "policy / reconcile",
                                            "isRequired": False,
                                        },
                                    ],
                                }
                            }
                        }
                    }
                ]
            },
        }
        with patch.object(
            collect_access,
            "api_get",
            return_value={
                "ok": True,
                "data": {"data": {"repository": {"pullRequest": pr}}},
            },
        ):
            result = collect_access.required_checks("owner/repo", 1)
        nodes = result["data"]["commits"]["nodes"][0]["commit"]["statusCheckRollup"][
            "contexts"
        ]["nodes"]
        self.assertTrue(nodes[0]["isRequired"])
        self.assertFalse(nodes[1]["isRequired"])

    def test_output_is_private_and_never_overwrites_an_existing_report(self):
        with tempfile.TemporaryDirectory() as scratch:
            output = Path(scratch) / "report.json"
            collect_access.write_report(output, {"read_only": True})
            self.assertEqual(output.stat().st_mode & 0o777, 0o600)
            with self.assertRaises(FileExistsError):
                collect_access.write_report(output, {})

    def test_wrong_operator_is_rejected_before_repository_reads(self):
        with (
            tempfile.TemporaryDirectory() as scratch,
            patch.object(
                collect_access,
                "api_get",
                return_value={"ok": True, "data": {"login": "agent"}},
            ) as read,
        ):
            result = collect_access.main(
                [
                    "--expected-user",
                    "operator",
                    "--output",
                    str(Path(scratch) / "report.json"),
                ]
            )
        self.assertEqual(result, 1)
        self.assertEqual(read.call_count, 1)


if __name__ == "__main__":
    unittest.main()
