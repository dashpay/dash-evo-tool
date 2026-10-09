# Scenario: Wallet pickers — shared selector on identity screens and the Wallets page

**Verifies:** how each wallet is labelled, how its balance is shown and
explained, how unusable wallets are presented, and what choosing a wallet does,
on the Wallets page, Create identity, Add funds and Load existing identity.
(Risk area: #1060, #1061.)

**Tier justification:** Needs real wallet balances (Core, Platform, shielded,
identities) in a live app and visual confirmation of hover text, greyed rows and
selection persistence.

**Run against BOTH builds with this identical procedure** — the baseline and
development binaries selected for the current campaign (record their exact
SHAs in that campaign's own artifacts, not here). Describe what you observe
on each build without assuming which one is "correct", and note where a
control or screen exists on only one build (a feature-presence difference,
not a step failure).

All steps are read-only (nothing is sent). No fixture duplication is needed
beyond holding equivalent wallets on both sides so balances are comparable.

## Prerequisites

- Network: testnet
- Environment variables (names only):
  - `E2E_WALLET_MNEMONIC` — funded wallet (import it in both builds)
- At least two wallets: the picker on Create identity, Add funds and Load
  existing identity is hidden while only one wallet exists. Prepare the second
  wallet before the run (see the README's "Procedure lessons for A/B
  campaigns"); funding it is fixture preparation, not part of this read-only
  scenario, and each build needs its own equivalent funded wallets
- Also an imported single key (step 2) and one wallet unusable for some
  funding method, e.g. zero balance (steps 3-4); if either is missing, record
  those checks as not exercised
- Verify exact labels during execution

## Setup

```bash
DATADIR=$(mktemp -d)
cp .env.example "$DATADIR/.env"
pgrep -af dash-evo-tool

BIN=<path to the build under test — baseline or development worktree binary>
test -x "$BIN"
LOG="$DATADIR/wallet-pickers.log"
: "${DISPLAY:?Set DISPLAY to the desktop used for GUI testing}"
xdpyinfo >/dev/null
DASH_EVO_DATA_DIR="$DATADIR" nohup "$BIN" >"$LOG" 2>&1 &
```

Read the network indicator after launch. A fresh data directory starts on
Mainnet, so if it is not **Testnet**, select Testnet in Settings ▸ Networks
and confirm the indicator before continuing.

Resize the window (see the README's "Known UI/environment quirks") before
judging layout. Check `det-stderr.log` / `det.log` in `$DATADIR` for panics
after each run.

## Procedure

1. **Wallets page.** Open Wallets. Record how each wallet reads in the open
   list and the closed picker (prefix, name, balance, decimals), whether a
   separate balance label sits next to it, how an unnamed wallet is named, and
   hover text on a wallet.
2. **Switch wallet.** Choose another wallet (and the imported key). Record the
   header total, the selected account tab, and what happens on re-clicking the
   wallet already chosen. Restart the app and record the remembered wallet.
3. **Create identity.** Open the create-identity screen. If no picker is shown,
   record that and the wallet count. Record the picker rows
   and hover text, and which funding methods grey out a wallet and the reason
   in the hover.
4. **Add funds.** Identity Home → "Add funds" → "From your wallet". Record
   the same items; note whether any wallet is greyed out and why.
5. **Load existing identity.** Open the load screen with its "From my wallet"
   source. Record the picker rows and whether identity balances are included
   in the shown total.
6. **Edge cases.** Record how a balance below 0.0001 DASH reads, the wording
   with no wallet chosen, and with no wallets at all (fresh data directory).
7. **Consistency.** For one wallet, compare the balance on every screen above
   and the Wallets page header total; record the differences and whether
   each picker's meaning of "balance" is explained.

## Safety constraints specific to this scenario

- Read-only; nothing is submitted.

## Expected outcome / pass criteria

Record each build's text for every step in a side-by-side table.

- A wallet shown with a balance the wallet does not hold, a usable wallet
  greyed out without reason, or a selection that does not persist, on one
  build only, are differences to classify under the contract.

Apply the [A/B build comparison contract](../README.md#ab-build-comparison-contract):
its blocker rule decides what blocks, and its equivalent-fixture rule applies
to every step marked **MUTATES** below.

## Known gotchas

- The Wallets page header keeps full precision while pickers cut to four
  decimals; a small mismatch between them is expected on the newer build.
- Hover text needs the pointer to rest on the row; take the screenshot while
  hovering.

<sub>🤖 Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
