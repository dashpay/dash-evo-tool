"""Regression cases for input, exports, funding and draft lifecycle; offline only."""

import json
import hashlib
import os
import subprocess
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

import yaml
from mnemonic import Mnemonic

os.environ.setdefault("QA_ROOT", "/tmp/release-qa-tests-unused")
import logic
from passive_html import sanitize_html
import run
import ui
import wallet
from test_wallet import ADDRESS

SOURCE = Path(__file__).resolve().parents[3]


def base58check(payload, corrupt=False):
    checksum = hashlib.sha256(hashlib.sha256(payload).digest()).digest()[:4]
    if corrupt:
        checksum = bytes([checksum[0] ^ 1]) + checksum[1:]
    number, encoded = int.from_bytes(payload + checksum, "big"), ""
    while number:
        number, digit = divmod(number, 58)
        encoded = (
            "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"[digit]
            + encoded
        )
    return encoded


class HardeningTests(unittest.TestCase):
    def test_installed_driver_resolves_shared_scanner(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            driver = root / "qa-ui"
            driver.symlink_to(Path(ui.__file__))
            (root / "window").write_text("42")
            result = subprocess.run(
                ["/usr/bin/python3", str(driver), "unsupported"],
                cwd=root,
                env={"QA_ROOT": temp},
                capture_output=True,
                text=True,
                timeout=10,
                check=False,
            )
            self.assertEqual(result.returncode, 1)
            self.assertIn("Unsupported GUI command", result.stderr)
            self.assertNotIn("ModuleNotFoundError", result.stderr)

    def test_keyboard_focuses_det_and_uses_xtest(self):
        with (
            tempfile.TemporaryDirectory() as temp,
            patch.object(ui, "ROOT", Path(temp)),
        ):
            (Path(temp) / "window").write_text("42")
            for command, value in (("key", "Tab"), ("type", "public text")):
                with (
                    patch("sys.argv", ["qa-ui", command, value]),
                    patch.object(ui, "run") as execute,
                ):
                    ui.main()
                args = execute.call_args.args
                self.assertEqual(
                    args[:5], ("xdotool", "windowactivate", "--sync", "42", command)
                )
                self.assertNotIn("--window", args)
                self.assertIn("--clearmodifiers", args)

    def test_text_export_withholds_real_mnemonics_extended_keys_and_wif(self):
        # Generate secrets at runtime; never keep reusable private keys in fixtures.
        alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"

        def chars(n):
            return "".join(alphabet[b % 58] for b in os.urandom(n))

        secrets = [
            Mnemonic("english").generate(strength=n) for n in (128, 160, 192, 224, 256)
        ]
        secrets += [prefix + chars(100) for prefix in ("tprv", "xprv")]
        # Dash mainnet/testnet and Bitcoin mainnet, both compression modes.
        for version in (0xCC, 0xEF, 0x80):
            for compressed in (False, True):
                secrets.append(
                    base58check(
                        bytes([version])
                        + os.urandom(32)
                        + (b"\x01" if compressed else b"")
                    )
                )
        for content in ["Public address: " + ADDRESS, *secrets]:
            with self.subTest(kind=content[:1]), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                out = root / "report"
                out.mkdir()
                (out / "evidence.md").write_text(content)
                with patch.object(run, "ROOT", root), patch.object(run, "OUT", out):
                    run.finish()
                withheld = "export withheld" in (root / "export/report.md").read_text()
                self.assertEqual(withheld, content in secrets)
                self.assertEqual((root / "export/evidence.md").exists(), not withheld)

    def test_wif_requires_checksum_version_length_and_compression_flag(self):
        key = os.urandom(32)
        for version in (0xCC, 0xEF, 0x80):
            for flag in (b"", b"\x01"):
                payload = bytes([version]) + key + flag
                self.assertTrue(logic.has_secret(base58check(payload)))
                self.assertFalse(logic.has_secret(base58check(payload, corrupt=True)))
        for payload in (
            b"\xee" + key,
            b"\xef" + key[:-1],
            b"\xef" + key + b"\x02",
            b"\xef" + key + b"\x01\x01",
        ):
            self.assertFalse(logic.has_secret(base58check(payload)))

    def test_data_uri_secrets_are_scanned_and_withheld_from_text_exports(self):
        wif = base58check(b"\xef" + os.urandom(32) + b"\x01")
        phrase = Mnemonic("english").generate(256).replace(" ", "-")
        for content in (f"data:,{wif}", f"data:,{phrase}"):
            with self.subTest(kind="wif" if content.endswith(wif) else "mnemonic"):
                self.assertTrue(logic.has_secret(content))
                with tempfile.TemporaryDirectory() as temp:
                    root, out = Path(temp), Path(temp) / "report"
                    out.mkdir()
                    (out / "evidence.md").write_text(content)
                    with patch.object(run, "ROOT", root), patch.object(run, "OUT", out):
                        run.finish()
                    self.assertIn(
                        "export withheld", (root / "export/report.md").read_text()
                    )
                    self.assertFalse((root / "export/evidence.md").exists())

    def test_html_keeps_clean_image_disclosure_without_active_attributes(self):
        for src in ("annotated/clean/screen.png", "data:image/png;base64,aGVsbG8="):
            with self.subTest(src=src):
                text = sanitize_html(
                    '<details ontoggle="alert(1)"><summary onclick="alert(2)">'
                    "Version without labels</summary>"
                    f'<img src="{src}" alt="Clean screenshot" onerror="alert(3)">'
                    "</details>"
                )
                self.assertIn(
                    "<details><summary>Version without labels</summary>"
                    f'<img src="{src}" alt="Clean screenshot"></details>',
                    text,
                )
                self.assertNotIn("alert", text)

    def test_html_export_is_passive_and_keeps_local_images(self):
        with tempfile.TemporaryDirectory() as temp:
            root, out = Path(temp), Path(temp) / "report"
            out.mkdir()
            (out / "hostile.html").write_text(
                '<script>alert(1)</script><img src="https://evil.invalid/x" onerror="alert(2)"><a href="javascript:alert(3)">click</a><iframe src="https://evil.invalid"></iframe><img src="annotated/screen.png"><a href="https://evil.invalid">external</a>'
            )
            with patch.object(run, "ROOT", root), patch.object(run, "OUT", out):
                run.finish()
            text = (root / "export/hostile.html").read_text()
            for forbidden in (
                "<script",
                "onerror",
                "javascript:",
                "<iframe",
                "https://evil.invalid",
            ):
                self.assertNotIn(forbidden, text)
            self.assertIn('src="annotated/screen.png"', text)
            self.assertTrue(text.startswith('<!doctype html><html lang="en"><head>'))
            self.assertIn(
                '<meta name="viewport" content="width=device-width,initial-scale=1">',
                text,
            )
            self.assertEqual(text.count("<html"), 1)

    def test_expired_challenges_not_redeemed(self):
        with (
            patch.object(
                wallet,
                "request",
                return_value=(200, {"token": "test", "challenge": {}}),
            ) as request,
            patch.object(wallet, "cap_solve"),
            patch.object(wallet.time, "monotonic", side_effect=[0, 550]),
        ):
            with self.assertRaises(TimeoutError):
                wallet.token("https://faucet.thepasta.org/cap/")
            self.assertEqual(request.call_count, 1)
        with self.assertRaises(TimeoutError):
            logic.cap_solve(
                "test", {"c": 1, "s": 8, "d": 1}, deadline=time.monotonic() - 1
            )

    def test_faucet_retry_contract(self):
        stale = (503, {"detailMessage": "stale coin view, please retry"})
        hard = (429, {"requiresHardCaptcha": True})
        sent = (200, {"txid": "a" * 64})
        for responses, expected in (
            ([stale] * 5, "not_reached"),
            ([stale, sent], "sent"),
            ([hard], "not_reached"),
            ([(429, {"detail": "limit"})], "not_reached"),
            ([(503, {"error": "Faucet has insufficient funds"})], "not_reached"),
            ([(503, {"detailMessage": "broadcast rejected"})], "not_reached"),
            (
                [(503, {"error": "broadcast could not be confirmed; try later"})],
                "unknown",
            ),
            ([(503, {})], "unknown"),
            ([(503, {"error": "Broadcast failed: timeout"})], "unknown"),
            ([(503, {"detailMessage": "provider response lost"})], "unknown"),
            ([(503, {"error": "broadcast rejected after timeout?"})], "unknown"),
            ([(500, {"error": "Faucet has insufficient funds"})], "unknown"),
            ([(429, {})], "not_reached"),
            ([(403, {})], "not_reached"),
            ([(200, {})], "unknown"),
        ):
            with (
                self.subTest(responses=responses),
                tempfile.TemporaryDirectory() as temp,
            ):
                path = Path(temp) / "wallet.json"
                path.write_text(
                    json.dumps(
                        {
                            "wallet_id": "id",
                            "address": ADDRESS,
                            "funding": "not_reached",
                            "refund": "not_needed",
                        }
                    )
                )
                config = {
                    "network": "testnet",
                    "depositAddress": ADDRESS,
                    "coreFaucetAmount": 1,
                    "capEndpoint": "https://faucet.thepasta.org/soft/",
                    "hardCapEndpoint": "https://faucet.thepasta.org/hard/",
                }
                with (
                    patch.object(wallet, "STATE", path),
                    patch.object(
                        wallet, "token", side_effect=[str(n) for n in range(5)]
                    ) as token,
                    patch.object(
                        wallet, "request", side_effect=[(200, config), *responses]
                    ) as request,
                    patch.object(wallet, "det") as det,
                ):
                    wallet.fund()
                det.assert_not_called()  # GUI is running: funding must use HTTP only.
                state = json.loads(path.read_text())
                self.assertEqual(state["funding"], expected)
                self.assertEqual(
                    state["refund"],
                    "not_needed" if expected == "not_reached" else "pending",
                )
                self.assertEqual(request.call_count, 1 + len(responses))
                payloads = [call.args[1] for call in request.call_args_list[1:]]
                self.assertEqual(
                    len({p.get("capToken", p.get("hardCapToken")) for p in payloads}),
                    len(responses),
                )
                self.assertTrue(
                    all(
                        call.args[0] == config["capEndpoint"]
                        for call in token.call_args_list
                    )
                )
                self.assertTrue(
                    all("hardCapToken" not in payload for payload in payloads)
                )

    def test_workflows_seal_before_qa_fund_after_smoke_and_run_tests(self):
        def workflow(name):
            return yaml.load(
                (SOURCE / ".github/workflows" / name).read_text(),
                Loader=yaml.BaseLoader,
            )

        weekly = workflow("weekly-build.yml")["jobs"]
        self.assertIn("seal", weekly["qa"]["needs"])
        self.assertNotIn("qa", weekly["seal"]["needs"])
        seal = "\n".join(s.get("run", "") for s in weekly["seal"]["steps"])
        self.assertIn("<!-- weekly-version:", seal)
        self.assertIn("DO NOT PUBLISH", weekly["seal"]["steps"][0]["run"])
        push = next(s["run"] for s in weekly["tag"]["steps"] if s.get("id") == "push")
        self.assertLess(push.index("DO NOT PUBLISH"), push.index("git tag -f"))
        steps = workflow("release-qa.yml")["jobs"]["review"]["steps"]
        ids = [s.get("id") for s in steps]
        self.assertLess(ids.index("smoke"), ids.index("fund"))
        self.assertEqual(
            steps[ids.index("fund")]["if"],
            "steps.resume.outcome == 'success' && steps.wallet.outcome == 'success'",
        )
        tests = workflow("tests.yml")
        self.assertIn(".github/workflows/release-qa.yml", tests["on"]["push"]["paths"])
        qa_tests = workflow("release-qa-tests.yml")
        scripts = "\n".join(
            s.get("run", "") for s in qa_tests["jobs"]["release-qa-tests"]["steps"]
        )
        self.assertIn("unittest discover -s scripts/ci/release-qa", scripts)
        self.assertIn("python3-mnemonic", scripts)
        self.assertIn(".claude/skills/visual-diff/scripts/selftest.py", scripts)
        self.assertIn(".claude/skills/visual-diff/**", qa_tests["on"]["push"]["paths"])
        pin = tests["jobs"]["pin-ancestry"]
        self.assertNotIn("apt-get", "\n".join(s.get("run", "") for s in pin["steps"]))
        self.assertNotIn("needs", pin)
        self.assertNotIn("schedule", qa_tests["on"])

    def test_release_notes_match_event(self):
        jobs = yaml.load(
            (SOURCE / ".github/workflows/weekly-build.yml").read_text(),
            Loader=yaml.BaseLoader,
        )["jobs"]
        script = next(
            s["run"]
            for s in jobs["finalize"]["steps"]
            if s["name"] == "Record QA outcome on retained draft"
        )
        for event in ("schedule", "workflow_dispatch"):
            with self.subTest(event=event), tempfile.TemporaryDirectory() as temp:
                subprocess.run(
                    ["bash", "-c", "gh() { :; }; " + script],
                    cwd=temp,
                    check=True,
                    env={
                        **os.environ,
                        "QA_RESULT": "success",
                        "QA_VERDICT": "PASS",
                        "TAG": "weekly",
                        "VERSION": "1.0",
                        "EVENT_NAME": event,
                        "GITHUB_REPOSITORY": "owner/repo",
                    },
                    capture_output=True,
                )
                notes = (Path(temp) / "qa-notes.md").read_text()
                self.assertIn("<!-- weekly-version: 1.0 -->", notes)
                self.assertIn("GUI QA: PASS.", notes)
                self.assertEqual(
                    "Manual runs remain drafts" in notes, event == "workflow_dispatch"
                )
