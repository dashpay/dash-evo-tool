# GUI Testing Guidelines

This directory holds standing guidance and reusable scenarios for testing the
**real, compiled application** through a real display — not `kittest` (which
drives an in-process harness with no display, no network) and not
`backend-e2e` (which calls `BackendTask`s directly, skipping the UI entirely).

Use this tier when a change needs to be verified as it actually behaves for a
user: real screen navigation, real timing (SPV sync, async task completion),
real visual state, or a flow that spans multiple screens in ways `kittest`
doesn't cover.

## Which test tier do I need?

| Tier | Drives | Network | Good for |
|---|---|---|---|
| `tests/kittest/` | In-process `egui_kittest` harness, no display | None | Screen logic, widget state, gating — fast, deterministic, runs in CI |
| `tests/backend-e2e/` | `BackendTask`s directly, no UI | Live testnet | Backend correctness (SDK calls, signing, persistence) without UI concerns |
| **`docs/gui-testing/`** (this dir) | The actual compiled binary, real display | Live testnet (usually) | End-to-end flows a user would actually experience — navigation, real async timing, visual verification |

If a scenario can be expressed as a `kittest` test, write a `kittest` test
instead — it's faster, deterministic, and runs in CI on every PR. Reach for
this tier only when the thing under test genuinely needs a real display, real
network timing, or a flow `kittest` can't simulate.

## How to run a scenario

The mechanical launch recipe (X display setup, accessibility tree, gotchas
like stderr redirection) lives in the global `desktop-gui` Claude Code skill
(`~/.claude/skills/desktop-gui/SKILL.md`) — read that first, it's not repeated
here. This directory covers what's specific to *this project*: which
scenarios exist, what credentials they need, and the safety rules for running
them against a live network.

## Non-negotiable safety rules

1. **Always use an isolated data directory.** Never point `DASH_EVO_DATA_DIR`
   at a real user's default location. Use a fresh `mktemp -d` per run, copy
   `.env.example` into it, and adjust network config as the scenario requires.
2. **Never touch an already-running instance.** Check `pgrep -af dash-evo-tool`
   before launching; if one is already running, leave it alone and launch a
   second, separately-windowed instance for your test.
3. **Never hardcode secrets in a scenario file.** Reference environment
   variable *names* only (matching the `tests/backend-e2e/` convention — e.g.
   `E2E_WALLET_MNEMONIC`, `E2E_MN_*`). Read actual values at run time via
   `grep`/`env`, never paste them into a scenario doc, a report, or a commit.
4. **Default to testnet.** Only use mainnet if a scenario explicitly requires
   it and says why — and if it does, treat every fund-moving step as
   irreversible and get explicit confirmation before broadcasting.
5. **Cap fund movement.** When a scenario broadcasts a real transaction
   (withdrawal, send, etc.), move the smallest amount that still proves the
   behavior (e.g. ≤10% of an available balance), never "whatever's available."
6. **Prefer destination paths that can't misfire.** Where the app offers a
   consensus-enforced or otherwise fixed destination (e.g. a masternode
   owner-key withdrawal forcing the registered payout address), use that path
   over one where you type an arbitrary address by hand.
7. **Check the logs, not just the screen.** A crash or panic doesn't always
   show a UI error — check `det-stderr.log` / `det.log` in the test's data
   directory (or the default location if unset) for a Rust panic
   (`location=...` line) even when the UI appeared to work.

## Scenario file format

Each scenario is a Markdown file under `docs/gui-testing/scenarios/`, named
`<short-slug>.md`. Use [`scenarios/TEMPLATE.md`](scenarios/TEMPLATE.md) as the
starting point. A good scenario is written at the level of "what to look for"
rather than pixel coordinates or exact window sizes, so it survives minor UI
changes — describe the screen/control by its label or role, not its position.

Keep a scenario file updated when the flow it describes changes shape; a
stale scenario that no longer matches the app is worse than no scenario, since
it wastes the next run re-discovering the drift. If a run surfaces a new gotcha
(timing, an unexpected intermediate screen, a naming mismatch), fold it back
into the scenario file rather than letting it live only in that run's report.

## Sequencing many scenarios in one session

A large regression pass runs dozens of scenarios back-to-back against the same
app instance and data directory. Practices that keep this efficient and avoid
one scenario silently invalidating another:

- **Order by dependency, not catalog order.** Identify the handful of
  scenarios that create prerequisite state (wallet funding, identity
  registration, contract registration) and run those first — everything
  downstream goes faster and produces fewer false BLOCKED results.
- **Reuse one running instance by default.** Only restart the app when a
  scenario's own acceptance criteria specifically require a cold boot (e.g.
  "settings persist across restart"). Restarting between every scenario wastes
  wall-clock time for no benefit on scenarios that don't need it.
- **Hold state-destroying scenarios until last**, and only once nothing
  remaining in the queue depends on current state. Running a data wipe/reset
  mid-campaign can silently invalidate hours of already-established fixture
  state for scenarios still to come.
- **Watch for restart-triggered failure modes.** Some defects only manifest on
  the *next* app launch, not immediately (e.g. a row written once that only
  fails to decode on a later rehydration). If a scenario creates new durable
  state, treat any later restart in the same session as elevated risk until
  that state has been sanity-checked.
- **End every handoff with a state dump.** When a campaign passes from one
  agent/session to the next with no shared memory, record the app's PID, the
  binary's hash, and every piece of fixture state already created (wallet/
  identity names and balances, contract/token IDs, established contacts). A
  vague handoff costs the next session real time rediscovering what already
  exists.
- **For "two users" scenarios**, register a second identity/contact in the
  same wallet rather than treating the scenario as untestable solo — this is
  normally fully sufficient and avoids fabricating a "needs another tester"
  excuse.

## Telling a real defect from a log/DB-reading artifact

- **A generic-looking log line isn't proof the details don't exist.** A call
  site can log a typed error with `Display`-only formatting (`%error` in a
  `tracing::error!` call) instead of `Debug` (`?error`), silently discarding
  the structured detail you need. If a log line reads suspiciously generic
  given how specific the underlying error type should be, find the call site
  and check which format specifier it uses before concluding "the logs don't
  say more than this."
- **Verify persistence independently of the UI.** For anything claiming to
  write data, a direct read-only query against the underlying SQLite file
  confirms whether the write actually landed — this is the only way to tell
  "the feature is broken" apart from "the write worked but its own display has
  an unrelated rendering/cache bug."
- **Always open a live app's SQLite file with `sqlite3 -readonly`** when
  inspecting it out-of-band. A plain (non-readonly) `sqlite3 file.db "SELECT
  ..."` can still trigger a WAL checkpoint on open/close, mutating the
  `-wal`/`-shm` sidecar files even for a pure read — this can silently destroy
  the exact on-disk state you're trying to preserve as evidence.
- **Rule out an environmental cause before committing to a root-cause
  theory.** Reproduce against a brand-new, zero-state data directory. If the
  failure still occurs there, whatever theory tied it to specific prior test
  data or fixture state is wrong.
- **Hold competing root-cause theories loosely until they agree.** A narrower
  differential test and a later, more precise investigation can both be
  correct while answering different-scoped questions — write up findings so a
  later contradiction doesn't require silently discarding earlier work.

## Known UI/environment quirks

- **Default window is small (800×600) and clips controls** (sidebar items,
  settings sections below the fold). Resize immediately after launch — see the
  `desktop-gui` skill's launch recipe. Some settings sections are collapsible
  *and* below the fold even after resizing: expect to expand a section, then
  scroll, before a control becomes visible — don't conclude a control doesn't
  exist from the first screenshot after expanding.
- **Confirmation dialogs can self-dismiss on a very fast synthetic click.**
  Several dialogs share a common "click outside closes the dialog" helper
  (`clicked_outside_window()` in `src/ui/helpers.rs`). A scripted click (e.g.
  `xdotool click`) can register its press+release within the same UI frame the
  dialog opens in, which some call sites read as a click "outside" the dialog
  and dismiss it immediately. If a dialog flashes shut the instant it opens,
  suspect this pattern before assuming a mis-click — take a screenshot a frame
  later and retry with the click and the opening action clearly separated.
- **The shared `/data/target` build output is not campaign-exclusive.** If
  other worktrees/sessions on the same box can rebuild concurrently, the
  binary under test can be silently overwritten mid-campaign by an unrelated
  build. For any run spanning hours, build to a private path and hash-verify
  (`sha256sum`) before each relaunch rather than trusting the shared path
  throughout.

## Scenario index

| Scenario | What it verifies |
|---|---|
| _(none yet — first one lands once the masternode-withdrawal-without-a-wallet run is verified)_ | |

<sub>🤖 Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
