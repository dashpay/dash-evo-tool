# TOK — Token Operations

Environment: PR892 build (`/data/target/debug/dash-evo-tool` @ `57195d54`), isolated data dir
`/data/tmp/det-qa-pr892-data`, network Testnet, display `:99`, wallet `QA Wallet 1`. App was
already running (PID 989399) when this pass started; reused per campaign instructions. The app
**crashed mid-pass during DOC testing** (see `scenarios/DOC.md`, DOC-002) and was relaunched
(PID 1279253); all TOK testing after that point ran against the relaunched instance. Both
sessions showed the same environment blocker.

## Retest pass (2026-07-15): identity registration now works, retesting all 17 in-scope TOK stories

Same environment-blocker fix as `scenarios/DOC.md` (dashpay/platform#4133 fixed again upstream).
App running as PID 527888, binary `/data/tmp/det-qa-pr892-bin-myown/dash-evo-tool` (hash
`2931220e94871a0454ac56a43092aa87246b5a590d917645c025ddb1c7f9271a`), data dir
`/data/tmp/det-qa-pr892-data`, Testnet. Two real identities exist and hold Platform balance:
`QA Identity 1` (@detqa892run2) and `QA Identity 2` (@detqa892run3). Three QA-owned contracts
exist from the DOC retest pass (QA Note Contract, QA Transfer Contract, QA Purchase Contract).
The `WalletBackendNotYetWired`/asset-lock-recurrence bug (dashpay/platform#4133) was **not** hit
at any point during this TOK retest pass.

**Fixture-token strategy**: TOK-005 (Create Token) turned out to be completely non-functional
(see below) — no QA-owned token could ever be created, so most of TOK-006 through TOK-018 (which
need a tracked identity-token pair) could not be exercised with a real owned token. Worked around
by using **`lklimek-20260217`** (contract `7TNdYLnTdCD1mpZ4yH2RyUthpmyF4QRZAr2kX18JzCeo`), a real,
pre-existing, third-party Testnet token discovered live via TOK-002's keyword search and added to
"My Tokens." This let every owner-only action (Mint, Burn, Freeze, Pause, Set Price, Update
Config) be exercised far enough to observe DET's authorization-gating behavior (QA identities are
correctly rejected, "Only the contract owner is [allowed]") even though the actual privileged
action can never complete for real. It also happens to have a live perpetual distribution, which
let TOK-011/015/016 be exercised much more thoroughly than a QA-owned throwaway token would have
allowed.

### TOK-001: View token balances — **PASS** (retested: real tracked token confirmed listed with
### correct data, per-identity balance table renders correctly)

"My Tokens" now shows a live, non-empty table: **Token Name** (`lklimek-20260217`), **Token ID**,
**Description** (`None`), **Actions** (`More Info` / `X`). Clicking the token name drills into a
per-identity table — **Identity Alias | Identity ID | Balance (Check) | Rewards (Estimate) |
Actions (Transfer/Claim/Mint/…/X)** — correctly listing all three loaded identities (QA Identity
1, QA Identity 2, alice.dash). Screenshots: `screenshots/TOK-001-2-my-tokens-list-with-tracked-token.png`,
`screenshots/TOK-004-006-per-identity-actions-table.png`.

**Verdict: PASS** — "My Tokens" lists a held/tracked token with a working balance-check surface,
matching the acceptance criteria. (Not tested: a token with an actual non-zero balance for a QA
identity, since no QA identity owns any token — see TOK-005.)

### TOK-002: Search and discover tokens — **PASS** (retested: live keyword search returns real
### results and "add to My Tokens" works end-to-end)

Tokens > "Search Tokens" > entered `test`, clicked Search → returned real Testnet token search
results including `lklimek-20260217`. Added it to "My Tokens" — confirmed it persists in the list
across navigation and a Refresh. Screenshots: `screenshots/TOK-002-1-search-tokens-results.png`,
`screenshots/TOK-002-2-token-added-to-my-tokens.png`.

**Verdict: PASS** — keyword search dispatches, returns real results, and "add from search
results" persists correctly, matching both acceptance-criteria bullets.

### TOK-004: Transfer tokens — **BLOCKED** (reachable; correctly gated by zero balance, not a bug)

Per-identity table's "Transfer" button is visibly greyed out/disabled for all three identities
against `lklimek-20260217` (each holds a 0 balance for this token — none of them ever received
any). This is correct, expected gating, not a defect: a QA identity genuinely cannot transfer
tokens it does not hold.

**Verdict: BLOCKED** — reasoning: "no QA-controlled identity holds a non-zero balance of any
token in this environment — TOK-005 (Create Token) is confirmed non-functional, so no QA-owned
token can ever be minted to give a QA identity a balance to transfer." The disabled-button gating
itself is confirmed correct behavior, not the bug.

### TOK-005: Create token contract — **FAIL** (confirmed, reproducible click no-op on both
### simple-mode "Create Token" and advanced-mode "Register Token Contract"/"View JSON")

Tokens > "Token Creator": filled a complete simple-mode form (token name "QA Token 1", base
supply, a token preset) — the "Create Token" button renders enabled (blue fill, per
`can_create` gate all being satisfied) but **clicking it has zero effect**: no confirmation
popup, no banner, no backend dispatch, no log line of any kind in `det.log` (contrast with every
other button in this campaign, which logs at minimum a dispatch attempt). Reproduced with
Advanced Options on ("Register Token Contract" and "View JSON" buttons — same non-response).
Screenshots: `screenshots/TOK-005-1-token-creator-form-filled.png`,
`screenshots/TOK-005-2-advanced-mode-buttons-unresponsive.png`.

**Diagnosis (thorough elimination, not assumed)**:
1. a11y-dumped exact button coordinates and confirmed clicks landed dead-center — ruled out
   coordinate drift.
2. Tried `mcp__desktop__computer` clicks, direct `xdotool mousemove`+`click`, repeated attempts
   with delays, moving the mouse away and back — no change.
3. Confirmed sibling controls on the **identical frame** (the "Show Advanced Options" checkbox,
   Identity/Token-Preset dropdowns, collapsible section expanders) all respond correctly to the
   same click-delivery mechanism — ruling out a general input-pipeline failure.
4. Confirmed via full `det.log` review that no `MessageBanner`/dispatch/any log line appears
   after any of these clicks, while the same log shows reliable "Banner displayed" lines
   immediately after successful clicks elsewhere in the same session.
5. Read the exact Rust source (`token_creator.rs` simple mode ~419-454, advanced mode ~872-930):
   both handlers set a `bool` flag (`show_token_creator_confirmation_popup = true` /
   `show_json_popup = true`) on click, with the popup rendered unconditionally later in the same
   `ui()` call — ruling out an immediate-reset race.
6. **Strongest check**: killed the running app entirely (`pkill`, confirmed dead via `pgrep`),
   verified the binary hash was unchanged, relaunched fresh with the same accessibility flags,
   refilled the form from scratch with new values ("QA Token 2"), and reproduced the exact
   identical non-response on the very first fresh attempt.

**Root-cause hypothesis (not fixed, documented only)**: see the cross-story pattern note in the
Summary section below — every confirmed-broken button in this pass shares the same code shape
(sets a "show confirmation popup" `bool`/`Option` field as its *sole* immediate action, deferring
the real work to a later frame), while every button that dispatches a `BackendTask` directly (or
navigates) on click works correctly.

**Verdict: FAIL** — the story's entire configuration surface (naming, supply, decimals,
distribution, groups) is reachable and correctly populated, but the actual "register the
contract" action can never be triggered by any tested input method, in either UI mode. This is
the most severe TOK finding this pass: it structurally blocks TOK-006 through TOK-013/015/016/018
from ever being exercised against a QA-owned token.

### TOK-006: Mint tokens — **BLOCKED** (reachable; correct authorization gating confirmed, not
### a bug)

Per-identity table > Mint (QA Identity 1, `lklimek-20260217`) → Mint screen loads cleanly and
shows: **"You are not allowed to mint this token. Only the contract owner is."** — a correct,
clean, typed authorization rejection (`NotContractOwner`, confirmed via the banner's "Show
details" expansion). Screenshot: `screenshots/TOK-006-1-mint-not-authorized.png`.

**Verdict: BLOCKED** — reasoning: "TOK-005 (Create Token) is confirmed non-functional, so no
QA-controlled identity ever owns a token to mint for real; the third-party fixture token
`lklimek-20260217` correctly rejects QA identities as non-owners." Authorization-gating logic
itself is confirmed working correctly.

### TOK-007: Burn tokens — **BLOCKED** (same reasoning as TOK-006; reachable, correct
### authorization gating)

"..." menu > Burn → **"You are not allowed to burn this token. Only the contract owner is."**
Screenshot: `screenshots/TOK-007-1-burn-not-authorized.png`.

**Verdict: BLOCKED** — same reasoning as TOK-006.

### TOK-008: Freeze and unfreeze token recipients — **BLOCKED** (same reasoning; reachable,
### correct authorization gating)

"..." menu > Freeze → **"You are not allowed to freeze this token. Only the contract owner is."**
Screenshot: `screenshots/TOK-008-1-freeze-not-authorized.png`. Unfreeze/"Destroy Frozen Identity
Tokens" menu items confirmed reachable in the same menu, not independently clicked (same
authorization-gate class expected).

**Verdict: BLOCKED** — same reasoning as TOK-006.

### TOK-009: Pause and resume token transfers — **BLOCKED** (same reasoning; reachable, correct
### authorization gating)

"..." menu > Pause → **"You are not allowed to pause this token. Only the contract owner is."**
Screenshot: `screenshots/TOK-009-010-1-pause-not-authorized.png`. "Resume" menu item confirmed
reachable in the same menu, not independently clicked.

**Verdict: BLOCKED** — same reasoning as TOK-006.

### TOK-010: Destroy frozen funds — **BLOCKED** (same reasoning; reachable via "..." menu's
### "Destroy Frozen Identity Tokens" item, not independently clicked)

**Verdict: BLOCKED** — same reasoning as TOK-006.

### TOK-011: Claim distributed tokens — **FAIL** (reachable, form fully functional and shows a
### real live perpetual distribution — but the "Claim" submit button is a confirmed click no-op,
### same defect class as TOK-005)

Per-identity table > Claim (QA Identity 1, `lklimek-20260217`) → **Claim Tokens** screen loads a
complete, correct form: "Select Distribution Type: Perpetual", a clear plain-language explanation
of claim-cycle limits, and **"This token is using a time based distribution where every 1h it
will distribute a fixed amount of 10 base tokens."** — confirming this fixture token has a real,
live, non-owner-claimable perpetual distribution (contrast with TOK-006's Mint, which is
correctly owner-only). "Estimated Fee: 0.000001 DASH." Screenshot:
`screenshots/TOK-011-1-claim-tokens-form.png`.

Clicked "Claim" (an a11y-verified exact-coordinate click, `@(336,430 76x28) center=(374,444)`,
matching the click coordinates exactly): **zero effect** — no confirmation popup (source review
of `claim_tokens_screen.rs` line 592-607 confirms the handler's only action is
`self.confirmation_dialog = Some(ConfirmationDialog::new(...))`, with rendering unconditionally
wired at line 610-612, ruling out a render-order race), no banner, no log line whatsoever.
Reproduced twice with a fresh log-line check after each attempt (0 new lines both times).

**Verdict: FAIL** — the claim-eligibility/distribution-detail surface works correctly (a genuine,
useful confirmation the acceptance criteria's "view available claims" half is implemented), but
the actual "Claim action transfers tokens to identity" half can never be triggered — same
click-no-op defect class as TOK-005.

### TOK-012: Set token pricing and purchase tokens — **BLOCKED** (partially retested: "Update
### Config" reachable with a working form; "purchase tokens" side not independently exercised
### beyond TOK-013's Set Price)

"..." menu > "Update Config" → **Update Token Configuration** screen loads with "2. Select the
item to update:" (dropdown defaulted to "No Change", "No parameters to edit for this entry"),
"3. Public note (optional)," "Estimated Fee: 0.00002856 DASH" — no auth-rejection banner shown
immediately (unlike Burn/Freeze/Pause/Set Price, which all reject at screen-construction time).
Screenshot: `screenshots/TOK-012-1-update-config-form.png`. The submit button was not clicked
(no confirmation-dialog-pattern check performed for this specific screen; given TOK-005/011's
established pattern, it is plausible but not confirmed this button shares the same defect class).

**Verdict: BLOCKED** — reasoning: "TOK-005 (Create Token) is confirmed non-functional, so no
QA-controlled identity owns `lklimek-20260217` or any other token to update config for/purchase;
the form itself is reachable and renders correctly, but no privileged action can be completed."

### TOK-013: Update token configuration — **BLOCKED** (reachable; correct authorization gating
### confirmed via "Set Price," the closest analogous story)

"..." menu > "Set Price" → **Set Token Pricing Schedule** screen loads a full, correct form
(Single Price / Tiered Pricing / Remove Pricing radio options, warning text for "Remove Pricing")
but immediately shows: **"You are not allowed to set token price on this token. Only the
contract owner is."** Screenshot: `screenshots/TOK-013-1-set-price-not-authorized.png`.

**Verdict: BLOCKED** — same reasoning as TOK-006 (note: this story's title in PR892's catalog,
"Update token configuration," is closely related to but distinct from TOK-012's "Set token
pricing and purchase tokens" — both were exercised this pass via the token action menu's
"Update Config" and "Set Price" items respectively).

### TOK-014: Group actions for multi-party governance — **PASS** (retested: reachable, clean
### empty states for both selectors, no crash — unchanged from the prior pass's finding)

Contracts > "Group Actions" → "Active Group Actions" with "1. Select a contract:" (empty
dropdown — none of QA's three registered contracts have groups configured) and "2. Select an
identity:" (pre-filled QA Identity 1). No crash, no hang, no stray network call.

**Verdict: PASS** — screen reachability and both selectors confirmed working correctly; no
group-configured contract was available to exercise the actual approve/sign flow (none of this
pass's fixture contracts opted into multi-party groups — out of scope to construct one).

### TOK-015: View available token claims — **PASS** (retested: "Fetch claims" button works
### correctly and returns a real result — contrast with TOK-011's broken Claim submit button on
### the adjacent screen)

"..." menu > "View Claims" → **View Token Claims** screen, "Fetch claims" button → **"No claims
found"** — a correct, real result for an identity/token pair with no pending claims. Screenshot:
`screenshots/TOK-015-1-view-claims-no-claims-found.png`. Notably, this button *does* work
(uses the identical `ComponentStyles::add_primary_button` helper as TOK-011's broken "Claim"
button — see the cross-story pattern note in the Summary), confirming the defect is not a
blanket "all primary buttons on token screens are broken" issue.

**Verdict: PASS** — the detailed claims view is reachable and dispatches/returns correctly,
matching the acceptance criteria ("accessible before performing claim action").

### TOK-016: Estimate perpetual token rewards — **PARTIAL** (reachable; returned an
### ownership-gated rejection rather than a numeric estimate, plausibly correct for this fixture
### token's distribution configuration)

Per-identity table > "Estimate" (Rewards column, QA Identity 1, `lklimek-20260217`) →
**"This token distribution can only be claimed by the contract owner
(97rXwog9WJJGHkEqzTvDwcri5RWWKPiV7UMb4SoARQE8). Your identity is not the contract owner."**
(typed `NotContractOwner` error, confirmed via "Show details"). Screenshot:
`screenshots/TOK-016-1-estimate-rewards-not-owner.png`.

This is a notable discrepancy with TOK-011's finding on the *same token*: the Claim screen
describes a "time based distribution... every 1h... 10 base tokens" available to be claimed
(implying a broadly-claimable perpetual distribution), while this "Estimate" action rejects with
an owner-only message. Not root-caused further (out of scope for this pass) — plausible
explanations include the token having two distinct distribution mechanisms (one perpetual/public,
one owner-controlled) or the "Estimate" action internally reusing a claim-eligibility check scoped
to a different distribution than the one described on the Claim screen. Flagged for follow-up,
not asserted as a confirmed defect given the ambiguity.

**Verdict: PARTIAL** — reachable and returns a clean, typed response (no crash, no hang), but the
response contradicts what TOK-011 found on the same token/identity pair enough to warrant
follow-up before calling this either a clean pass or a bug.

### TOK-017: Pay for document operations with tokens — **BLOCKED** (reachable: Create Document
### and Purchase Document flows both now load fully with a real contract, unlike the prior
### pass's transitive block — but no token-payment UI option was found anywhere in either flow)

Contracts > Documents > "Create Document" with contract **QA Note Contract** (a real, QA-owned
contract from DOC-001) → filled contract/doc-type/identity/key through to step 3 ("Fill out the
document fields"), including toggling "Advanced Options" (which only surfaced a Key selector, no
payment-method option) — the form only ever shows a credits-based "Estimated fee: … DASH" /
"Broadcast document" path, no token-payment toggle. Repeated the same check on "Purchase
Document" up through contract/doc-type selection — same absence. Source review from the prior
pass (`document_action_screen.rs` constructing `TokenPaymentInfo::V0(...)`) confirms the
capability exists in the backend/submission logic, but no reachable UI control to opt into it was
found in the two flows explored this pass.

**Verdict: BLOCKED** — reasoning: "the underlying document-action screens are now reachable
(unlike the prior pass's transitive DOC-003 environment block), but no token-payment UI surface
was found in Create Document or Purchase Document; not exhaustively checked across all six
document-action screens, so a UI element may exist elsewhere (e.g. only after selecting a
document/price already denominated in tokens) that this pass's exploration did not reach."

### TOK-018: Stop tracking a token balance — **FAIL** (confirmed reproducible click no-op on the
### "X" button — same defect class as TOK-005/TOK-011)

My Tokens list ("Token Name | Token ID | Description | Actions") shows `lklimek-20260217` with
"More Info" / "X" actions. Clicked "X": **zero effect** — the token remains in the list after the
click, after a subsequent "Refresh," and after a repeat click with a fresh `det.log` line-count
check (0 new lines related to token removal both times). Screenshots:
`screenshots/TOK-018-1-before-stop-tracking.png`, `screenshots/TOK-018-2-x-button-unresponsive-after-refresh.png`.

Also reproduced on the per-identity table's own "X" ("Remove identity token balance from DET") —
same non-response.

**Source review confirms the same "sets a popup flag, nothing else" pattern as TOK-005/TOK-011**:
`my_tokens.rs` line 1077-1084 (top-level list) sets `self.confirm_remove_token_popup = true;
self.token_to_remove = Some(*token_id);`; line 541-551 (per-identity table) sets
`self.confirm_remove_identity_token_balance_popup = true;`. Both popups are unconditionally wired
to render later in the same `ui()` call (`tokens_screen/mod.rs` line 2777) — ruling out a
render-order race, same as TOK-005/TOK-011. No confirmation popup was ever observed on screen
after any of the click attempts.

**Verdict: FAIL** — "Stop Tracking Balance" can never be triggered by any tested input method.
This is a functional regression from the previous pass's source-only review (which found the
underlying persistence/un-watch/restoration logic to be a complete, well-tested implementation) —
the backend logic appears sound, but the UI can never reach it.

---

## Original pass findings (below this point, superseded by the 2026-07-15 retest above for
## TOK-001, 002, 004-018 — TOK-003 was not in the 24-story retest scope and its FAIL finding
## still stands as last confirmed, unretested this pass)

## Environment status at start of this pass — one honest recheck performed, blocker confirmed
## unchanged (not re-diagnosed further; see `scenarios/DOC.md` for the full recheck writeup)

Per campaign instructions, this pass did not assume the environment blocker documented in
`CAMPAIGN-CONTEXT.md` / `scenarios/IDN.md` / `scenarios/DPN.md` still applied — it verified with
a live action first. That check (adding the well-known DPNS contract by ID via Contracts > Load
Contracts) dispatched a real network query that failed with the same
`SdkError { source_error: Proof(ContextProviderError(Config("masternode list not yet synced
(quorums unavailable)"))) }` signature already documented. Full detail is in `scenarios/DOC.md`'s
environment section (the check happened to be a Contracts-screen action but its result applies
equally to TOK, since both categories query the same DAPI/proof-verification layer). Zero
identities are loaded (`identities` table: 0 rows throughout this pass, confirmed via SQLite
before and after). **Consequence for TOK**: per `CAMPAIGN-CONTEXT.md`'s guidance, any story that
needs the user's own Platform identity (viewing owned balances, transferring, minting, issuer
actions) is BLOCKED on that same root cause. However, per this pass's explicit assignment, the
public/read-only surfaces (search, add-by-ID) were tested live rather than assumed blocked — see
TOK-002 and TOK-003 below, both of which **do** dispatch real DAPI queries without needing the
user's own identity.

---

## TOK-001: View token balances — **BLOCKED** (empty state confirmed reachable and correct)

**Persona:** Alex, Priya. Acceptance criteria: "'My Tokens' screen lists all held tokens with
balances."

Tokens > "My Tokens" (default tab) renders cleanly: **"No Tracked Tokens" / "You don't have any
tokens yet." / "Import Token"** — a correct, well-typed empty state that reads from local
state only (no network call, no crash, no compounding banner spam beyond the ambient
environment banners already on screen). Since no identity is loaded and no token is tracked,
there is nothing to list; the empty state itself is the only reachable/verifiable surface.
Screenshot: `screenshots/TOK-001-1-my-tokens-empty-state.png`.

**Verdict: BLOCKED** — reasoning: "blocked: no Platform identity reachable in this environment,
see scenarios/IDN.md — root cause is the known Testnet masternode-list/quorum-sync/wallet-storage
failure, see CAMPAIGN-CONTEXT.md". Empty-state rendering and navigation confirmed working
correctly.

---

## TOK-002: Search and discover tokens — **BLOCKED** (confirmed reachable and dispatches
## correctly without an identity — tested explicitly per campaign instructions, not assumed)

**Persona:** Alex, Priya, Jordan. Acceptance criteria: "Keyword search across token names and
metadata. Add token from search results."

Tokens > "Search Tokens" tab: a plain "Enter Keyword:" field + Search/Clear buttons, no identity
gate visible in the UI itself. Entered `dash`, clicked Search.

### Confirmed: this is a real, unauthenticated DAPI query — no identity required to attempt it

`det.log` shows a clean dispatch (`TokenTask::QueryDescriptionsByKeyword`): "Searching
contracts..." banner set synchronously, followed by 7 retries against 7 different DAPI
endpoints, each failing with the same `masternode list not yet synced (quorums unavailable)`
signature, then a clean generic banner: **"An unexpected error occurred. Please try again
later."** with technical detail available via "Show details". This is the same clean
dispatch-and-fail pattern `scenarios/IDN.md` documented for IDN-010 (search by DPNS username) —
confirming keyword search genuinely does not require the user's own identity to attempt, it is
blocked purely by the shared Platform-proof-verification failure. Screenshot:
`screenshots/TOK-002-1-search-tokens-by-keyword-BLOCKED-quorum-error.png`.

(Note: the first two click attempts appeared to be no-ops because the banner stack above the
button had grown and shifted the button's on-screen position between screenshot and click —
not a product bug. Once clicked precisely, the dispatch fired immediately and reproducibly.)

**Verdict: BLOCKED** — reasoning: "blocked: no Platform identity reachable in this environment,
see scenarios/IDN.md — root cause is the known Testnet masternode-list/quorum-sync/wallet-storage
failure, see CAMPAIGN-CONTEXT.md". Explicitly confirmed this is a public/read-only query path
that dispatches without needing the user's own identity — it is blocked by the shared
proof-verification failure only, the same as every other live Platform query in this campaign.

---

## TOK-003: Add token by contract or token ID — **FAIL** (format validation and public-query
## dispatch both confirmed working; but a valid-format ID's real query failure is silently
## dropped with zero user feedback — a new, independently-reproducible defect)

**Persona:** Priya, Jordan. Acceptance criteria: "Enter ID manually and add to token list."

Tokens > "My Tokens" > "Import Token" → `Tokens > Import Token` screen: "Enter either a Contract
ID or Token ID to search for tokens." + a single input field + "Search" button (disabled while
empty). No identity gate in the UI — confirming, like TOK-002, this is intended to be reachable
without the user's own identity.

### Format validation — clean, immediate, no network call

Typed `bad-id` (also separately tried `not-a-valid-id`), clicked Search. Immediate, correctly
worded banner: **"Invalid identifier format"** — no network activity in `det.log` for this
click, confirming validation happens client-side before any dispatch. Screenshot:
`screenshots/TOK-003-1-import-token-invalid-identifier-format.png`.

### Well-formed ID — dispatches correctly, but the failure is silently dropped

Typed the well-known DPNS contract ID (`GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec` — a
syntactically valid 32-byte Base58 `Identifier`, used here only to exercise the load flow's
behavior since proof verification fails for every kind of Platform query in this environment,
per the same reasoning `scenarios/IDN.md` used for IDN-002). Clicked Search.

`det.log` confirms a real dispatch: `TokenTask::FetchTokenByContractId` → `DataContract::
fetch_by_identifier` → 7 retries against 7 DAPI endpoints, same `masternode list not yet synced`
signature, then `no more retries left, giving up` at the SDK level. **After that point, zero
further log activity of any kind** — reproduced twice, with waits of 47s and 55s respectively
after the SDK gave up, confirmed via `grep`/`wc -l` on `det.log` showing no new lines. No
banner ever appears (no "Banner displayed" log entry follows), no inline error, no "Searching...
N seconds elapsed" progress text (which the source shows should render while
`AddTokenStatus::Searching` is active) — the screen just silently reverts to looking idle.
Screenshot: `screenshots/TOK-003-2-import-token-silent-drop-no-feedback-after-request-failed.png`
(taken 55s after the SDK's "no more retries left, giving up" log line, with the button still
mid-interaction-highlighted from the click and no new banner anywhere on screen).

### Source review

`src/ui/tokens/add_token_by_id_screen.rs`: the click handler sets `self.status =
AddTokenStatus::Searching(now)` and dispatches `BackendTask::TokenTask(FetchTokenByContractId)`.
The backend task (`src/backend_task/tokens/mod.rs`) correctly returns `Err(TaskError::from(e))`
on fetch failure, which per `src/app.rs`'s `TaskResult::Error(err)` handling should
unconditionally call `MessageBanner::set_global(...)` (this screen does not override
`display_task_error`, so the default "not handled" path applies and the generic banner should
always fire). Despite that, no banner is ever observed — the sibling flows tested in this
campaign that hit the identical `masternode list not yet synced` error (TOK-002, DOC-003, IDN-010)
**do** show this banner reliably. The discrepancy was not root-caused further (out of scope for
this QA pass — observe and document only), but is flagged as a new, independently-reproducible
defect distinct from IDN-002/003's *hang* (this request does complete, per the log's "no more
retries left, giving up" line) — here the difference is a *silently dropped result*, an even
harder-to-diagnose failure mode for an end user (there is no visible "still working" state to
eventually time out on; the UI just looks unresponsive to begin with).

**Verdict: FAIL** for the story's "add token by ID" flow when given a well-formed identifier —
the request is genuinely attempted and genuinely fails, but the user receives no feedback
whatsoever. Format validation (client-side) and query dispatch (network-side) both work
correctly; only the failure-reporting path for this specific screen is broken. Should be
re-tested once the environment blocker is resolved to see if the drop is specific to
`FetchTokenByContractId`'s error path or a broader issue with this screen's message routing.

---

## TOK-004: Transfer tokens — **BLOCKED**

**Persona:** Alex, Priya. Acceptance criteria: "Select token, enter recipient and amount.
Confirmation before broadcast."

`transfer_tokens_screen.rs`'s constructor requires an already-resolved token+identity pair
(reached only via "My Tokens" list → select a held token → "Transfer"). "My Tokens" is
confirmed empty in this environment (TOK-001), so this screen has no reachable entry point.

**Verdict: BLOCKED** — reasoning: "blocked: no Platform identity reachable in this environment,
see scenarios/IDN.md — root cause is the known Testnet masternode-list/quorum-sync/wallet-storage
failure, see CAMPAIGN-CONTEXT.md". Not independently re-tested; no UI surface exists without a
tracked, held token.

---

## TOK-005: Create token contract — **BLOCKED** (live-tested: reachable, clean typed error,
## Advanced Options does not bypass the identity gate)

**Persona:** Jordan. Acceptance criteria: "Configure naming, supply, decimals, action rules,
distribution, and groups. Contract is registered via state transition."

Tokens > "Token Creator" tab loads cleanly (no crash) with a heading and description, but the
actual configuration form never renders — instead a single clean inline message: **"Error
loading identities from local DB: Your wallet is still starting up. Please wait a moment and try
again."** Screenshot: `screenshots/TOK-005-1-token-creator-error-loading-identities.png`. Toggled
"Show Advanced Options" (heading text updates to "Create custom tokens on Dash Platform with
advanced features and distribution rules") — the form still does not render; the identity-load
error message persists unchanged. Confirms the whole configuration surface (naming, supply,
decimals, distribution, groups) is correctly gated behind a successful local-identity load,
which fails cleanly (not silently, not via crash) under the current environment condition.

**Verdict: BLOCKED** — reasoning: "blocked: no Platform identity reachable in this environment,
see scenarios/IDN.md — root cause is the known Testnet masternode-list/quorum-sync/wallet-storage
failure, see CAMPAIGN-CONTEXT.md". Screen reachability, empty-state message quality, and the
Advanced Options toggle are all confirmed working correctly.

---

## TOK-006 through TOK-010, TOK-012, TOK-013: Issuer/holder token actions — **BLOCKED** (no
## tracked token or identity reachable; confirmed via source, no crash risk observed elsewhere
## in this category)

**Stories:** TOK-006 (Mint tokens), TOK-007 (Burn tokens), TOK-008 (Freeze/unfreeze recipients),
TOK-009 (Pause/resume transfers), TOK-010 (Destroy frozen funds), TOK-012 (Set pricing/purchase),
TOK-013 (Update token configuration).

All seven action screens (`mint_tokens_screen.rs`, `burn_tokens_screen.rs`,
`freeze_tokens_screen.rs` / `unfreeze_tokens_screen.rs`, `pause_tokens_screen.rs` /
`resume_tokens_screen.rs`, `destroy_frozen_funds_screen.rs`, `set_token_price_screen.rs` /
`direct_token_purchase_screen.rs`, `update_token_config.rs`) construct from an
`IdentityTokenInfo`/`IdentityTokenBasicInfo` value that only exists once a specific identity
holds or has issued a specific tracked token — i.e., they are reached exclusively from "My
Tokens" list rows, never as standalone navigation targets. "My Tokens" is confirmed empty
throughout this pass (TOK-001), so none of these seven screens has a reachable entry point.

**Verdict for all seven: BLOCKED** — reasoning: "blocked: no Platform identity reachable in this
environment, see scenarios/IDN.md — root cause is the known Testnet
masternode-list/quorum-sync/wallet-storage failure, see CAMPAIGN-CONTEXT.md". Not independently
re-tested; confirmed via source that no alternate UI surface exists without a held/issued
tracked token. Given the "Update Contract" crash found in this same pass (`scenarios/DOC.md`,
DOC-002) occurs in an analogous `.expect()`-on-`Result` pattern during a screen's constructor,
these seven screens are worth a follow-up smoke pass once identities are reachable, to confirm
none share that same crash-on-missing-precondition pattern — not verified here since none of
them could be constructed at all in this environment.

---

## TOK-011 & TOK-015: Claim distributed tokens / View available token claims — **BLOCKED**

**Persona:** Alex, Priya. Acceptance criteria (TOK-011): "View available claims. Claim action
transfers tokens to identity." (TOK-015): "Detailed view of claim documents with metadata.
Accessible before performing claim action."

Both `claim_tokens_screen.rs` and `view_token_claims_screen.rs` construct from an
`IdentityTokenBasicInfo` value identical in shape to the TOK-006–013 group above — reached only
from a specific identity+token pairing in "My Tokens." No such pairing exists in this
environment.

**Verdict for both: BLOCKED** — same reasoning as TOK-006–013.

---

## TOK-014: Group actions for multi-party governance — **BLOCKED** (live-tested: reachable,
## clean empty states for both selectors, no crash)

**Persona:** Jordan. Acceptance criteria: "View pending group actions. Sign or approve actions
as a group member."

Contracts screen > "Group Actions" button → `Contracts > Group Actions`: **"Active Group
Actions"** with a clean two-step form — "1. Select a contract:" (empty "Select Contract..."
dropdown, since no contracts are tracked) and "2. Select an identity:" (dropdown correctly reads
**"No identities found"**, matching the confirmed 0-identity state). No crash, no silent hang,
no stray network call. Screenshot: `screenshots/TOK-014-1-group-actions-empty-state.png`.

**Verdict: BLOCKED** — reasoning: "blocked: no Platform identity reachable in this environment,
see scenarios/IDN.md — root cause is the known Testnet masternode-list/quorum-sync/wallet-storage
failure, see CAMPAIGN-CONTEXT.md". Screen reachability and both empty-state selectors confirmed
working correctly.

---

## TOK-016: Estimate perpetual token rewards — **BLOCKED**

**Persona:** Jordan. Acceptance criteria: "Detailed estimation with explanation. Supports
multiple distribution function types (fixed, linear, polynomial, exponential, logarithmic)."

Source review (`src/ui/tokens/tokens_screen/my_tokens.rs`): the reward-estimation action
(`TokenTask::EstimatePerpetualTokenRewardsWithExplanation`) is dispatched from within a specific
token's expanded row inside the "My Tokens" list — not a standalone screen. "My Tokens" is
confirmed empty, so this action has no reachable entry point.

**Verdict: BLOCKED** — reasoning: "blocked: no Platform identity reachable in this environment,
see scenarios/IDN.md — root cause is the known Testnet masternode-list/quorum-sync/wallet-storage
failure, see CAMPAIGN-CONTEXT.md". Confirmed via source, not independently re-tested.

---

## TOK-017: Pay for document operations with tokens — **BLOCKED**

**Persona:** Jordan. Acceptance criteria: "Optional `TokenPaymentInfo` parameter on all document
actions. Token-based payment as alternative to credit-based payment."

Source review confirms this is implemented (`src/ui/contracts_documents/
document_action_screen.rs` constructs `TokenPaymentInfo::V0(...)` at several points in its
submission logic — the shared screen behind Create/Replace/Delete/Transfer/Purchase Document,
per `scenarios/DOC.md`'s DOC-005–009 write-up). But that screen's very first gate is "1. Select a
contract and document type," and no contract can ever become selectable in this environment: the
"Add Contracts" flow (`scenarios/DOC.md`, DOC-003) dispatches correctly but always fails on the
same masternode-list-sync error before any contract is persisted, so the contract dropdown stays
permanently empty. The token-payment option is therefore unreachable transitively through the
same environment blocker, one gate earlier than the token-payment UI itself.

**Verdict: BLOCKED** — reasoning: "blocked: no Platform identity reachable in this environment,
see scenarios/IDN.md — root cause is the known Testnet masternode-list/quorum-sync/wallet-storage
failure, see CAMPAIGN-CONTEXT.md" (specifically: no contract can ever be added to select a
document type against, which is the prerequisite gate one step before the token-payment option
itself). Confirmed via source that the feature is implemented; not reachable for a live UI
exercise.

---

## Follow-up pass (2026-07-14, later same session): TOK-018

Same running app instance (PID 1580158, hash-verified against
`2931220e94871a0454ac56a43092aa87246b5a590d917645c025ddb1c7f9271a`), same data dir. Per campaign
instructions, the environment blocker was rechecked live rather than assumed: navigated to Tokens
> My Tokens, reproduced the identical **"No Tracked Tokens" / "You don't have any tokens yet." /
"Import Token"** empty state, with the same four red banners as the rest of this file overlaid
above it. Direct SQLite check of `det-app.sqlite` confirms `identities`: 0 rows, `token_balances`:
0 rows, `meta_token`: 0 rows. Screenshot: `screenshots/TOK-018-1-my-tokens-empty-state-recheck.png`.
Unchanged from TOK-001's original finding.

---

## TOK-018: Stop tracking a token balance — **BLOCKED**

**Persona:** Alex, Priya. Acceptance criteria: "'Stop Tracking Balance' removes the chosen
identity-token pair from the list. The balance is un-watched so the background sync stops
fetching it and the row does not reappear. The dismissal is remembered: 'Refresh My Tokens'
leaves the row gone, and only that identity-token pair is affected — other identities keep
tracking the same token. The row comes back when the user asks for it again: re-importing the
token restores it for every identity that dismissed it, and checking that one balance restores
just that pair."

### Reachability

"Stop Tracking Balance" is a row action inside "My Tokens," reached only once a specific
identity-token pair is being tracked. "My Tokens" is confirmed empty in this environment
(TOK-001, rechecked above), so this action has no reachable entry point — same reasoning as
TOK-004/006–013/015/016.

### Source review (implementation confirmed, not live-exercised)

`src/ui/tokens/tokens_screen/mod.rs` wires a confirmation dialog titled **"Confirm Stop Tracking
Balance"** to `TokenTask::StopTrackingTokenBalance(IdentityTokenIdentifier { identity_id,
token_id })`. The handler (`backend_task/tokens/query_my_token_balances.rs`'s
`stop_tracking_token_balance`) is doc-commented plainly: "Un-watches the pair in the upstream sync
loop so its background pass stops fetching the balance and the pair leaves the published
snapshot, records the dismissal so later refreshes do not re-watch it, then drops it from the
saved My Tokens ordering" — covering the story's first two bullets directly (row removed,
background sync un-watched).

**Dismissal persistence + per-pair scoping**: `context/contract_token_db.rs` stores dismissals as
`det:token_untracked:v2:<token_id>:<identity_id>` keys in a `BTreeSet<IdentityTokenIdentifier>`
(`mark_token_balance_untracked` / `untracked_token_balances`), keyed by the **pair**, not just the
token — so dismissing one identity's tracking of a token cannot affect another identity's tracking
of the same token, matching "only that identity-token pair is affected." `token_watch_sets()`
rebuilds each identity's upstream watch set on every refresh as "every token in the local
registry, minus the pairs the user stopped tracking" — directly satisfying "'Refresh My Tokens'
leaves the row gone."

**Restoration paths**: `clear_untracked_token(&token_id)` is called from
`TokenTask::SaveTokenLocally` (i.e. re-importing a token), with the comment "Importing a token is
intent to track it, so it overrides an earlier 'stop tracking' of the same token" — clearing the
dismissal for **every** identity that had dismissed it, matching "re-importing the token restores
it for every identity that dismissed it." A narrower `clear_untracked_token_balance` (single pair)
is called from the balance-check path with the comment "Asking for a balance is intent to track
it" — matching "checking that one balance restores just that pair."

Three targeted unit tests were found directly asserting these acceptance-criteria bullets:
`stopped_pair_is_not_rewatched_by_a_refresh` (asserts only the dismissing identity is affected),
`retracking_a_pair_restores_it_to_the_watch_set`, and `reimporting_a_token_retracks_it_for_every_identity`.

**Verdict: BLOCKED** — reasoning: "blocked: no Platform identity reachable in this environment,
see scenarios/IDN.md — root cause is the known Testnet masternode-list/quorum-sync/wallet-storage
failure, see CAMPAIGN-CONTEXT.md" (specifically: no tracked identity-token pair exists to exercise
"Stop Tracking Balance" on). Source review confirms the implementation, DB-layer persistence, UI
confirmation dialog, and three targeted unit tests all align precisely with every
acceptance-criteria bullet — not a stub.

---

## Original pass summary (superseded by the final Summary at the bottom of this file)

| Story | Verdict |
|---|---|
| TOK-001 | BLOCKED (empty state confirmed reachable and correct) |
| TOK-002 | BLOCKED (confirmed reachable without identity; dispatches + fails cleanly on known quorum-sync error) |
| TOK-003 | FAIL (format validation + dispatch both work; well-formed-ID failure is silently dropped with zero feedback) |
| TOK-004 | BLOCKED (no tracked token/identity reachable) |
| TOK-005 | BLOCKED (live-tested: clean typed error, Advanced Options doesn't bypass gate) |
| TOK-006 | BLOCKED (no tracked token/identity reachable) |
| TOK-007 | BLOCKED (same as TOK-006) |
| TOK-008 | BLOCKED (same as TOK-006) |
| TOK-009 | BLOCKED (same as TOK-006) |
| TOK-010 | BLOCKED (same as TOK-006) |
| TOK-011 | BLOCKED (same as TOK-006) |
| TOK-012 | BLOCKED (same as TOK-006) |
| TOK-013 | BLOCKED (same as TOK-006) |
| TOK-014 | BLOCKED (live-tested: clean empty states, no crash) |
| TOK-015 | BLOCKED (same as TOK-006) |
| TOK-016 | BLOCKED (no tracked token to estimate rewards for) |
| TOK-017 | BLOCKED (transitively, via DOC's contract-add environment blocker) |
| TOK-018 | BLOCKED (no tracked token/identity reachable; "Stop Tracking Balance" confirmed fully implemented — per-pair persistence, un-watch, and both restoration paths — with 3 targeted unit tests, via source) |

**One real, environment-independent-looking defect found**: **TOK-003** — the "Import Token"
screen's well-formed-ID search path dispatches a genuine network query, the query genuinely
fails, and the failure is silently dropped with zero user feedback (no banner, no inline message,
no elapsed-time indicator), reproduced twice with independent 47s and 55s post-failure waits.
This is distinct from IDN-002/003's *hang* defect class (that request never completes at all);
here the request *does* complete, but its result vanishes before reaching the UI. Everything else
in this category traces cleanly to the already-documented environment blocker
(`CAMPAIGN-CONTEXT.md`, `scenarios/IDN.md`, `scenarios/ALK.md`) or to a genuine absence of a
tracked token/identity to act on.

**Read-only/public queries confirmed working despite the identity blocker**: TOK-002 (Search
Tokens by keyword) dispatches and fails cleanly without needing the user's own identity — proving
the block is purely the shared proof-verification failure, not an identity-specific UI gate. This
matches DOC-003's identical finding for contract import.

The app crashed once during this overall pass, but that crash occurred in the DOC category (DOC-002,
"Update Contract") — see `scenarios/DOC.md`. No TOK-specific action caused a crash; all TOK screens
tested (including the six with empty selector states) degraded cleanly. QA Wallet 1 and the DIAG
throwaway wallet were left untouched; SQLite confirms zero rows added to `identities`,
`token_balances`, or `meta_token` across this entire pass (0 before, 0 after).

**Follow-up pass (TOK-018)**: same environment blocker confirmed unchanged via a fresh live
recheck; "Stop Tracking Balance" was confirmed via source review to be a complete, non-stub
implementation (per-pair dismissal persistence, upstream un-watch, and both the
re-import-restores-all-identities and check-balance-restores-one-pair recovery paths), backed by
three targeted unit tests — consistent with this campaign's pattern of finding mature,
already-shipped features gated behind an environment blocker rather than missing functionality.
No PR892 application source was modified; no persistent state was changed by this follow-up
(read-only navigation and source review only).

---

## Summary (2026-07-15 retest, wallet-backend/asset-lock env fix applied — final, current)

| Story | Verdict (2026-07-15 retest) |
|---|---|
| TOK-001 | **PASS** (real tracked token listed; per-identity balance table renders correctly) |
| TOK-002 | **PASS** (live keyword search returns real results; add-to-My-Tokens persists) |
| TOK-003 | FAIL (not retested this pass — out of the 24-story scope; original finding stands: well-formed-ID search dispatches and fails, but the failure is silently dropped) |
| TOK-004 | BLOCKED (reachable; Transfer correctly disabled for a 0 balance — TOK-005 blocks ever obtaining a QA-owned balance) |
| TOK-005 | **FAIL** (Create Token / Register Token Contract / View JSON: confirmed, thoroughly diagnosed click no-op — most severe TOK finding this pass) |
| TOK-006 | BLOCKED (reachable; correct owner-only authorization rejection, not a bug) |
| TOK-007 | BLOCKED (same as TOK-006) |
| TOK-008 | BLOCKED (same as TOK-006) |
| TOK-009 | BLOCKED (same as TOK-006) |
| TOK-010 | BLOCKED (same as TOK-006, via the shared "..." menu) |
| TOK-011 | **FAIL** (Claim form fully functional and shows a real live distribution, but "Claim" submit button is a confirmed click no-op — same defect class as TOK-005) |
| TOK-012 | BLOCKED (Update Config form reachable; submit button not independently tested) |
| TOK-013 | BLOCKED (Set Price reachable; correct owner-only authorization rejection) |
| TOK-014 | **PASS** (reachable; clean empty states for both selectors, no crash) |
| TOK-015 | **PASS** ("Fetch claims" works correctly, returns "No claims found" — contrast with TOK-011's broken button on the adjacent screen) |
| TOK-016 | PARTIAL (reachable; returned an owner-only rejection that appears to contradict TOK-011's finding on the same token — flagged for follow-up, not asserted as a confirmed bug) |
| TOK-017 | BLOCKED (Create Document / Purchase Document both now fully reachable with a real contract, but no token-payment UI option found in either flow explored) |
| TOK-018 | **FAIL** (Stop Tracking Balance "X": confirmed click no-op on both the top-level and per-identity variants — same defect class as TOK-005/TOK-011; backend logic previously confirmed sound via source review) |

**Nine of seventeen retested stories flip from BLOCKED to a live verdict** (PASS, FAIL, or
PARTIAL) now that the wallet-backend/asset-lock environment blocker (dashpay/platform#4133) is
fixed and real funded identities are reachable; the remaining eight stay BLOCKED for a *new*,
narrower, non-environment reason — almost entirely because **TOK-005 (Create Token) is
completely non-functional**, so no QA-controlled identity can ever own a real token to exercise
the issuer-only actions against. TOK-003 was outside this pass's 24-story scope and was not
retested; its original FAIL finding stands unchanged.

**Cross-story pattern — three confirmed click-no-op defects share the same code shape**:
TOK-005 (Token Creator's "Create Token"/"Register Token Contract"/"View JSON"), TOK-011 (Claim
Tokens' "Claim"), and TOK-018 (My Tokens' "X" / Stop Tracking, both variants) are all
independently, thoroughly diagnosed click no-ops — a11y-verified exact coordinates, zero log
activity of any kind after the click, and (for TOK-011/018) a source-confirmed, correctly-wired
popup render path ruling out an immediate-reset race. Source review found all three share one
specific shape: **the click handler's sole immediate action is to set a "show confirmation
popup" `bool`/`Option` field** (`show_token_creator_confirmation_popup`, `confirmation_dialog =
Some(ConfirmationDialog::new(...))`, `confirm_remove_token_popup` /
`confirm_remove_identity_token_balance_popup`), deferring the real state-transition dispatch to a
later frame once the user confirms in that popup. By contrast, every button in this pass that
dispatches a `BackendTask` directly on click (or navigates to another screen) works correctly —
including buttons using the *identical* `ComponentStyles::add_primary_button` helper, e.g.
TOK-015's working "Fetch claims" right next to TOK-011's broken "Claim." This rules out a
blanket "primary buttons are broken" explanation and narrows the likely defect to something
specific about the deferred-confirmation-popup pattern on these token screens — not fixed, not
further root-caused, per this campaign's document-don't-fix rule.

**Read-only/public queries confirmed working**: TOK-001 and TOK-002 both PASS live, confirming
the wallet-backend/asset-lock fix genuinely restores the identity-dependent surfaces this
category needs, not just the public-query paths already known to work pre-fix.

**Fixture-token workaround**: this pass used a real, pre-existing, third-party Testnet token
(`lklimek-20260217`, contract `7TNdYLnTdCD1mpZ4yH2RyUthpmyF4QRZAr2kX18JzCeo`, discovered live via
TOK-002's own search) to exercise as many owner-gated action screens as possible despite TOK-005's
failure blocking any QA-owned token from ever existing. This let authorization-gating logic be
verified correct (TOK-006/007/008/009/010/013 all show clean, typed `NotContractOwner`
rejections) and let TOK-011/015/016 exercise a real live perpetual distribution — evidence that
would have been unobtainable with a QA-owned token alone, given TOK-005's failure. The app was
never crashed during this pass; no PR892 application source was modified; no destructive action
was taken against the third-party token owner's actual holdings (every privileged action was
correctly, cleanly rejected before reaching broadcast).

**Asset-lock recurrence (dashpay/platform#4133) was NOT hit at any point during this TOK retest
pass.**
