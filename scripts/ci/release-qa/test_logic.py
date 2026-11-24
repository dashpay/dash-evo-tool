"""Offline contracts: python3 -m unittest discover -s scripts/ci/release-qa."""

import copy
import hashlib
import unittest

from logic import (
    cap_prng,
    cap_solve,
    cap_endpoint,
    payout_action,
    refund_amount,
    verdict,
)


class LogicTests(unittest.TestCase):
    def test_publish_requires_complete_valid_evidence(self):
        for value in (None, {}, {"summary": {"verdict": "PASS"}}):
            self.assertEqual(verdict(value, smoke=True, agent_ok=True), "INCOMPLETE")
        report = {
            "summary": {"verdict": "PASS", "complete": True},
            "scenarios": [{"id": "GUI-1", "status": "PASS"}],
            "findings": [],
            "not_reached": [],
        }
        self.assertEqual(verdict(report, smoke=True, agent_ok=True), "PASS")
        self.assertEqual(verdict(report, smoke=False, agent_ok=True), "FAIL")
        self.assertEqual(verdict(report, smoke=True, agent_ok=False), "INCOMPLETE")
        report["summary"]["verdict"] = "approved"
        self.assertEqual(verdict(report, smoke=True, agent_ok=True), "INCOMPLETE")

    def test_blocker_and_missing_fixture_cannot_be_pass(self):
        report = {
            "summary": {"verdict": "PASS", "complete": True},
            "scenarios": [{"id": "GUI-1", "status": "PASS"}],
            "findings": [{"id": "F1", "title": "Broken flow", "blocking": True}],
            "not_reached": [],
        }
        self.assertEqual(verdict(report, smoke=True, agent_ok=True), "FAIL")
        report["findings"] = []
        report["not_reached"] = [
            {"id": "F1", "category": "funding", "reason": "faucet unavailable"}
        ]
        self.assertEqual(verdict(report, smoke=True, agent_ok=True), "NOTE")

    def test_invalid_shapes_fail_closed(self):
        valid = {
            "summary": {"verdict": "PASS", "complete": True},
            "scenarios": [{"id": "GUI-1", "status": "PASS"}],
            "findings": [],
            "not_reached": [],
        }
        for key, bad in (
            ("summary", None),
            ("summary", []),
            ("scenarios", [None]),
            ("scenarios", [{}]),
            ("findings", [None]),
            ("findings", [{"blocking": "false"}]),
            ("not_reached", [None]),
        ):
            with self.subTest(key=key, value=bad):
                report = copy.deepcopy(valid)
                report[key] = bad
                self.assertEqual(
                    verdict(report, smoke=True, agent_ok=True), "INCOMPLETE"
                )
        for field in ("smoke", "agent_ok"):
            args = {"smoke": True, "agent_ok": True, field: "true"}
            self.assertEqual(verdict(valid, **args), "INCOMPLETE")
        valid["findings"] = [{"id": "F1", "title": "Layout note", "blocking": False}]
        self.assertEqual(verdict(valid, smoke=True, agent_ok=True), "NOTE")
        valid["findings"] = []
        valid["scenarios"][0]["status"] = "FAIL"
        self.assertEqual(verdict(valid, smoke=True, agent_ok=True), "FAIL")

    def test_cap_vectors(self):
        self.assertEqual(cap_prng("hello", 16), "eb492c6e1655ea8c")
        solutions = cap_solve("test-token", {"c": 2, "s": 8, "d": 1})
        for i, nonce in enumerate(solutions, 1):
            salt = cap_prng(f"test-token{i}", 8)
            target = cap_prng(f"test-token{i}d", 1)
            self.assertTrue(
                hashlib.sha256(f"{salt}{nonce}".encode()).hexdigest().startswith(target)
            )

    def test_cap_domain_and_limits(self):
        self.assertEqual(
            cap_endpoint("https://faucet.thepasta.org/cap/v1/"),
            "https://faucet.thepasta.org/cap/v1/",
        )
        for url in (
            "http://faucet.thepasta.org/cap/",
            "https://thepasta.org.evil/cap/",
            "https://user@thepasta.org/cap/",
            "file:///tmp/cap",
        ):
            with self.assertRaises(ValueError):
                cap_endpoint(url)
        with self.assertRaises(ValueError):
            cap_solve("x", {"c": 999999, "s": 1, "d": 100})

    def test_payout_ambiguity_never_retries(self):
        for status in (500, 502, 503, 504, None):
            self.assertEqual(payout_action(status, {}), "stop")
        self.assertEqual(payout_action(200, {"txid": "a" * 64}), "sent")
        self.assertEqual(
            payout_action(200, {"txid": "a" * 64, "replay": True}), "replay"
        )
        self.assertEqual(payout_action(429, {"requiresHardCaptcha": True}), "hard")
        self.assertEqual(payout_action(429, {}), "stop")

    def test_refund_integer_contract(self):
        self.assertEqual(refund_amount(100_000_000, 100_000_000), 99_999_774)
        for balance in (0, -1, 226, 500, 100_000_001, True, 1.25):
            with self.assertRaises(ValueError):
                refund_amount(balance, 100_000_000)


if __name__ == "__main__":
    unittest.main()
