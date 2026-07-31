# Scenario: Shielded feature availability right after platform sync

**Verifies:** The app detects the connected network's protocol version
successfully after SPV/platform sync completes, so shielded send/receive/
shield/unshield become available as soon as the network supports them,
without a spurious generic error banner as a side effect of that detection.
(Risk area: #936, #938.)

**Tier justification:** This is specifically about a real live-network
proof-verification call succeeding or failing against the actual connected
testnet — it cannot be simulated without a real SPV sync and a real
protocol-version-detection round trip.

**Run against BOTH builds with this identical procedure** (baseline
`v1.0.0-weekly.20260721`, then HEAD `origin/v1.0-dev`).

## Prerequisites

- Network: testnet
- Environment variables (names only):
  - `E2E_WALLET_MNEMONIC` — funded testnet wallet
- Verify exact banner/error wording during execution rather than assuming
  the text below is literal

## Setup

```bash
DATADIR=$(mktemp -d)
cp .env.example "$DATADIR/.env"
pgrep -af dash-evo-tool

BIN=<path to the build under test — baseline or HEAD worktree binary>
test -x "$BIN"
LOG="$DATADIR/shielded-availability.log"
DASH_EVO_DATA_DIR="$DATADIR" nohup "$BIN" >"$LOG" 2>&1 &
```

## Procedure

1. **Cold start.** Launch the app fresh (isolated data dir, no prior sync
   state) and wait for SPV sync to complete. Record whether any generic
   error banner (e.g. "An unexpected error occurred") appears purely as a
   side effect of sync completing.
2. **Shielded action availability.** Once sync completes, record whether
   Shield, Unshield, Send Privately (shielded), and shielded Receive are
   reachable/enabled.
3. **Send-fee estimate.** Open a Send flow and record whether a fee
   estimate is shown, and whether it appears to update or stays fixed at a
   default value.
4. **Network interruption during detection.** If you can simulate a brief
   network outage right at startup, record whether the app keeps retrying
   detection (shielded stays unavailable but the app doesn't misbehave) or
   assumes a version without confirmation.
5. **Restart after successful detection.** Restart the app once shielded
   features were confirmed available, and record whether they remain
   available immediately on the next launch.

## Safety constraints specific to this scenario

- None beyond the standing rules.

## Expected outcome / pass criteria

Record the observed behavior for each build at every step, then apply this
rule:

- **BLOCKING** only if HEAD is worse than baseline in the happy flow (e.g.
  shielded features activate correctly after sync on baseline but stay
  silently disabled on HEAD, or vice versa in a way that matters for this
  release; a spurious error banner appears on HEAD where baseline was
  clean), or a **data-loss** outcome (unlikely for this scenario, but note
  if found) on either build.
- **NOT blocking**: a difference in exactly which internal call detects the
  protocol version, as long as the user-visible outcome (shielded features
  available when the network supports them, generic error banner absent) is
  the same or better on HEAD; an issue reproducing identically on both
  builds.
- If baseline's shielded features never activate at all after sync (the bug
  this PR fixes), and HEAD's do — that's the expected direction of
  improvement, not something to flag as a baseline "failure" beyond noting
  it factually.

## Known gotchas

- The fee estimate may legitimately keep using a "last known rate" rather
  than a freshly fetched one on HEAD — this is a documented temporary
  limitation of the fix, not itself a defect, as long as it's not worse than
  baseline's own fee-estimate behavior.
- If testing via MCP/CLI tools rather than the GUI, the same underlying
  protocol-version-caching code path applies — a caching bug here previously
  affected `mcp::resolve`-backed tools too.

<sub>🤖 Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
