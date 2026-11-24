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
            "bad_vout",
            "bool_vout",
            "missing_vout",
            "excess_amount",
            "short_amount",
            "missing_txid",
            "invalid_txid",
            "overflow_vout",
            "negative_confirmations",
            "bool_amount",
            "malformed_list",
            "malformed_coin",
            "multiple",
            "missing_address",
            "bad_destination",
            "retry_http",
            "retry_parse",
            "retry_exhausted",
            "wrong_address",
            "bool_coin",
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
                    "vout": 0,
                    "txid": "a" * 64,
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
                if case == "bad_vout":
                    coin["vout"] = -1
                if case == "bool_vout":
                    coin["vout"] = True
                if case == "missing_vout":
                    del coin["vout"]
                if case == "excess_amount":
                    coin["satoshis"] += 1
                if case == "multiple":
                    coins.append(dict(coin, txid="b" * 64))
                if case == "wrong_address":
                    coin["address"] = REFUND_ADDRESS
                if case == "bool_coin":
                    coin["confirmations"] = True
                if case == "short_amount":
                    coin["satoshis"] -= 1
                if case == "missing_txid":
                    del state["txid"]
                if case == "invalid_txid":
                    coin["txid"] = "invalid"
                if case == "overflow_vout":
                    coin["vout"] = 2**32
                if case == "negative_confirmations":
                    coin["confirmations"] = -1
                if case == "bool_amount":
                    coin["satoshis"] = True
                if case == "malformed_list":
                    coins = {}
                if case == "malformed_coin":
                    coins = [None]
                path.write_text(json.dumps(state))
                now = [100.0]
                calls = []

                def http(kind, value, *, utxo=False, timeout):
                    self.assertEqual((kind, value, utxo), ("addr", ADDRESS, True))
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
                    if case == "retry_parse" and len(calls) == 1:
                        raise json.JSONDecodeError(
                            "Temporary malformed response", "", 0
                        )
                    return coins

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
                    if case in ("ok", "missing_address", "retry_http", "retry_parse"):
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
                        "retry_parse",
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
                    if case in ("slow_utxo", "retry_exhausted", "unconfirmed"):
                        self.assertEqual(now[0], 820)
                        self.assertLessEqual(len(calls), 48)
                    elif case in ("retry_http", "retry_parse"):
                        self.assertEqual(len(calls), 2)
                        sleep.assert_called_once_with(30)
                    else:
                        self.assertLessEqual(len(calls), 1)
                        sleep.assert_not_called()

    def check_confirmation_wait(
        self, confirmed_at, *, transient_error=False, initially_empty=False
    ):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "wallet.json"
            path.write_text(
                json.dumps(
                    {
                        "refund": "pending",
                        "funding": "sent",
                        "wallet_id": "id",
                        "address": ADDRESS,
                        "refund_address": REFUND_ADDRESS,
                        "txid": "a" * 64,
                        "payout_duffs": 100_000_000,
                    }
                )
            )
            now = [0.0]
            sleeps = []

            def sleep(seconds):
                self.assertGreater(seconds, 0)
                self.assertLessEqual(seconds, 30)
                sleeps.append(seconds)
                now[0] += seconds

            def query(kind, value, *, utxo=False, timeout):
                self.assertEqual((kind, value, utxo), ("addr", ADDRESS, True))
                self.assertGreater(timeout, 0)
                self.assertLessEqual(timeout, min(5, 720 - now[0]))
                if transient_error and 30 <= now[0] < 90:
                    raise OSError("Temporary Insight outage")
                if initially_empty and now[0] == 0:
                    return []
                confirmed = confirmed_at is not None and now[0] >= confirmed_at
                return [
                    {
                        "address": ADDRESS,
                        "satoshis": 100_000_000,
                        "confirmations": int(confirmed),
                        "vout": 0,
                        "txid": "a" * 64,
                    }
                ]

            def send(*args, timeout):
                self.assertEqual(json.loads(path.read_text())["refund"], "unknown")
                self.assertEqual(now[0], confirmed_at)
                if initially_empty:
                    self.assertEqual(insight_query.call_count, 3)
                self.assertEqual(args[0], "core-funds-send")
                self.assertIn("amount_duffs=99999774", args)
                self.assertEqual(timeout, 1485 - now[0])
                self.assertGreaterEqual(timeout, 765)
                now[0] += 600  # A cold standalone wallet can need ten minutes to sync.
                return {"txid": "c" * 64}

            with (
                patch.object(wallet, "STATE", path),
                patch.object(wallet.time, "monotonic", side_effect=lambda: now[0]),
                patch.object(wallet.time, "sleep", side_effect=sleep),
                patch.object(
                    wallet.insight, "query", side_effect=query
                ) as insight_query,
                patch.object(wallet, "det", side_effect=send) as det,
                patch.object(
                    wallet, "request", side_effect=AssertionError("No faucet")
                ),
            ):
                if confirmed_at is None:
                    with self.assertRaises(TimeoutError):
                        wallet.refund()
                    self.assertEqual(now[0], 720)
                    self.assertEqual(json.loads(path.read_text())["refund"], "unknown")
                    det.assert_not_called()
                else:
                    wallet.refund()
                    self.assertEqual(json.loads(path.read_text())["refund"], "sent")
                    det.assert_called_once()
                self.assertLess(now[0], 1500)
                self.assertGreater(sum(sleeps), 20)
                wallet.refund()
                self.assertEqual(det.call_count, int(confirmed_at is not None))

    def test_refund_waits_for_late_payout_confirmation(self):
        for confirmed_at in (60, 690):
            with self.subTest(confirmed_at=confirmed_at):
                self.check_confirmation_wait(confirmed_at)

    def test_refund_empty_then_unconfirmed_then_confirmed_sends_once(self):
        self.check_confirmation_wait(60, initially_empty=True)

    def test_refund_confirmation_never_arrives(self):
        self.check_confirmation_wait(None)

    def test_refund_retries_transient_insight_errors_while_waiting(self):
        self.check_confirmation_wait(120, transient_error=True)

    def test_det_honors_send_timeout(self):
        with patch.object(wallet.subprocess, "run") as execute:
            execute.return_value.returncode = 0
            execute.return_value.stdout = "{}"
            wallet.det("core-funds-send", timeout=1400)
        self.assertEqual(execute.call_args.kwargs["timeout"], 1400)
