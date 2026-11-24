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

## Release GUI QA in GitHub Actions

`release-qa.yml` runs on GitHub-hosted `ubuntu-26.04`, called after the weekly
build legs, fixture capture and draft sealing. It verifies the Linux release zip's
attestation against `release.yml`
and the NEW tag's source SHA, then runs the zip's GUI and `det-cli`. There is no
local build, OLD-build A/B or code review. The baseline is the newest older
published pre-release that is an ancestor of NEW, or an explicitly selected older
published ancestor pre-release.
A selected `run_id` must come from a `release.yml` run invoked at the tag's commit
(as weekly builds are). `--source-digest` refuses builds dispatched from another
commit, even when `release.yml` checked out the requested tag.

Before merging workflow changes, an allowlisted maintainer can add the
**`release-qa`** label to a PR from this repository. Remove and re-add the label
to rerun. The first job checks the label event's `github.event.sender.login`
against the repository variable **`RELEASE_QA_LABEL_ACTORS`**, a comma- or
space-separated list of logins (for example, `lklimek, other-maintainer`).
An unset or empty variable defaults to `lklimek`; a nonempty list replaces that
default. Entries must match whole logins, case-insensitively. Repository-variable
changes require administrator access. Other senders (including
`Claudius-Maginificent` and `github-actions[bot]` under the default list), GitHub
Bot accounts, forks, other labels and other PR actions do not run QA.
Unrelated label events use separate concurrency groups and cannot displace a
queued QA run.

Label rehearsals use the newest published weekly; builds older than #1173 lack `--mnemonic-stdin`, so those rehearsals run unfunded and end NOTE.

Only label a PR whose exact head has been reviewed and trusted: this path checks
out the event's PR head SHA and executes its controller and skills with access
to the maintainer's OAuth token during the agent step. It reviews the newest
**published weekly pre-release** as NEW and selects the baseline automatically
as described above. It is report-only: no sealing, finalization, release edits or
publication; the burner may be funded and refunded normally. Every job reachable
from the label path has read-only repository permissions. Its `prepare_pr` job
reads only published assets. The mutually exclusive `prepare` job for weekly or
manual runs alone has `contents: write` to read draft assets; it never executes
release binaries. Normal weekly runs reuse their build artifact.
Once the workflow exists on the default branch, `workflow_dispatch` can override
`tag`, `baseline` and `run_id`.

For stronger protection, an administrator may move
`CLAUDE_CODE_OAUTH_TOKEN_LKLIMEK` into a protected GitHub Environment, remove the
repository/organization secret accessible to this repository, require a
maintainer's approval, and attach that Environment to the `review` job. This is
optional, not required by the workflow. Unlike a guard in PR-controlled workflow
code, Environment approval protects the secret even if a PR changes that guard.

The review job has `contents: read`, the `CLAUDE_CODE_OAUTH_TOKEN_LKLIMEK`
secret only on the Claude step, and no Slack/write secret. A separate report job
publishes a fixed verdict and artifact link. The existing weekly Slack job
includes that result. Neither the agent nor the standalone QA workflow publishes
or deletes a release.

The [release-review CI contract](../../.claude/skills/release-review/SKILL.md#ci-mode-gui-only)
defines scope limits, report categories, verdicts and publication eligibility.

Build or migration-fixture failure on a newly created tag deletes that tag and
its broken draft. Failed recuts retain their existing draft marked DO NOT PUBLISH.
QA, seal and finalize outcomes never trigger deletion. Retained drafts are never
pruned by later weekly runs; **a human must clean them up**.
A normal run skips an existing tag or unchanged source. To rebuild a retained
weekly, dispatch `weekly-build.yml` with `force=true` and its version: the tag
may move only if the existing release is still a draft bearing our weekly marker.
Before moving a recut tag, automation marks the existing draft **DO NOT PUBLISH**
while retaining its ownership marker. Successful build legs replace their assets;
a failed recut retains the draft and
may leave mixed old/new assets, so inspect a complete successful rebuild before
publishing. Published releases and unrelated tags cannot be recut. The seal job
writes the current weekly marker and a **DO NOT PUBLISH: QA pending**
notice before QA starts. Failed, cancelled or interrupted QA therefore retains an
owned, visibly unsafe draft. Finalization clears that notice only on PASS.
Cancellation before sealing still needs human inspection.
Use standalone QA to retest unchanged assets without recutting the tag.
The `<!-- weekly-version: … -->` marker remains unchanged.

### Burner wallet and boundaries

PRE generates one BIP39 phrase with Ubuntu's `python3-mnemonic` and pipes it
directly into the same release's `det-cli core-wallet-import --mnemonic-stdin`.
No phrase/env file is created; generated app `.env` files are removed before
Claude starts. Binaries run with an emptied environment, isolated HOME/data and BROWSER=/bin/true
to make external browser launches inert. Setup converts sudoers NOPASSWD grants to
PASSWD, invalidates cached authorization and refuses QA if passwordless sudo still
works. No later stage needs sudo. TODO: verify this on the first hosted dispatch;
the local offline checks do not exercise hosted sudoers policy.
The keyless wallet remains burnable and must never be used beyond this run.
The GUI can reveal a derived WIF or spend funds if the agent ignores its
instructions. Text/tree scanning blocks WIF exports, but screenshots have no
independent OCR scan. This residual is accepted for the low-value, at-most-1-tDASH
burner; never use mainnet or an operator wallet here.

Smoke launches an empty Testnet profile first and requires a graceful window close
with exit code 0. A timeout, forced termination or non-zero exit fails smoke.
The controller keeps the GUI stopped for PRE (wallet import and SPV-dependent receiving address), resumes it even if PRE
fails, and attempts HTTP-only funding when an address is available. Standalone
`det-cli` never runs against a live GUI.
Funding uses only `https://faucet.thepasta.org`: live `/api/status`, CAP PoW,
at most one payout/address/day, no faucet/IP rotation. Only the soft CAP tier is
automated; requiresHardCaptcha stops funding. The hard tier is browser-only.
Solving has a shared
540-second deadline measured before challenge retrieval, leaving a minute for
redemption within the 10-minute lifetime; expired solutions are never redeemed.
The testnet P2PKH refund
address comes from `depositAddress`, with optional repository variable
`QA_FAUCET_REFUND_ADDRESS` taking precedence when valid; an invalid override
falls back to a valid `depositAddress`. If neither is valid, POST is skipped
with a warning in the report and step summary. Funds are then abandoned with
the disposable testnet wallet. Funding is capped at 1 tDASH. Known funding refusal
is recorded while other reachable screens are still tested. An ambiguous payout is
never retried; it requires a refund attempt and can leave the result incomplete.

POST runs with `if: always()` after stopping the GUI. It validates testnet addresses,
then checks a single confirmed faucet UTXO against Insight's address balance
(up to three read-only attempts, one second apart, within 20 seconds). Only
Insight supplies these observations. `det-cli` is used solely for one Testnet send
of the balance minus 226 duffs back to the faucet, with the remaining 25-minute
step budget minus 15 seconds for the final state write. Nothing is retried after
the send starts. This allowance covers the current one-input P2PKH builder at 1 duff/byte; the sub-dust remainder is miner fee.
Unexpected inputs, fee/API drift, unconfirmed funds or an uncertain broadcast
produce INCOMPLETE without speculative sends. Refunds depend on testnet/SPV and
the Insight explorer; a lost runner or forced cancellation can prevent POST.
The ephemeral wallet is not uploaded and cannot be recovered after runner disposal.

Claude uses pinned CLI `2.1.331`, `--permission-mode default`, 600 turns and a
130-minute process timeout inside a 132-minute agent step and a 220-minute review
job. Setup (15 min), smoke (6), PRE (12), resume (5), funding (15), agent (132)
and POST (25) total 210 minutes, leaving 10 minutes for checkout/download,
initialization, pause/stop, finish and artifact upload. Graceful GUI close waits
70 seconds, beyond DET's 55-second shutdown deadline, with up to 10 seconds to
request close and 5 seconds to observe the SIGTERM fallback before SIGKILL.
A close timeout is INCOMPLETE, including a later exit caused by that fallback;
an observed crash or nonzero exit before fallback remains FAIL. The smoke step
has six minutes to accommodate startup and this longer close wait.
600 turns allow roughly 180 action/capture/read loops plus 60 turns for changelog
and reporting. The 130-minute timeout is the primary bound, not an expectation
that every turn will be used; reserve the final 20 minutes for evidence/reporting.
Measure actual turn usage in the first hosted dispatch before tuning this cap.
The mutually exclusive prepare jobs each have a 15-minute limit; report has
five minutes. Each QA path therefore totals at most four hours of job execution
(queue time excluded).
`--restricted` and an explicit PreToolUse hook permit public-input reads, report
writes and the fixed `qa-ui` helper only. The hook uses `onFailure: block`;
controller receipts, wallet status and smoke evidence are not agent-writable.
An empty working directory and explicit context/report/source directories avoid
repository startup instructions. Do not add `--bare`: it disables explicit hooks
and OAuth ([Claude automation documentation](https://code.claude.com/docs/en/headless)). No general shell, WebFetch, MCP servers or subagents.
Xvfb/lavapipe follow the render-spike setup; the helper provides xdotool/scrot
and AT-SPI labels/rectangles. Its fixed `insight addr ADDRESS` / `insight tx TXID`
commands allow read-only UI cross-checks against testnet Insight (GET only, no
redirects, five seconds, 1 MB); response text is untrusted data. Keyboard commands
activate the recorded DET window
then use XTEST (`key`/`type` without `--window`). GUI scenarios stop before any
broadcast or secret
export. These controls reduce exposure; they do not make public on-chain text
or an LLM verdict trustworthy by themselves.

### Reruns and verification

Dispatch `release-qa.yml` from `v1.0-dev` with `tag`, optional `baseline`, and
optional `run_id` of the build whose Linux zip should be reviewed. Without a
run ID, it downloads the release asset, including drafts. Other refs are skipped
and reported INCOMPLETE. **Re-run failed jobs** reuses stable input/report artifact names; successful
uploads replace the previous artifact for this run. A weekly rerun still obeys its tag/no-commits gate;
use a new forced manual dispatch when rebuilding an existing draft. Standalone reruns never publish; a maintainer can
publish the retained draft after inspecting the new report.

Download `release-qa-report` for `report.json`, Markdown, verified
changelog, scenarios/checklist, spotlight gallery, embedded visual changelog,
screenshots and GUI logs. `result.json` is the controller's effective verdict;
`report.json.summary` also records it. The controller validates report fields, gallery images and coverage of every
supplied non-merge commit. The report job also requires a successful finish step
and a complete export manifest whose file hashes match the downloaded artifacts.
Credential-like text (BIP39, extended private keys, Dash/Bitcoin mainnet/testnet
WIFs and Anthropic tokens) suppresses export; screenshots are checked by the agent.
Agent HTML writes are denied. All HTML is sanitized again at export: only passive
markup, local gallery image/anchor links and embedded PNGs survive. The controller
restores only the gallery's fixed CSS and UTF-8 declaration; agent CSS, scripts,
event handlers, frames and external links are removed. All text is scanned before
any file is copied, and a staged export becomes public only after completion.
Profiles, vaults, env files and raw Claude transcripts are never uploaded.

CI runs the Python suite in `release-qa-tests.yml` on release-QA script/workflow
changes and visual-diff/release-review skill changes, on pull requests and pushes
to `v*-dev`; it has no schedule. The same job runs the visual-diff selftest. Install Ubuntu `python3-yaml`, `python3-pil`,
`python3-numpy`, `python3-mnemonic` and `fonts-dejavu-core` for local checks; the secret tests use the real BIP39 library.
Offline checks (no faucet payouts or Claude usage):

```bash
python3 -m unittest discover -s scripts/ci/release-qa
bash scripts/ci/test-weekly-jobs.sh
actionlint .github/workflows/release-qa.yml .github/workflows/release-qa-tests.yml .github/workflows/weekly-build.yml .github/workflows/tests.yml
shellcheck scripts/ci/release-qa/setup.sh
```

The first hosted dispatch remains required to validate OAuth, AT-SPI, typed-input
delivery, turn usage, live sync,
funding/refund and artifact delivery together. The prior render spike validated
rendering only.

## How to run a scenario

Use a graphical session whose `DISPLAY` already points at the desktop you want
to observe. Do not assume a display number: local desktops, SSH forwarding,
CI, and headless X servers all use different values. Verify the selected
display before launching:

```bash
if [ -n "${DISPLAY:-}" ] && xdpyinfo >/dev/null 2>&1; then
  echo "Display $DISPLAY is reachable"
else
  echo "DISPLAY is unset or unreachable; set it to the desktop used for GUI testing and do not launch" >&2
fi
```

Do not launch the app if the check printed the error. Scenarios guard their own
launch line with the same test, so a pasted block skips the launch instead of
closing your terminal.

The check needs `xdpyinfo` (package `x11-utils` on Debian and Ubuntu). Without
it a working display reads as unreachable and every scenario skips its launch,
so confirm `command -v xdpyinfo` prints a path before you start.

On a minimal Ubuntu host, egui/eframe also needs an X keyboard library and a
wgpu backend. Install missing packages only after the launch log identifies
the corresponding failure:

```bash
sudo apt-get install -y libxkbcommon-x11-0 mesa-vulkan-drivers xdotool
```

Build in the checkout under test, then ask Cargo for the effective target
directory. This honors `CARGO_TARGET_DIR` and any `target-dir` configured in
Cargo's configuration files.

```bash
cargo build
TARGET_DIR=$(cargo metadata --format-version 1 --no-deps | \
  python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')
BIN="$TARGET_DIR/debug/dash-evo-tool"
test -x "$BIN"
```

Prepare isolated state and launch the binary as a detached process. The app
redirects some diagnostics into its data directory, so inspect both the launch
log and `det.log` (older builds also `det-stderr.log`) after a crash or panic.

```bash
DATADIR=$(mktemp -d)
cp .env.example "$DATADIR/.env"
LOG="$DATADIR/gui-test-launch.log"

pgrep -af dash-evo-tool
DASH_EVO_DATA_DIR="$DATADIR" nohup "$BIN" >"$LOG" 2>&1 &
APP_PID=$!

pgrep -af "$BIN"
WID=$(xdotool search --pid "$APP_PID" | head -1)
xdotool getwindowgeometry "$WID"
xdotool windowsize "$WID" 1260 780
xdotool windowactivate "$WID"
```

### Accessibility tree

Dash Evo Tool publishes its AccessKit tree over AT-SPI2 when accessibility is
enabled. A headless Ubuntu session needs the accessibility bus and Python
bindings:

```bash
sudo apt-get install -y at-spi2-core python3-pyatspi
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"

dbus-send --session --print-reply \
  --dest=org.a11y.Bus /org/a11y/bus org.a11y.Bus.GetAddress
gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus \
  --method org.freedesktop.DBus.Properties.Set \
  org.a11y.Status ScreenReaderEnabled '<true>'
gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus \
  --method org.freedesktop.DBus.Properties.Set \
  org.a11y.Status IsEnabled '<true>'
```

Add `DASH_EVO_TOOL_ACCESSIBILITY=1` to the launch command, focus the window
with `xdotool windowactivate "$WID"`, and inspect the tree with any AT-SPI
client. This self-contained Python example prints roles and labels:

```bash
python3 - <<'PY'
import pyatspi


def walk(node, depth=0):
    print(f"{'  ' * depth}{node.getRoleName()}: {node.name}")
    for child in node:
        walk(child, depth + 1)


for app in pyatspi.Registry.getDesktop(0):
    if app.name == "dash-evo-tool":
        walk(app)
PY
```

An empty application list normally means the AT-SPI status flags are still
disabled or the app window is not focused. The tree can lag during screen
transitions and omits purely decorative visuals, so use it for semantic labels
and structure while using screenshots for pixels, colors, and final visual
confirmation. Capture durable screenshots with `scrot -o <path>.png`.

The complete recipe is versioned here because an installed `desktop-gui`
automation skill is not available to every contributor. When present, that
skill may still provide convenient screenshot and input tooling.

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
   To start with the test wallet already imported, provision the data
   directory before launch with
   `.claude/skills/visual-diff/scripts/provision-wallet.sh` (the `det-cli` from
   the same release zip or commit as the GUI pipes the phrase in with
   `--mnemonic-stdin`).
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
   show a UI error — check `det.log` (older builds also `det-stderr.log`) in the test's data
   directory (or the default location if unset) for a Rust panic
   (`location=...` line) even when the UI appeared to work.

For annotated screenshots that show what changed in the UI between two builds (slides, release review), see the `visual-diff` skill in `.claude/skills/visual-diff/`.

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

## Procedure lessons for A/B campaigns

- Run only one build against Testnet at a time: endpoint rate limits are per
  IP, so two concurrent builds disturb each other's results.
- Take no screenshots or accessibility-tree dumps between opening a wallet
  import/creation form and leaving it; the recovery phrase may be on screen.
- Before planning Core-funded steps, check that the fixture wallet can really
  broadcast a Core transaction. If it cannot, use a temporary wallet funded
  from the fixture wallet via Send ▸ Platform Addresses ▸ "Withdraw to Wallet"
  (allow several minutes before the funds are spendable) and return the
  leftovers afterwards.
- A wallet created or imported in the running session cannot shield until the
  app is restarted; restart before any shielding step.
- When both builds must vote on one contest with the same node, the network
  may refuse the second identical vote; plan a second node or contest.
- The first balance right after an import can be partial until sync completes;
  read balances only after the sync has finished.

## Known UI/environment quirks

- **Default window is small (800×600) and clips controls** (sidebar items,
  settings sections below the fold). Resize immediately after launch with the
  `xdotool windowsize` command above. Some settings sections are collapsible
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
- **A shared Cargo target directory is not campaign-exclusive.** If other
  worktrees/sessions on the same box can rebuild concurrently, the binary under
  test can be silently overwritten mid-campaign by an unrelated build. For any
  run spanning hours, set `CARGO_TARGET_DIR` to a private path before building
  and hash-verify (`sha256sum`) before each relaunch.

## A/B build comparison contract

Every scenario in this library is run twice with the identical procedure: once
against the **baseline** binary and once against the **development** binary
selected for the current campaign. Both are roles, not specific versions —
record the two exact commit SHAs in that campaign's own artifacts, never in a
scenario file, so the library stays valid for the next campaign too.

**Judge by observation, not by expectation.** Record what each build actually
does at every step before classifying anything. Never assume which build is
"correct", which behavior a given change was meant to introduce, or which
direction a difference is supposed to point.

**Blocker rule** (a campaign may narrow this, never silently widen it):

- **Blocking**: the development build is worse than the baseline build from the
  user's perspective in the happy flow, or a **data-loss** outcome on either
  build.
- **Not blocking**: a purely cosmetic difference, or behavior reproduced
  identically on **both** builds (pre-existing — note it, don't flag it as a
  regression). Intermittency is not an exemption: a race that reproduces on one
  build and not the other is still a regression, and timing sensitivity only
  changes how many attempts a repro needs. Reproduce anything about to be
  marked blocking at least twice before confirming it.

**Equivalent fixture state per build.** Isolated data directories give the two
builds separate *local* state, but not separate *live-network* state. Any step
that spends a UTXO, consumes a deposit, registers a DPNS name, or moves credits
mutates shared on-chain state, so the second build's run would otherwise start
from whatever the first run left behind — an already-consumed deposit, a spent
UTXO set, a name that is no longer available — producing a false regression or
a false pass. Before any such step, give each build either its own
independently-funded, equivalent fixture (a separate wallet/identity per side)
or a restored snapshot of the same starting state, and record each side's
starting balances/deposits/identities/names first. Recording the difference and
accounting for it afterwards is **not** sufficient. Steps that only read state
(navigation, display checks) are unaffected.

## Scenario index

These scenarios are written **build-neutral**: the same procedure runs
unmodified against either binary of an A/B campaign, with the blocker rule and
the fixture requirement from the [A/B build comparison
contract](#ab-build-comparison-contract) above applied in each file's own
"Expected outcome" section.

| Scenario | What it verifies |
|---|---|
| [`identity-key-recovery-migration.md`](scenarios/identity-key-recovery-migration.md) | Key-placement resolution and legacy-key recovery (#941, #945, #946, #948): keys found/signed/restored/removed correctly regardless of internal filing convention; Keys screen reachable from every interface level; same-numbered-key collisions never cross-contaminate |
| [`wallet-max-send-asset-lock.md`](scenarios/wallet-max-send-asset-lock.md) | "Max" agrees with what the wallet can build/send across Shield/Fund-Platform-Address/Create-Identity/Top-Up/Transfer/Withdraw (#937, #927), including dust-wallet and rapid-UTXO-churn variants |
| [`dpns-registration-flow.md`](scenarios/dpns-registration-flow.md) | Usernames in Identities: list, request, availability and review, contested-vs-registered messaging, request status, make-main, and DashPay social-profile save feedback (#918, #901) |
| [`error-banners-identity-home-actions.md`](scenarios/error-banners-identity-home-actions.md) | Error-banner wording, validation consistency, identity-removal responsiveness, Identity Home's action row, already-consumed-deposit messaging, deposit-verification crash fix (#927, #934) |
| [`platform-shielded-availability-after-sync.md`](scenarios/platform-shielded-availability-after-sync.md) | Shielded features correctly activate right after SPV/platform sync instead of silently staying disabled (#936, #938) |
| [`dapi-budget-resilience-after-resync.md`](scenarios/dapi-budget-resilience-after-resync.md) | Repeated SPV `Syncing`↔`Synced` transitions no longer exhaust the shared DAPI request budget and break unrelated actions like identity top-up (#950) |
| [`startup-storage-gate-and-sync-cancel.md`](scenarios/startup-storage-gate-and-sync-cancel.md) | Cold-boot storage preparation raises exactly one blocking gate that always releases before chain sync starts, and cancelling the SPV overlay is a two-step choice no single keypress can turn into a disconnect (#970) |
| [`masternodes-votes-hub.md`](scenarios/masternodes-votes-hub.md) | Masternodes ▸ Votes hub: where contest voting lives, contest cards, staging, confirmation and scheduling, node detail, narrow-window fit, old-schedule startup notice (#901, #1052, #1053) |
| [`identity-add-funds-progress.md`](scenarios/identity-add-funds-progress.md) | Add funds progress dialog, background continuation, in-flight tracking across screens/network switches, and where the result is reported (#1054, #1058) |
| [`funding-minimums-and-fees.md`](scenarios/funding-minimums-and-fees.md) | Smallest accepted amount, displayed vs charged fee and existing-funding lists across Add funds, Create identity, Send and Shield-from-Core (#1059, #1063, #1064, #1065) |
| [`wallet-pickers.md`](scenarios/wallet-pickers.md) | Shared wallet selector: label/balance format, hover breakdown, unusable-wallet reasons, selection persistence on identity screens and the Wallets page (#1060, #1061) |
| [`spv-disconnect-and-import.md`](scenarios/spv-disconnect-and-import.md) | Manual Disconnect across network switches and during startup, and import-time identity discovery while connecting (#1055, #1056, #1057) |
| [`dates-and-light-mode.md`](scenarios/dates-and-light-mode.md) | Date/time zone and format across screens, and text readability in light vs dark theme (#1052, #1053) |
| [`migration-fixture-capture.md`](scenarios/migration-fixture-capture.md) | **Not an A/B comparison** (see its own "Deviation from the A/B contract" note) — the one-time recipe for capturing a real, GUI-produced data directory as a cross-version migration-test fixture (`tests/migration-fixtures/`) |

<sub>🤖 Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
