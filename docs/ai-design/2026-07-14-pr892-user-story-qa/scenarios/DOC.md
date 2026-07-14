# DOC — Contracts and Documents

Environment: PR892 build (`/data/target/debug/dash-evo-tool` @ `57195d54`), isolated data dir
`/data/tmp/det-qa-pr892-data`, network Testnet, display `:99`, wallet `QA Wallet 1`. App was
already running (PID 989399) when this pass started; reused per campaign instructions. **The app
crashed mid-pass** (see DOC-002 below) and was relaunched (PID 1279253, launched with the same
`DASH_EVO_TOOL_ACCESSIBILITY=1 DASH_EVO_DATA_DIR=/data/tmp/det-qa-pr892-data` env per
`CAMPAIGN-CONTEXT.md`'s recipe); remaining DOC and TOK testing ran against the relaunched
instance. Both sessions showed the same underlying environment blocker.

## Environment status at start of this pass — one honest recheck performed, blocker confirmed
## unchanged

Per this campaign's instructions, rather than assuming `CAMPAIGN-CONTEXT.md` / `scenarios/IDN.md`
/ `scenarios/DPN.md`'s documented blocker still applied, this pass verified it live first: on the
already-running instance (still showing the four-banner "worse than DEV.md's snapshot" state
`scenarios/IDN.md` and `scenarios/DPN.md` documented — wallet-storage-layer failure plus
masternode-list/quorum-sync failure), navigated to Contracts > Contracts > "Load Contracts" and
attempted to add the well-known DPNS system contract by ID
(`GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec` — the same fixture `scenarios/IDN.md` used,
chosen because it is a real, syntactically valid 32-byte Base58 `Identifier` from a deployed
system contract, not a masternode/identity guess). Result: **unchanged**. The "Adding
contract..." banner dispatched correctly, retried 7 times against 7 different DAPI endpoints, and
failed with the exact `SdkError { source_error: Proof(ContextProviderError(Config("masternode
list not yet synced (quorums unavailable)"))) }` signature already documented in
`scenarios/DEV.md`/`scenarios/IDN.md`. Screenshot:
`screenshots/TOK-DOC-000-environment-recheck-add-contract-quorum-error.png`. After the app was
relaunched following the DOC-002 crash, this was re-verified once more from a clean session
(`screenshots/DOC-003-1-add-contracts-BLOCKED-quorum-error.png`) with the identical failure
signature and dispatch behavior — the fresh session showed a shorter banner stack ("SPV sync
failed. Go to Settings for connection details." plus the standard three-banner wallet-startup
trio, rather than the compounded four-banner state from the older session) but the same
underlying `Failed to start chain sync error=The wallet service could not complete this
operation. Please retry in a moment.` failure `CAMPAIGN-CONTEXT.md` documents as the known,
open Testnet wallet-backend connectivity issue.

**Root cause**: known Testnet masternode-list/quorum-sync/wallet-backend failure, see
`CAMPAIGN-CONTEXT.md` and `scenarios/ALK.md`. **Consequence for DOC**: zero identities are loaded
(`identities` table: 0 rows before, during, and after this pass) and no contract can ever be
persisted as tracked (every "Add Contracts" attempt fails before reaching persistence,
`contracts`-equivalent local cache stays empty throughout). Per `CAMPAIGN-CONTEXT.md`'s guidance,
identity-authoring stories are BLOCKED on this root cause — but, per this pass's explicit
assignment, the public/read-only surfaces were tested live rather than assumed blocked. Two were:
**DOC-003** (Load/Add Contracts — a genuine unauthenticated DAPI query, see above) and **DOC-004**
(Fetch Documents against the built-in "domain"/DPNS query template — see below, which surfaced a
new, independent silent-hang defect rather than the expected clean environment-blocked failure).

---

## DOC-001: Register a new data contract — **BLOCKED** (live-tested: reachable, clean typed
## error)

**Persona:** Jordan. Acceptance criteria: "Define contract schema and register. Contract ID
returned upon success."

Contracts > "Contracts" menu > "Register Contract" → `Contracts > Register Data Contract` loads
cleanly (no crash) with a single, correctly worded inline message: **"No identities loaded.
Please load an identity first."** No schema-editing form renders — the whole registration surface
is gated behind a non-empty local identity list, and that gate fails with a clean, actionable
message rather than a crash or blank screen. Screenshot:
`screenshots/DOC-001-1-register-data-contract-no-identities-loaded.png`.

**Verdict: BLOCKED** — reasoning: "blocked: no Platform identity reachable in this environment,
see scenarios/IDN.md — root cause is the known Testnet masternode-list/quorum-sync/wallet-storage
failure, see CAMPAIGN-CONTEXT.md". Screen reachability and the identity-gate message are both
confirmed working correctly.

---

## DOC-002: Update an existing data contract — **FAIL (application crash)**

**Persona:** Jordan. Acceptance criteria: "Submit updated contract definition. Version
incremented on Platform."

Contracts > "Contracts" menu > "Update Contract" → **the application crashed instantly**, taking
down the whole process (window went blank white, then the process disappeared entirely from
`pgrep`). This is a full, unrecoverable crash, not a UI hang or a soft error banner.

### Crash evidence

`det-stderr.log`:
```
thread 'main' (989399) panicked at src/ui/contracts_documents/update_contract_screen.rs:93:14:
Failed to load contracts: WalletBackendNotYetWired
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```
`det.log` confirms the same panic with a full (unsymbolized) backtrace and
`location=src/ui/contracts_documents/update_contract_screen.rs:93:14`, timestamped
`2026-07-14T21:05:56.470146Z`, immediately after a burst of the routine
`Error fetching contracts: Your wallet is still starting up. Please wait a moment and try again.`
log lines that every other screen in this campaign (correctly) treats as a recoverable,
displayable error rather than a panic source.

### Root cause (source-confirmed)

`src/ui/contracts_documents/update_contract_screen.rs`, `UpdateDataContractScreen::new()`:
```rust
let known_contracts = app_context
    .get_contracts()
    .expect("Failed to load contracts")   // line 93 — panics the whole app
    .into_iter()
    ...
```
`app_context.get_contracts()` returns `Err(WalletBackendNotYetWired)` whenever the wallet backend
hasn't finished wiring — exactly the condition this entire campaign's environment blocker
produces on every launch. The `.expect()` converts that recoverable, already-typed error straight
into a full `panic!`, which brings down the whole egui/eframe process (Rust panics on the main
thread of a GUI app are fatal, not caught per-frame). **This is a clear regression relative to its
sibling screen**: DOC-001's "Register Contract" (`register_data_contract_screen.rs`, not
inspected in detail but empirically confirmed) hits the identical missing-identities/
not-wired-backend condition and degrades to a clean inline message — "Update Contract" hits a
closely related condition (missing *contracts*, not missing *identities*, but triggered by the
exact same underlying wallet-backend-not-wired state) and crashes instead.

Note: checked whether this is a PR892 regression by diffing the same file against the `v1.0-dev`
base branch (via this docs worktree, which branches from `v1.0-dev`, not PR892). The identical
un-guarded `.expect("Failed to load contracts")` is present on `v1.0-dev` too — this is a
**pre-existing bug, not something PR892 introduced**. Interestingly, PR892's version of this same
file *does* fix an unrelated, structurally similar issue a few dozen lines further down (the
"submit" button's identity/key selection, which `v1.0-dev` still handles with raw
`.unwrap()`/`unwrap should be safe here` comments, while PR892 replaces it with an `if let
(Some(identity), Some(key))` guard and a proper banner) — so this class of "unwrap/expect on a
recoverable `Result` inside a screen constructor or handler" is a known pattern in this codebase
that gets fixed piecemeal; the `get_contracts()` call at the top of this same screen's
constructor was simply missed.

### Recovery

The app was relaunched cleanly (`DASH_EVO_TOOL_ACCESSIBILITY=1 DASH_EVO_DATA_DIR=/data/tmp/det-qa-pr892-data
/data/target/debug/dash-evo-tool`, PID 1279253) with no manual intervention needed beyond a
fresh launch. Direct SQLite check confirmed zero persistent state change from the crash: the
`identities`, `wallets`, and `meta_wallet` tables were unaffected (the crash occurred during
screen construction, before any write path was reached).

**Verdict: FAIL** — a user who opens "Update Contract" while their wallet backend has not yet
finished starting (which, per `CAMPAIGN-CONTEXT.md`, is not a rare edge case — it is this
environment's default state on every Testnet launch, and could plausibly happen briefly on any
network right after app startup even in a healthy environment) gets an unannounced full
application crash instead of an error message. This is a severe, real, and clearly
reproducible defect — most severe finding in this pass. Not counted as "environment-blocked"
because the failure mode itself (crash vs. clean message) is the bug, independent of whether the
underlying wallet-backend condition is expected to resolve.

---

## DOC-003: Import and manage contracts — **BLOCKED** (confirmed reachable and dispatches
## correctly without an identity — tested explicitly per campaign instructions)

**Persona:** Priya, Jordan. Acceptance criteria: "Enter contract ID to import. Remove cached
contracts when no longer needed."

Contracts > "Contracts" menu > "Load Contracts" → `Contracts > Add Contracts`: a plain "Enter
Contract Identifiers:" field (labeled "Contract 1:") with an "Add Another Contract Field" button
and an "Add Contracts" submit button — no identity gate visible in the UI.

Entered the well-known DPNS contract ID, clicked "Add Contracts." `det.log` confirms a real,
unauthenticated dispatch: "Adding contract..." banner shown synchronously, 7 retries against 7
DAPI endpoints, then a clean generic error: **"An unexpected error occurred. Please try again
later."** with `SdkError { source_error: Proof(ContextProviderError(Config("masternode list not
yet synced (quorums unavailable)"))) }` available via "Show details." Reproduced identically
across both the pre-crash and post-crash sessions. Screenshots:
`screenshots/TOK-DOC-000-environment-recheck-add-contract-quorum-error.png` (pre-crash session),
`screenshots/DOC-003-1-add-contracts-BLOCKED-quorum-error.png` (post-crash session). Confirmed
via the Contracts left panel ("Filter contracts:") that the contract list remained empty
afterward — the failed add correctly did not leave a partial/corrupt entry.

**Verdict: BLOCKED** — reasoning: "blocked: no Platform identity reachable in this environment,
see scenarios/IDN.md — root cause is the known Testnet masternode-list/quorum-sync/wallet-storage
failure, see CAMPAIGN-CONTEXT.md". Explicitly confirmed this is a public/read-only import path
that dispatches without needing the user's own identity — blocked purely by the shared
proof-verification failure, matching TOK-002's identical finding for token keyword search. The
"remove cached contracts" half of this story's acceptance criteria could not be exercised (no
contract can ever be successfully added to remove).

---

## DOC-004: Query and browse documents — **FAIL** (dispatches a real query but hangs silently
## forever — a new, independently-reproducible defect distinct from the clean environment-
## blocked failures elsewhere in this pass)

**Persona:** Priya, Jordan. Acceptance criteria: "Select contract and document type. View query
results as document list."

Contracts > "Documents" tab shows a pre-filled raw-query box reading `SELECT * FROM domain` (a
built-in template referencing the DPNS system contract's `domain` document type) with a "Fetch
Documents" button, alongside "Select a contract and document type on the left and hit 'Fetch
Documents' to query documents." — the left "Filter contracts" panel is empty (no contracts
tracked), but the query box itself is pre-populated and the button is enabled regardless.

### Reproduced twice, in two separate app sessions, both times hanging indefinitely

Clicked "Fetch Documents." `det.log` confirms a real dispatch: `encoding GetDocumentsRequest
feature_version=0 protocol_version=11` followed immediately by `Banner displayed banner="Querying
documents..."`. **No further log activity for that request ever appears** — confirmed via an
active 60-second polling wait (first session) and a second, independent 45-second polling wait
(post-crash session), both showing zero new log lines beyond the routine
`contract_chooser_panel` "wallet still starting up" spam that fires once per second regardless.
The "Querying documents..." banner itself stays on screen indefinitely with its elapsed-time
counter ticking up (observed past 800 seconds / >13 minutes in the first session, still present
and counting when that session ended via the DOC-002 crash — the crash was on an unrelated
screen and did not resolve or dismiss this banner). Screenshot:
`screenshots/DOC-004-1-fetch-documents-silent-hang-domain-query.png`.

This is a materially different failure mode from every other live Platform query tested in this
pass (DOC-003, TOK-002, IDN-010, DEV-005, DEV-007, etc.) — all of those complete their retry
sequence and settle on a clean typed/generic error banner within a few seconds. This one never
resolves at all, in either direction.

**Verdict: FAIL** — the story's core acceptance criteria ("view query results as document list")
cannot be exercised because the query never completes, and — unlike IDN-002/003's silent hangs,
which at least leave no misleading progress indicator — this one leaves an actively-counting
"Querying documents..." progress banner that gives the user false confidence something is still
happening, indefinitely. Flagged as a new defect, not purely environment fallout: even accepting
that the underlying DAPI query will fail due to the masternode-list-sync issue, the request
*should* eventually fail and report that failure the same way DOC-003/TOK-002 do on the identical
network condition. Worth re-testing once the environment blocker resolves to see if the hang
persists on a healthy backend.

---

## DOC-005 through DOC-009: Document mutation actions — **BLOCKED** (all six action screens
## reachable, all six degrade cleanly to the same "select a contract" empty state, no crashes)

**Stories:** DOC-005 (Create a document), DOC-006 (Replace or update a document), DOC-007
(Delete a document), DOC-008 (Transfer document ownership), DOC-009 (Purchase a document and set
document pricing — two menu entries, "Purchase Document" and "Set Document Price").

Contracts > "Documents" menu offers six actions: **Create Document, Delete Document, Replace
Document, Transfer Document, Purchase Document, Set Document Price**. Given the DOC-002 crash
discovered earlier in this same pass, each of these six was clicked individually with an explicit
process-liveness check (`pgrep`) immediately after, before proceeding to the next — none of them
crashed. All six render the identical clean two-field empty state: **"1. Select a contract and
document type:"** with "Filter contracts:", an empty "Select Contract…" dropdown, and an empty
"Select Doc Type…" dropdown — correctly empty, since no contract is (or can be, per DOC-003)
tracked in this environment. Clicking into the empty "Contract" dropdown produced no options and
no crash. Screenshot (representative, "Set Document Price" shown, all six are visually identical
apart from the header):
`screenshots/DOC-005-009-1-document-action-screens-empty-contract-state.png`.

Source review confirms DOC-009's "pay/set-price with tokens" and general token-payment support
(`TokenPaymentInfo`, per TOK-017) is implemented inside this same shared
`document_action_screen.rs`, gated behind the identical contract-selection step — not reachable
here for the same reason.

**Verdict for all five stories (six menu items): BLOCKED** — reasoning: "blocked: no Platform
identity reachable in this environment, see scenarios/IDN.md — root cause is the known Testnet
masternode-list/quorum-sync/wallet-storage failure, see CAMPAIGN-CONTEXT.md" (specifically: no
contract can ever be added to select a document type against, per DOC-003's finding that "Add
Contracts" always fails on the shared proof-verification issue before persisting anything). All
six screens confirmed reachable and crash-free — a meaningful positive finding given DOC-002's
crash was found in an adjacent, structurally similar screen (contract/identity loading in a
screen constructor) just one menu away.

---

## Summary

| Story | Verdict |
|---|---|
| DOC-001 | BLOCKED (live-tested: clean typed "No identities loaded" message, no crash) |
| DOC-002 | **FAIL — application crash** (`.expect()` on `get_contracts()` panics on `WalletBackendNotYetWired`) |
| DOC-003 | BLOCKED (confirmed reachable without identity; dispatches + fails cleanly on known quorum-sync error) |
| DOC-004 | FAIL (dispatches a real query that hangs silently forever, with a misleading ever-counting progress banner) |
| DOC-005 | BLOCKED (reachable, clean empty state, no crash) |
| DOC-006 | BLOCKED (same as DOC-005) |
| DOC-007 | BLOCKED (same as DOC-005) |
| DOC-008 | BLOCKED (same as DOC-005) |
| DOC-009 | BLOCKED (same as DOC-005; both "Purchase Document" and "Set Document Price" menu items tested) |

**Two real, environment-independent-looking defects found, one severe**:

1. **DOC-002 is a confirmed application crash** — the single most severe finding across this QA
   pass (TOK+DOC). "Update Contract" panics the entire process via an `.expect()` on a
   `Result` that its sibling screen ("Register Contract," DOC-001) handles cleanly under the
   identical underlying condition. Reproduced once, deliberately not reproduced a second time
   (crash mechanism fully confirmed via stderr + source), and worked around by relaunching the
   app — no persistent state was lost or corrupted (confirmed via direct SQLite check
   before/after).
2. **DOC-004 hangs silently and indefinitely** on the "Fetch Documents" action, unlike every
   other live Platform query in this campaign (including its close sibling DOC-003 on the exact
   same DPNS-adjacent surface), which all fail cleanly within seconds. Reproduced across two
   independent app sessions with independent polling waits (60s and 45s).

**Read-only/public queries confirmed working despite the identity blocker**: DOC-003 ("Add
Contracts") dispatches and fails cleanly without needing the user's own identity — the same
finding as TOK-002 for token search. This confirms the identity blocker specifically gates
*authoring* actions (register/update contract, create/replace/delete/transfer/purchase document),
not read/import paths, which are gated only by the shared Platform-proof-verification failure.

**Clean-state confirmation**: direct SQLite inspection of `det-app.sqlite` before and after this
entire TOK+DOC pass (including the crash and relaunch) shows zero rows in `identities`,
`meta_identity`, `token_balances`, and `meta_token`, and an unchanged 3-row `meta_wallet` table
(QA Wallet 1 + DIAG throwaway wallet metadata, both pre-existing) — this pass made no persistent
state changes of any kind. No PR892 application source was modified; the crash was observed,
diagnosed via logs and source, and documented, not fixed, per campaign rules.
