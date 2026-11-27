"""Pure release-QA contracts; all amounts are integer duffs."""

import hashlib
import re
import time
from urllib.parse import urlsplit


CAPTURE_NAME = r"[a-zA-Z0-9][a-zA-Z0-9_-]{0,80}"
CI_LIMITS = {
    "ci_no_identity",
    "ci_no_broadcast",
    "ci_no_secrets",
    "ci_new_only",
    "ci_fixed_viewport",
}
GAP_CATEGORIES = CI_LIMITS | {"funding", "network", "app_error", "navigation"}


def isolated_env(root, *, display=False):
    result = {
        "PATH": "/usr/bin:/bin",
        "HOME": str(root / "home"),
        "LANG": "C.UTF-8",
        "DASH_EVO_DATA_DIR": str(root / "data"),
        "BROWSER": "/bin/true",
        "RUST_LOG": "off",
    }
    if display:
        result.update(
            DISPLAY=":99",
            DASH_EVO_TOOL_ACCESSIBILITY="1",
            RUST_LOG="info,wgpu_hal=warn",
        )
    return result


def b58check_decode(value):
    alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
    number = 0
    for char in value:
        number = number * 58 + alphabet.index(char)
    decoded = b"\0" * (len(value) - len(value.lstrip("1"))) + number.to_bytes(
        (number.bit_length() + 7) // 8, "big"
    )
    payload, checksum = decoded[:-4], decoded[-4:]
    if (
        len(decoded) < 5
        or hashlib.sha256(hashlib.sha256(payload).digest()).digest()[:4] != checksum
    ):
        raise ValueError("Invalid Base58Check checksum")
    return payload


def verdict(report, *, smoke, agent_ok):
    if smoke is False:
        return "FAIL"
    if smoke is not True or agent_ok is not True or not isinstance(report, dict):
        return "INCOMPLETE"
    summary = report.get("summary")
    if not isinstance(summary, dict):
        return "INCOMPLETE"
    value = summary.get("verdict")
    if summary.get("complete") is not True or value not in ("PASS", "NOTE", "FAIL"):
        return "INCOMPLETE"
    if not report.get("scenarios") or any(
        not isinstance(report.get(k), list)
        for k in ("scenarios", "findings", "not_reached")
    ):
        return "INCOMPLETE"
    for scenario in report["scenarios"]:
        if (
            not isinstance(scenario, dict)
            or not isinstance(scenario.get("id"), str)
            or not scenario["id"].strip()
            or scenario.get("status") not in ("PASS", "NOTE", "FAIL", "NOT_REACHED")
        ):
            return "INCOMPLETE"
    for finding in report["findings"]:
        if (
            not isinstance(finding, dict)
            or type(finding.get("blocking")) is not bool
            or any(
                not isinstance(finding.get(k), str) or not finding[k].strip()
                for k in ("id", "title")
            )
        ):
            return "INCOMPLETE"
    gaps = {}
    for item in report["not_reached"]:
        if (
            not isinstance(item, dict)
            or item.get("category") not in GAP_CATEGORIES
            or any(
                not isinstance(item.get(k), str) or not item[k].strip()
                for k in ("id", "reason")
            )
            or item["id"] in gaps
        ):
            return "INCOMPLETE"
        gaps[item["id"]] = item["category"]
    if any(
        s["status"] == "NOT_REACHED" and s["id"] not in gaps
        for s in report["scenarios"]
    ):
        return "INCOMPLETE"
    if (
        any(f["blocking"] for f in report["findings"])
        or value == "FAIL"
        or any(s["status"] == "FAIL" for s in report["scenarios"])
    ):
        return "FAIL"
    if (
        report["findings"]
        or not any(s["status"] == "PASS" for s in report["scenarios"])
        or any(category not in CI_LIMITS for category in gaps.values())
        or any(s["status"] == "NOTE" for s in report["scenarios"])
    ):
        return "NOTE"
    return value


def cap_endpoint(url):
    parsed = urlsplit(url)
    host = parsed.hostname or ""
    if (
        parsed.scheme != "https"
        or parsed.username
        or parsed.password
        or parsed.port not in (None, 443)
        or parsed.query
        or parsed.fragment
        or "hard" in parsed.path.lower()
        or not (host == "thepasta.org" or host.endswith(".thepasta.org"))
    ):
        raise ValueError("CAP endpoint is outside the faucet's HTTPS domain")
    return url.rstrip("/") + "/"


def cap_prng(seed, length):
    x = 2166136261
    for byte in seed.encode():
        x = ((x ^ byte) * 16777619) & 0xFFFFFFFF
    result = ""
    while len(result) < length:
        x ^= (x << 13) & 0xFFFFFFFF
        x ^= x >> 17
        x ^= (x << 5) & 0xFFFFFFFF
        result += f"{x:08x}"
    return result[:length]


def cap_nonce(args):
    salt, target, deadline = args
    nonce = 0
    while True:
        if nonce % 4096 == 0 and time.monotonic() >= deadline:
            raise TimeoutError("CAP challenge expired before solving")
        if hashlib.sha256(f"{salt}{nonce}".encode()).hexdigest().startswith(target):
            return nonce
        nonce += 1


def cap_solve(token, challenge, *, deadline=None):
    count, size, difficulty = (challenge[k] for k in ("c", "s", "d"))
    if not (1 <= count <= 100 and 1 <= size <= 128 and 1 <= difficulty <= 6):
        raise ValueError("Unsupported CAP challenge cost")
    deadline = time.monotonic() + 540 if deadline is None else deadline
    tasks = [
        (cap_prng(f"{token}{i}", size), cap_prng(f"{token}{i}d", difficulty), deadline)
        for i in range(1, count + 1)
    ]
    return [cap_nonce(task) for task in tasks]


def payout_action(status, body):
    if not isinstance(body, dict):
        return "stop"
    if status == 200 and re.fullmatch(r"[0-9a-f]{64}", body.get("txid", "")):
        return "replay" if body.get("replay") is True else "sent"
    if status == 429 and (
        body.get("requiresHardCaptcha") is True
        or (
            isinstance(body.get("detail"), dict)
            and body["detail"].get("requiresHardCaptcha") is True
        )
    ):
        return "hard"
    if status == 503 and body.get("detailMessage") == "stale coin view, please retry":
        return "stale"
    message = body.get("detailMessage") or body.get("error")
    if (
        status == 503
        and isinstance(message, str)
        and message.lower() in {"faucet has insufficient funds", "broadcast rejected"}
    ):
        return "refused"
    return "stop"


def refund_amount(balance, payout):
    # One P2PKH faucet input, 1 duff/byte, 226 duffs including change allowance.
    # Its sub-dust remainder becomes miner fee; no change output is created.
    if (
        type(balance) is not int
        or type(payout) is not int
        or not 772 < balance <= payout
    ):
        raise ValueError("Refund requires one funded, spendable faucet output")
    return balance - 226


def is_wif(candidate):
    try:
        payload = b58check_decode(candidate)
    except ValueError:
        return False
    return (
        len(payload) in (33, 34)
        and payload[0] in (0xCC, 0xEF, 0x80)
        and (len(payload) == 33 or payload[-1] == 1)
    )


def has_secret(text):
    from mnemonic import Mnemonic

    if re.search(r"(?:xprv|tprv)[1-9A-HJ-NP-Za-km-z]{90,}|sk-ant-[A-Za-z0-9_-]+", text):
        return True
    if any(
        is_wif(match[0])
        for match in re.finditer(
            r"\b(?:[cKLX][1-9A-HJ-NP-Za-km-z]{51}|[579][1-9A-HJ-NP-Za-km-z]{50})\b",
            text,
        )
    ):
        return True
    mnemonic = Mnemonic("english")
    words, sequence = set(mnemonic.wordlist), []
    for word in re.findall(r"[a-z]+", text.lower()):
        sequence = (sequence + [word])[-24:] if word in words else []
        if any(
            len(sequence) >= n and mnemonic.check(" ".join(sequence[-n:]))
            for n in (12, 15, 18, 21, 24)
        ):
            return True
    return False
