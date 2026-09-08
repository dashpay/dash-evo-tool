---
name: release-review
description: "This skill should be used when preparing to QA a dash-evo-tool nightly/weekly build before it ships — comparing the last published pre-release against the current development-branch HEAD via a verified changelog, a scoped code review, and a true A/B GUI regression pass with a visual changelog."
---

# Release Review

End-to-end nightly-release QA campaign for dash-evo-tool: find what actually changed since the
last shipped pre-release, run a scoped code review, and prove the GUI isn't worse than before in
the happy flow. Produces a triageable findings report plus a visual (screenshot) changelog a human
can eyeball in minutes.

This procedure assumes nothing about the machine it runs on beyond a working Rust toolchain, `git`,
and the `gh` CLI, and nothing about which Claude Code capabilities are installed beyond the
built-in ones (Bash, file tools, and — if available — the ability to spawn sub-agents for
independent work). Anywhere a specialized skill/plugin would make a step faster or more rigorous,
it's named as an optional example in parentheses, never as a hard requirement — read "if you have
access to X" as "otherwise do the equivalent by hand."

**Pick a scratch root before starting** and reuse it throughout — a directory outside the repo's
own working tree that this run can write to freely (a fresh `mktemp -d`, or a fixed directory you
control). Every path below written as `<scratch-root>/...` means "somewhere under that root," not a
literal path — don't hardcode one run's actual path into anything durable (docs, other agents'
instructions, the final report).

## Rerunning after a fix lands (or a pause)

The trigger is usually "a fix/PR landed, redo the campaign against the new HEAD" or resuming after
a § Handling a Fixture/Infra Blocker pause. Don't rebuild from scratch:

1. **Re-verify the baseline is still current** (`gh release list --repo dashpay/dash-evo-tool` — a
   newer pre-release may have shipped since the prior run). A matching tag *name* is not enough:
   re-resolve the canonical peeled tag commit
   (`git ls-remote <canonical-remote> "refs/tags/<tag>^{}"`) and compare it against the baseline SHA
   recorded in this run's `qa-run-state.md`. Only when both the tag name and that SHA are unchanged
   is the prior run's worktree/binary for it still valid — check for an existing `git worktree` and a
   previously-built binary under your scratch root before rebuilding anything. Fail closed on a
   mismatch, and equally when no baseline SHA was recorded: re-establish the baseline per Phase 1 and
   rebuild its binary rather than reusing an artifact you can't prove.
2. **Only rebuild what moved.** If the baseline is unchanged *by that SHA comparison* and only HEAD
   advanced, reuse the baseline binary as-is; `git fetch` + `git checkout <new-sha>` the HEAD
   worktree in place and rebuild only that binary. This turns a two-cold-build Phase 2b into one
   incremental build.
3. **Archive, don't overwrite, the prior run's output.** Before writing fresh Phase 1-5 artifacts,
   move the previous run's files into a subfolder (e.g. `run-vs-<old-head-short-sha>/`) under the
   same artifacts location. Both runs stay comparable/auditable, and nothing silently clobbers a
   finding a human might still want (e.g. a confirmed-fixed crash repro).
4. **Reuse scenario files and prior findings as a starting point, not from scratch.** A changelog
   pass can diff the prior run's verified commit list against the new range instead of re-deriving
   the whole thing; a GUI pass can decide per-scenario whether the flow is unchanged enough to reuse
   the prior run's OLD-side (baseline) screenshots rather than recapturing them — a baseline binary
   unchanged by step 1's peeled-SHA comparison (not merely by tag name) still has valid screenshots.
   Always capture fresh NEW-side screenshots (HEAD moved), and log explicitly which screenshots were
   reused vs freshly captured so it's auditable rather than assumed.
5. If any part of this run is delegated (to a sub-agent or a fresh session), give it the prior run's
   key facts (SHAs, decisions already made, artifact paths) explicitly — it has no memory of a prior
   or paused run and will otherwise re-derive, and burn time on, things already verified.

## Phase 0 — Fixture preflight (do this FIRST, cheaply)

Before committing to a real GUI run, verify the end-to-end test fixtures are actually usable *now*
— testnet masternodes deregister and test wallets drain between runs.

**These fixture variable names are a scenario-doc naming convention, not app config the GUI reads
automatically.** `.env.example` does not define them, and the GUI binary itself doesn't consume any
of them directly — confirm this hasn't changed before relying on it (`grep -rn "E2E_" src/` from a
repo checkout; as of this writing it returns nothing). They exist purely so scenario files and
operators refer to the same fixture by the same name, matching the naming already used by
`tests/backend-e2e/` (which *does* read `E2E_WALLET_MNEMONIC` for its own harness, but that's a
separate test binary, not this GUI). As of this writing, the scenarios under
`docs/gui-testing/scenarios/` use:

- `E2E_MN_PROTX_HASH`: confirm the value you intend to use is a currently-registered
  masternode/evonode before relying on it in a scenario — a stale hash surfaces as a typed
  `MasternodeNotFound` error the first time something touches it in the GUI. Check via an SDK/CLI
  lookup or a quick platform-explorer query first, since nothing enforces this automatically.
- `E2E_WALLET_MNEMONIC`: confirm the wallet is actually funded on testnet *right now* (a live balance
  check, not "it was funded last time"). If empty, get a faucet drop (a testnet faucet — web, CLI,
  or whatever this project's e2e docs point at) or a fresh funded fixture BEFORE starting Phase 4 —
  don't discover this after a long SPV sync.
- `E2E_IDENTITY_ID`: the identity the top-up check in
  `docs/gui-testing/scenarios/dapi-budget-resilience-after-resync.md` funds. Preflight it with a live
  identity lookup (does the ID still resolve, and can it receive a top-up?), or decide explicitly and
  up front that provisioning will create the identity ad hoc — that costs credits and GUI time, so
  it's a decision to make before Phase 4, not to discover mid-scenario.

If any of these fixtures is broken and can't be fixed quickly: stop here, log it (see § Handling a
Fixture/Infra Blocker below), and don't burn hours running non-fund-dependent checks only — check
with the user whether a partial (navigation-only) pass is worth doing now or the whole campaign
should wait.

**A fresh data directory does not start in a usable state for these scenarios — provisioning it is a
real, non-optional GUI setup sequence, not a formality.** Confirmed: a fresh install's default
settings select **Mainnet**, not Testnet, and a fresh data directory has no imported wallet, no
identities, and no masternode linkage. Before running any scenario against a freshly-seeded data
directory, drive the GUI itself through: selecting Testnet in the network selector, importing/
restoring the wallet from the mnemonic fixture, discovering or importing the identities the scenario
needs, and — if the scenario needs it — linking a masternode or seeding legacy-migration state. Do
this explicitly and record it as setup, not as part of the scenario's own measured procedure.

**Confirmed false-alarm trap**: precisely because the network selector has no reliable default,
double-check it's actually on Testnet before concluding a fixture is dead — a prior run spent real
time diagnosing a "dead masternode" and "unfunded wallet" that were both fine; the app was just
sitting on Mainnet the whole time. Check the network selector FIRST, before treating a zero balance
or a `MasternodeNotFound` as a real fixture problem.

## Phase 1 — Baseline identification + verified changelog

1. **Resolve the canonical remote before running any remote-qualified git command.** In a fork-based
   checkout, `origin` commonly points at a contributor's fork, not the canonical repo — using it
   unconditionally can silently sync tags/branches from the wrong place with no error (confirmed: in
   one checkout, `origin` resolved the development branch to a materially different commit than the
   canonical repo did). Find the remote whose *fetch URL* is exactly the canonical repo: read the URL
   itself (`git remote get-url <remote>` per remote, or the `(fetch)` line of `git remote -v`) and
   require an exact match on `github.com/dashpay/dash-evo-tool` — HTTPS or SSH form, with or without
   the `.git` suffix. A loose `git remote -v | grep dashpay/dash-evo-tool` is not that check: it also
   matches a push-only line and any fork whose URL merely contains the string. If no remote matches,
   add a **named** one (`git remote add upstream https://github.com/dashpay/dash-evo-tool.git`) and
   fetch through that name — never fetch directly by bare URL, since step 4 resolves
   `<canonical-remote>/<base-branch>` and a URL leaves no remote-tracking ref to rev-parse.
   Call this `<canonical-remote>` below and use it consistently for every step in this skill — never
   assume `origin` is it, and don't re-derive it more than once per run.
2. **Find the baseline release**: `gh release list --repo dashpay/dash-evo-tool` filtered to
   `draft:false && prerelease:true`, most recent. Automated builds are commonly cut as drafts first
   and can be deleted before undrafting (check `.github/workflows/` for the exact release-cutting
   workflow and tag-naming scheme currently in use) — a newer *local* tag with no matching GitHub
   release is not a valid baseline.
3. **Force-sync the tag from `<canonical-remote>` before trusting it**:
   `git fetch <canonical-remote> "+refs/tags/<tag>:refs/tags/<tag>"`, then
   `git rev-parse <tag>^{commit}`. **Confirmed gotcha**: a local tag ref can silently diverge from the
   remote (stale from an earlier fetch/session) and point at an *ancestor* commit, producing a diff
   range that's wrong (larger than reality) without any error. Always re-verify against the remote
   before computing any diff range from a tag — and compare peeled commit to peeled commit:
   `git ls-remote <canonical-remote> "refs/tags/<tag>^{}"` returns the commit a tag points at, whereas
   `git ls-remote --tags <canonical-remote> <tag>` returns the *tag object* SHA for an annotated tag.
   This project's release tags are annotated, so comparing the local peeled commit against the
   unpeeled remote line mismatches on every healthy tag — that mismatch is an artifact of the wrong
   comparison, not evidence of drift. (If the remote prints no `^{}` line the tag is lightweight and
   its single SHA is already the commit. To check both forms explicitly, compare tag object against
   `git rev-parse <tag>` and peeled commit against `git rev-parse <tag>^{commit}` as two separate
   assertions.)
4. **Determine the active development branch and resolve it to a commit SHA — record that SHA, don't
   carry a branch name or bare `HEAD` forward.** Check the repo's own contribution docs (e.g.
   `CLAUDE.md`, `CONTRIBUTING.md`) rather than assuming `main`/`master`; this project may use a
   dedicated long-lived dev branch instead. Fetch that branch before resolving it — Phase 1's other
   fetch syncs tags only, and a stale remote-tracking ref freezes a wrong development SHA into every
   later phase with no error (confirmed: a checkout's `<canonical-remote>/<base-branch>` sat behind
   the canonical head that `git ls-remote` reported). Resolve it once:
   `git fetch <canonical-remote> "+refs/heads/<base-branch>:refs/remotes/<canonical-remote>/<base-branch>"`
   then `git rev-parse <canonical-remote>/<base-branch>`, and treat that output as
   **the development SHA** for the rest of this run — every later phase (the diff range, Phase 2b's
   second worktree, screenshots, the final report) uses this exact SHA. Never substitute a bare
   `HEAD` for it (that's the invoking checkout's HEAD, not necessarily the development branch's —
   wrong whenever this procedure is run from a feature/PR checkout, which is common) and never
   re-resolve the branch ref later in the run, since it can move while you're still working. Compute
   the range:
   `git log <tag>..<development-sha> --oneline`.
5. **Don't trust `CHANGELOG.md` alone.** Its unreleased section can contain stale/premature entries
   or miss real changes. For every commit in range, resolve its PR number and read the actual PR body
   (`gh pr view <n> --repo dashpay/dash-evo-tool`) — not just the commit subject, which can undersell
   what a PR really changed (confirmed: one PR's CHANGELOG entry omitted two of its own later
   review-round fixes). Cross-check both directions: CHANGELOG claims with no backing PR, and merged
   PRs with no CHANGELOG entry.
6. **Confirm things are actually in-range, not pre-existing**: `git merge-base --is-ancestor <sha>
   <baseline-tag>` for anything you suspect might already be shipped — don't regression-test
   pre-existing behavior as if it were new. This one check is necessary but not sufficient on its
   own — see the fuller treatment in the blocker-classification section of Phase 4, which this same
   caveat also applies to.
7. Produce a verified changelog grouped by theme, each entry citing commit SHA + PR number, tagged
   user-visible (GUI-relevant) vs internal-only (tests/CI/deps), plus a regression risk map by
   functional area. Save under your artifacts location for this run (see § Artifact Conventions).

## Phase 2 — Scoped code review

Review the diff `<baseline-tag>...<development-sha>` (three-dot range, using the exact SHA resolved
in Phase 1) thoroughly: security,
project/structural consistency, code quality, dependency risk, documentation accuracy. Run it from
an isolated worktree, never the invoking session's active checkout — a plain `git diff`/`git log`
between two refs doesn't need a checkout of either at all, but reviewing from a clean worktree
avoids any risk of touching the human's working tree. (If a multi-agent code-review skill/plugin is
available — e.g. one that fans reviewers out by dimension and file group — use it; otherwise cover
the same dimensions yourself, splitting the work across multiple passes for a large diff rather than
one shot.) Archive the resulting findings report to your artifacts location immediately if it was
produced somewhere scratch/ephemeral.

**Scale for the actual diff size.** A genuinely large range (many dozens of files, tens of thousands
of changed lines) deserves a proportionally wider review — more reviewers/passes across more file
groups, not the default handful. If findings are emitted with a file:line `location`, make sure it's
**repo-relative, not an absolute worktree path** — an absolute path like
`/some/scratch/worktree/src/foo.rs:12` breaks any downstream permalink generation and is hard to
read out of context. Also check the diff for review-comment IDs (from a *previous* review round)
that leaked into committed code or test comments — a real, recurring defect class: a prior review's
transient ID (e.g. referencing an old finding number) ending up quoted in a doc comment as if it were
permanent documentation.

**Severity scores are not negotiable by request.** If you ask a reviewer (human or agent) to
reclassify or re-score a finding, that request is not itself evidence. A reviewer that declines to
inflate a severity score to match your expectation and instead re-derives it honestly — even landing
lower than you expected — is doing its job correctly. Don't push back without a substantive
argument for the new score.

## Phase 2b — Build both binaries for A/B comparison

Create two isolated `git worktree`s under your scratch root — one at the baseline tag's commit, one
at **the development SHA resolved in Phase 1** (never a bare `HEAD` — if this procedure is running
from a feature/PR checkout, `HEAD` there is that checkout, not the development branch you diffed and
wrote the changelog against) — never inside the repo's own tracked working tree, and never let a
spawned sub-agent create worktrees on its own initiative if you're coordinating multiple agents
(create them yourself first, hand the paths out). Build each with:

```shell
CARGO_TARGET_DIR=<scratch-root>/<name>-target cargo build --bin dash-evo-tool --manifest-path <worktree>/Cargo.toml
```

Distinct `CARGO_TARGET_DIR`s are mandatory — both binaries must coexist afterward for the A/B run;
sharing a target dir means whichever built last silently overwrites the other's binary at the same
output path. Always pass `--manifest-path` explicitly (worktree auto-discovery can resolve to the
wrong checkout). Build sequentially if the machine is RAM-constrained (two full dependency-graph
cold builds concurrently risks OOM/thrashing) — check available memory / existing load first.

**Confirmed gotcha, background builds only, both Linux and macOS**: a `cargo build` backgrounded
(`... &`, `nohup`, or similar) from an automated shell can fail with a "cargo: No such file or
directory"-style error even though `cargo` works fine in a normal foreground call — a backgrounded
subshell doesn't always inherit the interactive profile that puts `cargo` on `PATH` (common when
installed via `rustup` under `~/.cargo/bin`). Capture the resolved path with `command -v cargo` (or
`rustup which cargo`) *while it's confirmed working*, and use that absolute path for any backgrounded
build invocation rather than the bare `cargo` name.

## Phase 3 — Author GUI regression scenarios

Informed by Phase 1's risk map, write **version-agnostic** scenario files under
`docs/gui-testing/scenarios/` (from that directory's `TEMPLATE.md`) — the same procedure runs
against both binaries, so never phrase a step as "confirm the fix works." Bake the blocker rule (§
below) directly into each scenario's "Expected outcome" section. Update the "Scenario index" table
in `docs/gui-testing/README.md`.

Also produce a risk-prioritized `docs/user-stories.md` checklist, two tiers:
- **Deep tier**: full acceptance-criteria comparison (old vs new) for every story plausibly touched
  by the diff.
- **Smoke tier**: everything else — screen opens, no crash, no error in logs — toured in one
  continuous session per binary rather than relaunching per story ID.

Save the checklist under your artifacts location (a QA-campaign planning doc, not permanent product
documentation — don't fold it into the tracked repo).

If a story text is found stale against shipped behavior while cross-referencing (confirmed: found
one describing a pre-redesign button layout weeks after the redesign shipped), flag it as a doc
defect for the final report — don't silently fix it mid-campaign, that's scope creep.

## Phase 4 — GUI A/B run

Drive the actual compiled binary through a real rendered window, twice (once per build) — how you
reach a real window differs by OS, so confirm before assuming:

- **Any headless/server Linux environment**: check whether `$DISPLAY` (or `$WAYLAND_DISPLAY`) is
  already set to something — if the session was reached over SSH with X11 forwarding, that ambient
  value points at the *human's own screen*, not a safe throwaway one. **Verify this before every GUI
  launch** — launching against the wrong display pops the app window on someone's real desktop. If
  no safe display exists, stand up a virtual one (e.g. `Xvfb`) and point `DISPLAY` at it explicitly
  for every launch — don't rely on an inherited/ambient value.
- **macOS**: normally has a real display attached already; if running headless/remote (SSH, CI
  runner), GUI automation typically needs Screen Recording/Accessibility permission granted to the
  driving process, or a remote-desktop/VNC session to attach to. There is no X-server equivalent to
  spin up standalone the way `Xvfb` works on Linux.
- Whatever the mechanism, **isolate a scratch data directory per launch** (`mktemp -d`, portable on
  both Linux and macOS) and point the app's data-dir env var at it — never reuse or mutate a real
  user's data directory.

For each scenario/story check: confirm no conflicting instance of the app is already running
(process list, e.g. `pgrep -f dash-evo-tool` or `ps aux | grep dash-evo-tool` if `pgrep` isn't
available) before launching. Launch **old**, perform the flow, screenshot, fully close (confirm
process exit); launch **new**, identical flow, screenshot, close. Never run both simultaneously —
one real window/display target at a time, and it keeps screenshot provenance unambiguous. Check the
app's log output for both runs (with an isolated data directory, verify where logs actually landed
for this run rather than assuming a fixed default path).

**Isolated data directories give the two builds separate local state, but not separate live-network
state — plan for that separately.** A scenario that spends a UTXO, consumes a deposit, registers a
DPNS name, or moves credits mutates shared on-chain/live-network fixture state; running the baseline
binary through it and then the development binary through the identical flow means the second run
starts from state the first run already changed (an already-consumed deposit, a spent UTXO set, a
name that's no longer available), which can produce a false regression or a false pass. For any
scenario step that mutates fund-moving or name-registering state, give each build its own
independently-funded/equivalent fixture — a separate wallet, identity, or name per side, or a fixture
restored to an equivalent starting state between the two runs — rather than sharing one. Recording
each side's starting balances/UTXOs/deposits/identities/DPNS names and "accounting for the
difference" afterwards is not an alternative: you cannot account your way past a DPNS name that no
longer exists or a deposit the first run already consumed, because the second build can't execute the
flow at all. Steps that only read state (navigation, display checks) aren't affected — reserve this
for anything that actually spends or registers something.

**Blocker rule** (confirm with the user for each run — the default below is a starting point, not a
universal constant):
- **Blocking**: new version worse than old from the user's perspective **in the happy flow**, or a
  **data-loss** scenario.
- **Not blocking**: timing noise with no user-visible effect; anything reproduced identically on
  **both** builds (pre-existing — note it, don't flag as a regression). Read that exemption
  narrowly — a *new* intermittent race stays blocking whenever it breaks the happy flow, crashes the
  app, or loses data. "It's a race" never downgrades those; intermittency only means you reproduce it
  more times before confirming.
- Reproduce anything about to be marked blocking at least twice before confirming.
- If a repro attempt genuinely can't be reproduced after several tries, don't leave it open
  indefinitely — write up the negative evidence, downgrade to backlog, and say explicitly that repro
  was attempted and failed, rather than silently dropping it or mandating another attempt on every
  future rerun.

**Blocking classification must say pre-existing vs diff-introduced, not just "blocking."** A defect
can be blocking-worthy on its own severity (real data loss) while NOT being a regression this diff
caused. But don't let a single check decide this either way: `git log <baseline>..<development-sha>
-- <suspect-file>` returning zero commits proves only that the file wasn't *directly* edited — a
changed caller, a shared model type, a persisted-data format change, a feature gate, or a bumped
dependency can still cause a regression that surfaces in an otherwise-untouched file. Treat old-vs-new
reproduction as the primary evidence (does the exact same trigger actually behave differently on the
two builds?), and use the git-log check as one corroborating signal, not proof by itself — if the
file is untouched but you suspect the behavior still changed, trace the transitive callers/
dependencies of the code path before concluding "pre-existing." Frame the finding as "we are choosing
to hold/flag a pre-existing defect," not "this release broke X" — the second is a factual claim that
can be wrong and, if wrong, undermines the whole report's credibility.

Emit confirmed findings in whatever findings format the rest of this campaign's report uses (see
Phase 2/5) — at minimum: title, severity, a location reference (a file:line if one applies, otherwise
a synthetic description like "GUI: `<scenario>` > `<screen>`"), description, recommendation, and
whether it's blocking per the rule above.

**If findings get consolidated/renumbered by any downstream tooling, treat IDs as provisional and
don't embed them anywhere durable.** A findings pipeline may reassign IDs when merging sections
(confirmed: GUI-section IDs were renumbered twice across two merge passes in one run). Never
reference a provisional or even a "final" ID in a scenario file, the visual changelog, or an
inter-agent message meant to outlive one exchange — reference findings by **title** or **scenario
name** instead, which survive renumbering. Warn whoever's writing durable artifacts about this
explicitly before they start, not after — catching and fixing already-embedded IDs later is possible
but wasteful.

## Escalating a claim that can't be settled from the repo alone

A review pass occasionally surfaces something it explicitly flags as unresolved from static analysis
— e.g. "does the live network actually behave the way I think the code implies?" Don't ship that as
a confirmed blocking finding, and don't dismiss it either: investigate it directly and narrowly
rather than waiting for a human to resolve it, if it's the kind of thing investigable from evidence
(read the actual pinned dependency's source, trace the real call order, and — if it's the only way to
get ground truth — make a strictly read-only live check, e.g. a version/status query against the
live network, never a transaction). Confirmed valuable: a claim about a possible mainnet outage was
fully resolved via a few read-only queries against live mainnet nodes in well under an hour — cheap
compared to either shipping a false alarm or silently dropping a real risk. When the verdict lands,
fold the correction into the report explicitly — state that an earlier pass got something wrong and
why, rather than quietly replacing it. A report that visibly corrects itself is more trustworthy than
one that never admits an error.

## Provenance: unrelated concurrent work on a shared environment

If this campaign runs on a machine or account that may have other unrelated work in flight (other
worktrees, other sessions, other automation touching the same repo), a QA pass may stumble on
evidence of a *different*, unaffiliated effort — a report file, a branch, a running build — that
looks relevant to something this campaign is investigating. Before treating it as this campaign's
own finding:
1. **Verify it's actually external**, don't assume: check for a still-live process tied to it, a
   moving `git log` HEAD in its worktree, a different session/transcript identity, or simply that
   nothing in this campaign's own roster claims authorship.
2. **If external, exclude it entirely from the formal report** — don't cite, reference, or fold in
   findings/fixes from work with no chain of custody to this campaign, even if independently verified
   as genuine. It may be unstable (still evolving), unauthorized for this release, or owned by
   someone who hasn't consented to having their in-flight work published in your report.
3. **Do surface its existence to the user as a side note** when reporting results — it's relevant
   context for a ship decision even though it's not your finding.
4. **Leak-scan the final artifacts** before declaring done: search the produced report/HTML for any
   branch name, commit hash, worktree path, or distinctive identifier from the external work, to
   confirm the exclusion actually held rather than assuming it did.

If you're confident this run is the only thing touching the repo/environment, this section doesn't
apply — but check rather than assume on a shared machine.

## Security incidents during GUI testing

**Prevent first — every control below is cheaper than the incident response after it.** When a run
handles a real secret (a testnet mnemonic, a password fixture):
- Enter it through the GUI's own masked/protected input wherever one exists, rather than any path
  that echoes the value back on screen.
- **Never pass a real mnemonic, password, or private key as a bare command-line argument** — it lands
  in process listings, shell history, and this session's transcript, all of which outlive the run.
  Pass it via an environment variable the app reads, or a file with restrictive permissions.
- Don't dump the environment (`env`, `printenv`, `set`) into a transcript, and redact fixture secrets
  out of any log output before quoting it; check what a raised log level actually prints before
  capturing it wholesale.
- **Check before saving or embedding, not after**: eyeball each screenshot for visible secret text,
  and grep transcripts, logs, and generated artifacts for the fixture's own patterns (the mnemonic's
  words, the password string, `xprv`/`tprv`/WIF prefixes) before they reach the artifacts location.
  Repeat that check immediately before Phase 5's base64-inline step — inlining bakes the image into a
  self-contained, trivially forwardable HTML file that is far harder to un-publish than a stray PNG.
- Restrict filesystem permissions where secrets and captures land (`chmod 700` the scratch/artifacts
  directories, `chmod 600` any file holding a secret) — on the machine they actually land on.

If GUI testing handles real secrets (a testnet mnemonic, a password fixture) and something goes
wrong — a screenshot with the secret visible, a secret typed into a visible log/transcript — the
self-report is not the end of it:
1. **Independently verify remediation** — don't just trust "I deleted it." Check the files are
   actually gone, check adjacent files/screenshots for spillover, confirm no other copy exists.
2. **Write an incident note** into this run's artifacts (what happened, exposure window, what was
   verified, actual risk given the context — testnet vs mainnet, network-restricted vs public,
   closed window vs ongoing).
3. **Don't unilaterally rotate a shared fixture.** A fixture used beyond just this campaign (e.g. by
   other test suites in the repo) shouldn't be silently burned and replaced — queue the
   rotate-or-accept decision for the user.
4. **Never screenshot a populated secret-entry field going forward** — capture before typing or
   after the field is cleared/masked, for the rest of the run.
5. Log the incident somewhere that survives this session ending (an issue tracker entry, a durable
   notes/memory mechanism if one is available, or at minimum a clearly-named file in this run's
   artifacts) so it isn't lost if the user doesn't act on it immediately.

## Phase 5 — Visual changelog + consolidated report

Build a self-contained HTML page (`visual-changelog.html`) presenting old/new screenshot pairs side
by side per scenario/story, with a verdict badge and a one-line rationale each — inline the images
(e.g. base64) so the page has no external file dependencies. Run the pre-embedding secret check from
§ Security incidents during GUI testing over every image and caption first: once inlined, a leaked
secret travels with every copy of the page. Save it to your artifacts location.
Merge Phase 4's findings into Phase 2's report as an additional section, re-validate the combined
report against whatever schema/format it uses, and re-render any human-readable version. The
combined report is then ready to hand to the user for triage — code-review and GUI-regression
findings reviewed together in one pass. (If an interactive triage tool is available, use it;
otherwise a written summary the user can respond to works fine.)

## Handling a fixture/infra blocker

If Phase 4 hits a real environment blocker (dead fixture, unreachable network, broken CI runner —
not an app bug), don't push through with silent scope cuts:
1. Report the exact blocker (the actual error, what was verified, what's still needed) to the user
   and ask how to proceed — a scope trim is a real thoroughness-vs-speed tradeoff, not something to
   decide alone.
2. If the user says to pause: cleanly close any running app instances, save interim findings under
   this run's artifacts location, and log what's blocked, what already completed and is reusable on
   a rerun, and what a fresh run needs from scratch (baseline/HEAD will likely have moved on by
   then) — durably enough to survive the current session ending.
3. Updating this skill file with anything newly learned doesn't depend on Phase 4/5 completing — do
   it whenever asked, even mid-block.

## Artifact conventions

Pick one consistent location outside the tracked repo for everything durable in a given run (a dated
directory under your scratch root, or wherever your own setup keeps generated reports) and use it
for: the verified changelog, the code-review report (JSON/Markdown or whatever format is
in use), GUI findings, the GUI test log, `visual-changelog.html`, screenshots, and the user-stories
checklist. If your environment has a way to share generated files with the user (a hosting mechanism,
an upload/artifact tool, or simply pointing at local paths they can open), use whatever's actually
available — don't assume a specific serving mechanism exists. Note that "git-ignored" is not a safety
property here: `.gitignore` only governs paths inside the repository, so a directory outside it needs
its own access controls (restrictive permissions, and not sitting under anything published or synced)
rather than an ignore rule.

Keep a small `qa-run-state.md` at the top of that location as a durable scratchpad: verified SHAs,
worktree/binary paths, a phase checklist, and an explicit "open items for the user" list. That's what
lets a rerun (or a resumed session after a context loss) pick up cleanly instead of re-deriving
everything. On a same-day rerun, archive the prior run's files into a subfolder first (see §
Rerunning above) rather than overwriting them.

Don't bake one run's actual findings, SHAs, or open items into *this* skill file — they belong in
that run's own artifacts. When updating this file after a run, fold in newly-learned *procedure*
(a gotcha, a missing step, a better ordering) and leave the specific bug numbers/dates/commits out.

## Running this unattended (user stepped away)

This campaign is long enough that the user often steps away mid-run. If so:
- **Corroborate status against direct evidence, not just self-reports.** A status message saying
  "still working" is not itself proof of progress — check actual file timestamps in the scratch
  area and running processes before concluding anything is stuck, or conversely before assuming
  silence means done.
- **Queue decisions instead of fabricating approval.** Anything that's normally the user's call —
  merging/pushing anything, rotating a shared fixture, a scope tradeoff on a fixture blocker — gets
  written up and queued, never assumed. Keep working on whatever doesn't need that decision rather
  than stalling the whole campaign on one open question.
- **Notify once, at the end**, with the full queue: ship read, report location, and every item that
  needs a decision — not a running commentary as each phase completes. Use whatever notification
  mechanism your environment provides (a desktop/mobile notification tool, a chat message); if none
  is available, just deliver the full summary in your final response.

## Skills used (optional accelerators, not requirements)

Every step above works performed directly with standard tools (`git`, `gh`, `cargo`, a real GUI
session, plain file writes). If your environment happens to have compatible skills/plugins
installed, they can speed things up — for example: a multi-agent orchestration capability for
running independent phases in parallel and tracking a long multi-phase run durably; a multi-agent
code-review skill (e.g. something like `grumpy-review`) instead of a single-pass manual review; a
structured findings-report format/schema and severity-scoring convention instead of a plain
Markdown table; an interactive browser-based triage tool instead of a written summary; a GUI-driving
skill that already handles the headless-display setup for you; a testnet-faucet helper instead of a
manual funding step; and an end-of-run notification skill for alerting a user who's stepped away.
None of these are assumed to exist — treat every mention above as "if available," never as a
dependency this procedure requires.
