"""Offline contracts for the fixture a release carries as an asset."""

import copy
import hashlib
import tempfile
import unittest
from datetime import date
from pathlib import Path

import release_fixture

TAG = "v1.0.0-weekly.20261013"
SOURCE = {
    "git_tag": "v1.0.0-weekly.20261006",
    "archive": "v1.0.0-weekly.20261006-wallet-only.tar.zst",
    "sha256": "a" * 64,
}


class ReleaseFixtureTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.archive = Path(self.tmp.name) / release_fixture.asset_name(TAG, "tar.zst")
        self.archive.write_bytes(b"packed profile")
        self.receipt = {
            "capture_method": "release-chain",
            "git_tag": TAG,
            "network": "testnet",
            "source": SOURCE,
            "binary_sha256": "b" * 64,
            "wallets": [{"seed_hash": "c" * 64, "alias": "weekly-fixture"}],
            "archive": f"{TAG}-wallet-only.tar.zst",
            "sha256": hashlib.sha256(b"packed profile").hexdigest(),
            "bytes": len(b"packed profile"),
        }

    def build(self, **overrides):
        arguments = {
            "receipt": self.receipt,
            "archive": self.archive,
            "tag": TAG,
            "det_version": "1.0.0-weekly.20261013+abcdef012",
            "platform_rev": "d" * 40,
            "run_id": 42,
            "today": date(2026, 10, 13),
        }
        return release_fixture.build(**(arguments | overrides))

    def test_build_describes_the_archive_as_a_manifest_entry(self):
        entry = self.build()
        self.assertEqual(entry["id"], f"{TAG}-wallet-only")
        self.assertEqual(entry["captured_at"], "2026-10-13")
        self.assertEqual(entry["provenance"]["source"], SOURCE)
        self.assertEqual(entry["artifact"]["archive_filename"], self.archive.name)
        self.assertEqual(entry["artifact"]["sha256"], self.receipt["sha256"])
        self.assertEqual(entry["artifact"]["bytes"], 14)
        self.assertEqual(entry["artifact"]["workflow_run_id"], 42)
        self.assertEqual(
            entry["expect"],
            {"wallet_aliases": ["weekly-fixture"], "derive_address": False},
        )
        release_fixture.check(entry, self.archive, TAG)

    def test_build_rejects_a_receipt_for_another_release_or_archive(self):
        with self.assertRaisesRegex(ValueError, "receipt"):
            self.build(receipt=self.receipt | {"git_tag": "v1.0.0-weekly.20261006"})
        with self.assertRaisesRegex(ValueError, "receipt"):
            self.build(receipt=self.receipt | {"sha256": "e" * 64})
        with self.assertRaisesRegex(ValueError, "wallet"):
            self.build(receipt=self.receipt | {"wallets": []})

    def test_check_rejects_an_entry_that_does_not_describe_this_release(self):
        cases = {
            "another tag": lambda e: e.update(git_tag="v1.0.0-weekly.20261006"),
            "another id": lambda e: e.update(id="v0.9.3-wallet-only"),
            "another archive name": lambda e: e["artifact"].update(
                archive_filename="x.tar.zst"
            ),
            "another digest": lambda e: e["artifact"].update(sha256="e" * 64),
            "another size": lambda e: e["artifact"].update(bytes=15),
            "mainnet": lambda e: e.update(network="mainnet"),
            "another method": lambda e: e.update(capture_method="gui"),
        }
        for name, change in cases.items():
            with self.subTest(name):
                entry = copy.deepcopy(self.build())
                change(entry)
                with self.assertRaises(ValueError):
                    release_fixture.check(entry, self.archive, TAG)
        with self.assertRaises(ValueError):
            release_fixture.check(self.build(), self.archive, "v1.0.0-weekly.20261020")

    def test_check_rejects_anything_beyond_a_passwordless_offline_profile(self):
        cases = {
            "unknown key": lambda e: e.update(extra="value"),
            "unknown artifact key": lambda e: e["artifact"].update(
                url="https://example.invalid"
            ),
            "password run": lambda e: e.update(password_runs=[{"source": "file"}]),
            "password variable": lambda e: e["contents"].update(
                wallets=[{"alias": "a", "password_env": "GITHUB_TOKEN"}]
            ),
            "network derivation": lambda e: e["expect"].update(derive_address=True),
            "no wallet": lambda e: e["expect"].update(wallet_aliases=[]),
            "not an object": lambda e: e.update(artifact="archive"),
        }
        for name, change in cases.items():
            with self.subTest(name):
                entry = copy.deepcopy(self.build())
                change(entry)
                with self.assertRaises(ValueError):
                    release_fixture.check(entry, self.archive, TAG)

    def fixtures(self, *tags):
        return {
            "fixtures": [{"id": f"{tag}-wallet-only", "git_tag": tag} for tag in tags]
        }

    def release(self, tag, extensions=("tar.zst", "json"), draft=False):
        """A release as the API lists it, carrying the named fixture assets."""
        assets = [{"name": release_fixture.asset_name(tag, e)} for e in extensions]
        return {"tag_name": tag, "draft": draft, "assets": assets}

    def test_source_is_the_newest_fixture_older_than_the_release(self):
        manifest = self.fixtures(
            "v0.9.3", "v1.0.0-weekly.9", "v1.0.0-weekly.10", "v1.0.0-weekly.30"
        )
        releases = [
            self.release(tag, ())
            for tag in ("v0.9.3", "v1.0.0-weekly.10", "v1.0.0-weekly.30")
        ]
        self.assertEqual(
            release_fixture.source(manifest, "v1.0.0-weekly.20", releases),
            "v1.0.0-weekly.10",
        )

    def test_source_is_an_attached_fixture_however_many_newer_ones_exist(self):
        manifest = self.fixtures("v1.0.0")
        attached = [self.release(f"v1.1.{patch}") for patch in range(1, 11)]
        self.assertEqual(
            release_fixture.source(manifest, "v1.1.11", attached), "v1.1.10"
        )
        # v1.1.2 lost its fixture and is captured again; nine releases are newer.
        attached[1] = self.release("v1.1.2", ())
        self.assertEqual(release_fixture.source(manifest, "v1.1.2", attached), "v1.1.1")

    def test_source_refuses_a_release_that_already_has_a_fixture(self):
        with self.assertRaisesRegex(ValueError, "already"):
            release_fixture.source(self.fixtures("v1.0.0", "v1.0.1"), "v1.0.1", [])
        with self.assertRaisesRegex(ValueError, "already"):
            release_fixture.source(
                self.fixtures("v1.0.0"), "v1.0.1", [self.release("v1.0.1")]
            )

    def test_source_refuses_to_skip_a_published_release_without_a_fixture(self):
        manifest = self.fixtures("v1.0.0")
        for name, uncovered in {
            "no asset": self.release("v1.0.1", ()),
            "archive only": self.release("v1.0.1", ("tar.zst",)),
            "entry only": self.release("v1.0.1", ("json",)),
        }.items():
            with self.subTest(name), self.assertRaisesRegex(ValueError, "v1.0.1"):
                release_fixture.source(manifest, "v1.0.2", [uncovered])

    def test_source_ignores_draft_releases(self):
        manifest = self.fixtures("v1.0.0")
        drafts = [
            self.release("v1.0.1", (), draft=True),
            self.release("v1.0.2", draft=True),
            self.release("v1.0.3", draft=True),
        ]
        self.assertEqual(release_fixture.source(manifest, "v1.0.3", drafts), "v1.0.0")

    def test_source_needs_an_older_fixture_and_a_semver_release(self):
        with self.assertRaisesRegex(ValueError, "older"):
            release_fixture.source(self.fixtures("v1.0.1"), "v1.0.0", [])
        with self.assertRaises(ValueError):
            release_fixture.source(self.fixtures("v1.0.0"), "not-a-release", [])
        self.assertEqual(
            release_fixture.source(
                self.fixtures("v1.0.0"),
                "v1.0.1",
                [self.release("test-fix-directory-searching")],
            ),
            "v1.0.0",
        )


if __name__ == "__main__":
    unittest.main()
