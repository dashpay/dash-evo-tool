# Scenario: Usernames in Identities — list, request, availability, status

**Verifies:** how a user sees their usernames and pending requests, asks for a
new name, learns whether it is available / needs a community vote, reviews the
cost, and follows a request; and how a contested registration is reported. It
also covers DashPay social-profile save feedback. (Risk area: #918, #901,
#1054.)

**Tier justification:** Needs a real contested-name registration against a
live testnet contest window, real identity data, and real async save timing
(a save in flight when the user switches identity) — none of which a
no-display harness can drive.

**Run against BOTH builds with this identical procedure** — the baseline and
development binaries selected for the current campaign (record their exact
SHAs in that campaign's own artifacts, not here). Describe what you observe
on each build without assuming which one is "correct", and note where a
control or screen exists on only one build (a feature-presence difference,
not a step failure).

Before any step that spends funds, consumes a deposit, or registers a
name, each build needs its own independently-funded equivalent fixture (or
a restored snapshot of the same starting state) — see [A/B build comparison
contract](../README.md#ab-build-comparison-contract). Name registration also
needs a name that is free **on each side**: use a different candidate name per
build.

## Prerequisites

- Network: testnet
- Environment variables (names only):
  - `E2E_WALLET_MNEMONIC` — funded testnet wallet
  - `E2E_IDENTITY_ID` — an identity with enough credits (optional if the
    wallet's identities are discovered in the app)
- At least two identities on the same wallet (for the identity-switch-
  mid-save step), at least one with no username
- Candidate names: one short/generic (likely contested), one long/unusual
  (likely uncontested), one already taken
- Verify exact labels during execution rather than assuming the wording
  below is literal

## Setup

```bash
DATADIR=$(mktemp -d)
cp .env.example "$DATADIR/.env"
pgrep -af dash-evo-tool

BIN=<path to the build under test — baseline or development worktree binary>
test -x "$BIN"
LOG="$DATADIR/dpns-registration.log"
DASH_EVO_DATA_DIR="$DATADIR" nohup "$BIN" >"$LOG" 2>&1 &
```

Resize the window (see the README's "Known UI/environment quirks") before
judging layout. Check `det-stderr.log` / `det.log` in `$DATADIR` for panics
after each run.

## Procedure

1. **Where usernames live.** Record where the identity's usernames are listed
   (Identity Home, the identity's Profile/Settings, the identity switcher),
   with their row actions (copy, QR code, make main) and any pending requests
   or recent outcomes shown there.
2. **Start a request.** Find the control that starts a new username request
   from the identity: on a build with a Usernames card it is Identity ▸
   Settings ▸ Usernames ▸ "Get another username" ("Get a username" when the
   identity has none, and right after creating an identity). On a build
   without that card the equivalent lives under Tools ▸ DPNS (Register Name).
   Record the entry point on each build and whether an identity or signing-key
   picker is offered.
3. **Availability check.** Type, in turn, a taken name, a likely-contested
   name and an unusual name, **without paying**. Record the state text for
   each (available / needs a vote / joinable / taken / locked / window closed
   / already requested), the live-check wording, and any fee text. Typical
   wording: "is already taken. Try another name.", "is available.", "is
   available, but it needs a community vote." with an alternative offered as
   "Try one without a vote". Also try a too-short name and one with invalid
   characters.
4. **Review step.** For a contested name a consent dialog ("@name needs a
   community vote", choices "Choose another name" / "I understand, continue")
   may precede a "Review and pay" screen whose button reads like "Pay <total>
   DASH and request @name"; a build without them goes straight to
   registration. Record the consent/review text:
   fee amounts, whether the fee is described as refundable, the join-window and
   vote-length wording, the total, and the balance it is paid from. Change the
   name afterwards and record whether the review is invalidated and asked
   again. Record any alternative offered that needs no vote.
5. **Low credits.** With an identity whose balance is too low, record how the
   review step offers more funds and whether the chosen name is preserved when
   returning from the add-funds screen.
6. **Register a contested name (MUTATES: spends credits, registers a
   request).** Pay for the likely-contested name. Record the progress dialog
   text (does it name the username?), the completion message (registered vs.
   pending a community vote), and whether the app can be used while it runs.
7. **Register an uncontested name (MUTATES).** On a second identity register
   the unusual name and record the completion message; confirm it differs from
   the contested case.
8. **Status of the request.** A fresh contest's contender row may need a
   manual Refresh in the votes view before it appears; press it and record
   that. The contest name is shown in normalized form (for example `0` for
   `o`). Open the pending request. Record the timeline,
   tally, "what happens next" text, and any link to the votes view. Record
   the indicator on Identity Home, the identities list, and the onboarding
   checklist, and the indicator's tooltip.
9. **After the estimated deadline.** If the contest window can be waited out
   (testnet), record how the request is labelled once the estimated end passes
   and when the outcome arrives (won / lost / locked).
10. **Make a name the main one.** If the identity has two names, use the make-
    main row action; record where the main name then appears (header, lists,
    switcher).
11. **Social profile save, normal case (MUTATES: publishes a paid profile
    write).** Open Contacts → set up/edit social
    profile, change the display name or bio, save. Record the feedback during
    and after the save and whether the progress indicator clears.
12. **Social profile save, forced failure (MUTATES).** If you can force a failure
    (briefly disconnect the network mid-save), record the same as step 11.
13. **Switch identity mid-save (MUTATES).** Start a profile save on identity A and
    switch to identity B before it completes. Record which identity (if any)
    shows the result banner and whether it is ever attributed to the wrong
    identity.
14. **Contacts setup CTA.** On an identity with no DashPay profile, open
    Contacts and record the setup card's CTA wording and whether any
    "Why?"/explanation control opens an explanation.

## Safety constraints specific to this scenario

- Steps 6-7 spend credits and register names; use testnet only and the
  smallest-fee names available.
- Steps 11-13 write the identity's published DashPay profile (it is created
  or updated through the signer and costs credits). Per the equivalent-fixture
  rule, give each build an equivalent, independent identity with the same
  profile state (profile present or absent) and the same credit balance;
  never run both builds against the same identity's profile.
- Do not retry a payment whose outcome is uncertain before checking the
  usernames list; record the app's own wording for that case if it occurs.

## Expected outcome / pass criteria

Record the observed behavior for each build at every step, then:

- **Precedence: intermittency never downgrades a wrong-identity result.** If
  a save result is attributed to the wrong identity (step 13) on one build and
  never on the other, it stays a regression however rarely it reproduces;
  attempt it more times instead of waiving it.
- A contested registration reported as plain "registered" on one build and
  as pending on the other is a difference to classify under the contract,
  not to wave through.

Apply the [A/B build comparison contract](../README.md#ab-build-comparison-contract):
its blocker rule decides what blocks, and its equivalent-fixture rule applies
to every step marked **MUTATES** below.

## Known gotchas

- Contest resolution timing depends on the live testnet schedule; defer
  step 9 to a later session if needed.
- A name consumed by the first build's run is gone for the second: use
  different names per build (see the fixture rule above).
- The identity-switch race (step 13) is timing-sensitive; several attempts
  may be needed.

<sub>🤖 Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
