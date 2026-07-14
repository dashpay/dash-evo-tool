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

All ten in-scope DPY stories are BLOCKED on the same root cause established in `scenarios/IDN.md`:
zero identities can be loaded or registered in this environment, and DashPay has no reachable UI
surface independent of the Identity Hub. This pass followed `CAMPAIGN-CONTEXT.md`'s instruction
to treat two-party stories as self-testable rather than reflexively BLOCKED "needs another user"
— the distinction matters for the record even though the practical outcome (BLOCKED) is the same
here, since the actual blocker is one level more fundamental (no *first* identity, let alone a
second). No PR892 application source was modified; no persistent state was changed by this pass.
