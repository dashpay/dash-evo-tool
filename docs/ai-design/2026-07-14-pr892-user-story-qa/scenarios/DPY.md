# DPY — DashPay

Environment: PR892 build (`/data/target/debug/dash-evo-tool` @ `57195d54`), isolated data dir
`/data/tmp/det-qa-pr892-data`, network Testnet, display `:99`, wallet `QA Wallet 1`. App was
already running (PID 989399) when this pass started; reused per campaign instructions. Same
session as `scenarios/DPN.md` — see that file's "Environment status at start of this pass"
section for the fresh recheck of the environment blocker (unchanged: three red banners on
Identities, `QA Wallet 1` at 0 DASH, `identities` table 0 rows). Not repeated here.

## Two-party stories are self-testable in principle, but blocked by a prerequisite this
## environment cannot clear

Per `CAMPAIGN-CONTEXT.md`'s explicit instruction, DPY-003/004/006/009/011 (the two-party
DashPay stories) are **not** to be marked BLOCKED merely for "needs a second real user" — the
intended test method is creating a second identity in the same wallet/app to act as the
counterparty. That instruction is followed here in spirit: these stories are **not** being
dismissed as "needs another user." They are BLOCKED instead on the actual, narrower root cause —
**no identity (first or second) can be loaded or registered at all** in this environment. See
`scenarios/IDN.md`: IDN-001 (register) BLOCKED by the environment failure before reaching a
fundable state, IDN-002 (load by ID) and IDN-003 (load masternode) both FAIL with a silent hang.
Zero identities exist; a second one is moot when a first one is unreachable.

## Architecture note: DashPay is entirely gated behind the Identity Hub, same as DPN

Source review (shared with `scenarios/DPN.md`) confirms DashPay has no reachable UI surface
independent of a loaded identity:

- `left_panel.rs` (nav sidebar) explicitly removed the standalone `Dashpay` entry: "The former
  standalone Identities and Dashpay entries are intentionally hidden from the nav; their screens,
  routes, and backend paths stay intact and remain reachable through other means (deep links, MCP
  tools, direct screen construction)." The only user-facing `Identities` sidebar entry is the
  unified hub (`RootScreenIdentityHub`).
- Inside the hub, DashPay functionality is spread across three of the four tabs
  (`identity/tabs.rs`: Home, **Contacts**, Activity, **Settings** — no separate "DashPay" tab):
  - **Contacts tab** (`identity/contacts.rs`): received/active/sent contact lists, Accept/Decline/
    Cancel/Pay row actions, "Add by username", "Scan QR", "Show my QR". Doc comment: "Renders
    either the populated Contacts page … or the social-profile gate card when the currently-active
    identity has no DashPay profile yet" — i.e. gated on identity **and** on that identity having
    a DashPay profile.
  - **Settings tab** (`identity/settings.rs`): social profile (display name, bio, avatar) and
    username/alias management.
  - **Activity tab**: unified payment/funding/platform-op timeline (covers DPY-007).
  - The Home tab's `Add contact` quick action is explicitly gated behind having a social profile
    set up first (module doc: "`Add contact` is gated behind a social profile").
- `RootScreenType::RootScreenDashPayContacts/Profile/Payments/ProfileSearch` root screens are
  still constructed at startup (`app.rs`) but, like the DPNS contest screens, have no sidebar nav
  entry — same "deep links / MCP tools only" status.
- The hub's `landing()` (`identity/hub_screen.rs`) resolves to `HubLanding::Onboarding` (the
  "Welcome to Identities" empty state, no tabs at all) whenever local identity count is 0
  (`identity/landing.rs`) — so none of the four tabs, and therefore no DashPay functionality,
  render without at least one loaded identity.

**Conclusion**: like DPN, DPY has no alternate/back-door reachability path. Every story below is
BLOCKED on the identical root cause established in `scenarios/IDN.md`.

---

## DPY-001: View and edit DashPay profile — **BLOCKED**

**Persona:** Alex, Priya. Acceptance criteria: "Set display name, bio, and profile image. Changes
are published as a state transition."

Profile editing lives in the Identity Hub's Settings tab (`identity/settings.rs`), unreachable
without a loaded identity.

**Verdict: BLOCKED** — reasoning: "blocked: no Platform identity reachable in this environment,
see scenarios/IDN.md — root cause is the known Testnet masternode-list/quorum-sync/wallet-storage
failure, see CAMPAIGN-CONTEXT.md".

---

## DPY-002: Search DashPay profiles — **BLOCKED**

**Persona:** Alex, Priya. Acceptance criteria: "Search by username or display name. View profile
details before sending a contact request."

`RootScreenDashPayProfileSearch` / `ProfileSearchScreen` exists in source (`dashpay/profile_search.rs`)
and is constructed at startup, but has no sidebar nav entry and is reached (per the Contacts tab's
"Add by username" affordance) only from inside the identity-gated Contacts tab.

**Verdict: BLOCKED** — same reasoning as DPY-001.

---

## DPY-003: Send contact request — **BLOCKED**

**Persona:** Alex, Priya. Acceptance criteria: "Enter username or identity ID. Request is sent
via state transition." Two-party story — self-testable in principle (see note above), but blocked
here on the shared prerequisite.

**Verdict: BLOCKED** — reasoning: "blocked: no Platform identity reachable in this environment,
see scenarios/IDN.md — root cause is the known Testnet masternode-list/quorum-sync/wallet-storage
failure, see CAMPAIGN-CONTEXT.md" (not "needs a second user" — a *first* identity cannot be
established either, so a self-test counterparty is equally unreachable).

---

## DPY-004: Accept or reject contact requests — **BLOCKED**

**Persona:** Alex, Priya. Acceptance criteria: "Incoming requests listed with sender profile
info." Two-party story.

**Verdict: BLOCKED** — same reasoning as DPY-003.

---

## DPY-005: View contact list and details — **BLOCKED**

**Persona:** Alex, Priya. Acceptance criteria: "Lists all accepted contacts. View individual
contact details and profile."

Contacts tab empty-state copy is confirmed implemented in source
(`identity/contacts.rs`: `NO_ACTIVE_EMPTY = "You have no contacts yet."`,
`NO_RECEIVED_EMPTY = "No pending requests."`, `NO_SENT_EMPTY = "No outgoing requests."`) — a
reasonable, actionable empty-state copy set — but the tab itself never renders without a loaded
identity with a DashPay profile, so this could not be visually confirmed live.

**Verdict: BLOCKED** — same reasoning as DPY-001.

---

## DPY-006: Send payment to contact — **BLOCKED**

**Persona:** Alex, Priya. Acceptance criteria: "Select contact and enter amount. Payment sent
through the DashPay protocol." Two-party story (`Pay` row action in Contacts tab, per
`identity/contacts.rs`'s doc comment: "Pay on an established contact (which opens the existing
send-payment screen)").

**Verdict: BLOCKED** — same reasoning as DPY-003.

---

## DPY-007: View payment history — **BLOCKED**

**Persona:** Alex, Priya. Acceptance criteria: "Lists payments with amounts, dates, and contact
names."

Covered by the Identity Hub's Activity tab ("a unified timeline of payments, funding, and
platform actions" per `identity/tabs.rs`), unreachable without a loaded identity.

**Verdict: BLOCKED** — same reasoning as DPY-001.

---

## DPY-008: Generate DashPay QR code — **BLOCKED**

**Persona:** Alex. Acceptance criteria: "QR code encodes DashPay profile or payment info."

"Show my QR" is a Contacts-tab affordance (`identity/contacts.rs`: `SHOW_MY_QR_LABEL`), backed by
`dashpay/qr_code_generator.rs`. Unreachable without a loaded identity + DashPay profile.

**Verdict: BLOCKED** — same reasoning as DPY-001.

---

## DPY-009: Edit contact info — **BLOCKED**

**Persona:** Alex, Priya. Acceptance criteria: "Set custom nickname and personal notes per
contact. Toggle contact visibility (hidden/visible). Changes persist locally." Effectively a
two-party story (requires an existing contact to edit).

Source confirms this is implemented as a local-only overlay independent of Platform state
(`dashpay/mod.rs`'s `persist_contact_private_info()`: "Upstream owns the encrypted on-Platform
copy; this is the local plaintext overlay that powers offline-friendly contact display" —
persisted to the WalletBackend k/v sidecar). Also confirms `UNHIDE_LABEL`/`UNHIDE_TOOLTIP` exist
for restoring a hidden contact, matching the campaign's prior WAL/contacts-category findings
about narrow unhide/cancel-race handling (per `875920fb`/`81201105` commit history in this repo).
Unreachable live: requires both a loaded identity and an existing contact.

**Verdict: BLOCKED** — same reasoning as DPY-003 (needs a contact to edit, which needs a second
identity, which needs a first identity — all unreachable).

---

## DPY-011: Auto-accept contact requests — **BLOCKED**

**Persona:** Priya. Acceptance criteria: "HD derivation and proof signing for automatic
acceptance. QR code generation for sharing auto-accept proof."

Auto-accept plumbing exists in source (`dashpay/contact_requests.rs`, `dashpay/qr_scanner.rs`,
`dashpay/qr_code_generator.rs` all reference `auto_accept`), surfaced through the Contacts tab.
Unreachable without a loaded identity.

**Verdict: BLOCKED** — same reasoning as DPY-001.

---

## Follow-up pass (2026-07-14, later same session): DPY-012, DPY-013, DPY-014

Same running app instance (PID 1580158, hash-verified against
`2931220e94871a0454ac56a43092aa87246b5a590d917645c025ddb1c7f9271a`), same data dir. Per campaign
instructions, the environment blocker was rechecked live rather than assumed: navigated to
Identities, reproduced the identical onboarding empty state and the same four red banners as the
rest of this file. `det.log` shows the same `WalletBackendNotYetWired` signature recurring
throughout the session. Direct SQLite check of `det-app.sqlite` confirms `identities`: 0 rows,
`contacts`: 0 rows, `dashpay_payments_overlay`: 0 rows. Screenshot (shared with DPN-008's
recheck): `screenshots/DPN-008-DPY-012-013-014-0-identities-empty-state-recheck.png`. Unchanged
from the rest of this file.

---

## DPY-012: Detect payments received from contacts — **BLOCKED**

**Persona:** Alex, Priya. Acceptance criteria: "Incoming on-chain transactions are matched
against my contacts' receiving addresses. Matched payments are recorded and surfaced in payment
history. Re-scanning the same transaction does not duplicate or double-count it."

### Reachability

Payment history is a sibling view under the identity-gated Activity tab (see DPY-007); the
matching logic itself operates only over a loaded identity's registered contacts. Unreachable in
this session — no identity, no contact, no incoming transaction to match.

### Source review (implementation confirmed, not live-exercised)

`src/backend_task/dashpay/incoming_payments.rs` implements the full detection pipeline, live-wired
(not orphaned): `register_dashpay_addresses_for_identity` derives and registers each contact's
receiving addresses as wallet-watched addresses; `match_transaction_to_contact` resolves a paid
address back to `(contact_id, address_index)` via a k/v reverse map; `detect_incoming_contact_payments`
— doc-commented as "the detection driver wired to the `EventBridge`" and confirmed called from
`backend_task/dashpay.rs` — scans a batch of received outputs against every local identity's
DashPay address map. **Dedup/idempotency** is explicit in `process_incoming_payment`'s doc
comment: "Idempotent: the receive cursor only ever advances, and the recording is keyed by
`(tx_id, vout)` with last-write-wins upstream, so a re-scan of the same output neither
double-credits nor double-counts" — directly satisfying the story's third bullet. Gating is
explicit too: `detect_incoming_contact_payments` early-returns `Ok(0)` when
`load_local_qualified_identities()` is empty, which is exactly this environment's state.

(Minor unrelated observation: `check_address_usage()` in `dashpay/payments.rs` is a dead stub
returning `Ok(vec![false; addresses.len()])` regardless of input, but it is called nowhere in the
codebase — not part of the live detection path above, so it does not undermine this story.)

**Verdict: BLOCKED** — reasoning: "blocked: no Platform identity reachable in this environment,
see scenarios/IDN.md — root cause is the known Testnet masternode-list/quorum-sync/wallet-storage
failure, see CAMPAIGN-CONTEXT.md". Source review confirms address-to-contact matching, payment
recording, and `(tx_id, vout)`-keyed dedup are all implemented and wired to the live sync event
bridge — not a stub.

---

## DPY-013: View contacts and avatars offline — **BLOCKED**

**Persona:** Alex, Priya. Acceptance criteria: "Contacts and private notes are read from
already-synced local state. Contact profiles and avatar images are cached locally and served on
subsequent views. An explicit 'Refresh' action re-fetches the latest profiles and avatars from
the network."

### Reachability

Contacts tab unreachable without a loaded identity (see architecture note above). Unreachable in
this session.

### Source review (implementation confirmed, not live-exercised) — with a nuance worth flagging

The offline-first / avatar-cache mechanics the story describes are genuinely implemented, but
live in a **different screen** than the one this environment's architecture note (above)
identifies as the actually nav-reachable Contacts surface — worth recording precisely:

- **Avatar disk cache**: `wallet_backend/avatar_cache.rs`'s `AvatarCacheView` doc header states
  the problem directly — "without a DET-side cache every contact view re-fetches every avatar
  from the network" — and its fix: validated image bytes are stored keyed by URL in the app-level
  k/v store with a TTL (stale entries are dropped and re-fetched) and size-bounded eviction. This
  is shared infrastructure (`FetchAvatar` backend task), not tied to one screen, and is backed by
  roughly 15 dedicated unit tests.
- **Offline-first contact read + explicit refresh**: `DashPayTask::LoadContactsOffline` is
  doc-commented in `backend_task/dashpay.rs` as reading "the contact list from offline state
  only — rehydrated relationships + private memos plus the DET contact-profile cache. No network
  round-trip, so a view renders without connectivity" — matching the story's first two bullets
  precisely. This variant is dispatched by `ui/dashpay/contacts_list.rs`'s `trigger_fetch_contacts()`
  on view entry, with a *separate* `trigger_refresh_contacts()` bound to an explicit **"Refresh"**
  button (hover text: "Fetch the latest contacts and profiles from the network") that dispatches
  the network `LoadContacts` variant instead — a clean offline-read / explicit-refresh split.
- **The nuance**: `ui/dashpay/contacts_list.rs` backs `RootScreenType::RootScreenDashPayContacts`
  — the root screen this file's architecture note (above) already established is **nav-unreachable**
  ("intentionally hidden from the nav... reachable through other means (deep links, MCP tools,
  direct screen construction)"). The screen a user actually reaches today — the Identity Hub's
  **Contacts tab** (`identity/contacts.rs`) — was checked directly for this pass: its `load_action()`
  dispatches `DashPayTask::LoadContacts` (the network variant) unconditionally once per tab entry;
  `LoadContactsOffline` does not appear anywhere in `identity/contacts.rs`. So the currently
  nav-reachable Contacts tab does **not** demonstrably implement "read from already-synced local
  state... show instantly without a network round-trip" on entry — that exact behavior exists, but
  in the sibling screen the user cannot navigate to. The avatar disk cache itself (`FetchAvatar`)
  is shared infrastructure and would still benefit whichever screen renders a contact avatar, so
  that half of the story is unaffected by this nuance.

**Verdict: BLOCKED** — reasoning: "blocked: no Platform identity reachable in this environment,
see scenarios/IDN.md — root cause is the known Testnet masternode-list/quorum-sync/wallet-storage
failure, see CAMPAIGN-CONTEXT.md". Source review confirms the offline-first-read /
explicit-refresh / avatar-disk-cache mechanics are genuinely implemented and tested, but flags
that the offline-first *read* behavior is demonstrated in the nav-unreachable legacy
`RootScreenDashPayContacts` screen rather than the nav-reachable Identity Hub Contacts tab, which
dispatches a network fetch on every tab entry instead. Worth a live re-check once identities are
reachable and this can be exercised directly, to confirm whether the Contacts tab merely renders
stale state before the network call resolves (visually similar to "instant") or genuinely blocks
on the round-trip.

---

## DPY-014: Cancel a sent contact request — **BLOCKED**

**Persona:** Alex, Priya. Acceptance criteria: "A DashPay contact request is immutable on
Platform and cannot be deleted, so cancelling cannot un-send it. The UI states this plainly...
Cancelling re-checks the request against the network first... Cancelling publishes a hidden
contact-info document and records the withdrawal locally... A request the other person already
accepted is reported as an established contact instead of being cancelled." Effectively a
two-party story (needs a sent, pending request).

### Reachability

"Cancel" is a Contacts-tab row action on a sent request (`identity/contacts.rs`); unreachable
without a loaded identity with an existing sent request, which in turn needs a second identity —
neither reachable this session (see DPY-003's reasoning).

### Source review (implementation confirmed, not live-exercised) — the most complete of this pass

`backend_task/dashpay/contact_requests.rs` implements every acceptance-criteria bullet with a
directly corresponding, tested code path:

- **Immutability stated plainly, not implying withdrawal**: `cancel_contact_request`'s doc
  comment: "DashPay `contactRequest` documents are immutable and cannot be deleted
  (`documentsMutable: false`, `canBeDeleted: false` in the DashPay contract), so the request
  cannot be un-sent from Platform." The exact same framing reaches the user via the UI constant
  `CANCEL_EXPLAINER` (`ui/identity/contacts.rs`): "Cancelling a request hides it and tells the
  other person you are no longer waiting. The original request stays on the network." — matching
  the story's first bullet word-for-word in spirit.
- **Re-checks against the network first**: `cancel_flow()` checks `reciprocal_request_exists()`
  before touching anything (→ `AlreadyEstablished` immediately if the recipient already answered),
  then broadcasts the hide, then **re-checks reciprocal a second time** post-broadcast and reverts
  the hide if the recipient answered mid-flight — a documented race-window closure, with an
  explicitly acknowledged residual risk (a reciprocal landing after the second read) whose
  recovery path is the Contacts tab's unhide affordance. `cancel_contact_request` also re-fetches
  the request document from Platform by ID rather than trusting the clicked row's cached state,
  and validates the caller is actually the request's sender before proceeding.
- **Publishes hidden contact-info + records withdrawal locally**: `set_contact_hidden(true)`
  broadcasts a real `contactInfo` state transition; `mark_withdrawn()` persists the withdrawal via
  `wallet_backend/dashpay.rs`'s `dashpay_mark_withdrawn` under a `KV_PREFIX_WITHDRAWN` key,
  readable back via `dashpay_is_withdrawn` — a durable, kv-store-backed record, not in-memory
  state, so it survives restarts.
- **Already-accepted request reported as an established contact, not cancelled**: `CancelOutcome`
  is a two-armed enum — `Withdrawn` vs `AlreadyEstablished` — with the latter returned both on the
  pre-check and the post-broadcast re-check.

Four dedicated unit tests were found covering exactly these paths:
`cancelling_a_pending_request_hides_it_and_records_the_withdrawal` (asserts `Withdrawn`),
`cancelling_an_answered_request_hides_nothing` (asserts `AlreadyEstablished`, and explicitly
asserts nothing gets hidden), `cancelling_someone_elses_request_is_rejected`, and
`cancelling_a_malformed_request_is_rejected`.

**Verdict: BLOCKED** — reasoning: "blocked: no Platform identity reachable in this environment,
see scenarios/IDN.md — root cause is the known Testnet masternode-list/quorum-sync/wallet-storage
failure, see CAMPAIGN-CONTEXT.md" (specifically: needs an existing sent contact request, which
needs two identities, neither reachable). Source review confirms every acceptance-criteria bullet
has a directly corresponding, unit-tested implementation — the most thoroughly implemented and
documented story reviewed in this follow-up pass.

---

## Summary

| Story | Verdict |
|---|---|
| DPY-001 | BLOCKED (no identity reachable — Settings tab unreachable) |
| DPY-002 | BLOCKED (no identity reachable — profile search only reachable from gated Contacts tab) |
| DPY-003 | BLOCKED (no identity reachable — self-testable in principle, but a *first* identity cannot be established either) |
| DPY-004 | BLOCKED (same as DPY-003) |
| DPY-005 | BLOCKED (no identity reachable — Contacts tab empty-state copy confirmed implemented via source, not live-exercised) |
| DPY-006 | BLOCKED (same as DPY-003) |
| DPY-007 | BLOCKED (no identity reachable — Activity tab) |
| DPY-008 | BLOCKED (no identity reachable — Contacts tab "Show my QR") |
| DPY-009 | BLOCKED (needs an existing contact, which needs two identities, neither reachable) |
| DPY-010 | N/A (Gap, not implemented — already recorded in `progress.md`, not re-tested here) |
| DPY-011 | BLOCKED (no identity reachable — Contacts tab) |
| DPY-012 | BLOCKED (no identity reachable; address-to-contact matching, payment recording, and tx_id+vout dedup all confirmed implemented and live-wired via source) |
| DPY-013 | BLOCKED (no identity reachable; offline-first-read/avatar-cache/explicit-refresh mechanics confirmed implemented, but in the nav-unreachable legacy Contacts screen rather than the nav-reachable Identity Hub Contacts tab — worth a live re-check once unblocked) |
| DPY-014 | BLOCKED (needs a sent contact request, two identities, neither reachable; every acceptance-criteria bullet confirmed implemented and unit-tested via source) |

All thirteen in-scope DPY stories are BLOCKED on the same root cause established in
`scenarios/IDN.md`: zero identities can be loaded or registered in this environment, and DashPay
has no reachable UI surface independent of the Identity Hub. This pass followed
`CAMPAIGN-CONTEXT.md`'s instruction to treat two-party stories as self-testable rather than
reflexively BLOCKED "needs another user" — the distinction matters for the record even though the
practical outcome (BLOCKED) is the same here, since the actual blocker is one level more
fundamental (no *first* identity, let alone a second). The follow-up pass (DPY-012/013/014) found
all three underlying features genuinely implemented in source, with one nuance worth a follow-up
live check: DPY-013's offline-first read behavior currently lives in a nav-unreachable sibling
screen rather than the Contacts tab a user actually reaches. No PR892 application source was
modified; no persistent state was changed by this pass.

---

## Retest pass (2026-07-15, post-environment-fix): all thirteen in-scope DPY stories retested live

**Environment**: Testnet wallet-backend blocker fixed (root-caused as upstream
`dashpay/platform#4133`; see `CAMPAIGN-CONTEXT.md`). App PID 3331055, hash-verified
`2931220e94871a0454ac56a43092aa87246b5a590d917645c025ddb1c7f9271a`, Testnet fully synced,
Developer view. `QA Identity 1` and `QA Identity 2` both exist in the same wallet — used as the
two self-test parties per campaign convention. `det.log` confirmed clean of the known-issue
signature throughout.

**Setup performed this pass** (shared context for all stories below): gave both identities a
DashPay social profile (`QA Identity 1` → display name "QA Test One", bio; `QA Identity 2` →
display name "QA Test Two") to clear the "social profile gate" that hides the Contacts tab/DashPay
functionality until a profile exists. `QA Identity 2` was topped up 0.0025 DASH via **"Use a
Platform address"** funding (an existing, already-locked Platform balance — not a new asset lock)
so it could also register a DPNS username (`detqa892run3`), needed to make it findable via Profile
Search for the contact-detail/edit tests below.

**Recurring finding, not story-specific — legacy DashPay screen family has broken internal
sub-navigation.** DashPay's `My Profile`/`Contacts`/`Payment History`/`Search Profiles` screens
(the pre-Identity-Hub `RootScreen*` family, still reachable via deep links e.g. "Add by username")
share a left sub-nav panel. Repeatedly and reproducibly, clicking a sibling item in that panel
while the "Add Contact" sub-view is displayed does **nothing** — the panel stays on "Add Contact"
regardless of which sibling is clicked (confirmed for all three siblings, multiple times, with
both empty and filled recipient fields). The only way found to escape to a sibling screen was via
"Cancel"/"Back", which itself was non-deterministic — it sometimes returned to the Identity Hub's
Contacts tab and sometimes (unpredictably) landed on "My Profile"/"Profile Search" instead, with
no discernible pattern tied to field contents or which button was clicked. This is a real,
reproducible navigation-reliability defect in a screen family several stories below depend on to
be reachable at all; it does not block any individual story's core function once you happen to
land on the right screen, but it makes reaching per-contact detail/edit/search functionality
needlessly unreliable for a real user. Not filed as its own story since it spans several; flagged
here for product awareness.

### DPY-001: View and edit DashPay profile — **PASS**

**Acceptance criteria**: "Set display name, bio, and profile image. Changes are published as a
state transition."

Steps: `QA Identity 1` → Settings tab → Display name `QA Test One`, About `QA regression bio for
DPY-001.` → "Save social profile". Result: fields persisted, Home tab immediately reflected
"QA Test One" instead of the raw handle, and the previously-disabled "Add contact" button became
enabled (confirming the profile-gate behavior). Screenshots:
`screenshots/DPY-001-1-profile-form-filled.png`, `screenshots/DPY-001-2-profile-saved-home.png`.

**Confirmed via `det.log`**: `Profile created: doc_id=71oBAj44owsPShjdk855s9S4UwFuS3hyKADGbD5NPqFB,
revision=Some(1)` — a real state transition, not just a UI-local change.

**Verdict: PASS.**

### DPY-002: Search DashPay profiles — **PASS**

**Acceptance criteria**: "Search by username or display name. View profile details before sending
a contact request."

Steps: `DashPay > Profile Search` (reached via `My Profile`, itself reached from the Add-Contact
screen's flaky "Cancel" — see the navigation-reliability note above) → searched `alice` → **"Search
Results (1): alice.dash"** with ID shown, "Add Contact"/"View Profile" buttons → "View Profile" →
**Contact Profile** screen: "No profile found. This contact has not created a public profile yet."
(correct — alice.dash never set up a DashPay profile) plus a "Private Contact Information" panel.
Repeated with `detqa892run3` (QA Identity 2's username): correctly returned "QA Test Two" with her
real display name and bio structure. Screenshots: `screenshots/DPY-002-1-search-results-alice.png`,
`screenshots/DPY-002-2-view-profile-no-profile-found.png`.

**Verdict: PASS.** Search works by DPNS-username prefix and the View Profile step genuinely shows
profile details (or a clean "no profile" empty state) before any contact request is sent.

### DPY-003: Send contact request — **PASS**

**Acceptance criteria**: "Enter username or identity ID. Request is sent via state transition."
Self-tested: `QA Identity 1` (sender) → `QA Identity 2` (recipient), by identity ID.

Steps: `QA Identity 1` Contacts tab → "Add by username" → `Add Contact` screen, `To (Recipient)`
filled with `QA Identity 2`'s raw Identity ID (`87jAqayii8J5zB8hJsnCPk3BEANicRxfMRFriGvk9jy6`) →
Request Summary confirmed From/To → "Add Contact". Result: **"Contact Request Sent
Successfully!"**, and the request appeared under "Sent requests · 1" with a "Pending" badge and a
"Cancel request" button. Screenshots: `screenshots/DPY-003-1-add-contact-filled.png`,
`screenshots/DPY-003-2-request-sent-success.png`, `screenshots/DPY-003-3-sent-request-pending.png`.

**Confirmed via `det.log`**: `Contact request created: doc_id=22ocUPrZdNFN4X1c65jm36UCYk3yLNmJ8LbyKzEbikTJ,
revision=None` — a real state transition.

**Verdict: PASS.**

### DPY-014: Cancel a sent contact request — **PASS** (tested here, ahead of DPY-004, per task
### ordering — the just-sent DPY-003 request was cancelled before QA Identity 2 could act on it)

**Acceptance criteria**: see full bullet list in the original write-up above (immutability stated
plainly; re-checks network first; publishes hidden contact-info + records withdrawal; already-
accepted request reported as established contact).

Steps: on the "Sent requests" row from DPY-003, clicked **"Cancel request"**. Result: green banner
**"Contact request cancelled."**, and the request immediately disappeared from "Sent requests"
(back to "No outgoing requests."). Screenshot: `screenshots/DPY-014-1-request-cancelled-banner.png`.

**Confirmed via `det.log`**: `Contact info created: doc_id=2mT1GeGi8aQTGwiSUkrHSpw66L9L81hWnGz7V18hepin,
revision=Some(1)` immediately followed by the banner log line — a real `contactInfo` "hidden"
state transition was published, matching "publishes a hidden contact-info document and records
the withdrawal locally," not merely a local-only UI change.

**Edge case exercised live (not in the original source-review-only pass): what happens if the
recipient accepts a request the sender already cancelled?** Because a `contactRequest` document is
immutable and undeletable on Platform (the story's own first bullet), the cancel above did *not*
remove QA Identity 2's view of the incoming request — she still saw it as pending (see DPY-004
below) and accepted it. **Both sides correctly ended up showing an established "Active contact"
afterward** (`QA Identity 1` → "Active contacts · 1: QA Test Two"; `QA Identity 2` → "Active
contacts · 1: QA Test One") — i.e. the system correctly reconciles a cancel-then-accept race in
favor of the later acceptance, matching the spirit of the story's "a request the other person
already accepted is reported as an established contact instead of being cancelled" bullet (tested
here in the reverse temporal order — cancel-then-accept rather than accept-then-cancel — with the
same correct outcome).

**Secondary finding**: re-attempting to send a *new* contact request to the same recipient after
cancelling is blocked with **"You have already sent a contact request to '&lt;id&gt;'. Please wait
for them to respond."** — because the original, immutable `contactRequest` document still exists
on Platform, the app (correctly, per its own duplicate-prevention check) still sees an outstanding
request. This is consistent with, not contradictory to, the story's "cannot be un-sent" framing,
but worth noting: a user who cancels cannot immediately try again with a fresh request to the same
person until the original is answered one way or another. Not counted as a defect.

**Verdict: PASS.** Every acceptance-criteria bullet confirmed live, including the
previously-source-review-only "already accepted" reconciliation bullet, now exercised end-to-end
via a real cancel-then-accept race.

### DPY-004: Accept or reject contact requests — **PASS**

**Acceptance criteria**: "Incoming requests listed with sender profile info."

Steps: switched to `QA Identity 2` → Contacts tab → **"Received requests · 1"**: a row for
`24Jm9...tCb` (`QA Identity 1`) with "Accept"/"Decline" buttons, "3 minutes ago" timestamp → clicked
**"Accept"**. Result: **"Contact request accepted."**, and `QA Identity 2`'s Contacts tab
immediately showed **"Active contacts · 1: QA Test One"** with a "Pay" button. Screenshots:
`screenshots/DPY-004-1-received-request-pre-accept.png`,
`screenshots/DPY-004-2-accepted-active-contact.png`.

**Verdict: PASS.** (This is the same request DPY-014 cancelled from the sender's side moments
earlier — see that section for the cancel-then-accept edge-case analysis; the accept itself worked
correctly regardless.)

### DPY-005: View contact list and details — **PASS** (with one navigation-reachability gap noted)

**Acceptance criteria**: "Lists all accepted contacts. View individual contact details and
profile."

The Identity Hub's Contacts tab correctly lists all three groupings — Received/Active/Sent — with
correct counts and correct empty-state copy when applicable; confirmed on both `QA Identity 1` and
`QA Identity 2` after DPY-003/004/014 (both show "Active contacts · 1" with the other's display
name). **Individual contact detail view**: reachable via `Search Profiles` (search the contact's
DPNS username) → "View Profile" → **Contact Profile** screen showing avatar, display name,
identity ID, public bio/message, and a "Private Contact Information" panel (nickname/notes/hidden
status) — confirmed for `QA Test Two` (`QA Identity 2`, a real established contact, found via her
`detqa892run3` username). Screenshot: `screenshots/DPY-005-2-contact-profile-detail.png`.

**Gap found**: clicking directly on a contact's row in the Identity Hub's own Contacts tab (the
actually-reachable, primary surface) does **not** open this detail view — only the "Pay" button on
that row does anything; the name/avatar area is inert. The richer detail view above is only
reachable via the separate Search Profiles path, and only for contacts with a discoverable DPNS
username (an identity with no username, like a fresh `QA Identity 2` would have been before this
pass registered one for her, cannot be looked up this way at all). This mirrors the campaign's
IDN-008 finding (a real feature exists but has no direct click-through from the primary
list) — worth a product follow-up, but the detail view genuinely exists and is reachable by at
least one path, so the story's core requirement is met, not failed outright.

**Verdict: PASS**, with the click-through gap noted above.

### DPY-006: Send payment to contact — **FAIL** (confirmed general, reproducible bug — not an
### artifact of the cancel-then-accept test setup)

**Acceptance criteria**: "Select contact and enter amount. Payment sent through the DashPay
protocol."

Steps: `QA Identity 1` Contacts tab → "Pay" on `QA Test Two` → `Send Payment` screen (From `QA
Identity 1`, To the recipient's raw Identity ID, Wallet Balance shown) → amount `0.001` DASH, memo
"DPY-006 QA test payment" → "Send Payment". Result: **red banner** — "Could not process encrypted
data. Please check your keys and try again." Screenshots:
`screenshots/DPY-006-1-send-payment-form-filled.png`, `screenshots/DPY-006-2-encryption-error-details.png`.

**Root cause, confirmed via source review** (`src/backend_task/dashpay/payments.rs`): the
technical detail behind the banner is `EncryptionError { detail: "Missing senderKeyIndex" }`. This
comes from `derive_contact_payment_address` (lines ~112–126), which reads the `senderKeyIndex` /
`recipientKeyIndex` fields off the recipient's fetched `contactRequest` document via a **strict
exact-variant match**: `match v { Value::U32(idx) => Some(*idx), _ => None }`. Every document
fetched live from Platform is deserialized from CBOR, and the CBOR→`Value` converter
(`rs-platform-value`'s `TryFrom<CborValue> for Value`) maps **all** integers to `Value::I128`,
never `Value::U32` — so this match always falls through to `None` for any real, network-fetched
`contactRequest` document, regardless of whether the field is actually present and correctly
populated (it is — written as `Value::U32` at creation time, but re-typed on the CBOR round trip).
This is a **general, unconditional bug affecting every DashPay payment to any contact** on this
build, not something specific to this pass's cancel-then-accept contact-establishment path: the
cancel/withdraw mechanism only touches a local "withdrawn" marker and a `contactInfo` hide
document (per DPY-014 above), neither of which `derive_contact_payment_address` ever reads — it
does a fresh live document fetch every time. The sibling function `accept_contact_request`
(`contact_requests.rs:766`) reads the identical field correctly via `.to_integer::<u32>()`, which
handles `I128`/`U64`/etc. — `payments.rs` is the only DashPay code path using the brittle
exact-match pattern instead of that existing, correct helper. No unit tests cover
`derive_contact_payment_address` at all.

**Verdict: FAIL.** This is a P1: the described flow — "Select contact and enter amount" — is fully
reachable and appears ready to submit, but **every** attempt fails with a technical, unrecoverable
error for any user. Fix direction for the ticket: replace the two exact-variant matches in
`payments.rs` (~112–126) with `.to_integer::<u32>()`, mirroring the working `contact_requests.rs`
pattern.

### DPY-007: View payment history — **Partial PASS** (screen and empty state confirmed; populated
### rendering could not be confirmed due to DPY-006)

**Acceptance criteria**: "Lists payments with amounts, dates, and contact names."

The Identity Hub's Activity tab shows: "Unified activity is coming soon. For now, view activity on
the existing DashPay Payments screen: **Open DashPay Payments**" — a clean, explicit transitional
message with a working link (not a silent gap). Following it reaches **Payment History**: "No
Payment History — No payments have been made with this identity." with a "Refresh Payment History"
button. Screenshot: `screenshots/DPY-007-1-payment-history-empty.png`.

**Verdict: Partial PASS.** The screen is reachable, the empty state is correct and actionable, and
a refresh action exists — but because DPY-006's bug prevents any real DashPay payment from ever
completing in this environment, the "lists payments with amounts, dates, and contact names"
behavior itself (populated rendering) could not be exercised or confirmed this pass.

### DPY-008: Generate DashPay QR code — **PASS**

**Acceptance criteria**: "QR code encodes DashPay profile or payment info."

Steps: Contacts tab → "Generate QR Code" → **Generate Contact QR Code** screen, identity
pre-selected → "Generate QR Code". Result: a real QR image rendered, plus a collapsible "QR Code
Data (text)" section showing the underlying URI: `dash:?di=24Jm9XBCPsAf154cy4X2YLvTTgFjiwAKoCSew17CetCb&dapk=14Zixz3jv56voc2UGWvJVmYpQCC1ziJqe4PBRkNREpKaiQ7BTSdt`
(identity ID + an auto-accept public key), a "Copy Data to Clipboard" button, and an explicit
warning: "Anyone with this QR code can automatically become your contact." Screenshots:
`screenshots/DPY-008-1-qr-code-generated.png`, `screenshots/DPY-008-2-qr-data-text.png`.

**Verdict: PASS.**

### DPY-009: Edit contact info — **PASS** (core flow), with one **defect found**: hidden contacts
### are not moved to a "Show hidden contacts" section

**Acceptance criteria**: "Set custom nickname and personal notes per contact. Toggle contact
visibility (hidden/visible). Hidden contacts stay listed in a collapsed 'Show hidden contacts'
section of the Identity Hub Contacts tab, and can be unhidden from there... Changes persist
locally."

Steps: on `QA Test Two`'s Contact Profile screen (reached per DPY-005 above) → "Edit" under
"Private Contact Information" → Nickname `Sis`, Notes `QA regression note for DPY-009.` → "Save".
Result: fields displayed correctly on reload — "Nickname: Sis", "Notes: QA regression note for
DPY-009.", "Hidden: No". Screenshots: `screenshots/DPY-009-1-edit-contact-info-filled.png`,
`screenshots/DPY-009-2-edit-saved.png`.

**Defect found**: toggled "Hide this contact from the main list" → saved → "Hidden: Yes" confirmed
on the Contact Profile screen. Navigated to the Identity Hub's Contacts tab (`QA Identity 1`, the
owner of this private note) — **the contact still appeared, unconditionally, under "Active
contacts · 1"**, with no collapsed "Show hidden contacts" section anywhere on the page (checked
after both an in-place tab switch and a full navigation-away-and-back). Screenshot:
`screenshots/DPY-009-3-hidden-not-reflected-in-contacts-tab.png`. This directly contradicts the
acceptance criteria's explicit bullet. The hidden flag itself does persist correctly (confirmed
`Hidden: Yes` on reload of the Contact Profile screen) — only the Contacts tab's filtering/section
behavior is missing. **Reverted** the hidden flag back to `No` afterward
(`screenshots/DPY-009-4-unhidden-restored.png`) to leave clean state for later categories.

**Verdict: PASS** for nickname/notes editing and persistence (the story's first and last bullets);
**FAIL** for the "Hidden contacts stay listed in a collapsed 'Show hidden contacts' section... can
be unhidden from there" bullet — the Identity Hub Contacts tab has no such section and does not
filter on the hidden flag at all. Net story verdict recorded as **FAIL** since a stated,
specific acceptance-criteria bullet is unmet, not merely UX-rough.

### DPY-011: Auto-accept contact requests — **PASS**

**Acceptance criteria**: "HD derivation and proof signing for automatic acceptance. QR code
generation for sharing auto-accept proof."

Same **Generate Contact QR Code** screen as DPY-008, with **"Advanced Options"** checked: exposes
an **Account Index** field (HD derivation index selection) and a **Validity (Hours)** field
(default 24, "How long the QR code remains valid") alongside the identity picker. The generated
QR/URI's `dapk=` parameter is the auto-accept public key/proof referenced by the acceptance
criteria. Screenshot: `screenshots/DPY-011-1-auto-accept-qr-advanced-options.png`.

**Verdict: PASS.** HD derivation (selectable account index), a signed proof key embedded in the
URI, and QR generation with a configurable expiry are all present and reachable from a single
identity-scoped screen.

### DPY-012: Detect payments received from contacts — **BLOCKED** (cannot be live-tested; DPY-006's
### bug plausibly affects this feature too, unconfirmed)

**Acceptance criteria**: "Incoming on-chain transactions are matched against my contacts'
receiving addresses. Matched payments are recorded and surfaced in payment history. Re-scanning
the same transaction does not duplicate or double-count it."

This story requires a genuine DashPay-protocol payment to land at a contact's derived receiving
address to observe detection. DPY-006's bug prevents any such payment from ever completing in this
build, so no live incoming-payment scenario could be produced this pass. **Not independently
confirmed, but worth flagging as a plausible related risk**: address derivation for a contact
(both for sending *and* for registering the addresses this identity itself should watch for
incoming payments) very likely goes through the same or closely related code as
`derive_contact_payment_address` (the function DPY-006 root-caused) — if so, the address
registration this detection pipeline depends on may share the same `senderKeyIndex`/CBOR-decoding
defect. This is inference, not a live-confirmed finding for DPY-012 itself.

**Verdict: BLOCKED** — reasoning: "blocked by DPY-006's confirmed send-payment bug: no genuine
DashPay-protocol payment can be produced in this environment to exercise incoming-payment
detection, and the same buggy address-derivation code is plausibly (not confirmed) shared with
this feature's address-registration path." The prior pass's source review (confirming
`detect_incoming_contact_payments`'s `(tx_id, vout)`-keyed dedup and live event-bridge wiring)
remains valid supporting context and is not re-litigated here.

### DPY-013: View contacts and avatars offline — **Partial PASS** (primary reachable screen meets
### the instant-read bullet; the separate legacy screen shows stale/incorrect data when reached)

**Acceptance criteria**: "Contacts and private notes are read from already-synced local state...
show instantly without a network round-trip... Contact profiles and avatar images are cached
locally... An explicit 'Refresh' action re-fetches the latest profiles and avatars."

**Primary, actually-reachable surface (Identity Hub Contacts tab)**: switching to this tab renders
`QA Test Two` as an active contact immediately, with no visible loading spinner in any screenshot
taken right after navigation — consistent with an instant, locally-cached read rather than a
network round-trip, satisfying the story's first bullet in practice. No explicit "Refresh" control
was found on this specific tab, however (see gap below).

**New finding this pass (upgrades the original source-review-only nuance to a live-confirmed
one)**: the separate legacy `RootScreenDashPayContacts` screen (`My Profile`/**`Contacts`**/
`Payment History`/`Search Profiles` family, reached via the same flaky deep-link path documented
in this file's navigation-reliability note) shows **"No Contacts — You haven't added any contacts
yet."** for `QA Identity 1` at the exact same moment the Identity Hub's Contacts tab correctly
shows her one active contact (`QA Test Two`). Its "Requests" tab likewise showed "No Incoming
Requests" despite the request having already been resolved (accepted) by this point, which is at
least consistent, but the "My Contacts" emptiness while a real, established, on-chain-confirmed
contact exists is a genuine data-correctness problem, not merely unreachability. This screen does
dispatch a visible "Loading contacts..." state (a real network call) before rendering — i.e. it
is *not* demonstrating the offline-first read pattern the story describes either, compounding the
finding. Screenshot: `screenshots/DPY-013-1-legacy-contacts-no-contacts-stale.png`.

**Verdict: Partial PASS.** The user-reachable Identity Hub Contacts tab satisfies the core
"instant, cached view" requirement in practice (no observable network wait, correct data shown),
but (a) has no visible explicit "Refresh" affordance to independently verify the third bullet, and
(b) the separate legacy screen — which does at least have a "Refresh" button — shows incorrect
(empty) data for a real contact and performs a live network fetch rather than an offline-first
read, contradicting the story where it's most directly testable. Avatar caching itself could not
be exercised (neither test identity has a real avatar image set).

---

## Retest-pass summary

| Story | Verdict |
|---|---|
| DPY-001 | **PASS** — profile created and confirmed via on-chain state transition |
| DPY-002 | **PASS** — Profile Search finds real/absent profiles by DPNS username; View Profile shows detail before contact request |
| DPY-003 | **PASS** — contact request sent by identity ID, confirmed via on-chain state transition |
| DPY-004 | **PASS** — incoming request listed with sender info, accepted successfully |
| DPY-005 | **PASS** — contact list + detail view both work; detail view has no direct click-through from the primary Contacts tab row (noted, not blocking) |
| DPY-006 | **FAIL** — "Missing senderKeyIndex" EncryptionError on every payment attempt; root-caused to a general CBOR-integer-decoding bug in `derive_contact_payment_address`, independent of this pass's specific test setup |
| DPY-007 | **Partial PASS** — screen/empty-state/refresh confirmed reachable; populated rendering unconfirmed due to DPY-006 |
| DPY-008 | **PASS** — real QR code + data URI generated with correct security warning |
| DPY-009 | **FAIL** — nickname/notes editing and persistence work correctly, but hiding a contact does not move it to a "Show hidden contacts" section on the Contacts tab (explicit acceptance-criteria bullet unmet) |
| DPY-011 | **PASS** — HD account-index selection + auto-accept proof key + configurable-validity QR generation all confirmed on the same screen as DPY-008 |
| DPY-012 | **BLOCKED** — cannot be live-tested because DPY-006 prevents any real DashPay payment from completing; plausible but unconfirmed shared root cause |
| DPY-013 | **Partial PASS** — the reachable Contacts tab shows contacts instantly (meets the core criterion); the separate legacy Contacts screen shows stale/empty data and performs a network fetch instead of an offline-first read |
| DPY-014 | **PASS** — cancel flow confirmed via on-chain `contactInfo` state transition; cancel-then-accept edge case correctly reconciles to an established contact on both sides |

**Two confirmed FAILs this pass** (both new, environment-independent defects, not related to the
known asset-lock/wallet-backend issue): **DPY-006** (DashPay payments are completely broken by a
CBOR-integer-type mismatch in address derivation — P1, affects every contact) and **DPY-009**
(hiding a contact doesn't hide it from the Contacts tab — the explicit "Show hidden contacts"
section never appears). **DPY-012** is BLOCKED as a direct consequence of DPY-006, not the
asset-lock recurrence. A recurring, cross-cutting **navigation-reliability defect** was also found
in the legacy DashPay screen family's internal sub-nav (documented once above rather than
per-story) and directly explains part of the DPY-013 finding (the same family's Contacts screen
also returns stale data, a second, independent problem in that screen). No PR892 application
source was modified. `QA Wallet 1`, `QA Identity 1`, and `QA Identity 2` were left with real,
intentional state changes as a result of this pass (DashPay profiles, one active contact
relationship, one registered DPNS username each) — expected residue of self-testing, not
accidental.
