# IDN — Identities

Environment: PR892 build (`/data/target/debug/dash-evo-tool` @ `57195d54`), isolated data dir
`/data/tmp/det-qa-pr892-data`, network Testnet, display `:99`, wallet `QA Wallet 1`. App was
already running (PID 989399) when this pass started; reused per campaign instructions.

## Environment status at start of this pass (worse than DEV.md's snapshot — read before the
## individual story write-ups)

`CAMPAIGN-CONTEXT.md`'s known blocker was re-confirmed present (masternode-list/quorum-sync
failure blocking Platform proof verification for any query — same
`SdkError { source_error: Proof(ContextProviderError(Config("masternode list not yet synced
(quorums unavailable)"))) }` signature DEV.md documented). But this session additionally shows
the **wallet-storage-layer failure** `ALK.md`'s "App-restart failure" section flagged as a
live, unresolved risk: `det.log` at the start of this pass already contained repeated
```
WARN dash_evo_tool::backend_task: Wallet backend initialization deferred
  error=Could not access wallet data. Check available disk space and restart the application.
```
and Settings > Networks showed **four simultaneous red banners** — "SPV sync failed", "We
couldn't finish preparing your wallet. Try restarting the app.", "Your wallet is still starting
up.", and (new, Identities-specific) "Could not load your identities from this device." The
Wallets screen confirms this is not just a display glitch: `QA Wallet 1`'s balance renders as
**0 DASH** with "Sync Status: Core: Error, Addresses: never synced" — i.e., in this session the
wallet backend never wired at all, so even the local-DB-cached balance views that earlier
categories (WAL/SND/ALK) relied on as environment-independent do not render. Per campaign
instructions this was not re-diagnosed or restarted-and-retried beyond the single fresh-launch
check already performed by the main loop; all findings below are attributed to this
already-documented, dual-symptom blocker where applicable. Screenshot:
`screenshots/IDN-000-identities-empty-state-blocked-banners.png`.

Confirmed via direct SQLite inspection (`det-app.sqlite`) that this pass made **zero persistent
changes**: `identities` table has 0 rows before and after, `wallets`/`meta_wallet` unchanged —
consistent with every write-path attempted below being blocked before reaching persistence.

---

## IDN-001: Register a new identity — **BLOCKED** (wizard navigation/validation confirmed
## working; one independent UI bug found en route)

**Persona:** Alex, Priya, Jordan. Acceptance criteria: "Multi-stage confirmation flow. Identity
funded from an asset lock."

### Steps and observed result

1. Identities (empty state) > "Create my first identity" → `Identities > Create Identity`
   wizard. Step 1 ("Choose which wallet"): `QA Wallet 1` correctly pre-selected, shown as
   "QA Wallet 1 — 0 DASH" (the 0 DASH reflects the wallet-not-wired state above, not the
   wallet's real balance).
2. Step 2 ("Choose your funding method") dropdown: two options — **"Recover an unfinished
   funding"** and **"Receive a new deposit"**. Selected "Receive a new deposit" → step 3
   appeared titled "Deposit received. Choose how much to use, then continue." but rendered
   **no content at all** (no address, no QR, no amount field, no error) — likely because
   generating a fresh deposit address needs a wired wallet backend, but the screen gives no
   explicit "can't generate address right now" message the way other blocked flows do (see
   below). Minor UX gap, not re-tested in a healthy environment to confirm it's blocker-specific.
3. Selected "Recover an unfinished funding" instead → step 5 showed **"Couldn't load your
   unfinished funding."** with a **"Retry"** button — a clean, well-typed empty/error state.
   Clicked Retry twice; same message reproduced consistently, no crash, no hang.
   Screenshot: `screenshots/IDN-001-2-recover-unfinished-funding-couldnt-load-retry.png`.
4. Tested "Show Advanced Options" (step numbering shifts to 5 steps: wallet, identity index,
   key selection, funding method, funding step). Identity Index field defaults to `0` as
   recommended. Key Selection Mode dropdown offers **Default (Recommended)** / **Advanced**.
   Screenshot: `screenshots/IDN-001-1-create-identity-wizard-advanced-key-mode.png`.

### Bug found: "+ Add Key" button in Advanced key-selection mode is a no-op

Switched Key Selection Mode to "Advanced" — a "+ Add Key" button appeared. Clicked it twice;
**no new key row ever appeared**, no banner, no log line. Traced via source: `add_new_identity_
screen/mod.rs`'s `add_identity_key()` has a silent early return (`let Ok(backend) =
self.app_context.wallet_backend() else { return; }`) that fires whenever the wallet backend
isn't wired — guaranteed in this environment. Source review also surfaced a second, deeper
issue independent of this session's environment problem: even with a wired backend, the first
click misses the identity-key cache (the default 5 keys are pre-warmed, but "+Add Key" always
requests the 6th), and the async warm-completion handler (`ensure_correct_identity_keys()`)
unconditionally rebuilds the visible key list from a **fixed 5-entry default set**, discarding
the newly-requested key rather than appending it — so the first click is a functional no-op
even outside this campaign's degraded environment. Worth a real ticket; not fixed here per
campaign rules (observe/document only).

**Verdict: BLOCKED** — reasoning: "blocked by known environment issue: Testnet masternode-list/
quorum-sync failure prevents Platform proof verification, see CAMPAIGN-CONTEXT.md /
scenarios/ALK.md and scenarios/DEV.md for full diagnosis" (compounded this session by the
wallet-storage-layer failure documented above, which prevents the wizard from ever reaching a
fundable state at all). Wizard navigation, step sequencing, and the "Recover an unfinished
funding" empty-state error are all confirmed working correctly. The Advanced-mode "+ Add Key"
no-op is a real, independently-reproducible defect (see above) — flagged for product
awareness, not counted against the BLOCKED verdict since it's a secondary/advanced path, not
the story's core acceptance criteria.

---

## IDN-002: Load existing identity by ID — **FAIL** (silent hang on the exact acceptance-
## criteria flow; two alternate lookup methods on the same screen degrade gracefully)

**Persona:** Priya, Jordan. Acceptance criteria: "Enter identity ID and private key. Identity
details are fetched and displayed."

No known-real testnet identity ID fixture was found (checked `memcan:recall` for project
`dash-evo-tool` and the `dash-platform` skill's docs — no identity ID fixture, only the
well-known **DPNS contract ID** `GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec`, which is a valid
32-byte Base58 `Identifier` format but points to a contract, not an identity). Used it as a
syntactically-valid input to exercise the load flow's behavior — since proof verification is
broken for every kind of Platform query in this environment, the exact ID's real-world validity
does not change the failure mode being tested here.

`Identities > "I already have an identity — load it"` → `Load Existing Identity` screen, three
tabs: **"Identity ID & private key"** (the story's exact flow), **"From my wallet"**, **"My
username"**.

### "Identity ID & private key" tab — silent hang, zero feedback

1. Typed the DPNS contract ID into the "Identity ID" field (no private key — the field is
   present but the "Load Identity" button enables on ID format alone). Button turned solid
   blue (enabled).
2. Clicked "Load Identity". **Nothing happened**: no banner (info or error), no navigation, no
   new line in `det.log` — reproduced 3 times, including one immediate re-screenshot right
   after the click to rule out a flash-and-vanish. Screenshot:
   `screenshots/IDN-002-1-load-identity-by-id-silent-noop.png`.
3. Traced via source: the click handler (`add_existing_identity_screen.rs`) sets an info
   banner ("Loading identity...") **synchronously, before** dispatching the async backend
   task — so a banner should always appear immediately regardless of what happens next. Its
   absence points to `AppContext::run_backend_task`'s generic `ensure_wallet_backend` pre-check
   (used by this screen, unlike the direct `wallet_backend()` calls other screens use) hanging
   indefinitely inside the wallet-backend construction/lock rather than erroring — so the task
   never reaches the point where it would send a `TaskResult` back to the UI at all.

### "From my wallet" tab — works correctly, clean typed error

Selected `QA Wallet 1`, clicked "Search Wallet for Identities". This **dispatched correctly**:
`det.log` shows real gap-limited key-derivation attempts (indices 0–11) and a clean completion
with a proper typed error banner: **"No identities found up to wallet index 5. Try a higher
search range."** (`NoWalletIdentitiesFound { max_index: 5 }`). Screenshot:
`screenshots/IDN-002-2-search-wallet-for-identities-clean-typed-error.png`. This confirms the
silent hang above is not simply "everything on this screen is equally broken" — a sibling
button on the identical screen, under the identical environment condition, completes and
reports cleanly.

### "My username" tab — doubles as IDN-010, degrades gracefully (see IDN-010 below)

**Verdict: FAIL** for the story's stated acceptance-criteria flow ("Enter identity ID and
private key") — it hangs with **zero** user-facing feedback, which is a strictly worse failure
mode than the clean typed/generic errors every other blocked flow in this campaign shows
(including the other two tabs on this exact screen). This is flagged as an independent defect,
not purely environment fallout, precisely because sibling code paths on the same screen, in the
same session, degrade correctly. Should be re-tested once the environment blocker is resolved
to see if the hang persists on a healthy backend — if it does, it's a P1 (a legitimate ID+key
pair would leave a user staring at a frozen button forever with no explanation).

---

## IDN-003: Load evonode/masternode identity — reclassified `[Superseded by MN-001]` in the
## corrected catalog

**Reconciliation note**: PR892's real catalog (`docs/user-stories.md` in the PR892-build
worktree) tags this story `[Superseded by MN-001]`, not `[Implemented]` — the new MN
category's MN-001 ("Load a masternode by keys") now owns this capability. `progress.md`
tracks IDN-003 as N/A accordingly. The FAIL finding below (same silent-hang defect class as
IDN-002, on the exact "Load a masternode" flow this story describes) is kept as directly
relevant context for whoever tests MN-001 — the underlying screen and bug are the same one
MN-001 will exercise.

## IDN-003 (original write-up, kept as context for MN-001): Load evonode/masternode identity — **FAIL** (same silent-hang defect class as
## IDN-002; format validation and node-type toggle confirmed working)

**Persona:** Priya. Acceptance criteria: "Enter protx hash to load the associated identity."

No masternode/evonode identity fixture was found via `memcan:recall` (consistent with
`DEV.md`'s finding of no `.testnet_nodes.yml` fixture in this environment; real registration
needs ~1000 tDASH collateral this environment doesn't have). Masternodes screen: "No
masternodes loaded" empty state, "Load a masternode" button — matches `DEV-006`'s prior
screenshot.

### Steps and observed result

1. "Load a masternode" → form with **Masternode / Evonode** node-type toggle (clicked
   "Evonode", switched cleanly), ProTxHash field, optional Alias, and three optional private
   key fields (Voting/Owner/Payout).
2. Typed `not-a-valid-protxhash` into ProTxHash, clicked "Load masternode". Got a clean inline
   validation error: **"This doesn't look like a valid ProTxHash. Enter a hex or Base58
   ProTxHash from your masternode configuration."** — no crash, precise and actionable.
   Screenshot: `screenshots/IDN-003-1-load-masternode-protxhash-validation-then-silent-noop.png`
   (taken after the subsequent step below; the validation-error state was confirmed visually
   before proceeding).
3. Replaced with a syntactically-plausible 68-hex-character string (passed format validation —
   no error shown). Clicked "Load masternode". **Same silent-hang signature as IDN-002**: no
   banner, no navigation, no new log line, reproduced across waits of 2s/5s/15s after the
   click.

**Verdict: FAIL** for the same reasoning as IDN-002 — the ProTxHash format validation and the
Masternode/Evonode node-type toggle both work correctly (clean, actionable feedback), but the
actual "Load masternode" submission hangs with zero user feedback once given a well-formed
input. Per source review conducted for IDN-002, this screen's load button funnels through the
same `IdentityTask::LoadIdentity` task as the ID+key tab (with a `reconcile_pending_load`
backstop specifically built for this early-return scenario) — but no backstop resolution was
observed even after a 15-second wait, so either the backstop itself isn't firing in this
degraded environment or the underlying task truly never completes. Re-test once the environment
blocker is resolved.

---

## IDN-004 through IDN-009, IDN-013: Identity-detail operations — **BLOCKED** (no identity
## reachable to operate on)

**Stories:** IDN-004 (Top up identity credits), IDN-005 (Withdraw credits to Core address),
IDN-006 (Transfer credits between identities), IDN-007 (Add key to identity), IDN-008 (View
identity keys and details), IDN-009 (Refresh identity state), IDN-013 (Top up identity from
Platform addresses).

All of these are reached from an identity's detail screen (Identities > select a loaded
identity > …), which does not exist as a navigation target when zero identities are loaded
locally — confirmed via direct SQLite check (`identities` table: 0 rows, matching the
Identities screen's persistent "Welcome to Identities" empty state throughout this pass). Since
IDN-001 (register), IDN-002 (load by ID), and IDN-003 (load by ProTxHash) — the only three ways
to populate that list — all failed to produce a loaded identity in this environment (two via
silent hang, one via the environment blocker), there is no UI surface for IDN-004–009/013 to
exercise beyond what IDN-012's source review already established structurally (see below).

**Verdict for all seven: BLOCKED** — reasoning: "blocked by known environment issue: Testnet
masternode-list/quorum-sync failure prevents Platform proof verification, see
CAMPAIGN-CONTEXT.md / scenarios/ALK.md and scenarios/DEV.md for full diagnosis" (transitively,
via IDN-001/002/003's inability to produce a loaded identity to act on). Not independently
re-tested; no additional UI surface exists to test without an identity.

---

## IDN-010: Search identity by DPNS name — **BLOCKED** (dispatches and fails cleanly — same
## masternode-list-sync signature as DEV.md)

**Persona:** Alex, Priya. Acceptance criteria: "Enter username and retrieve associated
identity."

This is the `Identities > "I already have an identity — load it" > "My username"` tab (no
separate search screen exists elsewhere). Entered `alice` (the field's own placeholder example:
"Enter 'alice' to look up 'alice.dash'"), clicked "Search by Username".

`det.log` shows a real dispatch: 7 retries against 7 different DAPI endpoints, each failing with
the now-familiar `SdkError { source_error: Proof(ContextProviderError(Config("masternode list
not yet synced (quorums unavailable)"))) }`, then a clean (if generic) banner: **"An unexpected
error occurred. Please try again later."** with the technical detail available via "Show
details". Screenshot: `screenshots/IDN-010-1-search-by-username-FAIL-quorums-unavailable.png`.

**Verdict: BLOCKED** — reasoning: "blocked by known environment issue: Testnet masternode-list/
quorum-sync failure prevents Platform proof verification, see CAMPAIGN-CONTEXT.md /
scenarios/ALK.md and scenarios/DEV.md for full diagnosis." Unlike IDN-002/003's ID/ProTxHash
load buttons, this one **does** dispatch and fail gracefully with visible retry activity and a
banner — reinforcing that the ID+key and ProTxHash load buttons' silent hangs are a distinct,
narrower defect rather than "this whole category is universally broken the same way."

---

## IDN-012: Register identity from Platform addresses — **BLOCKED** (confirmed implemented and
## correctly gated in source; unreachable because the live balance cache never populates)

**Persona:** Priya, Jordan. Acceptance criteria: "Alternative funding method in identity
registration wizard. Uses existing Platform address balance."

The Create Identity wizard's funding-method dropdown only offered "Recover an unfinished
funding" / "Receive a new deposit" — no "Use a Platform address" option, despite `ALK.md`
documenting a real, non-zero Platform balance (0.01985204 DASH) on this same wallet's DIP-17
address earlier in the campaign.

Source review (`src/ui/identities/funding_common.rs`, `add_new_identity_screen/mod.rs`)
confirms this is **implemented, not a gap**: `FundingMethod::UsePlatformAddress` exists and is
gated behind `wallet.platform_address_info.values().any(|info| info.balance > 0)` — an
in-memory cache populated by a periodic (~15s) push from the wallet-backend coordinator
(`wallet_backend/event_bridge.rs` → `AppContext::apply_platform_address_push`), not fetched
on-demand by the screen itself. Because the wallet backend never wired in this session (see
environment status above), that cache stays empty regardless of what balance is actually
persisted in SQLite, so the option is correctly absent rather than shown-and-broken.

**Verdict: BLOCKED** — reasoning: "blocked by known environment issue: Testnet
masternode-list/quorum-sync failure prevents Platform proof verification, see
CAMPAIGN-CONTEXT.md / scenarios/ALK.md and scenarios/DEV.md for full diagnosis" (specifically,
the compounding wallet-backend-not-wired symptom prevents the live Platform-balance cache this
feature depends on from ever populating). Feature confirmed implemented via source, correctly
gated, not reachable for a live UI exercise in this environment. Worth a follow-up pass once the
environment recovers.

---

## Summary

| Story | Verdict |
|---|---|
| IDN-001 | BLOCKED (wizard/validation work; independent "+Add Key" no-op bug found) |
| IDN-002 | FAIL (ID+key load button silently hangs; other two tabs on same screen degrade cleanly) |
| IDN-003 | FAIL (same silent-hang defect; ProTxHash validation + node-type toggle both PASS) |
| IDN-004 | BLOCKED (no identity reachable) |
| IDN-005 | BLOCKED (no identity reachable) |
| IDN-006 | BLOCKED (no identity reachable) |
| IDN-007 | BLOCKED (no identity reachable) |
| IDN-008 | BLOCKED (no identity reachable) |
| IDN-009 | BLOCKED (no identity reachable) |
| IDN-010 | BLOCKED (dispatches + fails cleanly on known masternode-list-sync error) |
| IDN-011 | N/A (Gap, not implemented — pre-existing) |
| IDN-012 | BLOCKED (confirmed implemented + correctly gated in source; live cache never populates) |
| IDN-013 | BLOCKED (no identity reachable) |

Two genuinely new, environment-independent-looking findings: **IDN-002** and **IDN-003** both
hang completely silently (no banner, no log line, no timeout) on their core acceptance-criteria
action, despite the source code setting a "Loading..." banner synchronously before dispatch —
this is a materially worse failure mode than every other blocked flow tested in this pass (all
of which show a retry trail and/or a clean typed/generic error banner). The **"+Add Key"**
no-op in the Create-Identity wizard's Advanced mode is a second, narrower defect confirmed via
source to be a real bug (cache-miss + unconditional-rebuild race) independent of this
environment's issues. Everything else in this category traces cleanly back to the two
already-documented environment failures (`ALK.md`'s wallet-storage-open failure and `DEV.md`'s
masternode-list/quorum-sync failure), both of which were present and worse than DEV.md's
snapshot at the start of this pass (see environment status section above).

No PR892 application source was modified. QA Wallet 1 and the DIAG throwaway wallet were left
untouched — confirmed via direct SQLite inspection that this pass added zero rows to
`identities` and made no changes to `wallets`/`meta_wallet` (every load/create/search attempt
either failed before reaching persistence or hung without ever completing).
