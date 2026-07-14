# PR892 User-Story QA Campaign — Shared Context

Read this in full before starting. You are one agent in a sequential chain working through
`docs/user-stories.md` category by category against a PR892 build. You are running
**fully unattended** — the user is AFK. Never stop to ask a question; if something needs a
human decision, write it into `progress.md`/your scenario file as BLOCKED with reasoning and
move on. **Never `git push`. Never use `ghsudo`. Never post to GitHub. Everything is
local-only.**

## Your job

1. Load the `desktop-gui` skill first (`Skill` tool).
2. Work through **only the stories in your assigned category/categories** (given in your
   task prompt) that are still unchecked `- [ ]` in `progress.md`. Skip anything already
   checked `- [x]`.
3. For each `[Implemented]` story: actually execute the flow end-to-end, not just navigate
   past the screen. Take evidence screenshots. Record steps, observed result, and a verdict:
   PASS / FAIL / BLOCKED (with specific reasoning) / N/A.
4. This is QA-only — observe and document, do NOT modify PR892's application source code
   (anything under `/data/git-worktrees/home-ubuntu-git-dash-evo-tool-2-pr892-build`). If you
   find a bug, document it; do not fix it.
5. Update `progress.md` (flip `- [ ]` to `- [x] ... — PASS/FAIL/BLOCKED(reason)/N-A`) and
   append to (or create) `scenarios/<CATEGORY>.md` for every story you touch, **as you go**
   — not just at the end. If you run low on your own turn/context budget mid-category, stop
   at a clean point with `progress.md` accurate for everything you've finished; a fresh
   agent will resume by reading it. Commit what you have before stopping.
6. Commit locally (never push) when your assigned work is done, with a descriptive message.

## Environment

- **Binary under test**: `/data/target/debug/dash-evo-tool`, built from PR892's head commit
  `57195d54` (worktree `/data/git-worktrees/home-ubuntu-git-dash-evo-tool-2-pr892-build` —
  do not touch its source; you should not need to rebuild).
- **Data dir (isolated, already has state)**: `DASH_EVO_DATA_DIR=/data/tmp/det-qa-pr892-data`.
  This machine also has a live, separate `~/.config/dash-evo-tool` in concurrent use by other
  unrelated work — never touch it, never launch without the `DASH_EVO_DATA_DIR` override.
- **Network: Testnet** (already selected/persisted in the data dir — confirm on launch, the
  sidebar network indicator at the bottom-left should read "Testnet"; if it somehow reverts
  to Mainnet, switch back via Settings > Networks, see `scenarios/NET.md` for the exact
  steps already validated).
- **Display**: `:99`. Accessibility env: `DASH_EVO_TOOL_ACCESSIBILITY=1`.
- **Is the app already running?** Check with `pgrep -af "target/debug/dash-evo-tool"` first.
  If yes, reuse it (find its window via `DISPLAY=:99 xdotool search --name "Dash Evo Tool"`,
  `windowactivate`, resize to `1260x780` with `xdotool windowsize` if it's still at the
  default 800×600 — the default is too small, many controls get cut off). If not running,
  launch it fresh:
  ```bash
  DISPLAY=:99 DASH_EVO_TOOL_ACCESSIBILITY=1 DASH_EVO_DATA_DIR=/data/tmp/det-qa-pr892-data \
    nohup /data/target/debug/dash-evo-tool >/tmp/app-run.log 2>&1 &
  disown
  ```
  **Do not `kill` the app between stories unless a story specifically requires a restart**
  (e.g. a future cold-boot check) — most stories should be tested against the already-running
  instance to save time. If you do need to restart, use `kill -TERM <pid>` (graceful), not
  `-9`.
- **a11y dump tool is unreliable on this box** (observed serving stale/mismatched trees
  across screen transitions in this session) — rely on `mcp__desktop__computer`
  `get_screenshot` + pixel coordinates as your primary navigation method. You may still try
  `python3 ~/.claude/skills/desktop-gui/a11y_dump.py dash-evo-tool` opportunistically, but
  cross-check against a screenshot before trusting it.
- **Evidence screenshots**: use `DISPLAY=:99 scrot -o <path>.png` to save real PNG files
  (the `mcp__desktop__computer` tool's inline images cannot be persisted to disk directly).
  Save to
  `docs/ai-design/2026-07-14-pr892-user-story-qa/scenarios/screenshots/<STORY-ID>-<n>-<short-desc>.png`.
- **Funding**: the primary wallet `QA Wallet 1` (Testnet) already has a balance from the
  Pasta testnet faucet (started with 3 tDASH; may be lower now if prior agents in this chain
  spent some — check the live balance in-app first). If you need more, use the
  `dash-platform:dash-faucet` skill (rate limit: 3 requests/hour from this box — if it
  refuses, that's expected, not a bug; interleave other non-funding-dependent stories while
  waiting rather than blocking). The skill's captcha-solving script may need to be
  reconstructed from `memory` (search recalls "Faucet Cap PoW solver") since `/data/tmp` is
  wiped on reboot — the memory file now includes a full working Python reference, just paste
  it to a file and run it.
- **Wallet mnemonic reference** (only if you need to re-import or verify — do not display
  this to yourself unnecessarily, it's already saved in the wallet):
  `evidence borrow mushroom garment expire sight man trip senior index strike unable toward
  solution grunt duty nuclear arctic tide muscle short super spoon orbit` (24 words, English,
  no password). A second, unrelated wallet also exists under **Mainnet** (not Testnet) named
  "QA Wallet 1" with mnemonic `weird mercy trophy slice system you dove tone moment column
  balance daring chest lesson figure outside silk weather swap say surround luggage surprise
  crazy` — irrelevant to your Testnet work, ignore it unless a story specifically needs a
  second wallet/cross-network scenario.
- **Crash logs**: `/data/tmp/det-qa-pr892-data/det.log` and `det-stderr.log`.

## ⚠️ KNOWN ENVIRONMENT BLOCKER: Testnet wallet-backend currently fails to connect

As of ~2026-07-14 19:10 UTC, **Testnet chain-sync/wallet-backend wiring fails on every launch**
in this data dir (`/data/tmp/det-qa-pr892-data`), ~50-100ms after SDK init, with
`Failed to start chain sync error=The wallet service could not complete this operation.`
Reproduced across 10+ full process restarts and via the in-app Settings > Networks reconnect
path. **Mainnet works fine in the same process** (full sync confirmed), so this is Testnet-
specific, not a general backend/network/resource problem. Full diagnostic history (including
two disproven hypotheses) is in `scenarios/ALK.md`'s "App-restart failure" section and its
addendum — **read that before spending time re-diagnosing**. Root cause not found; further
investigation needs either destructive DB access or a debug-instrumented rebuild, both
appropriately gated behind explicit human authorization (the permission system has already
correctly blocked two non-destructive-in-intent repair attempts).

**What this means for your work**: if you hit this (SPV sync failing on Testnet, wallet
balance stuck at 0/stale, "SPV sync failed" banners), **don't burn time re-diagnosing or
re-attempting fixes** — it's a known, open issue. For any story that strictly requires a live
Testnet wallet-backend connection (funding, sending, identity registration requiring a fresh
asset lock, anything that needs current chain state), mark it BLOCKED with reasoning
`"blocked by known environment issue: Testnet wallet-backend fails to connect in this data dir
as of 2026-07-14, see scenarios/ALK.md for full diagnosis"`. Still test whatever UI/validation/
navigation is reachable without live connectivity (forms render, empty states, client-side
validation, screens that only need cached/already-persisted DB state like `QA Wallet 1`'s
already-confirmed 2.99999288 DASH balance and transaction history, which read from local SQLite
and don't require an active SPV connection to display). If you have reason to believe the issue
might have self-resolved (e.g., significant wall-clock time has passed since the timestamp
above, or a prior agent in the chain notes it recovered), a single retry is reasonable — just
don't loop on it.

A harmless empty diagnostic wallet ("DIAG throwaway", Testnet, zero funds) was created during
this investigation and left in place — ignore it, it's not part of your test matrix.

## Known findings so far (don't re-discover/re-report these — just reference them if relevant)

- **PR892's regression fix is CONFIRMED WORKING** (WAL-016, full quit + cold-boot relaunch
  correctly re-renders transaction history). Already done, don't redo unless asked.
- **SND-003 (Receive Dash with QR code) is a confirmed FAIL** — the "Receive" button on the
  Wallet screen (Expert view) does nothing (no modal, no QR, no navigation). Already
  documented in `scenarios/SND.md`. If your category's testing touches this area again,
  you don't need to re-verify unless you want to check Default view specifically (noted as
  an open follow-up in the existing writeup).
- App defaults to **Expert view** currently (selected during initial setup) — sidebar has
  Identities/Masternodes/Contracts/Tokens/Wallets/Tools/Settings + Expert-toggle + Dash-logo
  external link. At the default small window size, Settings is below the fold — scroll the
  sidebar to reach it, or just resize the window as noted above.

## Docs to read before starting

- `docs/user-stories.md` (in this worktree) — the source of truth for acceptance criteria.
  Read only the entries for your assigned category/categories.
- `docs/ai-design/2026-07-14-pr892-user-story-qa/progress.md` — the live checklist, and your
  resumability checkpoint.
- `docs/ai-design/2026-07-14-pr892-user-story-qa/scenarios/WAL.md`,
  `scenarios/SND.md`, `scenarios/NET.md` — worked examples of the expected write-up format,
  depth, and tone (steps taken, observed result, verdict, screenshot references, UX notes
  called out separately from pass/fail verdicts).

## Ordering and known-infeasible cases (mark BLOCKED with this exact reasoning if you hit them)

- **DashPay two-party stories are self-testable, not blocked**: DPY-003/004/006/009/014 etc.
  — create a SECOND identity in the same wallet/app to act as the counterparty. Don't mark
  these BLOCKED for "needs another user."
- **MN-* / masternode ownership-dependent aspects** (surfaced via IDN-003, DEV-006): real
  registration needs ~1000 tDASH collateral the faucet won't provide at that scale. Check
  first whether a masternode/evonode identity fixture is already loadable in this environment
  (`memcan:recall` search, project `dash-evo-tool`) — if none, mark the ownership-dependent
  parts BLOCKED with this reasoning, but still test UI-only aspects reachable without
  ownership (empty states, "load by keys" form validation).
- **DEV-008 (mine blocks on Regtest)** and anything else Regtest-only: no regtest node is
  running here and standing one up is out of scope — mark BLOCKED with that reasoning.
- **NET-011 (wipe platform data), NET-019 (clear all local data), NET-020 (clear cached SPV
  data)**: destructive/state-resetting. Do **NOT** test these unless your task prompt
  explicitly tells you the destructive pass has started — they'd erase state earlier/other
  categories depend on. If your assignment is NET and these are still unchecked, leave them
  unchecked and note in your scenario file that they're deferred to the final destructive
  pass, don't test them yourself unless told otherwise.

## Style

Apply `/coding-best-practices` conventions to the documentation itself: clear, precise, no
fluff. Match the tone/depth of the existing `scenarios/*.md` files — enough detail that
someone who never touched the app can understand what was tested and trust the verdict, but
no padding.
