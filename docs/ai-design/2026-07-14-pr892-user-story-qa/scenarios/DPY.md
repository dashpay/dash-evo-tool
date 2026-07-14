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
