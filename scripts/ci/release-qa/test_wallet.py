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
REFUND_ADDRESS = "yXH5g3P9cV3mLyZChHJv1F9qUtbu3MrzEu"


class WalletTests(unittest.TestCase):
    def test_refund_address_override_default_and_unavailable(self):
        for config, override, expected in (
            ({"depositAddress": ADDRESS}, "", ADDRESS),
            ({"depositAddress": ADDRESS}, "invalid", ADDRESS),
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
            "refund_address": REFUND_ADDRESS,
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
            "retry_http",
            "retry_balance",
            "retry_exhausted",
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
                attempts = (
                    3
                    if case
                    in ("retry_exhausted", "unconfirmed", "wrong_tx", "multiple")
                    else 2
                    if case.startswith("retry_")
                    else 1
                )
                responses = [(200, coins)] * attempts
                if case in ("retry_http", "retry_exhausted"):
                    responses = [(503, {})] * (attempts - (case == "retry_http")) + (
                        [(200, coins)] if case == "retry_http" else []
                    )
                balances = [balance] * attempts
                if case == "retry_balance":
                    balances[0] = {
                        "confirmed_duffs": 0,
                        "unconfirmed_duffs": 100_000_000,
                    }
                with (
                    patch.object(wallet.time, "sleep") as sleep,
                    patch.object(wallet, "STATE", path),
                    patch.object(wallet, "request", side_effect=responses) as request,
                    patch.object(
                        wallet,
                        "det",
                        side_effect=[
                            {"active": "testnet"},
                            *balances,
                            TimeoutError() if case == "timeout" else {"txid": "c" * 64},
                        ],
                    ) as det,
                ):
                    if case in ("ok", "missing_address", "retry_http", "retry_balance"):
                        wallet.refund()
                    else:
                        with self.assertRaises((ValueError, TimeoutError)):
                            wallet.refund()
                    saved = json.loads(path.read_text())
                    sends = [
                        c for c in det.call_args_list if c.args[0] == "core-funds-send"
                    ]
                    if case in ("ok", "timeout", "retry_http", "retry_balance"):
                        self.assertEqual(len(sends), 1)
                        self.assertIn("amount_duffs=99999774", sends[0].args)
                        self.assertIn("network=testnet", sends[0].args)
                        self.assertIn("address=" + REFUND_ADDRESS, sends[0].args)
                        self.assertEqual(
                            saved["refund"], "unknown" if case == "timeout" else "sent"
                        )
                        wallet.refund()
                        self.assertEqual(det.call_count, attempts + 2)
                        self.assertEqual(request.call_count, attempts)
                        self.assertEqual(sleep.call_count, attempts - 1)
                    else:
                        self.assertFalse(sends)
                        if case != "missing_address":
                            self.assertEqual(request.call_count, 3)
                            self.assertEqual(sleep.call_count, 2)
                            self.assertEqual(saved["refund"], "unknown")
                    if case == "missing_address":
                        self.assertEqual(saved["refund"], "skipped_no_address")
                        self.assertIn("warning", saved)
                        det.assert_not_called()
