#!/usr/bin/env python3
"""Extract each local masternode's platform-identity keys from the running
network, so they can be loaded into DET as a standalone evonode identity.

For a masternode/evonode, the platform OWNER identity id == the proTxHash, and
its keys are the owner / voting / payout address keys. dashmate only persists
the operator BLS key in config.json; the owner/voting/payout private keys live
in the local_seed Core wallet ("main"), retrievable via dumpprivkey.

Writes keys.json next to this script. keys.json is gitignored (contains WIF
private keys) — regenerate it locally, never commit it.

Requires: the network from 01_setup_v3_network.sh running.
"""
import json
import os
import subprocess

DASHMATE_CONFIG = os.path.expanduser("~/.dashmate/config.json")
SEED_RPC = "http://127.0.0.1:20302/"
OUT = os.path.join(os.path.dirname(__file__), "keys.json")


def rpc_password():
    cfg = json.load(open(DASHMATE_CONFIG))
    return cfg["configs"]["local_seed"]["core"]["rpc"]["users"]["dashmate"]["password"]


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
    pw = rpc_password()
    out = {}
    for p in call(pw, "protx", ["list", "registered", True]):
        st = p["state"]
        entry = {
            "proTxHash": p["proTxHash"],          # == platform owner identity id
            "type": p.get("type"),
            "ownerAddress": st["ownerAddress"],
            "votingAddress": st["votingAddress"],
            "payoutAddress": st["payoutAddress"],
            "operatorPubKey": st["pubKeyOperator"],
        }
        for role, addr_key in (("ownerPrivkey", "ownerAddress"),
                               ("votingPrivkey", "votingAddress"),
                               ("payoutPrivkey", "payoutAddress")):
            entry[role] = call(pw, "dumpprivkey", [st[addr_key]], wallet="main")
        out[p["proTxHash"][:8]] = entry
    json.dump(out, open(OUT, "w"), indent=2)
    print(f"wrote {OUT} ({len(out)} masternode identities)")
    for k, v in out.items():
        print(f"  {k} {v['type']} owner={v['ownerAddress']} payout={v['payoutAddress']}")


if __name__ == "__main__":
    main()
