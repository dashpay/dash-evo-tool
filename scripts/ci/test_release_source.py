"""Release authorization uses immutable commits from reviewed branch history."""

import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from release_source import (
    ReleaseSourceError,
    cleanup_plan,
    resolve_source,
    verify_remote_tag,
)


class ReleaseSourceTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.repo = Path(self.scratch.name)
        self.git("init", "-q", "-b", "master")
        self.git("config", "user.name", "Release tests")
        self.git("config", "user.email", "release-tests@example.invalid")
        self.base = self.commit("base")
        self.git("update-ref", "refs/remotes/origin/master", self.base)
        self.git("checkout", "-q", "-b", "v1.0-dev")
        self.reviewed = self.commit("reviewed")
        self.git("update-ref", "refs/remotes/origin/v1.0-dev", self.reviewed)
        self.git("checkout", "-q", "-b", "agent/work")
        self.unreviewed = self.commit("unreviewed")

    def git(self, *args):
        return subprocess.run(
            ["git", "-C", str(self.repo), *args],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()

    def commit(self, text):
        (self.repo / "content").write_text(text)
        self.git("add", "content")
        self.git("commit", "-qm", text)
        return self.git("rev-parse", "HEAD")

    def resolve(self, ref="refs/heads/v1.0-dev", sha=None, source="", tag=""):
        return resolve_source(self.repo, ref, sha or self.reviewed, source, tag)

    def test_weekly_uses_event_sha_even_when_branch_has_advanced(self):
        self.assertEqual(self.resolve()["source_sha"], self.reviewed)
        self.assertEqual(
            self.resolve(source=self.reviewed)["source_sha"], self.reviewed
        )

    def test_feature_branch_cannot_authorize_signing(self):
        with self.assertRaises(ReleaseSourceError):
            self.resolve("refs/heads/agent/work", self.unreviewed)

    def test_default_branch_dispatch_cannot_select_another_commit(self):
        with self.assertRaises(ReleaseSourceError):
            self.resolve(source=self.unreviewed)

    def test_tag_input_cannot_redirect_a_trusted_branch_build(self):
        self.git("tag", "v9.0.0", self.unreviewed)
        with self.assertRaises(ReleaseSourceError):
            self.resolve(tag="v9.0.0")

    def test_new_tag_names_the_current_source(self):
        result = self.resolve(tag="v1.0.0-weekly.20261028+abcdef123")
        self.assertEqual(result["source_sha"], self.reviewed)
        self.assertEqual(result["tag"], "v1.0.0-weekly.20261028+abcdef123")

    def test_annotated_tag_is_peeled_to_its_reviewed_commit(self):
        self.git("tag", "-a", "v1.0.0", self.reviewed, "-m", "release")
        result = self.resolve("refs/tags/v1.0.0", tag="v1.0.0")
        self.assertEqual(result["source_sha"], self.reviewed)

    def test_tag_from_unreviewed_history_is_rejected(self):
        self.git("tag", "v9.0.0", self.unreviewed)
        with self.assertRaises(ReleaseSourceError):
            self.resolve("refs/tags/v9.0.0", self.unreviewed)

    def test_stable_release_from_master_history_is_allowed(self):
        self.git("tag", "v0.9.0", self.base)
        self.assertEqual(
            self.resolve("refs/tags/v0.9.0", self.base)["source_sha"], self.base
        )

    def test_moved_tag_is_rejected(self):
        self.git("tag", "v1.0.0", self.unreviewed)
        with self.assertRaises(ReleaseSourceError):
            self.resolve("refs/tags/v1.0.0")

    def test_tag_trigger_cannot_publish_under_a_different_tag(self):
        self.git("tag", "v1.0.0", self.reviewed)
        with self.assertRaises(ReleaseSourceError):
            self.resolve("refs/tags/v1.0.0", tag="v2.0.0")

    def test_missing_commit_or_malformed_inputs_fail_closed(self):
        for sha in ("f" * 40, "", "HEAD", "\n" + self.reviewed):
            with self.subTest(sha=sha), self.assertRaises(ReleaseSourceError):
                resolve_source(self.repo, "refs/heads/v1.0-dev", sha, "", "")
        for tag in ("v1.0\nsource_sha=oops", "--help", "v1/../../main"):
            with self.subTest(tag=tag), self.assertRaises(ReleaseSourceError):
                self.resolve(tag=tag)

    def test_trusted_branch_name_does_not_authorize_unmerged_source(self):
        with self.assertRaises(ReleaseSourceError):
            self.resolve(sha=self.unreviewed)


class RemoteTagTests(unittest.TestCase):
    def reply(self, payload, code=0):
        import json

        return subprocess.CompletedProcess(
            [], code, stdout=json.dumps(payload), stderr=""
        )

    def test_annotated_tag_is_checked_against_the_authorized_commit(self):
        replies = [
            self.reply({"object": {"type": "tag", "sha": "a" * 40}}),
            self.reply({"object": {"type": "commit", "sha": "b" * 40}}),
        ]
        with patch("release_source.subprocess.run", side_effect=replies) as api:
            verify_remote_tag("owner/repo", "v1.0.0", "b" * 40)
        for call in api.call_args_list:
            self.assertEqual(call.args[0][:4], ["gh", "api", "--method", "GET"])

    def test_moved_remote_tag_is_rejected(self):
        reply = self.reply({"object": {"type": "commit", "sha": "c" * 40}})
        with (
            patch("release_source.subprocess.run", return_value=reply),
            self.assertRaises(ReleaseSourceError),
        ):
            verify_remote_tag("owner/repo", "v1.0.0", "b" * 40)

    def test_only_an_explicit_404_allows_a_new_tag(self):
        for status, allowed in (("404", True), ("403", False)):
            reply = self.reply({"status": status}, 1)
            with patch("release_source.subprocess.run", return_value=reply):
                if allowed:
                    verify_remote_tag(
                        "owner/repo", "v1.0.0", "b" * 40, allow_missing=True
                    )
                else:
                    with self.assertRaises(ReleaseSourceError):
                        verify_remote_tag(
                            "owner/repo", "v1.0.0", "b" * 40, allow_missing=True
                        )

    def test_cleanup_cannot_accept_a_deleted_tag(self):
        with patch(
            "release_source.subprocess.run",
            return_value=self.reply({"status": "404"}, 1),
        ):
            with self.assertRaises(ReleaseSourceError):
                verify_remote_tag("owner/repo", "v1.0.0", "b" * 40)

    def test_cleanup_refuses_published_or_unrecognized_releases(self):
        for release in (
            {"draft": False, "body": "<!-- weekly-version: test -->"},
            {"draft": True, "body": "official draft"},
        ):
            replies = [
                self.reply({"object": {"type": "commit", "sha": "b" * 40}}),
                self.reply(release),
            ]
            with patch("release_source.subprocess.run", side_effect=replies):
                with self.assertRaises(ReleaseSourceError):
                    cleanup_plan("owner/repo", "v1.0.0", "b" * 40)

    def test_cleanup_can_delete_only_its_draft_or_an_orphaned_tag(self):
        for release, expected in (
            (
                self.reply({"draft": True, "body": "<!-- weekly-version: test -->"}),
                "true",
            ),
            (self.reply({"status": "404"}, 1), "false"),
        ):
            replies = [
                self.reply({"object": {"type": "commit", "sha": "b" * 40}}),
                release,
            ]
            with patch("release_source.subprocess.run", side_effect=replies):
                self.assertEqual(
                    cleanup_plan("owner/repo", "v1.0.0", "b" * 40)["delete_release"],
                    expected,
                )


if __name__ == "__main__":
    unittest.main()
