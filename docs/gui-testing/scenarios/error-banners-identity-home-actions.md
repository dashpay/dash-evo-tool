# Scenario: Error-banner wording, validation consistency, and Identity Home actions

**Verifies:** Error banners show plain, actionable text rather than raw
upstream error strings; format-validation rules (DPNS name/memo/keyword/
label length) behave the same everywhere they're triggered; identity removal
doesn't block the UI thread; Identity Home shows one non-redundant row of
action buttons instead of duplicate destinations; `Send to wallet` is
disabled with an explanation when no usable withdrawal key is held; an
already-consumed deposit says so plainly instead of suggesting a retry; and
the app doesn't crash during deposit verification. (Risk area: #927, #934.)

**Tier justification:** Needs a real deposit broadcast/verification cycle to
exercise the worker-thread stack-size fix, and real visual confirmation of
button count/labels/layout that a no-display harness can assert on logic but
not on what actually renders.

**Run against BOTH builds with this identical procedure** — the baseline and
development binaries selected for the current campaign (record their exact
SHAs in that campaign's own artifacts, not here).

Before any step that spends funds, consumes a deposit, or registers a
name, each build needs its own independently-funded equivalent fixture (or
a restored snapshot of the same starting state) — see [A/B build comparison
contract](../README.md#ab-build-comparison-contract).

## Prerequisites

- Network: testnet
- Environment variables (names only):
  - `E2E_WALLET_MNEMONIC` — funded testnet wallet
- One identity with a locally-held TRANSFER/OWNER key, and ideally a second
  identity with only an on-chain authentication key and no locally-held
  signing key (for the `Send to wallet` disabled-state comparison)
- Verify exact button labels/wording during execution rather than assuming
  the text below is literal

## Setup

```bash
DATADIR=$(mktemp -d)
cp .env.example "$DATADIR/.env"
pgrep -af dash-evo-tool

BIN=<path to the build under test — baseline or development worktree binary>
test -x "$BIN"
LOG="$DATADIR/identity-home-actions.log"
DASH_EVO_DATA_DIR="$DATADIR" nohup "$BIN" >"$LOG" 2>&1 &
```

## Procedure

1. **Identity Home button row.** Open Identity Home. Record how many action
   buttons are present, their labels, and whether any two open the same
   destination screen.
2. **`Send to wallet` gating, enabled case.** On the identity with a
   locally-held signing key, record whether "Send to wallet" (or whatever
   the withdrawal-initiating control is called) is enabled and whether it
   opens a working withdrawal flow.
3. **`Send to wallet` gating, disabled case.** On the identity without a
   locally-held signing key, record whether the same control is disabled,
   and if so, what explanation (if any) is shown.
4. **Funding wording consistency.** Walk through the identity funding
   wizard (breadcrumb, heading, sub-heading, CTA buttons) and record whether
   the terminology is consistent throughout (vs. mixing two different terms
   for the same action).
5. **Already-consumed deposit.** Attempt to fund using a deposit that has
   already been consumed by another operation (re-use a completed one).
   Record the exact message shown — does it say the deposit cannot be
   reused and to choose/start a different one, or does it suggest retrying?
6. **Deposit verification stability.** Perform several deposit verifications
   in a row (fund an identity via deposit two or three times). Record whether
   the app crashes, then check the logs for a panic marker even if the UI
   appeared fine. Confirm the log targets exist first — `ls -l "$LOG"` and
   `ls -l "$DATADIR"`, which is where the app writes `det.log` /
   `det-stderr.log` — then search them:

   ```bash
   grep -R -e 'panicked' -e 'location=' "$LOG" "$DATADIR"
   ```

   Record "no panic" only after confirming the files exist and are non-empty —
   a missing log file means the check never ran, not that the app was clean.
7. **Format-validation consistency.** DPNS names, contract keywords, and
   DashPay account labels each have their own documented length limit (they
   are different fields with different rules by design — do not expect them
   to reject at the same length). Instead, pick ONE of these fields and
   trigger its format-validation error through two different entry points
   that both accept it (e.g. the same overlong DPNS name attempted through
   registration and, if another screen also accepts a DPNS name, through
   that screen too). Record whether both entry points reject at the same
   limit with the same message, for that one field.
8. **Error banner wording.** Trigger a database/parsing-style error if
   possible (e.g. an interrupted operation). Record whether the banner text
   is plain and actionable, or contains raw error strings/stack
   traces/internals.
9. **Identity removal responsiveness.** Remove a local identity and record
   whether the UI freezes/stutters during the removal, and how the result is
   reported.

## Safety constraints specific to this scenario

- Steps 5–6 move real (small) testnet funds — keep amounts minimal.

## Expected outcome / pass criteria

Record the observed behavior for each build at every step, then apply this
rule:

- **BLOCKING** only if HEAD is worse than baseline in the happy flow (e.g.
  HEAD crashes during deposit verification where baseline didn't; HEAD's
  already-consumed-deposit message is more misleading than baseline's; a
  button that worked correctly on baseline is now broken/duplicated on
  HEAD), or a **data-loss** outcome on either build.
- **NOT blocking**: a UI freeze/stutter that's cosmetic and self-resolves;
  wording differences with no functional consequence; an issue reproducing
  identically on both builds (e.g. if baseline also shows 6 duplicate
  buttons and HEAD also does — note it, don't block).
- If the two builds' button rows differ in count or labels, record both
  observations plainly rather than treating either as automatically wrong —
  apply the blocker rule based on functional correctness (do all buttons
  work, are destinations distinguishable), not on matching layouts.

## Known gotchas

- If `Send to wallet` behaves inconsistently, check whether you're looking
  at Identity Home, the identities-list popup, or the Withdrawal screen
  directly — these three surfaces historically used different gating
  criteria; note which surface any discrepancy is on.
- The change range this scenario covers touches many files across validation,
  fee estimation, error banners, and identity removal — if a defect surfaces,
  compare directly against the baseline build's behavior for the same step
  before concluding it's new.

<sub>🤖 Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
