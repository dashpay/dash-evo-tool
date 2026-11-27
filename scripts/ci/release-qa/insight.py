"""Bounded, read-only testnet Insight queries. Remote JSON is untrusted data."""

import json
import re
import subprocess

from logic import b58check_decode

BASE = "https://insight.testnet.networks.dash.org/insight-api/"
MAX_BYTES = 1_000_000


def address(value):
    if not isinstance(value, str) or not re.fullmatch(
        r"y[1-9A-HJ-NP-Za-km-z]{33}", value
    ):
        raise ValueError("Expected a testnet P2PKH address")
    payload = b58check_decode(value)
    if len(payload) != 21 or payload[0] != 140:
        raise ValueError("Expected a testnet P2PKH address")
    return value


def query_path(kind, value, *, utxo=False):
    if kind == "addr":
        return "addr/" + address(value) + ("/utxo" if utxo else "")
    if kind == "tx" and not utxo and re.fullmatch(r"[0-9a-fA-F]{64}", value):
        return "tx/" + value
    raise ValueError("Expected an Insight address or transaction query")


def query(kind, value, *, utxo=False, timeout=5):
    path = query_path(kind, value, utxo=utxo)
    if not 0 < timeout <= 5:
        raise ValueError("Insight request budget expired or exceeds five seconds")
    # curl >= 8.4 (Ubuntu 26.04) enforces size even without Content-Length.
    # No config, proxies, redirects or retries; max-time bounds slow streams too.
    result = subprocess.run(
        [
            "/usr/bin/curl",
            "--disable",
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--noproxy",
            "*",
            "--request",
            "GET",
            "--max-time",
            str(timeout),
            "--max-filesize",
            str(MAX_BYTES),
            "--write-out",
            "\n%{http_code}",
            BASE + path,
        ],
        env={"PATH": "/usr/bin:/bin", "LC_ALL": "C"},
        capture_output=True,
        timeout=timeout,
    )
    if result.returncode or len(result.stdout) > MAX_BYTES + 4:
        raise RuntimeError("Insight request failed or exceeded its response limit")
    body, status = result.stdout.rsplit(b"\n", 1)
    if status != b"200":
        raise ValueError("Insight did not return HTTP 200")
    return json.loads(body)
