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

## Scenario index

| Scenario | What it verifies |
|---|---|
| _(none yet — first one lands once the masternode-withdrawal-without-a-wallet run is verified)_ | |

<sub>🤖 Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
