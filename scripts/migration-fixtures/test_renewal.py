"""Offline contract tests for fixture renewal planning and manifest updates."""

import copy
import hashlib
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path

import renewal


class RenewalTests(unittest.TestCase):
    def setUp(self):
        self.now = datetime(2026, 11, 15, tzinfo=timezone.utc)
        self.entry = {
            "id": "historical-wallet",
            "contents": {"wallets": [{"password_env": "FIXTURE_PASSWORD"}]},
            "artifact": {
                "workflow_name": "migration-fixture-bootstrap.yml",
                "workflow_run_id": 10,
                "artifact_name": "migration-fixture-historical-wallet",
                "archive_filename": "historical.tar.zst",
                "sha256": hashlib.sha256(b"archive").hexdigest(),
                "bytes": 7,
                "expires_at": "2026-12-10T00:00:00Z",
                "retention_days": 90,
            },
        }
        self.manifest = {"schema_version": 1, "fixtures": [self.entry]}
        self.receipt = {
            "fixture_id": self.entry["id"],
            "source_artifact": copy.deepcopy(self.entry["artifact"]),
            "uploaded": {
                "name": self.entry["artifact"]["artifact_name"],
                "expired": False,
                "expires_at": "2027-02-13T00:00:00Z",
                "workflow_run": {"id": 20},
            },
        }

    def test_due_boundary_and_force(self):
        self.assertEqual(
            renewal.plan(self.manifest, self.now, 25)["include"],
            [{"fixture": self.entry}],
        )
        self.assertEqual(renewal.plan(self.manifest, self.now, 24)["include"], [])
        self.assertEqual(
            len(renewal.plan(self.manifest, self.now, 1, force=True)["include"]), 1
        )

    def test_expired_source_is_an_error_even_with_force(self):
        self.entry["artifact"]["expires_at"] = "2026-11-14T00:00:00Z"
        with self.assertRaisesRegex(ValueError, "expired"):
            renewal.plan(self.manifest, self.now, 30, force=True)

    def test_invalid_manifest_rejected_before_network(self):
        for field, value in [
            ("sha256", ""),
            ("bytes", 0),
            ("workflow_run_id", None),
            ("archive_filename", "../escape.tar.zst"),
            ("expires_at", "invalid"),
        ]:
            with self.subTest(field=field):
                manifest = copy.deepcopy(self.manifest)
                manifest["fixtures"][0]["artifact"][field] = value
                with self.assertRaises(ValueError):
                    renewal.plan(manifest, self.now, 30)

    def test_duplicate_artifact_names_rejected(self):
        duplicate = copy.deepcopy(self.entry)
        duplicate["id"] = "another-wallet"
        self.manifest["fixtures"].append(duplicate)
        with self.assertRaisesRegex(ValueError, "duplicate"):
            renewal.plan(self.manifest, self.now, 30)

    def test_empty_manifest_is_an_error(self):
        with self.assertRaises(ValueError):
            renewal.plan({"fixtures": []}, self.now, 30)

    def test_archive_bytes_and_digest_are_verified(self):
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "historical.tar.zst"
            archive.write_bytes(b"archive")
            renewal.verify_archive(self.entry, archive)
            archive.write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "checksum"):
                renewal.verify_archive(self.entry, archive)
            archive.write_bytes(b"short")
            with self.assertRaisesRegex(ValueError, "size"):
                renewal.verify_archive(self.entry, archive)

    def test_only_artifact_pointers_change(self):
        original = copy.deepcopy(self.manifest)
        result = renewal.renew_manifest(self.manifest, [self.receipt], self.now)
        expected = copy.deepcopy(original)
        expected["fixtures"][0]["artifact"].update(
            workflow_name="migration-fixture-renewal.yml",
            workflow_run_id=20,
            expires_at="2027-02-13T00:00:00Z",
            retention_days=90,
        )
        self.assertEqual(result, expected)
        self.assertEqual(self.manifest, original)

    def test_stale_manifest_is_not_overwritten(self):
        self.entry["artifact"]["workflow_run_id"] = 11
        with self.assertRaisesRegex(ValueError, "changed"):
            renewal.renew_manifest(self.manifest, [self.receipt], self.now)

    def test_bad_receipts_rejected(self):
        for field, value in [
            ("name", "other"),
            ("expired", True),
            ("expires_at", "2026-12-01T00:00:00Z"),
            ("workflow_run", {"id": 10}),
        ]:
            with self.subTest(field=field):
                receipt = copy.deepcopy(self.receipt)
                receipt["uploaded"][field] = value
                with self.assertRaises(ValueError):
                    renewal.renew_manifest(self.manifest, [receipt], self.now)

    def test_duplicate_or_unknown_receipts_rejected(self):
        with self.assertRaises(ValueError):
            renewal.renew_manifest(
                self.manifest, [self.receipt, self.receipt], self.now
            )
        self.receipt["fixture_id"] = "unknown"
        with self.assertRaises(ValueError):
            renewal.renew_manifest(self.manifest, [self.receipt], self.now)


if __name__ == "__main__":
    unittest.main()
