# Masternodes Page — Test Case Specification

**Repo:** `dash-evo-tool` · **Branch:** `feat/masternodes-tab` · **Date:** 2026-07-09
**Author:** Marvin (QA) · **Phase:** 1c (Test Case Specification) — spec only, no test code, no Rust changes.

Derived from `01-requirements.md` (FR-1…FR-12, NFR-1…NFR-7, US-1…US-11) and `02-ux-spec.md` +
`wireframes.html` (final, human-accepted 2026-07-09). Every case cites its traceability so
Nagatha's Phase 1d plan can reference "this task satisfies TC-X, TC-Y, TC-Z."

Legend: `[AMBIGUOUS]` = requirement as written cannot be reduced to a deterministic pass/fail
assertion; case is recorded for traceability but flagged back to the coordinator, not silently
dropped. Colour mapping for `IdentityStatus` was verified against
`src/model/qualified_identity/mod.rs:147-155` (not left as a doc-only assumption):
Active→Green(0,128,0), Unknown→Gray(128,128,128), PendingCreation→Orange(255,165,0),
NotFound→Red(255,0,0), FailedCreation→Red(255,0,0) — two statuses legitimately share Red, this is
not a spec gap.

---

## US-6 — RETIRED, no test cases

Auto-derive of Voting/Owner/Payout keys from a loaded wallet is architecturally impossible for
Masternode/Evonode identities (`derive_keys_from_wallets` hard-gated to `IdentityType::User` in
`backend_task/identity/load_identity.rs`). No test cases are written against auto-derive on the
load form. Its *absence* is instead asserted positively under **TC-FR4-01**.

---

## FR-1 — Masternodes root tab (Expert-Mode gated)

| ID | Description | Preconditions | Steps | Expected Outcome | Traces to |
|---|---|---|---|---|---|
| TC-FR1-01 | Nav item absent with Expert Mode off | Expert Mode = OFF | Inspect left-nav rail | No "Masternodes" entry exists in the nav item list (assert absence, not merely disabled/hidden-behind-click) | FR-1, NFR-7 |
| TC-FR1-02 | Route unreachable with Expert Mode off | Expert Mode = OFF | Attempt to activate/select the Masternodes root screen via any non-nav path available to the app (e.g. programmatic `AppState` screen switch) | Screen is not reachable / request is a no-op; app remains on prior screen | FR-1 |
| TC-FR1-03 | Nav item present with Expert Mode on | Expert Mode = ON | Inspect left-nav rail | "Masternodes" entry renders, positioned between "Identity Hub" and "Contracts" (locked decision #3) | FR-1, ux-spec §Locked decisions #3 |
| TC-FR1-04 | Nav item functional with Expert Mode on | Expert Mode = ON | Click "Masternodes" nav item | Masternodes root screen (list or empty state) is shown | FR-1, US-1 |
| TC-FR1-05 | Toggling Expert Mode off while tab is active | Expert Mode = ON, Masternodes tab currently visible | Toggle Expert Mode to OFF via Network/Settings screen | Nav item disappears | FR-1 |
| TC-FR1-05b | Falls back to Identities on live de-gating *(RESOLVED — was `[AMBIGUOUS]`; found outside Marvin's original 12-item list, closed 2026-07-09)* | Same as TC-FR1-05 | Same | Active screen falls back to the **Identities** root tab — the nearest neutral, always-available screen (§10.11; no existing DET precedent for a dev-gated root tab was found to reuse instead) | FR-1 |
| TC-FR1-06 | Root screen persists across network switch | Expert Mode = ON, on Masternodes tab | Switch network (Mainnet↔Testnet) | Masternodes remains the active root tab (same persistence behaviour as other root screens in `AppState.main_screens`) | FR-1, ux-spec §1 |
| TC-FR1-07 | Distinct glyph, not the Identities person-glyph | Expert Mode = ON | Compare nav icon for "Masternodes" vs "Identities" | Icons are visually distinct SVG/glyph identifiers (node/server glyph vs person glyph) | FR-1, wireframes.html rail markup |

---

## FR-2 — Empty state

| ID | Description | Preconditions | Steps | Expected Outcome | Traces to |
|---|---|---|---|---|---|
| TC-FR2-01 | Empty state renders with zero nodes loaded | Expert Mode ON, 0 MN/Evonode identities loaded | Open Masternodes tab | Centered empty-state card renders (surface, `RADIUS_LG`), matching `03-identities-empty.png` pattern | FR-2, NFR-1 |
| TC-FR2-02 | Empty-state heading exact copy | Same | Read heading text | Text is exactly `No masternodes loaded` | FR-2, §7 copy |
| TC-FR2-03 | Empty-state body exact copy | Same | Read body text | Text is exactly `Load a masternode or evonode to vote on DPNS name contests and manage its owner and payout keys.` | FR-2, §7 copy |
| TC-FR2-04 | Primary CTA present and enabled | Same | Inspect primary button | Button labeled `Load a masternode`, enabled (not disabled) | FR-2, §7 copy |
| TC-FR2-05 | Reassurance-line exact copy *(RESOLVED 2026-07-09 — was `[AMBIGUOUS]`)* | Same | Read the reassurance line below the primary CTA | Text is exactly `Have your node's ProTxHash to hand. Keys are optional — a node loads read-only without them.` — now canonical in 01-requirements.md §7; `02-ux-spec.md`'s ASCII wireframe A corrected to match | FR-2, §7 copy |
| TC-FR2-06 | CTA navigates to load form | Same | Click "Load a masternode" | Load form (FR-4) opens | FR-2, US-1 |
| TC-FR2-07 | Empty state does not render once ≥1 node loaded | ≥1 MN/Evonode identity loaded | Open Masternodes tab | Card grid (FR-3) renders instead of the empty-state card; empty-state card is entirely absent (regression boundary) | FR-2, FR-3 |

---

## FR-3 — Card list of loaded masternodes

| ID | Description | Preconditions | Steps | Expected Outcome | Traces to |
|---|---|---|---|---|---|
| TC-FR3-01 | Card grid renders with ≥1 node | ≥1 node loaded | Open tab | Card grid renders, empty state absent | FR-3, US-2 |
| TC-FR3-02 | Heading = shortened ProTxHash when alias unset | Node loaded, no alias | View card | Heading shows `shorten_id`-formatted ProTxHash | FR-3, US-2 |
| TC-FR3-03 | Heading = alias, ProTxHash beneath, when alias set | Node loaded with alias `mn-east-01` | View card | Heading = `mn-east-01`; shortened ProTxHash shown as sub-line beneath | FR-3, US-2 bullet 3 |
| TC-FR3-04 | Masternode badge colour/text | Node type = Masternode | View card | Badge text `Masternode`, fill = `PLATFORM_PURPLE` | FR-3, Domain Notes §3 |
| TC-FR3-05 | Evonode badge colour/text | Node type = Evonode | View card | Badge text `Evonode`, fill = `DASH_BLUE` | FR-3, Domain Notes §3 |
| TC-FR3-06 | Voter-ready indicator | `associated_voter_identity` present | View card | Shows `Voting ready` with green status dot | FR-3, US-2 bullet 2 |
| TC-FR3-07 | No-voting-key indicator | `associated_voter_identity` absent | View card | Shows `No voting key` text (not colour-only — NFR-6), warning/orange dot | FR-3, US-2 bullet 2, NFR-6 |
| TC-FR3-08 | Key-status indicator across all V/O/P combinations | 8 nodes, one per bit-combination of {Voting, Owner, Payout} present/absent | View each card | Compact indicator (e.g. `V O P`) correctly emphasises exactly the present keys per node; all-off and all-on are both rendered distinctly from partial states | FR-3, US-2 |
| TC-FR3-09 | DPNS status: open contests | Node has N>0 open contests it can vote on | View card | Shows `{N} contests to vote on` with correct N | FR-3, §7 copy |
| TC-FR3-10 | DPNS status: no open contests | Node has 0 open contests, no scheduled vote | View card | Shows `No open contests` | FR-3, §7 copy |
| TC-FR3-11 | DPNS status precedence, count-first *(RESOLVED — was `[AMBIGUOUS]`)* | Node simultaneously has ≥1 open contest AND a scheduled vote | View card | Shows `{count} contests to vote on` (open-contest count takes precedence whenever `count > 0`, regardless of a pending scheduled vote); `Vote scheduled` only shown when `count == 0` and a vote is pending, reusing the existing DPNS Scheduled Votes screen's state — no new backend concept (§10.1) | FR-3, §7 copy |
| TC-FR3-12 | IdentityStatus dot + label, all 5 states | One card per `IdentityStatus` value | View each card | Active→green+"Active"; Unknown→gray+"Unknown"; PendingCreation→orange+"Pending Creation"; NotFound→red+"Not Found"; FailedCreation→red+"Creation Failed" (verified mapping, `qualified_identity/mod.rs:147-155`) | FR-3, US-2 bullet 1 |
| TC-FR3-13 | Whole card is a single click target | ≥1 node loaded | Click anywhere on a card body (not just heading) | Detail view for that node opens | FR-3 bullet "whole card...single click target", NFR-6 |
| TC-FR3-14 | Responsive wrap to 1 column | Narrow viewport width | Resize app window narrow | Grid collapses from multi-column (`minmax(260,1fr)`) to a single column; `ScrollArea` handles overflow | ux-spec §6 |
| TC-FR3-15 | Card count matches loaded-identity count | N nodes loaded (N=1, N=5) | Open tab | Exactly N cards render, no duplicates, no omissions (assert count equality against DB row count for MN/Evonode identities) | FR-3, data integrity |

---

## FR-4 — Load a masternode/evonode

| ID | Description | Preconditions | Steps | Expected Outcome | Traces to |
|---|---|---|---|---|---|
| TC-FR4-01 | No auto-derive affordance present | Load form open | Inspect full field set | Fields present: ProTxHash, Node-type toggle, Alias, Voting/Owner/Payout key inputs, Encryption password. **No** "Try to derive from wallet" checkbox anywhere on the form (explicit negative assertion — corrects the superseded ux-spec §Locked-decisions #4) | FR-4, ux-spec §4c, US-6 retirement |
| TC-FR4-02 | Node-type toggle defaults to Masternode | Load form freshly opened | Inspect toggle | `Masternode` segment is active/selected by default | FR-4, wireframe C |
| TC-FR4-03 | Selecting Evonode switches active segment and affects submit type | Load form open | Click `Evonode` segment, fill ProTxHash, submit | `Evonode` segment shows active styling; loaded identity has `IdentityType::Evonode` | FR-4 |
| TC-FR4-04 | No "User" option on toggle | Load form open | Inspect toggle | Exactly two segments: Masternode, Evonode — no third "User" option | FR-4 |
| TC-FR4-05 | Load button disabled when ProTxHash empty | Load form open, ProTxHash field empty | Inspect Load button | Button is disabled | FR-4, US-1 bullet 2 |
| TC-FR4-06 | Disabled-tooltip exact text | Same as TC-FR4-05 | Hover disabled Load button | Tooltip text exactly `Enter a ProTxHash to continue.` | FR-4, §7 copy |
| TC-FR4-07 | Load button enables once ProTxHash populated | Load form open | Type a valid ProTxHash | Button becomes enabled (regardless of key fields) | FR-4 |
| TC-FR4-08 | ProTxHash accepts hex | Load form open | Enter a valid hex ProTxHash, submit | Node loads with the entered ProTxHash | FR-4, Domain Notes §3 |
| TC-FR4-09 | ProTxHash accepts Base58 | Load form open | Enter a valid Base58 ProTxHash, submit | Node loads with the entered ProTxHash resolved correctly | FR-4 |
| TC-FR4-10 | Load with ProTxHash only → view-only node | Load form open | Enter ProTxHash, leave all 3 key fields blank, submit | Node loads; detail view Keys section shows Voting/Owner/Payout all absent (read-only per empty-state copy) | FR-4, FR-2 copy "loads read-only without them" |
| TC-FR4-11 | Load with all three keys present | Load form open | Enter ProTxHash + Voting + Owner + Payout keys, submit | Detail view Keys section shows all three `loaded ✓` | FR-4, FR-5 |
| TC-FR4-12 | Load with only Voting key | Load form open | Enter ProTxHash + Voting key only, submit | Detail view shows Voting loaded, Owner/Payout absent | FR-4 |
| TC-FR4-13 | Alias becomes card heading when set | Load form open | Enter ProTxHash + alias `mn-east-01`, submit | Card list shows `mn-east-01` as heading (cross-ref TC-FR3-03) | FR-4, FR-3 |
| TC-FR4-14 | Alias is local-only, not sent to Platform | Load form open | Enter ProTxHash + alias, submit | Alias persisted only in local DB; no outbound state-transition/network call includes the alias value | FR-4, §7 copy "not saved to the Dash network" |
| TC-FR4-15 | Key inputs accept WIF | Load form open | Paste a WIF-formatted private key into Voting field, submit | Key accepted and loaded | FR-4, Domain Notes §3 |
| TC-FR4-16 | Key inputs accept hex | Load form open | Paste a hex-formatted private key into Owner field, submit | Key accepted and loaded | FR-4 |
| TC-FR4-17 | Reveal control is hold/press semantics | Load form open, key entered | Press-and-hold the eye icon on a key field, then release | Plaintext visible while pressed; masked again on release (per password-input hold-to-reveal pattern, not a persistent toggle) | ux-spec §3, docs/ux-design-patterns.md §5 |
| TC-FR4-18 | Warning-tone note always visible | Load form open, regardless of field state | Inspect form | Note text exactly `Set an optional password to encrypt these keys on this device. Without one, they are stored unencrypted and you can add protection later from the key screen.` is always rendered (non-blocking, unconditional) | FR-4, NFR-4, §7 copy |
| TC-FR4-19 | Load-error path shows friendly MessageBanner | Backend load task returns a `TaskError` (e.g. network failure resolving ProTxHash) | Submit a ProTxHash that triggers a backend error | Error `MessageBanner` shown with a user-friendly message; technical detail attached via `.with_details(e)`; no raw error string/stack trace visible in the banner text itself | FR-4, US-1 "Failure paths", CLAUDE.md error-message rules |
| TC-FR4-20 | Successful load returns to list with fresh form state | Load form open | Submit a valid load, later reopen "Load a masternode" | New card appears in list; reopened form has no residual data from the previous submission | FR-4, US-1 bullet 3 |
| TC-FR4-21 | Cancel discards without loading | Load form open, fields populated | Click Cancel | Returns to list; no new card created; no backend load task dispatched | FR-4, wireframe C |
| TC-FR4-22 | Old advanced-options arm is removed *(RESOLVED — was `[AMBIGUOUS]`)* | Legacy `add_existing_identity_screen.rs` Identity Type dropdown | Open Show Advanced Options on the legacy Add Existing Identity screen | Masternode/Evonode options are **removed** from the dropdown (User-only remains) — no duplicate entry point for loading node identities (§10.2, extends FR-6) | FR-4, FR-6 |

---

## FR-5 — Masternode detail / voting view composition

| ID | Description | Preconditions | Steps | Expected Outcome | Traces to |
|---|---|---|---|---|---|
| TC-FR5-01 | **Section order — Actions ABOVE Keys (explicit late correction)** | Detail view open for a fully-keyed node | Read section order top→bottom | Order is exactly: Header → Credit-actions row (incl. Evonode-only cross-link) → Keys section (with "Manage keys ›") → collapsible DPNS voting section → Remove. **Actions row must render before the Keys section**, not after. | FR-5 "Grouping (top→bottom)", explicit human-requested change |
| TC-FR5-02 | Header alias line conditional | Node with alias vs. node without alias | Open both detail views | Aliased node shows alias in header; unaliased node's header omits the alias line entirely (no empty placeholder) | FR-5 |
| TC-FR5-03 | Header ProTxHash + copy affordance | Detail view open | Click the copy icon next to the shortened ProTxHash | ProTxHash is written to clipboard (full value, not the shortened display string) | FR-5 |
| TC-FR5-04 | Header badge matches card badge | Same node, compare card vs detail | Open card then detail | Badge colour/text identical between card and detail header | FR-5, FR-3 |
| TC-FR5-05 | Header status + label | Detail view open | Read status | `IdentityStatus` dot + text label shown (never colour-only) | FR-5, NFR-6 |
| TC-FR5-06 | Detail reachable via card click and via masternode pill | ≥1 node loaded | (a) click card (b) select node from masternode-pill dropdown | Both paths open the same detail view for the chosen node | FR-5, FR-GLOBAL-NAV-3 |
| TC-FR5-07 | Back row returns to list | Detail view open | Click `‹ All masternodes` | Returns to card list (content-panel back row, header/global switcher unchanged) | FR-5, FR-GLOBAL-NAV-5 |

---

## FR-6 — Filter masternode/evonode out of user-only pickers (Hub-picker-only scope)

| ID | Description | Preconditions | Steps | Expected Outcome | Traces to |
|---|---|---|---|---|---|
| TC-FR6-01 | Masternode excluded from Hub picker | 1 Masternode identity loaded | Open Identity Hub picker | Masternode identity is absent from the picker list | FR-6, US-5 bullet 1 |
| TC-FR6-02 | Evonode excluded from Hub picker | 1 Evonode identity loaded | Open Identity Hub picker | Evonode identity is absent from the picker list | FR-6, US-5 bullet 1 |
| TC-FR6-03 | Masternode still visible in legacy Identities table | 1 Masternode identity loaded | Open legacy `identities_screen.rs` table | Masternode identity IS listed (locked decision #2 — Hub-picker-only scope, not a full removal) | FR-6, ux-spec §Locked decisions #2 |
| TC-FR6-04 | Evonode still visible in legacy Identities table | 1 Evonode identity loaded | Open legacy Identities table | Evonode identity IS listed | FR-6, ux-spec §Locked decisions #2 |
| TC-FR6-05 | User identity unaffected (control case) | 1 User identity loaded alongside MN/Evonode | Open both Hub picker and legacy table | User identity appears in BOTH surfaces (confirms filter is type-scoped, not a blanket removal) | FR-6, US-5 |
| TC-FR6-06 | MN/Evonode present on Masternodes tab | Same identities loaded | Open Masternodes tab | Both MN and Evonode identities appear as cards | FR-6, US-5 bullet 2 |
| TC-FR6-07 | Masternode-pill selection never leaks into app-global user-identity selection | Masternode selected via Masternodes-page pill | Navigate to Dashpay/Identities/Identity Hub | The app-global identity pill/selection there shows the User identity (or none), never the masternode just selected | FR-6 boundary, FR-GLOBAL-NAV-3, US-7 bullet 4 (duplicated for emphasis; full nav coverage under TC-NAV-12) |

---

## FR-7 — Refresh

| ID | Description | Preconditions | Steps | Expected Outcome | Traces to |
|---|---|---|---|---|---|
| TC-FR7-01 | Refresh button present, styled per `01-dashpay.png` | Card list open | Inspect top-right toolbar | `Refresh` button present, `add_toolbar_button` styling | FR-7 |
| TC-FR7-02 | Refresh dispatches re-fetch task | Card list open | Click Refresh | A backend task re-fetching masternode identity + voting state is dispatched (assert task variant, not merely a UI repaint) | FR-7 |
| TC-FR7-03 | Refresh reflects updated state | Underlying vote/identity state changed externally since last load | Click Refresh | Card list content updates to reflect the new state (data freshness assertion, not just "no crash") | FR-7 |
| TC-FR7-04 | Refresh also present on detail view | Detail view open | Inspect header | Refresh button present and functional on the detail screen too | FR-7, wireframe D |

---

## FR-8 / US-8 — Optional load-time key encryption password

| ID | Description | Preconditions | Steps | Expected Outcome | Traces to |
|---|---|---|---|---|---|
| TC-FR8-01 | Blank password → Tier-1 keyless persistence | Load form open, password field left blank, ≥1 key entered | Submit | Keys persisted via the unprotected path (`put_secret`/`store` — not `put_secret_protected`) | FR-8, US-8 bullet 1 |
| TC-FR8-02 | Non-blank password → Tier-2 sealed persistence at load | Load form open, password set, ≥1 key entered | Submit | Keys persisted via `put_secret_protected`/`store_protected` (Argon2id + XChaCha20-Poly1305) at load time, not only reachable post-load | FR-8, US-8 bullet 2 |
| TC-FR8-03 | Password reveal is hold-to-reveal, not toggle | Password field has a value | Press-and-hold eye icon, then release | Plaintext shown while held; masked again immediately on release | FR-8, ux-spec §3 "password-input pattern" |
| TC-FR8-04 | Password never logged | Load with a distinctive password value, e.g. `Tr0ub4dor&3-QA` | Submit, then inspect application logs (`RUST_LOG` output) | The literal password string never appears in logs | FR-8, "never logged, never stored" |
| TC-FR8-05 | Password/plaintext keys never stored unencrypted when Tier-2 chosen | Same as TC-FR8-02 | Inspect DB/secret-vault contents directly | Only the Argon2id-derived / XChaCha20-Poly1305-sealed envelope is present; no plaintext key material or password readable at rest | FR-8, security |
| TC-FR8-06 | Detail view reflects Tier-1 unprotected state | Node loaded with blank password | Open detail view | Shows `Keys: unprotected`; "Add password protection…" action IS offered | FR-8, FR-5, US-8 bullet 1 |
| TC-FR8-07 | Detail view reflects Tier-2 protected state, no redundant action | Node loaded with a password set | Open detail view | Shows `Keys: password-protected`; "Add password protection…" action is NOT offered (already protected) | FR-8, US-8 bullet 4 |
| TC-FR8-08 | "Add password protection…" reuses existing task | Detail view for a Tier-1 node | Click "Add password protection…" | Dispatches the existing `IdentityTask::ProtectIdentityKeys` (no new crypto path introduced) | FR-8, FR-5, NFR-4 |
| TC-FR8-09 | Password validation matches existing Add-password-protection rule *(RESOLVED — was `[AMBIGUOUS]`)* | Load form open | Enter a password, submit | No new policy is invented: validation is identical to whatever the existing Key Info screen's "Add password protection…" flow already enforces, since FR-8 reuses the same `store_protected`/`put_secret_protected` seal path (§10.3) — confirm the existing rule at implementation time | FR-8 |
| TC-FR8-10 | Identity key also sealed when password set (not just V/O/P) | Load form open, password set, load produces an identity key too | Submit, inspect secret store | The identity key (per FR-8 "(and identity) keys") is sealed Tier-2 alongside voting/owner/payout, not left unprotected | FR-8 |

---

## FR-9 / US-9 — Credit actions (Withdraw / Top up / Transfer)

| ID | Description | Preconditions | Steps | Expected Outcome | Traces to |
|---|---|---|---|---|---|
| TC-FR9-01 | Actions row present for Masternode | Masternode detail view | Inspect Actions row | `Withdraw`, `Top up`, `Transfer` all present | FR-9, US-9 bullet 1 |
| TC-FR9-02 | Actions row present for Evonode | Evonode detail view | Inspect Actions row | Same three actions present (FR-9 explicit "both Masternode and Evonode") | FR-9, US-9 bullet 1 |
| TC-FR9-03 | Withdraw scoped to correct identity | Detail view for node X | Click Withdraw | `withdraw_screen` opens with `QualifiedIdentity` == node X (not the app-global User identity or any other node) | FR-9 |
| TC-FR9-04 | Top up scoped to node, wallet = current wallet pill | Detail view for node X, wallet pill = Wallet A | Click Top up | `top_up_identity_screen` opens scoped to node X with source wallet = Wallet A | FR-9, FR-GLOBAL-NAV-3 |
| TC-FR9-05 | Transfer scoped to node | Detail view for node X | Click Transfer | `transfer_screen` opens scoped to node X | FR-9 |
| TC-FR9-06 | Owner-key withdraw forces payout-address destination | Withdraw flow initiated with the node's owner key | Attempt to set a custom destination address | Destination field is fixed to the node's registered Core payout address; not user-editable | FR-9, US-9 bullet 2, Domain Notes §3 |
| TC-FR9-07 | Transfer/payout-key withdraw allows free destination | Withdraw flow initiated with the transfer/payout key | Enter a custom destination address | Destination field accepts any user-chosen address | FR-9, US-9 bullet 2 |
| TC-FR9-08 | Reuse, not reimplementation | All three actions | Trigger each from the Masternodes detail view and independently from wherever else in the app they're already reachable | Same screen struct/type is pushed in both cases (structural reuse assertion — no parallel MN-specific implementation) | FR-9 "reuse existing screens", NFR-1 |

---

## FR-10 / US-10 — Manage-keys drill-in (KeyInfoScreen)

| ID | Description | Preconditions | Steps | Expected Outcome | Traces to |
|---|---|---|---|---|---|
| TC-FR10-01 | Drill-in opens existing KeyInfoScreen | Detail view, Keys section | Click "Manage keys ›" | Existing `KeyInfoScreen` opens scoped to this node's identity | FR-10, US-10 bullet 1 |
| TC-FR10-02 | View private key/WIF | KeyInfoScreen open for node | Select a loaded key | Private key/WIF viewable | FR-10 |
| TC-FR10-03 | Sign message | KeyInfoScreen open for node | Use sign-message action | Message signed using node's key | FR-10 |
| TC-FR10-04 | Add-key selector excludes OWNER (Masternode) | KeyInfoScreen for a Masternode | Open add-key purpose selector | `OWNER` not offered | FR-10, US-10 bullet 2, Domain Notes §3 |
| TC-FR10-05 | Add-key selector excludes VOTING (Masternode) | Same | Same | `VOTING` not offered | FR-10, US-10 bullet 2 |
| TC-FR10-06 | Add-key selector excludes OWNER (Evonode) | KeyInfoScreen for an Evonode | Open add-key purpose selector | `OWNER` not offered | FR-10 |
| TC-FR10-07 | Add-key selector excludes VOTING (Evonode) | Same | Same | `VOTING` not offered | FR-10 |
| TC-FR10-08 | Rule applies to User identities too (not MN-specific) | KeyInfoScreen for a User identity | Open add-key purpose selector | `OWNER`/`VOTING` also excluded here (regression check confirming this is a platform-wide rule, not new MN-only logic) | FR-10 "not MN-specific — document it" |
| TC-FR10-09 | TRANSFER offered | Any identity type's KeyInfoScreen | Open selector | `TRANSFER` present | FR-10 |
| TC-FR10-10 | AUTHENTICATION offered | Same | Same | `AUTHENTICATION` present | FR-10 |
| TC-FR10-11 | ENCRYPTION offered | Same | Same | `ENCRYPTION` present | FR-10 |
| TC-FR10-12 | DECRYPTION offered | Same | Same | `DECRYPTION` present | FR-10 |
| TC-FR10-13 | Remove-key reachable | KeyInfoScreen for node with ≥2 keys | Use remove-key action on a non-critical key | Key removed (existing capability, unchanged) | FR-10 |

---

## FR-11 / US-11 — Evonode token-rewards cross-link

| ID | Description | Preconditions | Steps | Expected Outcome | Traces to |
|---|---|---|---|---|---|
| TC-FR11-01 | Cross-link shown for Evonode | Evonode detail view | Inspect Actions row | `Claim token rewards ›` shown | FR-11, US-11 bullet 1 |
| TC-FR11-02 | Cross-link hidden for plain Masternode (explicit negative case) | Masternode detail view | Inspect Actions row | `Claim token rewards ›` is **absent** — not disabled, not present-but-greyed | FR-11, US-11 bullet 2 |
| TC-FR11-03 | Cross-link routes to existing ClaimTokensScreen | Evonode detail view | Click `Claim token rewards ›` | Existing `ClaimTokensScreen` opens scoped to this Evonode identity | FR-11 |
| TC-FR11-04 | No new claim UI on Masternodes page | Same | Same | Screen pushed is the same `ClaimTokensScreen` type used elsewhere in the app (structural reuse, no MN-page-local reimplementation) | FR-11, NFR-1 |

---

## FR-12 — "Fill Random Masternode/Evonode" (Testnet-only, fixture-conditional)

| ID | Description | Preconditions | Steps | Expected Outcome | Traces to |
|---|---|---|---|---|---|
| TC-FR12-01 | Button renders: Testnet + fixture present + Masternode toggle | Network=Testnet, `.testnet_nodes.yml` present and parses, toggle=Masternode | Open load form | `🎲 Fill Random Masternode` button + hint row render | FR-12 |
| TC-FR12-02 | Button label follows toggle: Evonode | Same fixture conditions, toggle=Evonode | Open load form | Button label reads `Fill Random Evonode` (matches toggle, per FR-12 "label follows Node-type toggle") | FR-12 |
| TC-FR12-03 | Button absent when fixture file missing (not disabled) | Network=Testnet, `.testnet_nodes.yml` absent | Open load form | The entire button+hint row is absent — assert widget/element count for that row is 0, not `disabled=true`; form flows directly from Node-type to ProTxHash | FR-12 "MUST be conditional... never shown-but-disabled" |
| TC-FR12-04 | Button absent when fixture unparseable, no crash/no error banner | Network=Testnet, `.testnet_nodes.yml` present but malformed YAML | Open load form | Button absent; no panic; no `MessageBanner` error surfaced for this case. **Note (PROJ-004):** the loader returns `Ok(None)` only for a *missing* file; a *malformed* file returns `Err(_)` which the legacy screen banners — so the new form must **swallow `Err(_)` → absent** (a deliberate divergence from legacy, `tracing::debug!` the parse error), not verbatim reuse | FR-12; loader `Ok(None)` on absent, `Err` on malformed (`add_existing_identity_screen.rs:58-71,151-161`) |
| TC-FR12-05 | Button absent on Mainnet regardless of fixture | Network=Mainnet, fixture present | Open load form | Button absent (Testnet-only gate) | FR-12 |
| TC-FR12-06 | Button absent on Devnet | Network=Devnet, fixture present | Open load form | Button absent | FR-12 |
| TC-FR12-07 | Clicking Fill Random autofills from a real fixture entry *(CORRECTED 2026-07-09, PROJ-003 — Masternode fixture has no payout key)* | Testnet + fixture present, toggle=Masternode | Click `Fill Random Masternode` | ProTxHash + **Voting + Owner** fields populate from one of the fixture's `masternodes` entries (`MasternodeInfo` has no payout field, so `fill_random_masternode()` fills V+O only — Payout stays blank; the Evonode/`hp_masternodes` path fills all three incl. Payout, TC-FR12-08) — not blank, not synthetic | FR-12, `add_existing_identity_screen.rs:30-35,979-993` |
| TC-FR12-08 | Correct fixture list consulted per node type | Testnet + fixture present | Click Fill Random with toggle=Masternode, then separately with toggle=Evonode | Masternode toggle pulls from fixture's `masternodes` list; Evonode toggle pulls from `hp_masternodes` list (verified against `fill_random_masternode()`/`fill_random_hpmn()`) | FR-12 |
| TC-FR12-09 | `[DEFERRED, not ambiguous]` Defense-in-depth `developer_mode` re-check at button call-site | Load form reachable only via Expert-Mode-gated nav (FR-1) | N/A | FR-12 explicitly defers this as an implementation judgment call for Nagatha's plan, not an open requirements question — Nagatha's plan should record its own decision, not this spec | FR-12 "Flag for Nagatha" |
| TC-FR12-10 | Node type toggle clears autofilled fields *(RESOLVED — was `[AMBIGUOUS]`)* | Fields already autofilled via Fill Random for Masternode | Switch toggle to Evonode | ProTxHash, Alias, and all key fields are **cleared** — a real node's identity is tied to one type; data for one is never valid for the other (§10.6) | FR-12, wireframe C |

---

## Global Nav — US-7 / FR-GLOBAL-NAV-1…6

| ID | Description | Preconditions | Steps | Expected Outcome | Traces to |
|---|---|---|---|---|---|
| TC-NAV-01 | Page-aware leftmost crumb | On Masternodes tab | Read header crumb | Leftmost segment reads `Masternodes` (not `Identities`), links to the Masternodes root | FR-GLOBAL-NAV-6, US-7 bullet 1 |
| TC-NAV-02 | Same switcher component as Identity Hub | Masternodes tab open | Inspect rendered widget | Same `breadcrumb_switcher.rs` component via `add_top_panel_with_breadcrumb` used, not a bespoke reimplementation | FR-GLOBAL-NAV-1, NFR-1 |
| TC-NAV-03 | Wallet pill Interactive on Masternodes | Any Masternodes screen (list/load/detail) | Inspect wallet pill | Renders `Interactive` mode: caret, clickable | FR-GLOBAL-NAV-3, US-7 |
| TC-NAV-04 | Masternode pill Placeholder when 0 nodes | Empty state | Inspect masternode pill | Renders `Placeholder`: `(no masternode yet)`, italic, no caret | FR-GLOBAL-NAV-3, wireframe A |
| TC-NAV-05 | Masternode pill "Choose a masternode" on list with no selection | ≥1 node loaded, list view, no node opened yet | Inspect masternode pill | `Interactive`, text `Choose a masternode ▾` | wireframe B |
| TC-NAV-06 | Wallet selection is silent, no forced navigation | On Masternodes tab | Select a different wallet from the wallet-pill dropdown | `AppContext::selected_wallet_hash` updates; page remains on Masternodes tab (no navigation away) | FR-GLOBAL-NAV-2 rule 1, US-7 bullet 2 |
| TC-NAV-07 | Wallet-pill → Top-up binding (pill drives page) | Wallet pill changed to Wallet B | Open Top up on a node | Top-up screen's source wallet = Wallet B | FR-GLOBAL-NAV-2 rule 2, US-7 bullet 2 |
| TC-NAV-08 | Top-up → wallet-pill binding (page drives pill) | Top-up screen open with source wallet initially = Wallet A | Change source wallet inside the Top-up flow to Wallet C | Top-nav wallet pill updates to show Wallet C | FR-GLOBAL-NAV-2 rule 2 ("two-way"), US-7 bullet 2 |
| TC-NAV-09 | Card click → masternode-pill binding | List view, ≥2 nodes | Click card for node X | Masternode pill updates to show node X | FR-GLOBAL-NAV-3, US-7 bullet 3 |
| TC-NAV-10 | Masternode-pill selection → detail navigation | List or detail view, ≥2 nodes | Pick node Y from masternode-pill dropdown | Detail view for node Y opens | FR-GLOBAL-NAV-3, US-7 bullet 3 |
| TC-NAV-11 | Masternode-pill dropdown content correctness | 3 nodes loaded | Open masternode-pill dropdown | Exactly the 3 loaded MN/Evonode identities are listed, no others, no duplicates | FR-GLOBAL-NAV-3 |
| TC-NAV-12 | **FR-6 boundary — masternode selection never leaks to user pages (critical)** | Masternode selected via Masternodes-page pill | Navigate to Dashpay, then Identities, then Identity Hub | On every one of these pages, the identity pill/selection reflects the app-global **User** identity (or none) — never the masternode | FR-GLOBAL-NAV-3, FR-6, US-7 bullet 4 |
| TC-NAV-12b | **FR-6 boundary — first-loaded fallback never resolves a masternode** *(added 2026-07-09, PROJ-001)* | Exactly one identity loaded and it is a Masternode/Evonode; nothing explicitly selected | Open Dashpay/Identities/Identity Hub | `resolve_selected_identity()` returns the User identity or `None`, **never** the masternode via the first-loaded fallback (`context/mod.rs:1099-1105` filters MN/Evonode at the resolution layer, not just display) | FR-6, PROJ-001a |
| TC-NAV-12c | **FR-6 boundary — stale persisted MN selection sanitized on load** *(added 2026-07-09, PROJ-001)* | A masternode was persisted as `selected_identity_id` in a prior session (masternodes were Hub-pickable then) | Launch/build with the new FR-6 filter and open an everyday-user page | The stale MN/Evonode selection is cleared on context load; the identity pill shows a User identity or none, and operate-as reads never resolve the masternode | FR-6, PROJ-001b |
| TC-NAV-13 | Unwired-page pill renders Subdued | A page not yet wired to consume a given selection | Inspect its pill | Dimmed, no caret, no visible text tag | FR-GLOBAL-NAV-2 rule 3, US-7 bullet 5 |
| TC-NAV-14 | Subdued pill tooltip explains how to change selection | Same | Hover the Subdued pill | Tooltip text present, non-empty, page-specific (e.g. "Change the active wallet from the Wallets tab") | FR-GLOBAL-NAV-2 rule 3 |
| TC-NAV-15 | Per-page composition: wallet-only page shows one pill | A page with no identity/object context (e.g. a Wallet page) | Inspect switcher | Only the wallet pill renders; no third segment at all | FR-GLOBAL-NAV-2 rule 4, US-7 bullet 6 |
| TC-NAV-16 | Sub-screen nav doesn't disturb the global switcher | On list view, open load form or a node's detail | Compare header before/after | Global switcher stays single-line/unchanged; a separate `‹ All masternodes` back row appears in the content panel instead | FR-GLOBAL-NAV-5 |
| TC-NAV-17 | Everyday-page identity-pill dropdown never lists MN/Evonode | On Dashpay/Identities/Identity Hub, MN+Evonode+User identities all loaded | Open the identity pill's dropdown | Only User identities listed | FR-GLOBAL-NAV-4 |
| TC-NAV-18 | Masternode-pill resets to placeholder on list return *(RESOLVED — was `[AMBIGUOUS]`)* | Detail view for node X open | Click `‹ All masternodes` | Pill resets to `Choose a masternode ▾` placeholder — it reflects the current screen's context (specific node only on the detail view), not "last node opened" (§10.4, matches wireframe B as drawn) | FR-GLOBAL-NAV-3, §4b |

---

## DPNS voting section (FR-5 collapsible + US-3)

| ID | Description | Preconditions | Steps | Expected Outcome | Traces to |
|---|---|---|---|---|---|
| TC-DPNS-01 | Collapsed by default | Detail view freshly opened | Observe section state | Collapsing section starts collapsed (`▸`) | FR-5, ux-spec §3 |
| TC-DPNS-02 | Count visible in collapsed header | Node has 3 open contests | Read collapsed header | Reads `DPNS name contests to vote on (3)` — count matches actual open-contest count | FR-5 |
| TC-DPNS-03 | Expand reveals per-contest choices | Collapsed section | Click header | Expands (`▾`); each contested name shows Abstain / Lock / Vote-for-candidate | FR-5, US-3 bullet 1 |
| TC-DPNS-04 | Candidate dropdown scoped per contest | Section expanded | Open the "Vote for" dropdown on one contested name | Only candidates registered for that specific name appear (not a global candidate list) | FR-5 |
| TC-DPNS-05 | Cast votes dispatches real backend | Section expanded, choice selected for ≥1 name | Click "Cast votes" | Vote is dispatched through the existing `contested_names/vote_on_dpns_name.rs` backend with the correct name/choice/identity parameters | FR-5, US-3 bullet 2 |
| TC-DPNS-06 | Success feedback auto-dismisses | Vote cast successfully | Observe banner | Success `MessageBanner` shown, auto-dismisses (per journey 2.2) | US-3 |
| TC-DPNS-07 | Scheduled/past votes are out of scope for this section *(RESOLVED — was `[AMBIGUOUS]`)* | Node has both a scheduled vote and past votes | Expand section | Only **active, open contests** render here (exactly as wireframe D draws it); scheduled/past-vote history is not duplicated on this page — it already lives on the existing DPNS Scheduled Votes root screen (§10.7) | FR-5 |
| TC-DPNS-08 | Zero-open-contests empty copy | Node has 0 open contests | Expand section | Body text exactly `There are no open name contests for this node to vote on right now.`, no contest table | §7 copy |
| TC-DPNS-09 | **Missing voter identity → actionable message, not raw error (critical)** | `associated_voter_identity` is `None` | Open/expand voting section | Shows exactly `This node has no voting key loaded. Add its voting private key to cast votes.` plus a secondary `( Add voting key )` action — the raw `NoVotingIdentity` error type/string is never surfaced to the user | US-3 bullet 3, §7 copy, CLAUDE.md error-message rules |
| TC-DPNS-10 | "Add voting key" opens the scoped in-place prompt *(CORRECTED 2026-07-09, PROJ-002 — was "load form opens pre-filled", which contradicted TC-DPNS-11/§10.8)* | Voting section showing the missing-voter-identity state | Click `( Add voting key )` | A **scoped voter-key-input prompt** opens with this node's context pre-bound — it is **not** FR-4's load form (consistent with §10.8 / TC-DPNS-11); no ProTxHash re-entry | ux-spec wireframe D note, §10.8 |
| TC-DPNS-11 | "Add voting key" is a scoped in-place action, not a load-form resubmission *(RESOLVED — was `[AMBIGUOUS]`)* | Load form pre-filled per TC-DPNS-10 | Submit with a new voting key entered | "Add voting key" (US-3) opens a small, scoped key-input prompt that updates the voter identity on the already-loaded node in place — it is a different flow from FR-4's load form, so the duplicate-ProTxHash rejection (TC-EDGE-07) does not apply here (§10.8) | FR-4, wireframe D |

---

## US-4 — Remove a masternode

| ID | Description | Preconditions | Steps | Expected Outcome | Traces to |
|---|---|---|---|---|---|
| TC-US4-01 | Remove button present | Detail view open | Inspect footer | `Remove masternode` danger button present | US-4 bullet 1, FR-5 |
| TC-US4-02 | Confirmation dialog with specific verb | Detail view open | Click Remove | `ConfirmationDialog` opens, `danger_mode(true)`, verb label `Remove masternode` | US-4 bullet 1, §7 copy |
| TC-US4-03 | Cancel/Remove button placement | Confirmation open | Inspect dialog | Cancel positioned left, Remove positioned right | ux-spec §3 |
| TC-US4-04 | Escape cancels | Confirmation open | Press Escape | Dialog closes, node NOT removed | ux-spec §3 |
| TC-US4-05 | Confirm removes node AND its voter identity | Node with an associated voter identity, confirmation open | Confirm | Both the masternode/evonode identity row AND its associated voter identity row are deleted from the DB (assert both, not just the card disappearing) | US-4 bullet 2, journey 2.3 |
| TC-US4-06 | Returns to list, card gone | Same, post-confirm | Observe screen | Back on card list; removed node's card no longer present | US-4 bullet 2 |
| TC-US4-07 | Isolation — other nodes unaffected | ≥2 nodes loaded, remove one | Confirm removal of node X | Node Y's card, keys, and voter identity remain fully intact (no over-deletion) | US-4, data integrity |

---

## Edge / failure cases

| ID | Description | Preconditions | Steps | Expected Outcome | Traces to |
|---|---|---|---|---|---|
| TC-EDGE-01 | Enter-key submit bypass with empty ProTxHash | Load form open, ProTxHash empty, focus in another field | Press Enter | No backend load task is dispatched (disabled state cannot be bypassed via keyboard submit) | FR-4, US-1 bullet 2 |
| TC-EDGE-02 | Load error is friendly + non-leaking | Backend load fails (e.g. simulated network error) | Submit a ProTxHash that triggers the failure | `MessageBanner` (Error), persistent (not auto-dismiss), `.with_details(e)` attached, visible message contains no raw SDK/DB error text or stack trace | FR-4, journey 2.1 "Failure paths" |
| TC-EDGE-03 | Missing voter identity at vote time is actionable | Voting section, no voter identity | Attempt to vote | Actionable message shown (cross-ref TC-DPNS-09), never the raw `NoVotingIdentity` error | US-3 bullet 3 |
| TC-EDGE-04 | No wallet loaded, attempt Top up *(RESOLVED 2026-07-09 — was `[AMBIGUOUS]`)* | 0 wallets loaded, Masternodes tab open | Inspect wallet pill and attempt Top up | Behavior matches whatever the existing, reused `top_up_identity_screen` already does with 0 wallets loaded — this design adds an entry point only and does not redefine that screen's no-wallet handling (§10.5) | FR-9 |
| TC-EDGE-05 | Card list scoped per active network | Node A loaded under Testnet, network switched to Mainnet | Switch network, open Masternodes tab | Card list shows only Mainnet-scoped nodes; node A (Testnet) is not shown until switching back | FR-1 "survives network switches", NFR consistency with other root tabs |
| TC-EDGE-06 | Network switch mid-sub-screen returns to the list *(RESOLVED — was `[AMBIGUOUS]`)* | Load form (with Fill Random visible, Testnet) or detail view open | Switch network away from Testnet mid-screen | App returns to the Masternodes **list** for the new network, matching TC-EDGE-05's "card list scoped per active network" rule — no stale sub-screen referencing a now-foreign identity (§10.10) | FR-12, FR-1 |
| TC-EDGE-07 | Duplicate-ProTxHash load is rejected *(RESOLVED — was `[AMBIGUOUS]`)* | Node with ProTxHash P already loaded | Submit the load form again with the same ProTxHash P | Duplicate-node error shown (§7 copy: "This masternode is already loaded…"); no second card created, existing node not silently updated (§10.9) | FR-4 |
| TC-EDGE-08 | Malformed ProTxHash validated inline *(RESOLVED — was `[AMBIGUOUS]`)* | Load form open | Enter a syntactically invalid ProTxHash (wrong length/charset), attempt submit | Client-side format validation fires inline/on-blur (not only gated on emptiness); error copy per §7: "This doesn't look like a valid ProTxHash…" (§10.9) | FR-4 |

---

## NFR checks (execution-verifiable subset)

| ID | Description | Preconditions | Steps | Expected Outcome | Traces to |
|---|---|---|---|---|---|
| TC-NFR4-01 | Warning note is non-blocking | Load form open | Fill only ProTxHash, leave password/note area untouched | Load button enabled purely on ProTxHash presence; note's existence does not gate submission | NFR-4, FR-4 |
| TC-NFR6-01 | Card has a single accessible label | Card list, ≥1 card | Query accessibility tree (kittest) | Card exposes `WidgetInfo::labeled(Button, ..., "Open {node}")`, single click target | NFR-6 |
| TC-NFR6-02 | Focus order top-to-bottom on load form | Load form open | Tab through fields | Order follows visual top-to-bottom layout, ending on the primary action (Load masternode) last | NFR-6 |
| TC-NFR6-03 | No colour-only status anywhere in the feature | Card list + detail view, all status indicators (voter readiness, identity status, key protection tier) | Inspect each | Every status indicator pairs its colour with a text label | NFR-6 (regression across FR-3/FR-5/FR-8) |
| TC-NFR6-04 | Disabled Load button carries disabled-tooltip semantics | Load form, ProTxHash empty | Inspect button state via kittest | `enabled() == false`; tooltip text retrievable and matches TC-FR4-06 | NFR-6, FR-4 |

---

## Coverage summary

| Group | Count |
|---|---|
| FR-1 (Expert Mode gating) | 8 |
| FR-2 (Empty state) | 7 |
| FR-3 (Card list) | 15 |
| FR-4 (Load flow) | 22 |
| FR-5 (Detail composition) | 7 |
| FR-6 (Hub-picker filter) | 7 |
| FR-7 (Refresh) | 4 |
| FR-8 / US-8 (Load-time password) | 10 |
| FR-9 / US-9 (Credit actions) | 8 |
| FR-10 / US-10 (Manage-keys drill-in) | 13 |
| FR-11 / US-11 (Evonode cross-link) | 4 |
| FR-12 (Fill Random) | 10 (1 intentionally deferred, see below) |
| Global Nav (US-7 / FR-GLOBAL-NAV) | 20 |
| DPNS voting section (FR-5 / US-3) | 11 |
| US-4 (Remove) | 7 |
| Edge/failure cases | 8 |
| NFR (execution-verifiable subset) | 5 |
| **Total** | **166** (0 open ambiguities remain; +2 = TC-NAV-12b/12c added 2026-07-09 for the PROJ-001 FR-6 resolution-layer boundary) |

US-6 retired — 0 test cases, documented above.

---

## Ambiguities/gaps surfaced — RESOLVED 2026-07-09 (folded back before Nagatha's plan)

All 12 gaps originally surfaced in this closing section were resolved by the coordinator in `01-requirements.md`
§10 and reflected in the corresponding test-case rows above (each row now reads "RESOLVED" inline with a
§10.N citation). One item (TC-FR12-09) was reclassified `[DEFERRED, not ambiguous]` — it was always an
intentional implementation judgment call for Nagatha's plan, not an open requirements question. A **13th gap,
TC-FR1-05b** (live de-gating fallback screen), existed in the FR-1 test cases but was omitted from this
closing list in the original pass — found by the coordinator during a direct sweep and resolved in §10.11.
Original list, kept for traceability:

1. ~~TC-FR2-05~~ — empty-state reassurance-line copy → canonicalized in 01-requirements.md §7 (wireframes.html's wording wins).
2. ~~TC-FR3-11~~ — DPNS status precedence → §10.1 (count-first, then scheduled, then none; reuses existing Scheduled Votes state).
3. ~~TC-FR4-22~~ — legacy advanced-options arm → §10.2 (removed, extends FR-6).
4. ~~TC-FR8-09~~ — password strength rule → §10.3 (reuses existing Add-password-protection validation, no new policy).
5. TC-FR12-09 → reclassified `[DEFERRED, not ambiguous]`; TC-FR12-10 → §10.6 (node-type toggle clears autofilled fields).
6. ~~TC-NAV-18~~ — masternode-pill state on list return → §10.4 (resets to placeholder).
7. ~~TC-DPNS-07~~ — scheduled/past votes → §10.7 (out of scope by design; already covered by the existing Scheduled Votes screen).
8. ~~TC-DPNS-11~~ — "Add voting key" resubmission → §10.8 (scoped in-place action, not a load-form resubmission).
9. ~~TC-EDGE-04~~ — no-wallet Top-up behavior → §10.5 (inherits the existing reused Top-up screen's behavior, unchanged).
10. ~~TC-EDGE-06~~ — network switch mid-sub-screen → §10.10 (returns to the list, matches TC-EDGE-05).
11. ~~TC-EDGE-07~~ — duplicate-ProTxHash load → §10.9 (rejected with a friendly error, §7 copy).
12. ~~TC-EDGE-08~~ — malformed-ProTxHash validation → §10.9 (inline/on-blur, §7 copy).
13. ~~TC-FR1-05b~~ *(found outside this list)* — live de-gating fallback → §10.11 (falls back to Identities).

None of these were silently dropped — each had a recorded, traceable test-case ID, and each now has a decision
+ an updated expected-outcome to assert against.
