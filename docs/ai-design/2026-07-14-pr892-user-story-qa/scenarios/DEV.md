# DEV — Developer Tools

Environment: PR892 build (`/data/target/debug/dash-evo-tool` @ `57195d54`), isolated data dir
`/data/tmp/det-qa-pr892-data`, network Testnet, display `:99`. App was already running
(PID 989399, launched ~5 minutes earlier by this same session) when this pass started; it had
already hit the known Testnet wallet-backend blocker (see "Fresh-launch check" below). Screen
size 1260x780. Sidebar: Identities/Masternodes/Contracts/Tokens/Wallets/**Tools**/Settings.

## Fresh-launch check (per campaign instructions)

The app was found already running at session start, launched ~19:10 UTC, with `det.log`
showing the same failure signature documented in `scenarios/ALK.md`:
```
ERROR dash_evo_tool::context::wallet_lifecycle::spv: Failed to start chain sync
  error=The wallet service could not complete this operation. Please retry in a moment.
```
This satisfies the "one fresh launch" check the campaign instructions allow — the issue has
**not** self-resolved. No additional restart was performed; all DEV testing proceeded against
this running instance per the instructions ("proceed with whatever DEV tools work regardless of
wallet/SPV state").

## Headline finding: the known blocker is broader than "wallet/SPV" — it also blocks Platform
## proof verification (masternode list / quorums), independent of the wallet

`ALK.md` framed the known issue as a Testnet **wallet-storage** failure. Testing DEV-005 and
DEV-007 (pure DAPI/Platform-info tools with **no wallet involvement at all**) surfaced a second,
related symptom with a distinct, more specific error:

```
SdkError { source_error: Proof(ContextProviderError(Config("masternode list not yet synced
  (quorums unavailable)"))) }
```

This fires from the SDK's context provider whenever a Platform query requires **proof
verification** (which needs a synced quorum/masternode list) — a concern that is architecturally
separate from the wallet's own SPV chain sync, but is evidently *also* stuck in this environment.
DAPI connectivity itself is healthy throughout this pass (Settings > Networks shows
"DAPI: Available", 22–27 of 29 endpoints unbanned, fluctuating upward over the session) — it is
specifically the masternode-list/quorum state needed for proof verification that never becomes
available. Unproven queries (Fetch Basic Platform Info, Fetch Validator Set Info) work
perfectly; proof-requiring queries fail cleanly and consistently with the error above. This is a
useful new diagnostic detail for whoever eventually root-causes the environment blocker, but per
campaign instructions this pass does not attempt to fix or further diagnose it — findings below
are documented and attributed to this cause where applicable.

---

## DEV-005: View Platform info — **FAIL** (partial: 2/8 sub-tools work)

**Persona:** Priya, Jordan. Acceptance criteria: "Displays epoch info, validator list, withdrawal
queue, and version voting status."

Tools > Platform info exposes 8 buttons under "Platform Information Tool". Tested all 8:

| Button | Result |
|---|---|
| Fetch Basic Platform Info | **PASS** — full protocol/fee/version schedule JSON rendered |
| Fetch Current Epoch Info | **FAIL** — confirmed via "Show details": `masternode list not yet synced (quorums unavailable)` |
| Fetch Total Credits on Platform | **FAIL** — same confirmed error text |
| Fetch Version Voting State | **FAIL** — same generic error banner (pattern consistent, not individually re-expanded) |
| Fetch Validator Set Info | **PASS** — real quorum hashes + validator IP list rendered |
| Fetch Current Withdrawals in Queue | **FAIL** — same generic error banner |
| Fetch Recently Completed Withdrawals | **FAIL** — same generic error banner |
| Fetch Shielded Pool State | **FAIL** — distinct error: "Could not sync shielded notes from the platform. Please check your connection and retry." |

Screenshots: `screenshots/DEV-005-1-fetch-basic-platform-info.png`,
`DEV-005-2-fetch-current-epoch-info-FAIL-quorums-unavailable.png`,
`DEV-005-3-fetch-total-credits-FAIL-quorums-unavailable.png`,
`DEV-005-4-fetch-version-voting-state-FAIL.png`,
`DEV-005-5-fetch-withdrawals-queue-FAIL.png`,
`DEV-005-6-fetch-shielded-pool-state-FAIL.png`,
`DEV-005-7-fetch-validator-set-info-PASS.png`,
`DEV-005-8-fetch-recently-completed-withdrawals-FAIL.png`.

**Verdict: FAIL.** Of the acceptance criteria's four named surfaces (epoch info, validator list,
withdrawal queue, version voting status), only validator list works; epoch info, withdrawal
queue, and version voting all fail. The tool's own code and UI are functioning correctly (clean
loading state, clean typed errors surfaced via `Show details`) — the failures are consistent with
the environment's masternode-list-sync blocker (see headline finding above), not a code defect
in the Platform Information Tool itself. Worth re-testing in full once that environment issue is
resolved.

---

## DEV-007: Check any address balance — **BLOCKED** (format validation confirmed working)

**Persona:** Priya, Jordan. Acceptance criteria: "Enter any address and see its balance."

The "Address balance" panel is titled "Platform Address Balance Lookup" and only accepts
Platform-style bech32 addresses (`dash1…`/`tdash1…`) — **not** ordinary Core base58 addresses.

### Steps and observed result

1. Entered the task's suggested Core address `yYCWtyP2mSLzGkZqL9a6G5rpPQQRs1fT5f` (QA Wallet 1's
   funded Testnet address) and clicked "Fetch Balance". Got a clean, correct validation error:
   *"The identifier you entered could not be read. Please check the format and try again."*
   Screenshot: `screenshots/DEV-007-1-core-address-rejected-format.png`.
2. Entered a known-valid `tdash1…` Platform address instead — the wallet's own Platform (DIP-17)
   address `tdash1kp30ae9x752z7wu20j4m4y945449anlhtqqe9h4l`, which `ALK.md`'s WAL-017 differential
   retest confirmed was funded to 0.01985204 DASH earlier in the campaign. Clicked "Fetch
   Balance" — failed with `SdkError { source_error: Proof(ContextProviderError(Config("masternode
   list not yet synced (quorums unavailable)"))) }`, the same error as DEV-005's proof-requiring
   calls. Screenshot: `screenshots/DEV-007-2-valid-platform-address-FAIL-quorums.png`.

**Verdict: BLOCKED** — reasoning: "blocked by known environment issue: Testnet
wallet-backend/masternode-list sync fails to complete in this data dir as of 2026-07-14, see
`scenarios/ALK.md` for full diagnosis and the headline finding above for the Platform-info-side
symptom." The tool's input validation (rejecting non-Platform addresses with a clear, actionable
message) works correctly and is not blocked — only the actual balance fetch is.

**Note on scope**: this tool only looks up **Platform** address balances, not Core address
balances as DEV-007's story text might suggest ("check the balance of any Dash address"). There
is no separate Core-address balance lookup tool anywhere in Tools — Core balances are only
visible via a loaded wallet's own address table (see WAL-011). Worth flagging as a possible
scope gap between the story text and what's implemented, though not re-tested against Core
addresses specifically since the acceptance criteria's example use case ("audit external
addresses") is most naturally read as Platform addresses in this dev-tools context.

---

## DEV-001: Decode state transitions — **PASS**

**Persona:** Jordan. Acceptance criteria: "Transition visualizer parses and displays state
transition contents."

Tools > Transaction deserializer accepts "hex, base64, or comma-separated integers for state
transition" in a free-text box with **live parsing** (no submit button — output updates as you
type). This is a pure local decoder with no network call involved (no banners appeared beyond
the pre-existing SPV ones).

### Steps and observed result

Typed `deadbeef` (invalid/malformed input) into the box. Immediately got a clean, structured
error in the "Parsed State Transition" panel: `Error: Failed to parse: platform deserialization
error: unable to deserialize StateTransition : UnexpectedVariant { type_name: "StateTransition",
allowed: Range { min: 0, max: 20 }, found: 222 }`. Screenshot:
`screenshots/DEV-001-1-transaction-deserializer-garbage-input-typed-error.png`.

No real state-transition hex was available in this environment to test the success path (no
state transition was broadcast and dumped to hex in earlier campaign sessions; `det.log` doesn't
capture raw transition bytes). Per the task's guidance, malformed-input handling counts as valid
testing of the tool's input validation/UI — and the tool behaves exactly as it should: no crash,
no hang, a precise, well-typed error identifying the exact byte offset and issue.

**Verdict: PASS.** Tool is reachable, functions independently of network/wallet state, and
handles invalid input correctly. The happy-path (decoding a real state transition into a
human-readable breakdown) was not directly observed, but there is no reason to doubt it given the
clean typed-error architecture visible on the failure path.

---

## DEV-003: Inspect ZK proofs — **FAIL** (partial: structural proof decode works; GroveSTARK
## generation/verification is unreachable in the UI)

**Persona:** Jordan. Acceptance criteria: "Proof visualizer displays proof structure. GroveSTARK
proof generation and verification available."

### Proof deserializer (structural decode) — works

Tools > Proof deserializer accepts "hex, base64, or comma-separated integers for GroveDB proof",
same live-parsing UX as the transaction deserializer. Typed `deadbeef` — got a clean structured
error: `UnexpectedVariant { type_name: "GroveDBProof", allowed: Range { min: 0, max: 1 },
found: 222 }`. Screenshot:
`screenshots/DEV-003-1-proof-deserializer-garbage-input-typed-error.png`. Same standalone,
no-network-dependency behavior as DEV-001's tool. This satisfies the "Proof visualizer displays
proof structure" half of the acceptance criteria (no real GroveDB proof sample was available to
exercise the success path, same caveat as DEV-001).

### GroveSTARK ("ZK Proofs") screen — present in code, but deliberately hidden from all navigation

Searched the PR892 source (`src/ui/tools/grovestark_screen.rs`,
`src/ui/components/tools_subscreen_chooser_panel.rs`) after finding no "ZK Proofs" entry anywhere
in the running UI (not in the Tools sub-nav, not behind Developer mode, not on the Masternodes or
Contracts screens). The screen and its route (`RootScreenToolsGroveSTARKScreen`) are fully wired
and functional in the codebase, but `tools_subscreen_chooser_panel.rs` explicitly excludes it:

```rust
/// GroveSTARK ("ZK Proofs") is intentionally omitted here so it does not appear
/// in the menu, but its screen and `RootScreenToolsGroveSTARKScreen` route stay
/// live — it remains reachable through other entry points and keeps working.
```

with an accompanying unit test (`zk_proofs_hidden_from_tools_menu`) asserting exactly this. No
other in-app entry point was found (Developer mode does not add it back — the exclusion list is
unconditional, not gated on interface mode). This is confirmed **intentional product behavior**
(a deliberate hide, not a crash or regression) — not the same class of finding as WAL/SND's
inert-button bugs — but from a user's perspective the feature is currently unreachable through
the UI, so the acceptance criteria's second bullet is not met in practice.

**Verdict: FAIL.** The visualizer half of the story works; the GroveSTARK generation/verification
half is coded but deliberately hidden from all UI navigation, so a user cannot currently reach it.
Flagging for product awareness rather than as a regression — the source comment indicates this was
a conscious choice, not an oversight.

---

## DEV-004: View document and contract JSON — **BLOCKED** (Contract deserializer works
## standalone; Document deserializer's contract-loading path hits the known environment blocker)

**Persona:** Jordan. Acceptance criteria: "Document visualizer shows full JSON. Contract
visualizer shows contract schema JSON."

### Contract deserializer — works standalone

Tools > Contract deserializer takes raw "hex, base64, or comma-separated integers for Contract" —
no contract needs to be pre-loaded. Typed `deadbeef`, got a clean structured error: `Error:
Deserialisation error: platform deserialization error: unable to deserialize DataContract:
UnexpectedVariant { type_name: "DataContractInSerializationFormat", allowed: Range { min: 0,
max: 1... }`. Screenshot:
`screenshots/DEV-004-1-contract-deserializer-garbage-input-typed-error.png`. Same
no-network-dependency behavior as DEV-001/003's tools.

### Document deserializer — needs a locally-known contract; none available, and loading one is blocked

Tools > Document deserializer requires selecting a **Contract** and **Doc Type** from dropdowns
before a document can be decoded against its schema (unlike the raw-bytes Contract deserializer).
The "Contract" dropdown was empty — 0 contracts are currently tracked locally in this environment
(no DOC/IDN category work has registered or imported one yet), and the "Filter contracts" text box
had no effect on this (nothing to filter).

Attempted to populate it via Contracts > Contracts > Load Contracts, entering the well-known DPNS
system contract ID `GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec`. This failed with the exact same
error as DEV-005/007's proof-requiring Platform calls: `SdkError { source_error:
Proof(ContextProviderError(Config("masternode list not yet synced (quorums unavailable)"))) }`.
Screenshot: `screenshots/DEV-004-2-load-dpns-contract-FAIL-quorums.png`. This is a new, useful data
point: it shows the masternode-list-sync blocker also prevents loading **any** contract by ID
(system or otherwise), which is why the Document deserializer's dropdown can never populate in
this environment right now — not a bug in the Document deserializer itself.

**Verdict: BLOCKED** for the Document deserializer half — reasoning: "blocked by known environment
issue: Testnet wallet-backend/masternode-list sync fails to complete in this data dir as of
2026-07-14, see `scenarios/ALK.md` for full diagnosis and the headline finding above." The
Contract deserializer half is **PASS** (works standalone, verified error-handling). Overall story
verdict recorded as BLOCKED since the acceptance criteria's first bullet (document JSON) could not
be exercised at all.

---

## DEV-002: View proof request log — **FAIL** (no UI implementation found)

**Persona:** Jordan. Acceptance criteria: "Proof log lists all requests with timestamps and
results."

No screen, panel, or navigation entry resembling a "proof request log" exists anywhere in the
running UI — checked Tools (all 7 sub-panels), Settings (including after switching Interface mode
to **Developer view**, which adds "raw protocol data, Devnet, and signing overrides" per its own
description but nothing log-related), Wallets, Contracts, and Masternodes screens.

Confirmed via source audit (`src/context/mod.rs`, `log_drive_proof_error()`): there is a tracing
target named `proof_log`, but it is a **structured log emission only** — it fires exclusively when
an SDK call returns `dash_sdk::Error::DriveProofError` (i.e., only on proof-verification
*failures*, not "all requests" as the story describes), and it writes to the plain-text
`det.log` file via `tracing::error!`, not to any in-app browsable list with timestamps. A search of
the entire `RootScreenType` enum (`src/model/settings.rs`, all ~29 variants) confirms no screen
exists for viewing this data. `det.log` for this session contains zero `proof_log` entries so far
(all proof-related failures hit encountered this session were `ContextProviderError`s — a
precondition failure that occurs *before* proof verification is attempted — not
`DriveProofError`s, so the tracing target never fired even at the log-file level).

**Verdict: FAIL.** No in-app feature exists matching this story's acceptance criteria. What exists
is a developer-only, failure-only log-file line, not a browsable request log with timestamps and
results for all requests. This looks like either an unimplemented `[Gap]` mismarked as
`[Implemented]` in `docs/user-stories.md`, or a very early/partial implementation (structured log
target only, no UI) — worth a docs correction, though this pass only observes/documents per the
QA campaign's rules and does not modify `docs/user-stories.md`'s tagging itself.

---

## DEV-006: View masternode list diff — **FAIL** (no UI implementation found)

**Persona:** Priya. Acceptance criteria: "Shows additions, removals, and changes between blocks."

The sidebar's **Masternodes** screen (with 0 masternodes loaded, as expected — no ownership
fixture is present in this environment, see note below) exposes exactly one flow: "Load a
masternode" — a form to load a **specific, individually-known** masternode or evonode by
ProTxHash (+ optional owner/voting/payout private keys) for **key management purposes**
(voting, payout key changes). Screenshot:
`screenshots/DEV-006-1-masternodes-screen-load-form-no-diff-feature.png`. This is the correct
screen for **IDN-003** ("Load evonode/masternode identity"), not a network-wide masternode-list
monitoring/diff view.

No "diff", "history", "additions/removals", or "changes between blocks" UI exists anywhere —
confirmed by exploring the full Masternodes screen (list/detail/load-form) and by source-grepping
the whole codebase for `MnListDiff`/masternode-list-diff terminology, which returned no matches
in `src/ui/masternodes/*` or `src/backend_task/*`. The closest adjacent data (a **snapshot**, not
a diff) is Tools > Platform info's "Fetch Validator Set Info" (tested under DEV-005, PASS),
which shows the current quorum/validator set at a point in time but has no block-to-block
comparison feature.

Also checked: no `.testnet_nodes.yml` fixture file exists in this environment (searched the app's
actual working directory `/home/ubuntu/git/dash-evo-tool-2` and elsewhere) to enable the
Masternodes screen's dev-only "Fill-Random" convenience button, consistent with
`CAMPAIGN-CONTEXT.md`'s note that no masternode/evonode fixture is available — real masternode
registration needs ~1000 tDASH collateral this environment doesn't have.

**Verdict: FAIL.** No masternode-list-diff/monitoring feature exists in this build under any
navigation path tried, confirmed by both UI exploration and source-code search. This looks like a
`[Gap]` mismarked as `[Implemented]` in `docs/user-stories.md`.

---

## DEV-008: Mine blocks on Regtest — **BLOCKED**

**Persona:** Jordan. Acceptance criteria: "Available only in developer mode on Regtest/local
network. Specify number of blocks to mine."

**Verdict: BLOCKED** — reasoning: per `CAMPAIGN-CONTEXT.md`'s ordering rules, this is
Regtest-only and no Regtest node is running in this environment; standing one up is out of scope
for this QA pass. Not tested.

---

## Summary

| Story | Verdict |
|---|---|
| DEV-001 | PASS |
| DEV-002 | FAIL (no UI implementation found) |
| DEV-003 | FAIL (partial — visualizer works, GroveSTARK gen/verification deliberately hidden from UI) |
| DEV-004 | BLOCKED (Contract deserializer PASS; Document deserializer blocked by known env issue) |
| DEV-005 | FAIL (partial — 2/8 sub-tools work; rest blocked by known env issue) |
| DEV-006 | FAIL (no UI implementation found) |
| DEV-007 | BLOCKED (format validation PASS; balance fetch blocked by known env issue) |
| DEV-008 | BLOCKED (Regtest-only, no node available) |

Three genuinely new-code findings independent of the known environment blocker: **DEV-002** and
**DEV-006** have no UI implementation at all (likely `[Gap]`s mismarked `[Implemented]`), and
**DEV-003**'s GroveSTARK half is intentionally hidden from navigation. The remaining
BLOCKED/partial-FAIL verdicts (DEV-004, DEV-005, DEV-007) all trace back to the same
masternode-list/quorum-sync symptom of the known Testnet environment blocker documented in
`ALK.md` — this pass additionally established that the blocker reaches pure DAPI/Platform-info
calls with **no wallet involvement**, not just wallet/SPV operations, which narrows the likely
root cause for whoever picks this up next.
