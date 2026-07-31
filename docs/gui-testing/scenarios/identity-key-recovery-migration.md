# Scenario: Identity key recovery, migration, and key-placement consistency

**Verifies:** A private key is found, signed with, restored, and removed
consistently regardless of which internal convention filed it; a legacy
(pre-1.0) install's stranded keys can be recovered without re-entering
secrets; the Key Info / "Manage keys" screen is reachable from the standard
identity view and from a masternode's own detail page, and the two never
disagree about a key's role or held-state. (Risk area: #941, #945, #946,
#948 — key-placement-resolution rework, issue #889.)

**Tier justification:** Needs a real, previously-migrated v0.9.3-shaped
identity/masternode fixture, real password-prompt cancel timing, and
cross-screen navigation (masternode detail page ↔ Key Info ↔ identities keys
list) that a no-display harness can't drive end-to-end.

**Run against BOTH builds with this identical procedure** — the baseline and
development binaries selected for the current campaign (record their exact
SHAs in that campaign's own artifacts, not here) — do not skip or alter
steps between runs. Record what actually happens on each build; do not
presuppose which behaviors are "the fix."

## Prerequisites

- Network: testnet
- Environment variables (names only):
  - `E2E_WALLET_MNEMONIC` — funded testnet wallet
  - `E2E_MN_PROTX_HASH` — (optional) a testnet masternode ProTxHash this
    wallet controls, for the masternode-side half of this scenario
- A password-protected standalone identity (create one via the identity's
  key-protection settings if none exists) — needed for the password-cancel
  step
- Ideally, an identity/masternode with data left over from a pre-1.0
  install (or one seeded to resemble that state) — if unavailable, note the
  gap rather than fabricating one
- Verify exact widget names during execution — the labels below (e.g.
  "Manage keys", "Restore") describe intent, not a confirmed literal string

## Setup

```bash
DATADIR=$(mktemp -d)
cp .env.example "$DATADIR/.env"
pgrep -af dash-evo-tool

BIN=<path to the build under test — baseline or development worktree binary>
test -x "$BIN"
LOG="$DATADIR/identity-key-recovery.log"
DASH_EVO_DATA_DIR="$DATADIR" nohup "$BIN" >"$LOG" 2>&1 &
```

## Procedure

1. **Keys screen reachability.** Open an identity in the default (least
   advanced) interface level. Look for a way to reach a screen listing the
   identity's keys (e.g. under Settings) without switching to a more
   advanced level and without starting a payment. Record whether it's
   reachable, and from where.
2. **Masternode key screen.** Open a masternode's detail page and look for
   an equivalent "manage keys" entry point. Record what it opens.
3. **Role-label consistency.** Pick a key visible on both the identity's own
   keys list and (if applicable) the masternode detail page. Record the
   role name shown in each place — are they the same wording?
4. **Held-but-unpublished key.** If a key is saved on this device but not
   on any on-chain key list for the identity, look for where the app
   surfaces it. Record whether it's visible/reachable anywhere, and what
   options are offered (view, remove).
5. **Restore stranded keys.** On an identity/masternode with legacy data
   holding keys not yet in the current store, look for a "restore" affordance
   on both the masternode detail page and the identity's own keys screen.
   Trigger it, restore one key, and record: does it now show as held? Does
   the offer narrow or disappear once nothing is left? Run it again and
   record whether it's a safe no-op.
6. **Password-protected restore, cancel path.** On the password-protected
   identity, start a restore and cancel/dismiss the password prompt.
   Record the state of the Restore control afterward — does it look ready
   to try again, or does it look stuck/already-started?
7. **Password prompt necessity.** For a key held in the clear in one place
   and password-protected in another (if your fixture has this shape),
   use/sign with it and record whether a password prompt appears.
8. **Same-numbered-key collision.** If reachable in your fixture (a
   masternode's own record and its linked voting identity sharing a key
   number), enter/paste a private key for one of the two same-numbered keys.
   Record what happens to *both* keys afterward — check each one's own page,
   not just the confirmation banner.
9. **Key removal.** Remove a saved private key. Record: does the key's page
   immediately reflect the removal, or does it require leaving and
   returning? Can the removed key still be revealed/signed with afterward?
10. **Live update while a key's page is open.** With a key's page open,
    trigger a restore or edit for the same identity from elsewhere (a
    second window on the same data dir, or via Back/forward navigation),
    then return to the open key's page and make an edit. Record whether the
    other change survives.

## Safety constraints specific to this scenario

- Use a real WIF/private key only for a throwaway testnet
  identity/masternode fixture — never a mainnet key, never a value pasted
  into a scenario file, report, or commit.
- Step 8 can destroy a key's private half if something goes wrong — rehearse
  first with a fixture that has a recovery path (mnemonic-derived), not a
  hand-imported unique WIF with no backup.

## Expected outcome / pass criteria

Record the observed behavior for each build (baseline, then HEAD) at every
step above, then apply this rule:

- **BLOCKING** only if HEAD behaves *worse than baseline* in the happy path
  described above (e.g. a key that was findable/usable on baseline is no
  longer findable/usable on HEAD; a restore that worked cleanly on baseline
  now corrupts or loses a key on HEAD), or if any step reveals a **data-loss**
  outcome (a key's private half becoming permanently unrecoverable) on
  either build.
- **NOT blocking**: a timing-dependent race that only sometimes reproduces;
  a UI glitch with no functional consequence; or an issue that reproduces
  identically on both builds (pre-existing — note it, don't block on it).
- If a step's affordance doesn't exist at all on baseline (e.g. the Keys
  screen or the restore offer are new features introduced within this
  diff), that's expected — record it as "not present on baseline, present
  on HEAD" rather than a failure of the baseline run.

## Known gotchas

- Verifying a restored voting key against its actual on-chain address is an
  explicitly deferred follow-up (issue #942) on both builds where the
  feature exists at all — a voting key on an unlinked separate voting
  identity correctly being listed as "cannot be restored automatically" is
  expected, not a defect.
- A voting key stored directly on the identity's own record (rather than a
  separate voting identity) has a documented residual on HEAD: saving/
  removing it by hand can affect a same-numbered voting key on a linked
  voting identity. After doing so, re-open the keys list and check every key
  still reads as expected before treating anything odd here as a new
  regression.

<sub>🤖 Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
