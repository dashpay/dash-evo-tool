"""Deterministic burner provisioning/refund. Never log CLI output or secret input."""

import json
import os
import re
import subprocess
import sys
import time
from decimal import Decimal
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import HTTPRedirectHandler, Request, build_opener

import insight
from atomic import atomic_write
from logic import (
    isolated_env,
    cap_endpoint,
    cap_solve,
    payout_action,
    refund_amount,
)

FAUCET = "https://faucet.thepasta.org"
ROOT = Path(os.environ["QA_ROOT"])
STATE = ROOT / "wallet.json"


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def request(url, body=None):
    data = None if body is None else json.dumps(body).encode()
    req = Request(
        url,
        data=data,
        headers={
            "User-Agent": "Mozilla/5.0 release-qa",
            "Content-Type": "application/json",
        },
    )
    try:
        with build_opener(NoRedirect).open(req, timeout=30) as response:
            return response.status, json.loads(response.read(1_000_000))
    except HTTPError as error:
        try:
            return error.code, json.loads(error.read(1_000_000))
        except (ValueError, UnicodeError):
            return error.code, {}


def det(*args, phrase=None, timeout=650):
    env = isolated_env(ROOT)
    result = subprocess.run(
        [str(ROOT / "app/dash-evo-tool/det-cli"), "--standalone", *args],
        env=env,
        cwd=ROOT,
        input=phrase,
        capture_output=True,
        text=True,
        timeout=timeout,
    )
    if result.returncode:
        raise RuntimeError("det-cli command failed (output withheld)")
    return json.loads(result.stdout)


address = insight.address

REFUND_WARNING = "Refund skipped: no valid faucet depositAddress or QA_FAUCET_REFUND_ADDRESS is available. Funds remain in the disposable testnet wallet."


def refund_address(config):
    for candidate in (
        os.environ.get("QA_FAUCET_REFUND_ADDRESS", "").strip(),
        config.get("depositAddress"),
    ):
        try:
            return address(candidate)
        except (ValueError, OverflowError, TypeError):
            pass
    return None


def save(state):
    atomic_write(STATE, json.dumps(state))


def token(endpoint):
    endpoint = cap_endpoint(endpoint)
    deadline = time.monotonic() + 540  # Leave a minute for redemption/network latency.
    status, challenge = request(endpoint + "challenge", {})
    if status != 200:
        raise RuntimeError("CAP challenge unavailable")
    solutions = cap_solve(challenge["token"], challenge["challenge"], deadline=deadline)
    if time.monotonic() >= deadline:
        raise TimeoutError("CAP challenge expired before redemption")
    status, redeemed = request(
        endpoint + "redeem", {"token": challenge["token"], "solutions": solutions}
    )
    if status != 200 or not isinstance(redeemed.get("token"), str):
        raise RuntimeError("CAP redemption unavailable")
    return redeemed["token"]


def provision():
    from mnemonic import Mnemonic

    state = {"funding": "not_reached", "refund": "not_needed"}
    save(state)
    det("network-switch", "network=testnet")
    phrase = Mnemonic("english").generate(strength=256)
    result = det(
        "core-wallet-import",
        "--mnemonic-stdin",
        "network=testnet",
        "alias=release-qa",
        phrase=phrase + "\n",
    )
    del phrase
    state["wallet_id"] = result["seed_hash"]
    save(state)
    state["address"] = address(
        det(
            "core-address-create", "network=testnet", "wallet_id=" + state["wallet_id"]
        )["address"]
    )
    save(state)


def fund():
    state = json.loads(STATE.read_text())
    if state.get("funding") != "not_reached" or not state.get("address"):
        return
    status, config = request(FAUCET + "/api/status")
    if status != 200 or config.get("network") != "testnet":
        return
    state["refund_address"] = refund_address(config)
    if state["refund_address"] is None:
        state["warning"] = REFUND_WARNING
    state["payout_duffs"] = int(Decimal(str(config["coreFaucetAmount"])) * 100_000_000)
    if not 0 < state["payout_duffs"] <= 100_000_000:
        raise ValueError("Faucet payout exceeds the 1 tDASH burner cap")
    for _ in range(5):
        cap = token(config["capEndpoint"])
        # Persist uncertainty BEFORE sending: a lost response must never be retried.
        state.update(funding="unknown", refund="pending")
        save(state)
        status, response = request(
            FAUCET + "/api/core-faucet",
            {"address": state["address"], "capToken": cap},
        )
        action = payout_action(status, response)
        if action in ("sent", "replay"):
            state.update(funding=action, txid=response["txid"])
            save(state)
            return
        if 400 <= status < 500 or action in ("stale", "refused"):
            state.update(funding="not_reached", refund="not_needed")
            save(state)
        if action != "stale":
            return
    state["funding"] = "not_reached"
    save(state)


def refund():
    deadline = time.monotonic() + 25 * 60 - 15  # Reserve final state-write time.
    observation_deadline = time.monotonic() + 20
    state = json.loads(STATE.read_text())
    if state.get("refund") != "pending":
        return
    if not state.get("refund_address"):
        state.update(refund="skipped_no_address", warning=REFUND_WARNING)
        save(state)
        print(REFUND_WARNING)
        return
    destination = address(state["refund_address"])
    source = address(state["address"])

    # Sweep only the known single confirmed faucet output; otherwise fail closed.
    def observe(*, utxo=False):
        return insight.query(
            "addr",
            source,
            utxo=utxo,
            timeout=min(5, min(observation_deadline, deadline) - time.monotonic()),
        )

    for attempt in range(3):
        try:
            balance = observe()
            utxos = observe(utxo=True)
            if not isinstance(utxos, list) or len(utxos) != 1:
                raise ValueError("Refund needs exactly one unspent faucet output")
            coin = utxos[0]
            if (
                not isinstance(balance, dict)
                or not isinstance(coin, dict)
                or balance.get("addrStr") != source
                or type(balance.get("balanceSat")) is not int
                or type(balance.get("unconfirmedBalanceSat")) is not int
                or balance["unconfirmedBalanceSat"] != 0
                or type(coin.get("satoshis")) is not int
                or coin["satoshis"] != balance["balanceSat"]
                or type(coin.get("confirmations")) is not int
                or coin["confirmations"] < 1
                or coin.get("address") != source
                or not isinstance(coin.get("txid"), str)
                or not re.fullmatch(r"[0-9a-f]{64}", coin["txid"])
            ):
                raise ValueError(
                    "Refund balance is not confirmed or disagrees with UTXO"
                )
            if state.get("txid") and coin["txid"] != state["txid"]:
                raise ValueError("Refund input is not the faucet payout")
            amount = refund_amount(balance["balanceSat"], state["payout_duffs"])
            break
        except (
            OSError,
            ValueError,
            KeyError,
            TypeError,
            RuntimeError,
            subprocess.SubprocessError,
        ):
            remaining = min(observation_deadline, deadline) - time.monotonic()
            if attempt == 2 or remaining <= 0:
                state["refund"] = "unknown"
                save(state)
                raise
            time.sleep(min(1, remaining))
    state["refund"] = "unknown"
    save(state)
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise TimeoutError("Refund deadline expired before send; do not resend")
    result = det(
        "core-funds-send",
        "network=testnet",
        "wallet_id=" + state["wallet_id"],
        "address=" + destination,
        f"amount_duffs={amount}",
        timeout=remaining,
    )
    if not re.fullmatch(r"[0-9a-f]{64}", result.get("txid", "")):
        raise ValueError("No refund transaction ID; do not resend")
    state.update(refund="sent", refund_txid=result["txid"], refund_duffs=amount)
    save(state)


if __name__ == "__main__":
    try:
        {"pre": provision, "fund": fund, "post": refund}[sys.argv[1]]()
    except Exception as error:
        # Exception text can contain remote data or CLI input. Keep it private.
        print(f"Wallet {sys.argv[1]} incomplete: {type(error).__name__}")
        sys.exit(1)
