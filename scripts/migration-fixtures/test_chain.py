"""Offline contracts for advancing archived profiles through released binaries."""

import io
import json
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import chain


class ChainTests(unittest.TestCase):
    def test_credentials_are_cleared_without_changing_network_configuration(self):
        with tempfile.TemporaryDirectory() as tmp:
            profile = Path(tmp)
            config = profile / ".env"
            config.write_text(
                ' export MCP_API_KEY = "disposable"\n'
                'TESTNET_core_rpc_password="disposable"\n'
                'TESTNET_core_rpc_user="disposable"\n'
                'LOCAL_wallet_private_key="disposable"\n'
                'TESTNET_dapi_addresses="example.invalid"\n'
            )
            chain.clear_credentials(profile)
            self.assertNotIn("disposable", config.read_text())
            self.assertIn(
                'TESTNET_dapi_addresses="example.invalid"', config.read_text()
            )

    def test_rejects_reverse_release_order(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / "cli"
            binary.touch()
            with patch.object(chain, "check_version"):
                with self.assertRaisesRegex(ValueError, "increasing"):
                    chain.releases([f"v1.0.1={binary}", f"v1.0.0={binary}"])

    def test_rejects_mainnet_profile(self):
        with patch.object(chain, "cli", return_value={"active": "mainnet"}):
            with self.assertRaisesRegex(ValueError, "testnet"):
                chain.probe(Path("det-cli"), Path("profile"))

    def run_chain(self, root, *, lose_wallet=False):
        archive = root / "source.tar.gz"
        with tarfile.open(archive, "w:gz") as stream:
            entry = tarfile.TarInfo(".env")
            data = b"MCP_API_KEY=\n"
            entry.size = len(data)
            stream.addfile(entry, io.BytesIO(data))
        original = archive.read_bytes()
        argv = [
            "chain.py",
            "--archive",
            str(archive),
            "--sha256",
            chain.digest(archive),
            "--source-tag",
            "v1.0.0",
            "--work-dir",
            str(root / "work"),
            "--output-dir",
            str(root / "output"),
        ]
        for index, version in enumerate(("1.0.0", "1.0.1", "1.0.2")):
            binary = root / f"cli-{index}"
            wallets = (
                []
                if lose_wallet and index == 2
                else [{"alias": "fixture", "seed_hash": "a"}]
            )
            binary.write_text(
                "#!/usr/bin/env python3\nimport json, os, sys\nfrom pathlib import Path\n"
                f"if '--version' in sys.argv: print('det-cli {version}')\n"
                "else:\n"
                "    assert '--standalone' in sys.argv\n"
                "    profile = Path(os.environ['DASH_EVO_DATA_DIR'])\n"
                f"    (profile / 'last-version').write_text('{version}')\n"
                f"    print(json.dumps({{'active': 'testnet'}} if sys.argv[-1] == 'network-info' else {{'wallets': {wallets!r}}}))\n"
            )
            binary.chmod(0o700)
            argv += ["--release", f"v{version}={binary}"]
        with patch.object(sys, "argv", argv):
            if lose_wallet:
                with self.assertRaises(ValueError):
                    chain.main()
            else:
                chain.main()
        self.assertEqual(archive.read_bytes(), original)
        return root / "output"

    def test_chain_snapshots_each_release_without_mutating_source(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = self.run_chain(Path(tmp))
            first = json.loads((output / "v1.0.1-wallet-only.receipt.json").read_text())
            second = json.loads(
                (output / "v1.0.2-wallet-only.receipt.json").read_text()
            )
            self.assertEqual(second["source"]["sha256"], first["sha256"])
            self.assertEqual(second["source"]["git_tag"], "v1.0.1")
            self.assertEqual(chain.digest(output / second["archive"]), second["sha256"])
            for version, receipt in (("1.0.1", first), ("1.0.2", second)):
                staged = Path(tmp) / version
                chain.extract(output / receipt["archive"], staged)
                self.assertEqual((staged / "last-version").read_text(), version)

    def test_failed_hop_is_not_packed(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = self.run_chain(Path(tmp), lose_wallet=True)
            self.assertTrue((output / "v1.0.1-wallet-only.receipt.json").exists())
            self.assertEqual(list(output.glob("v1.0.2*")), [])

    def test_rejects_wrong_binary_version(self):
        with patch.object(chain, "run", return_value="det-cli 1.0.0-weekly.9"):
            with self.assertRaisesRegex(ValueError, "version"):
                chain.check_version(Path("det-cli"), "v1.0.0-weekly.10")

    def test_accepts_build_metadata(self):
        with patch.object(chain, "run", return_value="det-cli 1.0.0-weekly.10+abc"):
            chain.check_version(Path("det-cli"), "v1.0.0-weekly.10")

    def test_rejects_missing_or_replaced_wallet(self):
        before = [{"alias": "fixture", "seed_hash": "a"}]
        for after in ([], [{"alias": "fixture", "seed_hash": "b"}]):
            with self.subTest(after=after), self.assertRaises(ValueError):
                chain.check_wallets(before, after)

    def test_rejects_empty_baseline(self):
        with self.assertRaises(ValueError):
            chain.check_wallets([], [])

    def test_wallet_order_is_irrelevant(self):
        wallets = [
            {"alias": "one", "seed_hash": "a"},
            {"alias": "two", "seed_hash": "b"},
        ]
        chain.check_wallets(wallets, list(reversed(wallets)))

    def test_rejects_traversal_and_links(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name, kind in (
                ("../escape", tarfile.REGTYPE),
                ("link", tarfile.SYMTYPE),
            ):
                archive = root / "input.tar.gz"
                with tarfile.open(archive, "w:gz") as stream:
                    entry = tarfile.TarInfo(name)
                    entry.type = kind
                    entry.linkname = "/outside"
                    stream.addfile(entry, io.BytesIO())
                with self.assertRaises(ValueError):
                    chain.extract(archive, root / "output")

    def test_cli_uses_isolated_standalone_mode(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with patch.object(
                chain, "run", return_value=json.dumps({"active": "testnet"})
            ) as run:
                chain.cli(Path("det-cli"), root, "network-info")
            args, kwargs = run.call_args
            self.assertIn("--standalone", args[0])
            self.assertEqual(kwargs["env"]["DASH_EVO_DATA_DIR"], str(root))
            self.assertEqual(kwargs["env"]["MCP_API_KEY"], "")


if __name__ == "__main__":
    unittest.main()
