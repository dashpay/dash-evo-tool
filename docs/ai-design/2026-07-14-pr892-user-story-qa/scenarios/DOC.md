# DOC — Contracts and Documents

Environment: PR892 build (`/data/target/debug/dash-evo-tool` @ `57195d54`), isolated data dir
`/data/tmp/det-qa-pr892-data`, network Testnet, display `:99`, wallet `QA Wallet 1`. App was
already running (PID 989399) when this pass started; reused per campaign instructions. **The app
crashed mid-pass** (see DOC-002 below) and was relaunched (PID 1279253, launched with the same
`DASH_EVO_TOOL_ACCESSIBILITY=1 DASH_EVO_DATA_DIR=/data/tmp/det-qa-pr892-data` env per
`CAMPAIGN-CONTEXT.md`'s recipe); remaining DOC and TOK testing ran against the relaunched
instance. Both sessions showed the same underlying environment blocker.

## Retest pass (2026-07-15): identity registration now works, retesting DOC-001/003/005-009

The asset-lock recurrence blocker was fixed again upstream (dashpay/platform#4133). App relaunched
as PID 3331055 (later PID 4113175 after a mid-pass restart, see below), binary
`/data/tmp/det-qa-pr892-bin-myown/dash-evo-tool` (hash `2931220e...c1b7f9271a`), same data dir.
Two real identities now exist and hold Platform balance: `QA Identity 1` (@detqa892run2) and
`QA Identity 2` (@detqa892run3).

**New finding, not the asset-lock-recurrence bug**: registering a contract requires ~0.12 DASH of
identity balance. Topping up `QA Identity 1` via "Add Funds > From your wallet" failed
deterministically 4 times in a row (both 1.5 DASH and 0.9 DASH amounts, including after a full
graceful app restart) with `WalletBackend { source: AssetLockTransaction("Asset lock builder
failed: Transaction builder error: Coin selection error: No UTXOs available for selection") }` —
despite `QA Wallet 1` showing 5.45 DASH Core balance across many addresses. Root-caused via
`memcan:recall` (project `dash-evo-tool`) to a previously-documented, real upstream bug: failed
asset-lock coin-selection attempts soft-lock the selected UTXOs in `platform-wallet`'s
`ReservationSet` (`managed_account/reservation.rs`) for a ~24-block TTL, keyed by block height (not
per-process), so an app restart does **not** clear it. Nearly all of `QA Wallet 1`'s balance sits
in `Change`-type addresses accumulated over many days of prior campaign testing — every one of
these had almost certainly been touched by an earlier coin-selection attempt at some point in this
long-running campaign, leaving them soft-locked. The single existing `Funds`-type (receiving)
address only held 0.02 DASH, too small.

**Workaround (not a fix, not out of scope — no PR892 source or DB touched)**: requested a fresh
1 tDASH payout from `dash-platform:dash-faucet` to a brand-new, never-before-touched receiving
address (`Add Receiving Address` on the Wallets screen, since the "Receive" button is the known-dead
SND-003 button), confirmed landed via InstantLock, then retried "Add Funds" — **succeeded
immediately** ("Identity Topped Up Successfully!"), confirming the diagnosis: a genuinely fresh,
never-reserved UTXO is unaffected. Also confirms this is a general environment/upstream wallet
limitation independent of PR892, not a product regression. `asset_locks` table went from 2 to 3
rows, all `status='consumed'` — no stuck/unconsumed locks left behind.

---

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

## DOC-001: Register a new data contract — **PASS** (retested 2026-07-15: full E2E registration)

**Persona:** Jordan. Acceptance criteria: "Define contract schema and register. Contract ID
returned upon success."

Original finding (BLOCKED, "No identities loaded" — see reasoning below the new result) is
superseded now that identity registration works in this environment.

Contracts > "Contracts" menu > "Register Contract" → `Contracts > Register Data Contract`: with
`QA Identity 1` selected (balance 0.512780 DASH after the workaround top-up described above),
pasted a minimal hand-built contract JSON — one document type `note` with a single `message`
string property, `additionalProperties: false` — set alias "QA Note Contract". The form live-
parsed it and showed **Estimated Fee: 0.120079586 DASH**. Clicked "Register Contract" →
**"Data Contract Registered Successfully!"** Screenshots:
`screenshots/DOC-001-1-register-contract-form-filled.png`,
`screenshots/DOC-001-3-data-contract-registered-successfully.png`.

Confirmed via Contracts screen: "QA Note Contract" now appears in the left panel; its Contract
JSON shows `id: DscQtuMqD5mjg68AxuXiuUZ1JHHJuzgRBJuvYVTHr8QQ`,
`ownerId: 24Jm9XBCPsAf154cy4X2YLvTTgFjiwAKoCSew17CetCb` — the owner ID matches `QA Identity 1`'s
real on-chain identifier exactly (cross-checked against `det.log`'s identity-discovery line),
confirming this is a genuine on-chain registration, not a cached/local-only artifact. This
contract (and its `note` document type) is used as the fixture for DOC-005 through DOC-009 below.

**Note on the JSON input widget**: typing multi-character strings via synthetic key events into
this screen's `TextEdit` code editor drops all but the first character, repeatably, the instant
the live-parse error banner first appears and shifts the layout (confirmed root cause: a focus
loss tied to the banner's one-time appearance, not an input-speed issue — typing continues fine
once the banner is already showing). Worked around by sending the first character alone, then
re-clicking the field before sending the rest. This is a testing-methodology note about driving
the UI with synthetic X11 key events, not a product defect — not verified whether a human typing
at normal keyboard speed would hit it (unlikely, since it depends on hitting the exact frame the
banner first mounts).

**Verdict: PASS** — a real data contract was registered on Testnet end-to-end, contract ID
returned and confirmed on-chain via owner-ID cross-check, matching both acceptance-criteria
bullets exactly.

### Original finding (superseded): BLOCKED — "No identities loaded"

Contracts > "Contracts" menu > "Register Contract" → `Contracts > Register Data Contract` loaded
cleanly (no crash) with a single, correctly worded inline message: **"No identities loaded.
Please load an identity first."** No schema-editing form rendered — the whole registration
surface was gated behind a non-empty local identity list, and that gate failed with a clean,
actionable message rather than a crash or blank screen. Screenshot:
`screenshots/DOC-001-1-register-data-contract-no-identities-loaded.png`. Superseded now that
identity registration works in this environment (see PASS result above).

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

## DOC-003: Import and manage contracts — **Partial PASS / FAIL** (retested 2026-07-15: import by
## ID works end-to-end with a genuinely new contract; "remove cached contract" is a confirmed
## click no-op)

**Persona:** Priya, Jordan. Acceptance criteria: "Enter contract ID to import. Remove cached
contracts when no longer needed."

### Import by ID — PASS

Contracts > "Contracts" menu > "Load Contracts" → `Contracts > Add Contracts`. Needed a
genuinely untracked-yet-real contract ID (not one of the 5 system contracts already pre-loaded:
DPNS, Token History, Withdrawals, Keyword Search, DashPay) — computed the Base58 ID of the
`wallet-utils-contract` system contract from its `ID_BYTES` constant in the pinned `dpp` crate
source (`7CSFGeF4WNzgDmx94zwvHkYaG3Dx4XEe5LFsFgJswLbm`), not previously loaded in this app.
Entered it, clicked "Add Contracts" → **"Successfully queried contracts" / "Found and added the
following contracts:"** listing the ID with a "Set Alias" option. Screenshot:
`screenshots/DOC-003-2-add-contracts-successfully-queried-wallet-utils.png`. Confirmed it now
appears in the Contracts left panel alongside the other tracked contracts, persisted (present
after navigating away and back).

**Verdict for "enter contract ID to import": PASS** — a real, previously-untracked contract was
imported end-to-end by ID.

### Remove cached contract — FAIL (confirmed click no-op)

Expanded "QA Note Contract" (from DOC-001) > "Contract JSON" > clicked the "Remove" button
beneath it. Verified via `python3 a11y_dump.py --grep "Remove"` that the click landed exactly on
the button's reported center (`@(151,688 63x16) center=(182,696)`) — 4 separate careful click
attempts at that exact coordinate, both via the `mcp__desktop__computer` tool and direct
`xdotool`. Result each time: **zero effect** — no banner, no log line of any kind for
`RemoveContract`/`remove_contract` in `det.log` (contrast with every other button in this
campaign, which at minimum logs a dispatch), and a direct SQLite check of
`spv/testnet/platform-wallet.sqlite`'s `meta_global` table (`det:contract:<id>` key, where
DET's contracts are actually persisted — there is no `contracts` table in either sqlite DB) shows
the row's `updated_at` timestamp unchanged (still the registration time, `11:15:56`) across all 4
attempts spanning several minutes. Source review (`src/ui/components/contract_chooser_panel.rs`
~487-495) confirms the button correctly dispatches `BackendTask::ContractTask(RemoveContract(...))`
when clicked and is not conditionally disabled for this contract's alias (the exclusion list only
covers `dpns`/`token_history`/`withdrawals`/`keyword_search`) — the click handler itself appears
never to fire, a genuine, reproducible UI defect distinct from this campaign's other
"dispatches-but-silently-fails" class of bugs (e.g. TOK-003): here the dispatch never happens at
all.

**Verdict for "remove cached contracts when no longer needed": FAIL** — confirmed
click no-op, reproduced 4 times with a11y-verified exact coordinates.

**Overall verdict for DOC-003: Partial PASS** — import-by-ID (the story's primary "browse
documents from any deployed contract" use case) works correctly end-to-end; the secondary
"remove cached contracts" bullet is a confirmed defect.

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

## DOC-005 through DOC-009: Document mutation actions — **PASS** (retested 2026-07-15: full
## E2E create/replace/delete/transfer/purchase+price flows, all confirmed on-chain)

**Stories:** DOC-005 (Create a document), DOC-006 (Replace or update a document), DOC-007
(Delete a document), DOC-008 (Transfer document ownership), DOC-009 (Purchase a document and set
document pricing — two menu entries, "Purchase Document" and "Set Document Price").

With the identity/wallet-backend blocker resolved (see the top-of-file retest-pass note) and a
real registered contract available ("QA Note Contract" from DOC-001), all five stories were
retested end-to-end against live Testnet, each broadcasting a real state transition and each
result cross-checked on-chain via the Contracts > Documents query tool (`SELECT * FROM note`,
re-fetched after every mutation to bypass any client-side cache).

### DOC-005: Create a document — PASS

Contracts > Documents > "Create Document" → selected "QA Note Contract" / doc type "note" /
identity "QA Identity 1" → filled `message: "hello DOC-005 QA test note"` → "Broadcast document"
→ **"Create Document successful!"**. Confirmed on-chain via the documents query tool: a `note`
document with `$ownerId` matching QA Identity 1's real identifier, `message` exactly as typed, and
a generated `$id` (`FFH8PsGd7h5nDARZPrRvyDjeeGeSSCuKi1dsUSqPtspt`). Screenshot:
`screenshots/DOC-005-1-create-document-successful.png`.

**Verdict: PASS** — a real document was created on Testnet end-to-end, matching the acceptance
criteria exactly.

### DOC-006: Replace or update a document — PASS

Contracts > Documents > "Replace Document" → same contract/doc-type/identity selection → "3. Enter
document ID and fetch existing document:" — pasted the DOC-005 document's `$id`, "Fetch" →
**"Document fetched successfully."**, pre-populating the `message` field with the existing value.
Cleared it and typed `"hello DOC-006 QA replaced note"` → "Replace document" →
**"Replace Document successful!"**. Re-queried on-chain: same `$id`
(`FFH8PsGd7h5nDARZPrRvyDjeeGeSSCuKi1dsUSqPtspt`), `message` now reads the new text — confirming an
in-place update, not a new document. Screenshots:
`screenshots/DOC-006-1-replace-document-form-filled.png`,
`screenshots/DOC-006-2-replace-document-successful.png`,
`screenshots/DOC-006-3-onchain-verification-message-updated.png`.

**Verdict: PASS** — replace/update round-trips correctly against Testnet, verified via document
ID stability + content change.

### DOC-007: Delete a document — PASS

Created a fresh scratch document (`message: "hello DOC-007 QA delete target"`,
`$id: 97WYEpzsYuY9WgfjcaHeYwp9romvA7HMFqB9RBWbKw6j`) specifically for this destructive test, to
avoid consuming the fixture reused by DOC-008/009. Contracts > Documents > "Delete Document" →
contract/doc-type/identity selection → step 3 notes "(Cannot use the Fetch Owned Documents feature
as this document type does not have an index on $ownerId)" and asks for the Document ID directly —
pasted the scratch document's ID → "Delete document" → **"Delete Document successful!"**.
Re-queried on-chain: the deleted document no longer appears in `SELECT * FROM note` results (only
the DOC-006 document remains). Screenshots:
`screenshots/DOC-007-1-delete-document-form-filled.png`,
`screenshots/DOC-007-2-delete-document-successful.png`,
`screenshots/DOC-007-3-onchain-verification-document-gone.png`.

**Verdict: PASS** — deletion is real and confirmed absent from a subsequent live query, not just a
local/optimistic UI removal.

### DOC-008: Transfer document ownership — PASS (required a purpose-built fixture contract)

First attempt (against the existing "QA Note Contract"/DOC-006 document, QA Identity 1 →
QA Identity 2 by raw Identity ID `87jAqayii8J5zB8hJsnCPk3BEANicRxfMRFriGvk9jy6`) failed with a
**genuine, correct platform-level rejection**, not a DET bug:
```
SdkError { source_error: Protocol(ConsensusError(BasicError(InvalidDocumentTransitionActionError(
InvalidDocumentTransitionActionError { action: "note is not a transferable document type" })))) }
```
Root cause (confirmed via `dpp` crate source, `data_contract/document_type/class_methods/
try_from_schema/v0/mod.rs`): document-type transferability is an opt-in JSON Schema flag
(`"transferable": 1`, `Transferable` enum, default `Never`) that DOC-001's hand-built minimal
"QA Note Contract" schema never set — a testing-fixture gap, not a product defect. Worked around
by registering a second contract, **"QA Transfer Contract"**, with the same `note` schema plus
`"transferable": 1` (fee 0.120084244 DASH, paid from QA Identity 1's 0.5128 DASH balance — the
same identity was well-funded well beyond this pass's original ~0.015 DASH starting point by the
time this story was reached, no faucet round needed). Created a fresh document on it
(`message: "hello DOC-008 QA transfer target"`,
`$id: 2JAiaKv8W4eaBSZuDp3jdaekJdqEzqc7x7L65VoZPWbd`), then Documents > "Transfer Document" →
sender identity QA Identity 1, Document ID + Recipient Identity =
`87jAqayii8J5zB8hJsnCPk3BEANicRxfMRFriGvk9jy6` (QA Identity 2) → "Transfer document" →
**"Transfer Document successful!"**. Re-queried on-chain: same `$id`, `$ownerId` now
`87jAqayii8J5zB8hJsnCPk3BEANicRxfMRFriGvk9jy6` — an exact match for QA Identity 2's real
identifier. Screenshots: `screenshots/DOC-008-1-transfer-document-form-filled.png` (the failed
first attempt), `screenshots/DOC-008-2-register-transferable-contract-form-filled.png`,
`screenshots/DOC-008-3-transferable-contract-registered.png`,
`screenshots/DOC-008-4-transfer-document-form-filled-transferable.png`,
`screenshots/DOC-008-5-transfer-document-successful.png`,
`screenshots/DOC-008-6-onchain-verification-owner-changed.png`.

**Verdict: PASS** — transfer works correctly end-to-end once the document type actually permits
it; the platform's rejection of the first attempt is itself evidence the enforcement path works
as designed (DET surfaced the raw `SdkError` behind "An unexpected error occurred / Show details"
rather than a friendly dedicated message — a minor UX polish opportunity, not a functional defect,
noted here for completeness but not filed as a standalone bug since generic-error-with-details is
this app's established, deliberate fallback pattern per its error-message conventions).

### DOC-009: Purchase a document and set document pricing — PASS (required a purpose-built
### fixture contract, same class of gap as DOC-008)

Anticipating the same transferability gate plus a second, independent trade-mode gate (`dpp`
`nft::TradeMode` enum — `tradeMode: 1` = `DirectPurchase`, required in addition to `transferable`
for purchase flows), registered a third contract, **"QA Purchase Contract"**, with both
`"transferable": 1` and `"tradeMode": 1` set on the `note` schema (fee 0.12008808 DASH). Created a
document on it as QA Identity 1 (`message: "hello DOC-009 QA purchase target"`,
`$id: CoUseqbMXnL5UCfwZcsdxTicCGEX7ZXWM5feNKM8JEtk`).

**Set price**: Documents > "Set Document Price" → identity QA Identity 1 (the document's owner) →
Document ID + `Price (credits): 100000000` (0.001 DASH) → "Set document price" →
**"Set Document Price successful!"**. Screenshots:
`screenshots/DOC-009-3-set-document-price-form-filled.png`,
`screenshots/DOC-009-4-set-document-price-successful.png`.

**Purchase**: confirmed QA Identity 2 held sufficient balance (0.002190 DASH, comfortably above
the 0.001 DASH price plus fees) → Documents > "Purchase Document" → identity **QA Identity 2** (the
buyer, not the owner) → Document ID → "Fetch Document Price" →
**"Document price: 100000000 credits"** (exact match) → "Purchase document" →
**"Purchase Document successful!"**. Re-queried on-chain: same `$id`, `$ownerId` now
`87jAqayii8J5zB8hJsnCPk3BEANicRxfMRFriGvk9jy6` — QA Identity 2, confirming the purchase both paid
the listed price and transferred ownership atomically. Screenshots:
`screenshots/DOC-009-1-register-purchase-contract-form-filled.png`,
`screenshots/DOC-009-2-purchase-contract-registered.png`,
`screenshots/DOC-009-5-purchase-document-form-price-fetched.png`,
`screenshots/DOC-009-6-purchase-document-successful.png`,
`screenshots/DOC-009-7-onchain-verification-owner-changed-to-buyer.png`.

**Verdict: PASS** — both acceptance-criteria bullets ("Set price on a document" / "Another
identity can purchase at the set price") confirmed end-to-end against live Testnet.

### Original finding (superseded): BLOCKED — all six action screens reachable, degraded cleanly

Contracts > "Documents" menu offers six actions: **Create Document, Delete Document, Replace
Document, Transfer Document, Purchase Document, Set Document Price**. Given the DOC-002 crash
discovered earlier in this same pass, each of these six was clicked individually with an explicit
process-liveness check (`pgrep`) immediately after, before proceeding to the next — none of them
crashed. All six rendered the identical clean two-field empty state: **"1. Select a contract and
document type:"** with "Filter contracts:", an empty "Select Contract…" dropdown, and an empty
"Select Doc Type…" dropdown — correctly empty, since no contract was (or could be, per DOC-003's
then-finding) tracked in this environment. Screenshot (representative, "Set Document Price" shown,
all six were visually identical apart from the header):
`screenshots/DOC-005-009-1-document-action-screens-empty-contract-state.png`.

Original verdict (all five stories, six menu items): BLOCKED — "blocked: no Platform identity
reachable in this environment... root cause is the known Testnet masternode-list/quorum-
sync/wallet-storage failure". Superseded now that identity registration and contract creation work
in this environment (see PASS results above for all five stories).

---

## Summary

| Story | Verdict (2026-07-15 retest, wallet-backend/asset-lock env fix applied) |
|---|---|
| DOC-001 | **PASS** (full E2E contract registration, owner ID cross-checked on-chain) |
| DOC-002 | **FAIL — application crash** (`.expect()` on `get_contracts()` panics on `WalletBackendNotYetWired`; not retested live post-fix, see note below) |
| DOC-003 | **Partial PASS** (import-by-ID PASS; "Remove cached contract" confirmed non-functional — a11y-verified no-op) |
| DOC-004 | FAIL (dispatches a real query that hangs silently forever, with a misleading ever-counting progress banner; not retested live post-fix, see note below) |
| DOC-005 | **PASS** (create, on-chain verified) |
| DOC-006 | **PASS** (replace/update, on-chain verified — same `$id`, new `message`) |
| DOC-007 | **PASS** (delete, on-chain verified absent from a subsequent query) |
| DOC-008 | **PASS** (transfer, on-chain verified `$ownerId` change; required a purpose-built `transferable: 1` fixture contract) |
| DOC-009 | **PASS** (set price + purchase, on-chain verified `$ownerId` change to buyer; required a purpose-built `transferable: 1` + `tradeMode: 1` fixture contract) |

**Five of nine stories flip from BLOCKED to PASS** now that the wallet-backend/asset-lock
environment blocker (dashpay/platform#4133) is fixed and a real funded identity + registered
contract are reachable. DOC-002 and DOC-004 were **not** retested live in this pass (out of this
campaign's 24-story scope) — their original crash/hang findings stand as last confirmed, and
should be prioritized for retest by whoever picks up the DOC-002/DOC-004 remainder, since both
were previously blocked by the very same environment issue this pass fixed and may now behave
differently.

**Two real, environment-independent-looking defects found (from the original pass), one severe,
neither retested live this session**:

1. **DOC-002 is a confirmed application crash** — the single most severe finding across the
   original QA pass (TOK+DOC). "Update Contract" panics the entire process via an `.expect()` on a
   `Result` that its sibling screen ("Register Contract," DOC-001) handles cleanly under the
   identical underlying condition. Reproduced once, deliberately not reproduced a second time
   (crash mechanism fully confirmed via stderr + source), and worked around by relaunching the
   app — no persistent state was lost or corrupted (confirmed via direct SQLite check
   before/after).
2. **DOC-004 hangs silently and indefinitely** on the "Fetch Documents" action, unlike every
   other live Platform query in this campaign (including its close sibling DOC-003 on the exact
   same DPNS-adjacent surface), which all fail cleanly within seconds. Reproduced across two
   independent app sessions with independent polling waits (60s and 45s).

**New defect found this retest pass**: DOC-003's "Remove cached contract" button is a confirmed
click no-op (a11y-verified exact coordinates, 4 attempts, zero backend dispatch, zero DB change) —
see the DOC-003 section above for full detail.

**Clean-state note**: the original TOK+DOC pass's SQLite before/after check (zero rows in
`identities`/`meta_identity`/`token_balances`/`meta_token`) predates this retest pass, which
intentionally created real on-chain state (contracts, documents, an identity-to-identity transfer
and purchase) as part of exercising the now-working authoring flows — this is expected and
correct for this pass, not a regression from the earlier clean-state finding. No PR892 application
source was modified in either pass; all bugs were observed, diagnosed via logs and source, and
documented, not fixed, per campaign rules.
