"""Refund contract tests use mocked HTTP/CLI; never request real funds."""

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

    def test_refund_observes_only_insight_and_never_retries_send(self):
        for case in (
            "ok",
            "timeout",
            "bad_send_txid",
            "unconfirmed",
            "wrong_tx",
            "multiple",
            "missing_address",
            "bad_destination",
            "retry_http",
            "retry_balance",
            "retry_exhausted",
            "mismatch",
            "wrong_address",
            "bool_balance",
            "bool_coin",
            "missing_balance",
            "negative_unconfirmed",
            "deadline",
            "slow_utxo",
        ):
            with self.subTest(case=case), tempfile.TemporaryDirectory() as temp:
                path = Path(temp) / "wallet.json"
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
                balance = {
                    "addrStr": ADDRESS,
                    "balanceSat": 100_000_000,
                    "unconfirmedBalanceSat": 0,
                }
                coins = [coin]
                if case == "missing_address":
                    del state["refund_address"]
                if case == "bad_destination":
                    state["refund_address"] = "invalid"
                if case == "unconfirmed":
                    coin["confirmations"] = 0
                if case == "wrong_tx":
                    coin["txid"] = "b" * 64
                if case == "multiple":
                    coins *= 2
                if case == "mismatch":
                    balance["balanceSat"] -= 1
                if case == "wrong_address":
                    balance["addrStr"] = REFUND_ADDRESS
                if case == "bool_balance":
                    balance["unconfirmedBalanceSat"] = False
                if case == "bool_coin":
                    coin["confirmations"] = True
                if case == "missing_balance":
                    del balance["balanceSat"]
                if case == "negative_unconfirmed":
                    balance["unconfirmedBalanceSat"] = -1
                path.write_text(json.dumps(state))
                now = [100.0]
                calls = []

                def http(kind, value, *, utxo=False, timeout):
                    self.assertEqual((kind, value), ("addr", ADDRESS))
                    self.assertGreater(timeout, 0)
                    self.assertLessEqual(timeout, 5)
                    calls.append(utxo)
                    now[0] += timeout if case == "slow_utxo" else 2
                    if case == "slow_utxo" and utxo:
                        raise OSError("slow response")
                    if case == "retry_exhausted" or (
                        case == "retry_http" and len(calls) == 1
                    ):
                        raise OSError("unavailable")
                    if case == "retry_balance" and len(calls) == 1:
                        return dict(balance, unconfirmedBalanceSat=1)
                    return coins if utxo else balance

                def send(*args, **kwargs):
                    self.assertEqual(args[0], "core-funds-send")
                    self.assertEqual(json.loads(path.read_text())["refund"], "unknown")
                    self.assertEqual(kwargs["timeout"], 1600 - 15 - now[0])
                    self.assertGreater(kwargs["timeout"], 650)
                    if case == "timeout":
                        raise TimeoutError()
                    return {"txid": "bad" if case == "bad_send_txid" else "c" * 64}

                original_save = wallet.save

                def save(current):
                    original_save(current)
                    if case == "deadline" and current["refund"] == "unknown":
                        now[0] = 1600

                with (
                    patch.object(wallet, "STATE", path),
                    patch.object(wallet, "save", side_effect=save),
                    patch.object(wallet.time, "monotonic", side_effect=lambda: now[0]),
                    patch.object(
                        wallet.time,
                        "sleep",
                        side_effect=lambda seconds: now.__setitem__(
                            0, now[0] + seconds
                        ),
                    ) as sleep,
                    patch.object(wallet, "insight", create=True) as insight,
                    patch.object(wallet, "det", side_effect=send) as det,
                    patch.object(
                        wallet,
                        "request",
                        side_effect=AssertionError("No faucet HTTP in refund"),
                    ),
                ):
                    insight.query.side_effect = http
                    if case in ("ok", "missing_address", "retry_http", "retry_balance"):
                        wallet.refund()
                    else:
                        with self.assertRaises(
                            (ValueError, TimeoutError, OSError, KeyError)
                        ):
                            wallet.refund()
                    saved = json.loads(path.read_text())
                    sends = case in (
                        "ok",
                        "timeout",
                        "bad_send_txid",
                        "retry_http",
                        "retry_balance",
                    )
                    self.assertEqual(det.call_count, int(sends))
                    if sends:
                        self.assertIn("amount_duffs=99999774", det.call_args.args)
                        self.assertIn("network=testnet", det.call_args.args)
                        self.assertIn("address=" + REFUND_ADDRESS, det.call_args.args)
                        self.assertEqual(
                            saved["refund"],
                            "unknown"
                            if case in ("timeout", "bad_send_txid")
                            else "sent",
                        )
                        wallet.refund()
                        self.assertEqual(det.call_count, 1)
                    elif case == "missing_address":
                        self.assertEqual(saved["refund"], "skipped_no_address")
                    elif case != "bad_destination":
                        self.assertEqual(saved["refund"], "unknown")
                    if case in ("missing_address", "bad_destination"):
                        insight.query.assert_not_called()
                    if case == "slow_utxo":
                        self.assertEqual(now[0], 120)
                        self.assertEqual(len(calls), 4)
                    self.assertLessEqual(len(calls), 6)
                    self.assertLessEqual(
                        sum(c.args[0] for c in sleep.call_args_list), 2
                    )

    def test_det_honors_send_timeout(self):
        with patch.object(wallet.subprocess, "run") as execute:
            execute.return_value.returncode = 0
            execute.return_value.stdout = "{}"
            wallet.det("core-funds-send", timeout=1400)
        self.assertEqual(execute.call_args.kwargs["timeout"], 1400)
