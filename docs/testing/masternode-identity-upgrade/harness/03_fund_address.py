#!/usr/bin/env python3
"""Fund a DET wallet receive address from the local_seed Core wallet ("main"),
then wait for one confirmation. Used in phase 1 to give the DET wallet UTXOs so
it can top up the identity via asset lock.

Usage: 03_fund_address.py <dash_address> [amount_dash]
Requires: the network running.
"""
import json
import os
import subprocess
import sys
import time

DASHMATE_CONFIG = os.path.expanduser("~/.dashmate/config.json")
SEED_RPC = "http://127.0.0.1:20302/"


def call(pw, method, params=None, wallet=None):
    url = SEED_RPC + (f"wallet/{wallet}" if wallet else "")
    body = json.dumps({"jsonrpc": "1.0", "id": "h", "method": method, "params": params or []})
    r = subprocess.run(
        ["curl", "-s", "--user", f"dashmate:{pw}", "--data-binary", body,
         "-H", "content-type: text/plain;", url],
        capture_output=True, text=True,
    )
    d = json.loads(r.stdout)
    if d.get("error"):
        raise RuntimeError(d["error"])
    return d["result"]


def main():
    if len(sys.argv) < 2:
        print("usage: 03_fund_address.py <dash_address> [amount_dash]")
        raise SystemExit(2)
    address = sys.argv[1]
    amount = float(sys.argv[2]) if len(sys.argv) > 2 else 10.0
    pw = json.load(open(DASHMATE_CONFIG))["configs"]["local_seed"]["core"]["rpc"]["users"]["dashmate"]["password"]

    txid = call(pw, "sendtoaddress", [address, amount], wallet="main")
    print(f"sent {amount} DASH to {address}: {txid}")
    for _ in range(30):
        conf = call(pw, "gettransaction", [txid], wallet="main").get("confirmations", 0)
        if conf >= 1:
            print(f"confirmed ({conf} conf)")
            return
        time.sleep(10)
    print("WARNING: not confirmed within timeout")
    raise SystemExit(1)


if __name__ == "__main__":
    main()
