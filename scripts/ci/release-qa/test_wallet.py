"""Refund contract tests use mocked CLI and HTTP; never request real funds."""

import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

os.environ.setdefault("QA_ROOT", "/tmp/release-qa-tests-unused")
import wallet

ADDRESS = "yPrynRN6ViX2rCcRL5SoSi8ntcWgdHSEaS"


class WalletTests(unittest.TestCase):
    def test_refund_address_override_default_and_unavailable(self):
        for config, override, expected in (
            ({"depositAddress": ADDRESS}, "", ADDRESS),
            ({"depositAddress": "invalid"}, ADDRESS, ADDRESS),
            ({}, "", None),
            ({"depositAddress": "invalid"}, "", None),
        ):
            with self.subTest(config=config, override=override):
                with patch.dict(os.environ, QA_FAUCET_REFUND_ADDRESS=override):
                    self.assertEqual(wallet.refund_address(config), expected)

    def test_post_checks_and_never_retries_ambiguous_send(self):
        state = {
            "refund": "pending",
            "funding": "sent",
            "wallet_id": "id",
            "address": ADDRESS,
            "refund_address": ADDRESS,
            "txid": "a" * 64,
            "payout_duffs": 100_000_000,
        }
        coin = {
            "address": ADDRESS,
            "satoshis": 100_000_000,
            "confirmations": 1,
            "txid": "a" * 64,
        }
        balance = {"confirmed_duffs": 100_000_000, "unconfirmed_duffs": 0}
        for case in (
            "ok",
            "timeout",
            "unconfirmed",
            "wrong_tx",
            "multiple",
            "missing_address",
        ):
            with self.subTest(case=case), tempfile.TemporaryDirectory() as temp:
                path = Path(temp) / "wallet.json"
                current, coins = dict(state), [dict(coin)]
                if case == "missing_address":
                    del current["refund_address"]
                if case == "unconfirmed":
                    coins[0]["confirmations"] = 0
                if case == "wrong_tx":
                    coins[0]["txid"] = "b" * 64
                if case == "multiple":
                    coins *= 2
                path.write_text(json.dumps(current))
                with (
                    patch.object(wallet, "STATE", path),
                    patch.object(wallet, "request", return_value=(200, coins)),
                    patch.object(
                        wallet,
                        "det",
                        side_effect=[
                            {"active": "testnet"},
                            balance,
                            TimeoutError() if case == "timeout" else {"txid": "c" * 64},
                        ],
                    ) as det,
                ):
                    if case in ("ok", "missing_address"):
                        wallet.refund()
                    else:
                        with self.assertRaises((ValueError, TimeoutError)):
                            wallet.refund()
                    saved = json.loads(path.read_text())
                    sends = [
                        c for c in det.call_args_list if c.args[0] == "core-funds-send"
                    ]
                    if case in ("ok", "timeout"):
                        self.assertEqual(len(sends), 1)
                        self.assertIn("amount_duffs=99999774", sends[0].args)
                        self.assertIn("network=testnet", sends[0].args)
                        self.assertEqual(
                            saved["refund"], "sent" if case == "ok" else "unknown"
                        )
                        wallet.refund()
                        self.assertEqual(det.call_count, 3)
                    else:
                        self.assertFalse(sends)
                    if case == "missing_address":
                        self.assertEqual(saved["refund"], "skipped_no_address")
                        self.assertIn("warning", saved)
                        det.assert_not_called()
