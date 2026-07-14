# Harness — masternode-identity upgrade test

Reproducible scripts for the test described in [`../SPEC.md`](../SPEC.md). They
regenerate everything locally; **no secrets or binaries are committed** (see
`.gitignore`). The DET UI steps are still manual today — the value here is the
deterministic network/keys/verification tooling an automated test can build on.

## Prerequisites

- Docker running; `npx` (Node) available.
- A checkout of `dashpay/platform` (for DAPI `.proto` files) — default path
  `~/Projects/dashpay/platform`.
- `grpcurl` (for `query_identity_dapi.sh`).
- Two DET builds: `v0.9.3` and the PR branch, each run against an **isolated
  profile** via `HOME` override, e.g.
  `HOME=/path/to/profile /path/to/dash-evo-tool` — DET derives its data dir from
  `$HOME` (`~/Library/Application Support/Dash-Evo-Tool` on macOS), so this keeps
  the test off your real profile. Snapshot the profile dir to back it up.

## Order of operations

| Step | Script / action | Phase |
|------|-----------------|-------|
| 1 | `01_setup_v3_network.sh` — dashmate v3 local net, protocol 11 | 0 |
| 2 | `02_extract_masternode_keys.py` — writes `keys.json` (gitignored) | 0 |
| 3 | *DET 0.9.3*: Load Identity (Evonode) with a proTxHash + its owner/voting/payout WIFs from `keys.json`; create wallet; fund from Core; top up; withdraw to L1. Snapshot the profile. | 1 |
| 4 | `04_upgrade_to_v4.sh` — in-place v3->v4, waits for protocol 12 | 2 |
| 5 | `p2p_proxy.py` — **required for the new DET** (see gotcha) | 3 |
| 6 | *New DET (PR)*: run on a copy of the 0.9.3 profile; confirm the identity migrated (Masternodes screen). | 3 |
| 7 | `query_identity_dapi.sh <proTxHash>` — on-chain liveness/balance | 3 |
| 8 | *New DET*: Masternodes -> identity -> Owner key / Payout key -> Sign Message; copy the signature; verify with `verify_signed_message.py` | 3 |

## Key-functionality check (the local substitute for a withdrawal)

In the new DET, open a migrated key (Owner or Payout), type a known message,
click **Sign Message**, copy the Base64 signature, then:

```
python3 verify_signed_message.py <key_address> <base64_signature> "<message>"
```

Exit 0 / `MATCH` means the migrated private key still produces a valid signature
for its address. For a byte-for-byte cross-check, `dashd signmessage <address>
"<message>"` (the address is in the local_seed `main` wallet) yields the
identical signature — deterministic RFC6979 — proving DET holds the same key.

## Gotchas

- **P2P port proxy (step 5).** The PR's SPV client hardcodes the regtest Core P2P
  port `19899`, but dashmate's `local_1` Core listens on `20001`. Run
  `python3 p2p_proxy.py` (bridges `127.0.0.1:19899 -> 20001`) before starting the
  new DET, or its SPV client can't connect at all.
- **dip0024 / withdrawal.** See SPEC "Known limitation" — the new DET's SPV
  verifier needs an `llmq_test_dip0024` rotation quorum that a 3-MN local net
  can't form, so proof-verified ops (including withdrawal) fail locally with
  "servers unreachable". The Sign Message check above is the workaround.
- **Signing a *fresh* message.** Always sign a message you control and verify
  against *that* message. Recovering a stale signature against the wrong message
  yields a valid-but-wrong address and looks like a failure.
- **Text entry on non-US keyboard layouts.** Automated typing tools may fail to
  map characters; paste via the clipboard instead.
