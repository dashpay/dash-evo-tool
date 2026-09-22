# PR860 QA Delta Campaign — Shared Context (2026-07-16)

You are running **fully unattended** — the user is AFK. Never stop to ask a question; if
something needs a human decision, write it into `progress.md` as BLOCKED/DECISION-NEEDED with
reasoning and move on. **Never `git push`. Never use `ghsudo`. Never modify application source
code — this is QA-only, document bugs, do not fix them** (fixes are dispatched separately by the
coordinator). **Never `git commit`** — just write/update files; the coordinator commits them.

## Why this is a "delta" campaign, not a from-scratch sweep

An exhaustive 175-story sweep already ran 2026-07-14/15 against PR892 (one of ~30 sub-PRs since
merged into this branch) and is fully written up at
`docs/ai-design/2026-07-14-pr892-user-story-qa/` (`progress.md` = per-story verdicts,
`summary-report.md` = the synthesized findings, `scenarios/*.md` = full repro detail per
category). **Read `summary-report.md` first, in full** — it is the baseline you are updating,
not something to redo blindly.

A diff of `docs/user-stories.md` between that sweep's commit and current HEAD found **exactly
one new story: WAL-032** (storage-update safety) — everything else in the catalog is unchanged
(same 175 IDs, same titles, same `[Implemented]`/`[Gap]` tags). So your job is NOT "test 175
stories from scratch." It is:

1. **Test WAL-032** (genuinely new, never tested).
2. **Re-verify every story the prior sweep marked FAIL** (34 stories) — confirm still reproducing
   at current HEAD. Most of these have precise root causes already identified (file:line) in
   `summary-report.md` — use those as your starting point, don't re-discover from scratch, just
   confirm the symptom still occurs live where you can reach it without heavy fixture setup.
3. **Re-verify BLOCKED stories that were blocked only by the fixed Testnet environment issue**
   (see summary-report.md's "Environment blocker" section) — that fix was data-dir-level, and the
   old data dir is gone, so you're starting fresh; note which of these you can newly unblock.
4. **Spot-check a broad sample of previously-PASS stories** (at least 2-3 per category, more for
   WAL/SND/IDN since wallet-secrets/SPV/migration code changed recently) for regressions — the
   5 commits `b11ab3ea..e6ba4857` fixed: an SPV start-flight race, advanced-send output/input
   overflow guards, network-label/duplicate-banner/sticky-sweep-banner bugs, dispatched-fetch
   correlation + token-refresh false-success, and an identity-resurrection race + seed zeroization.
   None obviously map 1:1 to a user story, so treat all of WAL/SND/NET/IDN as elevated regression
   risk and sample more heavily there.
5. **Do NOT re-run NET-011/NET-019** (destructive, wipes local data) — mark BLOCKED, "deferred to
   coordinator for explicit user authorization," same as the prior sweep's precedent.

## Environment

- **Binary**: build fresh yourself first via `cargo build --bin dash-evo-tool` from
  `/home/ubuntu/git/dash-evo-tool-2` (already on `docs/platform-wallet-migration-design` @
  `e6ba4857` or later — `git log -1` to confirm, do not proceed if it's behind that SHA), then
  copy `/data/target/debug/dash-evo-tool` to a private path (e.g. `/data/tmp/det-pr860-qa-bin/`)
  before launching — the shared `/data/target` path has previously been silently clobbered
  mid-campaign by concurrent builds (see the prior campaign's "Binary-provenance incident").
  Re-verify the sha256 of your private copy matches what you built before every relaunch if more
  than a few minutes have passed.
- **Data dir**: the prior campaign's funded data dir (`/data/tmp/det-qa-pr892-data`) no longer
  exists (cleaned up between sessions) — you're starting from an empty data dir:
  `DASH_EVO_DATA_DIR=/data/tmp/det-pr860-qa-data`. This means WAL/SND/IDN/DPN/DPY/TOK/DOC stories
  that need a funded identity/wallet will need fresh setup:
  1. Create a Testnet HD wallet in-app (WAL-001), note its receive address.
  2. Fund it via the `dash-platform:dash-faucet` skill (rate limit: 3 requests/hour from this
     box — interleave non-funding-dependent stories while waiting, don't block on it). The
     faucet's captcha-solver script may need reconstructing from `memcan:recall` (search "Faucet
     Cap PoW solver") since `/data/tmp` scripts from prior sessions may be gone.
  3. Once funded and synced, register 1-2 Testnet identities (IDN-001) to unlock the
     identity/DPNS/DashPay/token/document categories, same as the prior campaign did.
  This setup taxes real wall-clock time (faucet rate limit + SPV sync) — **do the zero-funding
  stories first** (see priority list below) while funding/sync happens in the background, then
  come back to funded-state stories once ready.
- **Display**: `:99` (confirmed free — no other GUI process running as of campaign start).
  Accessibility env: `DASH_EVO_TOOL_ACCESSIBILITY=1`. Load the `desktop-gui` skill for the full
  launch recipe, a11y tree usage, and defect-confirmation methodology (reproduce 3-5x, find a
  working sibling control, cross-check the log, read source-level gating) before starting.
- **Evidence screenshots**: `DISPLAY=:99 scrot -o <path>.png`, saved under
  `docs/ai-design/2026-07-16-pr860-qa-delta/scenarios/screenshots/`.
- **Crash logs**: `<data-dir>/det.log` and `det-stderr.log`.
- **det-cli** (for MCP-001 retest): build via `cargo build --bin det-cli --features cli`, then
  run standalone per the project CLAUDE.md's "Smoke-testing changes with det-cli" section
  (`MCP_API_KEY` unset, point `DASH_EVO_DATA_DIR` at a throwaway dir).

## Priority order (zero-funding stories first — do these regardless of faucet/sync state)

1. **WAL-032** (new story) — needs an old-schema wallet DB fixture to exercise the actual
   migration path; if none exists, do what's reachable (fresh-install path, confirm no crash) and
   mark the rest BLOCKED "no legacy-schema fixture available," noting what source review shows.
2. **WAL-005** (Rename wallet — prior verdict FAIL, inert button)
3. **WAL-006** (Lock/unlock wallet — prior verdict FAIL, self-lockout)
4. **WAL-007** (Remove wallet — prior verdict FAIL, missing confirmation for single-key)
5. **SND-003** (Receive Dash QR — prior verdict FAIL, inert button)
6. **UX-003** (Global switcher — prior verdict FAIL, missing on Contracts/Tokens/Tools/Settings)
7. **MCP-001** (CLI wallet visibility — prior verdict FAIL, via det-cli, no GUI/funding needed)
8. **NET-004/005/006** (theme/interface-mode — prior PASS, quick regression spot-check)
9. Then proceed to funded-state stories as the wallet syncs/funds, prioritizing: WAL-016/017/018
   (recently-touched wallet code), SND-001/005/014 (recently-touched send code), IDN-001/002/016
   (recently-touched migration/identity code), then the rest of the FAIL list (DOC-002/004,
   TOK-003/005/011/018, IDN-006/008/009/013a, ALK-002, MN-001), then PASS spot-checks for
   DPN/DPY/TOK/DOC/IDH/MN/DEV.

## Checkpointing

Create/update `docs/ai-design/2026-07-16-pr860-qa-delta/progress.md` (same format as the prior
campaign's `progress.md` — one line per retested story, verdict + short reasoning) **as you go**,
plus a `docs/ai-design/2026-07-16-pr860-qa-delta/summary-report.md` you keep current with: overall
tally, FAIL list (each item: still reproduces? new fix landed? note anything that changed vs the
2026-07-15 baseline), and a running "queued decisions" section for anything needing the user's
product/UX call (destructive-action tests, ambiguous by-design-vs-bug calls). If you run low on
turn budget, stop at a clean checkpoint — do not leave `progress.md` inconsistent with what you've
actually verified.

## Style

Match the tone/depth of `docs/ai-design/2026-07-14-pr892-user-story-qa/scenarios/WAL.md` and
`NET.md` — precise, evidence-backed, no padding. Apply `/coding-best-practices` conventions to the
documentation itself.
