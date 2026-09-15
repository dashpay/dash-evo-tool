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
  `$HOME` (`~/Library/Application Support/Dash-Evo-Tool` on macOS), so this
  keeps the test off your real profile. Snapshot the profile dir to back it up.

## Order of operations

1. **Phase 0:** Run `01_setup_v3_network.sh` to start a dashmate v3 local
   network on protocol 11.
2. **Phase 0:** Run `02_extract_masternode_keys.py` to write the gitignored
   `keys.json` file.
3. **Phase 1:** In *DET 0.9.3*, load an Evonode identity using a proTxHash and
   its owner, voting, and payout WIFs from `keys.json`. Create a wallet and copy
   its receive address.
4. **Phase 1:** Run
   `python3 03_fund_address.py <receive-address> [amount]` to fund the DET wallet
   from the Core `main` wallet and wait for confirmation.
5. **Phase 1:** In *DET 0.9.3*, top up the identity, withdraw to L1, and snapshot
   the profile.
6. **Phase 2:** Run `04_upgrade_to_v4.sh` to upgrade the network in place from
   v3 to v4 and wait for protocol 12.
7. **Phase 3:** Run `p2p_proxy.py`. This is **required for the new DET**; see the
   gotcha below.
8. **Phase 3:** Run the *new DET (PR)* on a copy of the 0.9.3 profile and confirm
   the identity migrated on the Masternodes screen.
9. **Phase 3:** Run `query_identity_dapi.sh <proTxHash>` to check on-chain
   liveness and balance.
10. **Phase 3:** In the *new DET*, open Masternodes -> identity -> Owner key or
    Payout key -> Sign Message. Copy the signature and verify it with
    `verify_signed_message.py`.

## Key-functionality check (the local substitute for a withdrawal)

In the new DET, open a migrated key (Owner or Payout), type a known message,
click **Sign Message**, copy the Base64 signature, then:

```bash
python3 verify_signed_message.py <key_address> <base64_signature> "<message>"
```

Exit 0 / `MATCH` means the migrated private key still produces a valid signature
for its address. For a byte-for-byte cross-check against the address in the
`local_seed` Core `main` wallet, run:

```bash
npx -y dashmate@4.0.0 core cli \
  "-rpcwallet=main signmessage <address> \"<message>\"" \
  --config=local_seed
```

The command yields the identical signature — deterministic RFC6979 — proving
DET holds the same key.

## Additional check — encrypted wallet survives the upgrade

Verifies a password-protected wallet created in 0.9.3 still unlocks in the new
DET (see SPEC "Additional scenario"). No network is needed for the wallet
itself; run it alongside or independently of the identity flow.

1. **In DET 0.9.3** — Wallets -> **Create Wallet**. Hover the entropy grid,
   click **Generate**, tick **"I wrote it down"**, give it a **name** and a
   **strong password** (the strength meter must read at least "Strong"), then
   **Save Wallet**. Record the name + password.
2. **Confirm it's encrypted at the storage layer** (profile still on 0.9.3):
   use `<profile>/Library/Application Support/Dash-Evo-Tool/data.db` on macOS or
   `<profile>/.config/dash-evo-tool/data.db` on Linux.

   ```sql
   sqlite3 "<database-path>" \
     "SELECT alias, uses_password FROM wallet;"
   ```

   The new wallet must show `uses_password = 1` (a no-password wallet shows
   `0`).
3. **Snapshot** the profile dir, then **run the new DET on a copy** of it.
   Choose *Just Explore*, dismiss the sync modal (*Continue in the background*),
   open **Wallets**, and select the wallet.
4. **Unlock it** — click **Unlock**, enter the original password, and confirm.
   Pass = the wallet is present and the **Unlock button flips to Lock**
   (authenticated AES-256-GCM decryption succeeded, so the password + seed
   survived migration). A wrong password shows an error and the button stays
   **Unlock**.

Note: address rows may stay empty while SPV is unsynced (the dip0024 limitation
below) — that does not affect the unlock result.

## Gotchas

- **P2P port proxy (step 7).** The PR's SPV client hardcodes the regtest Core
  P2P port `19899`, but dashmate's `local_1` Core listens on `20001`. Run
  `python3 p2p_proxy.py` (bridges `127.0.0.1:19899 -> 20001`) before starting
  the new DET, or its SPV client can't connect at all.
- **dip0024 / withdrawal.** See SPEC "Known limitation" — the new DET's SPV
  verifier needs an `llmq_test_dip0024` rotation quorum that a 3-MN local net
  can't form, so proof-verified ops (including withdrawal) fail locally with
  "servers unreachable". The Sign Message check above is the workaround.
- **Signing a *fresh* message.** Always sign a message you control and verify
  against *that* message. Recovering a stale signature against the wrong message
  yields a valid-but-wrong address and looks like a failure.
- **Text entry on non-US keyboard layouts.** Automated typing tools may fail to
  map characters; paste via the clipboard instead.
