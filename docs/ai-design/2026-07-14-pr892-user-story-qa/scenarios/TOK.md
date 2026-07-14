# TOK — Token Operations

Environment: PR892 build (`/data/target/debug/dash-evo-tool` @ `57195d54`), isolated data dir
`/data/tmp/det-qa-pr892-data`, network Testnet, display `:99`, wallet `QA Wallet 1`. App was
already running (PID 989399) when this pass started; reused per campaign instructions. The app
**crashed mid-pass during DOC testing** (see `scenarios/DOC.md`, DOC-002) and was relaunched
(PID 1279253); all TOK testing after that point ran against the relaunched instance. Both
sessions showed the same environment blocker.

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

## Summary

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
