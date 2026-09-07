# Scenario: Startup storage gate and SPV sync cancellation

**Verifies:** that a cold launch blocks the UI behind exactly one storage-preparation
gate which always releases, that chain sync only starts after that gate has released,
and that cancelling the SPV sync overlay is a deliberate two-step choice in which no
single keypress or reflexive click ever disconnects the wallet.

**Tier justification:** the behavior under test is the boot sequence itself — the
ordering between wallet-backend wiring, the legacy `data.db` drain and chain sync, and
the real wall-clock window during which the UI is blocked. `kittest` drives an
in-process harness with no display and no network, so it can assert the overlay's
widget state but cannot observe whether the real app releases the block, how long a
real user waits, or whether sync genuinely starts only afterwards. The migration and
first-run branches also depend on on-disk state a real launch creates.

## Prerequisites

- Network: testnet for the sync-cancellation part; the storage-gate part is
  network-independent and is also checked on the app's own first-run default.
- Environment variables (names only — see the project's `.env` /
  `tests/backend-e2e/README.md` for where real values come from):
  - `E2E_WALLET_MNEMONIC` — the wallet restored for the "gate with real wallet state"
    pass. Not needed for the first-run pass.
- A legacy `data.db` is needed only for the migration variant (step 5). If none is
  available, record that variant as NOT COVERED rather than fabricating one.
- The data directory's **entire ancestor chain** must be free of group/other-write
  permission. The secret store refuses to open otherwise and the app exits during
  startup with `AppCreation(SecretStore { source: InsecureParentDir { mode: … } })`.
  A `mktemp -d` under a world-writable `/tmp` or a `~/.config` at mode `0775` both
  trip this. See "Known gotchas".

## Setup

```bash
# Isolated data dir — never point at a real user's default location.
# Every level must be 0700; see Prerequisites.
SCRATCH=$(mktemp -d); chmod 700 "$SCRATCH"
DATADIR="$SCRATCH/datadir"; mkdir -p "$DATADIR"; chmod 700 "$DATADIR"
cp .env.example "$DATADIR/.env"

# Confirm no conflicting instance is already using this display/data dir
pgrep -af dash-evo-tool

TARGET_DIR=$(cargo metadata --format-version 1 --no-deps | \
  python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')
BIN="$TARGET_DIR/debug/dash-evo-tool"
test -x "$BIN"
LOG="$DATADIR/startup-storage-gate-and-sync-cancel.log"

# Verify the display before launching — never launch onto a forwarded/real desktop.
: "${DISPLAY:?Set DISPLAY to the desktop used for GUI testing}"
xdpyinfo >/dev/null

date -Ins   # record the launch instant; the gate's duration is measured against it
DASH_EVO_DATA_DIR="$DATADIR" nohup "$BIN" >"$LOG" 2>&1 &
```

## Procedure

### 1. Cold first run — the gate raises and releases

1. Launch against a **brand-new, empty** data directory as in Setup. Start a
   wall-clock timer at launch.
2. Watch the window from the moment it maps. Note whether a blocking progress overlay
   is shown, what it says, and whether anything behind it is clickable while it is up.
3. Wait until the overlay lowers on its own. Record how long that took. Do not click
   anything until it has lowered.
4. Confirm the app is now interactive: the left navigation responds and a screen other
   than the one first shown can be opened.
5. Read the launch log and record, in order, the storage-preparation lines and the
   first line showing chain sync starting. The point of this step is the **ordering**,
   not the exact wording.

### 2. Which network a fresh install starts on

6. Before touching anything else, read the network selector and record the network the
   app chose for itself on a fresh data directory. Do not assume; read it.

### 3. Cold run with real wallet state

7. Fully close the app and confirm the process has exited.
8. Relaunch against the **same** data directory, restore the wallet from
   `E2E_WALLET_MNEMONIC` through the app's own import flow, then fully close again.
9. Relaunch a third time against that now-populated data directory and repeat steps
   1–5. Record whether the gate takes noticeably longer than on the empty directory
   and whether it still releases without intervention.

### 4. The SPV sync overlay's Cancel is two-step

Perform on the launch from step 9, while chain sync is still running.

10. Locate the sync overlay's own action row. Record every button label on it.
11. Press **Enter**, then **Escape** (separately, one at a time, re-observing between
    them). Record what each key does. Neither may disconnect the wallet on its own.
12. Activate the row's **Cancel** control. Record what appears. Record which control
    holds keyboard focus at that moment.
13. With the confirmation on screen, press **Enter**. Record the result.
14. Bring the confirmation back if step 13 dismissed it, then choose the option that
    keeps syncing. Confirm sync is still running afterwards — check the connection/
    sync indicator, not only the overlay.
15. Bring the confirmation back once more and choose the option that stops syncing.
    Record what the app then shows and whether the log agrees that sync stopped.
16. Follow whatever route the confirmation named for starting again, and confirm sync
    can in fact be restarted from there.

### 5. Migration variant (only if a legacy `data.db` is available)

17. Seed a data directory containing a legacy `data.db`, then launch.
18. Record whether the gate reports migration/drain work, whether any banner is raised
    about it, and whether the gate still releases without intervention.
19. Relaunch once more against the same directory and confirm the migration work is not
    repeated and no banner reappears.

## Safety constraints specific to this scenario

- This scenario broadcasts nothing and moves no funds. If any step appears to offer a
  transaction, that is a deviation — stop and record it rather than proceeding.
- Never launch against a data directory outside the scenario's own scratch tree, and
  never against a display that could be a real person's desktop (see Setup).
- Step 15 disconnects the wallet deliberately. Do it last within its build's pass, so
  nothing after it inherits a disconnected wallet unnoticed.
- Do not type the mnemonic on a command line or leave it visible in a captured
  screenshot. Capture before typing, or after the field is cleared.

## Expected outcome / pass criteria

Per the A/B build comparison contract in the [README](../README.md#ab-build-comparison-contract),
run the identical procedure against both binaries. A finding is **blocking** when the
development build is worse than the baseline build from the user's perspective in this
happy flow, or when any step loses data. It is **not blocking** when it reproduces
identically on both builds (pre-existing — record it, do not report it as a
regression) or when it is timing noise with no user-visible effect. Reproduce anything
about to be called blocking at least twice.

Step by step, "it worked" means:

- **1–4**: the block is raised at most once, the UI behind it cannot be operated while
  it is up, and it lowers **on its own** with no user action. The app is fully
  interactive afterwards. A gate that never lowers, needs a click to lower, or lowers
  into a half-initialized app (screens present but erroring) is a failure. Record the
  duration on both builds; a large regression in how long the user is blocked is a
  user-visible regression even when the gate eventually releases.
- **5**: storage preparation completes **before** the first chain-sync line. Sync
  starting while preparation is still running is a failure of the property under test.
- **6**: whichever network is reported, it is recorded, not assumed. This step exists
  because a wrong assumption here has previously caused a healthy fixture to be
  misdiagnosed as dead — see "Known gotchas".
- **10–11**: neither Enter nor Escape stops the sync by itself. Either may open the
  confirmation or do nothing; neither may disconnect. A single keypress that
  disconnects the wallet is a failure.
- **12**: the Cancel control **asks** rather than acting. The question names both the
  consequence and the way back, and keyboard focus rests on the option that keeps
  syncing.
- **13**: Enter on the confirmation resolves to **keep syncing**. Enter resolving to
  stop is a failure — that is the exact defect the two-step design exists to prevent.
- **14**: after keeping, sync is demonstrably still running.
- **15–16**: stopping is honoured, the app says so, and the route the question named
  really does restart sync. A confirmation that names a route which does not work is a
  failure of the message, not only of the flow.
- **17–19** (if run): the gate reports the migration, still releases on its own, and
  the second launch neither repeats the work nor re-raises the banner. Repeating
  migration work on every launch is a failure.

Any Rust panic (`location=…`) in `det.log` / `det-stderr.log` for either build fails
the scenario regardless of what the screen showed.

## Known gotchas

- **The secret store validates the data directory's whole ancestor chain.** A single
  group-writable ancestor aborts startup with
  `AppCreation(SecretStore { source: InsecureParentDir { mode: 509 } })` (509 is
  decimal for `0775`). This bites the two most natural choices: a plain `mktemp -d`
  under a world-writable `/tmp`, and the default `~/.config/dash-evo-tool` on a system
  where `~/.config` is `0775`. The failure is written only to the log — the launching
  terminal shows a short "failed to start" message with no permission detail — so read
  `det.log` before assuming the binary is broken.
- **A fresh data directory does not start on testnet.** Read the network selector
  before concluding a fixture is dead; a zero balance or a "masternode not found" on a
  fresh install is far more often the wrong network than a broken fixture.
- **The window opens at 800×600 and clips controls**, including parts of the overlay's
  action row. Resize immediately after launch and before any interaction, otherwise a
  button can be judged absent when it is merely below the fold.
- **Confirmation dialogs can self-dismiss on a very fast synthetic click** (the shared
  `clicked_outside_window()` helper in `src/ui/helpers.rs`). If the Cancel confirmation
  flashes shut the instant it opens, separate the opening action and the next click by
  a frame and retry before recording it as a defect — and note which way a dismissal
  resolved, since a dismissal that stops the sync would defeat the two-step design.
- **The overlay is lowered and re-raised when its action row changes** between the
  progress row and the confirmation row. A screenshot taken during that swap can catch
  a frame with no overlay at all; take a second one before concluding the block was
  released.
- **Check the log, not only the screen.** A crash or a failed migration does not always
  surface in the UI. Read `det.log` and `det-stderr.log` in the scenario's own data
  directory for both builds.

<sub>🤖 Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
