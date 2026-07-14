# DPN — DPNS Usernames

Environment: PR892 build (`/data/target/debug/dash-evo-tool` @ `57195d54`), isolated data dir
`/data/tmp/det-qa-pr892-data`, network Testnet, display `:99`, wallet `QA Wallet 1`. App was
already running (PID 989399) when this pass started; reused per campaign instructions.

## Environment status at start of this pass — unchanged from `scenarios/IDN.md`

Per this campaign's instructions, one honest recheck was done before assuming the blocker still
applies (rather than blindly re-asserting IDN.md's conclusion). Result: **unchanged**. A fresh
navigation to Identities > empty state reproduced the identical three red banners IDN.md
documented — "We couldn't finish preparing your wallet. Try restarting the app.", "Your wallet
is still starting up. Please wait a moment and try again.", "Could not load your identities from
this device." — and the Wallets screen still shows `QA Wallet 1` at **0 DASH** with "Sync Status:
Core: Error, Addresses: never synced" (worse than `CAMPAIGN-CONTEXT.md`'s baseline description,
matching IDN.md's "worse than DEV.md's snapshot" finding — the wallet-storage layer, not just
Platform proof verification, is unwired this session). `identities` table: 0 rows (unchanged).
Screenshot: `screenshots/DPN-000-identities-empty-state-blocked-banners.png`.

**Root cause**: known Testnet masternode-list/quorum-sync/wallet-storage failure, see
`CAMPAIGN-CONTEXT.md` and `scenarios/ALK.md`. **Consequence for DPN**: IDN-001 (register),
IDN-002 (load by ID), and IDN-003 (load masternode/evonode) all failed to produce any loaded
identity in this environment (see `scenarios/IDN.md`) — two via a silent-hang defect, one via the
environment blocker directly. Zero identities of any kind (user or masternode/evonode) exist to
drive DPNS functionality from.

## Architecture note (source review, not a defect): DPNS screens are identity-gated, not
## independently reachable

Before concluding every DPN story is BLOCKED, the source was reviewed to confirm there is no
alternate DPNS entry point that sidesteps the identity requirement:

- The DPNS username **registration** flow (`register_dpns_name_screen.rs`) is invoked only from
  an existing identity's Home/Settings tab inside the Identity Hub (`identity/hub_screen.rs`,
  `identity/home.rs`) — the hub's `landing()` derives `HubLanding::Onboarding` (the "Welcome to
  Identities" empty state, no tabs) whenever the local identity count is 0
  (`ui/identity/landing.rs`), so the tabs that host DPNS registration/username-management never
  render without an identity.
- The contest/voting screens (`dpns_contested_names_screen.rs`, `DPNSSubscreen::{Active, Past,
  Owned, ScheduledVotes}`) are registered as root screens in `app.rs` but their nav sidebar
  entries are **intentionally removed** — `left_panel.rs` documents this: "The former standalone
  Identities and Dashpay entries are intentionally hidden from the nav; their screens, routes,
  and backend paths stay intact and remain reachable through other means (deep links, MCP tools,
  direct screen construction)." The GUI-reachable path for voting/contests today is the
  Masternode **detail** screen's inline per-contest vote controls
  (`masternodes/detail_screen.rs`), reached only after loading a masternode/evonode identity via
  IDN-003 — which fails with a silent hang in this environment.
- Empirically confirmed the identity-picker breadcrumb pill (the one place a "Create multiple
  test identities" dev shortcut lives, per `global_nav_switcher.rs`) is a **non-interactive
  placeholder** when zero identities exist — clicking `(choose an identity)` in the Identities
  breadcrumb produces no popup, no menu, nothing (screenshot below). Source confirms why:
  `render_app_global_identity_pill()` returns early on `data.pill_identity.is_none()` before ever
  building the popup that contains that shortcut. So there is no dev-tool bypass to bootstrap an
  identity in this build either. Likewise, the "Developer tools: Create multiple test identities
  · Load identity by ID" footer text on the onboarding screen is decorative only (a `ui.label`,
  not a button — confirmed by source and by clicking it with no effect).

**Conclusion**: DPN is fully identity-gated with no alternate/back-door reachability path. Every
story below is BLOCKED on the same root cause already established in `scenarios/IDN.md`.

---

## DPN-001: Register a DPNS username — **BLOCKED**

**Persona:** Alex, Priya. Acceptance criteria: "Choose identity, enter desired name. Cost
estimate displayed before confirmation."

### Reachability

No identity loaded (see environment status above) → the Identities hub never leaves the
onboarding empty state → the registration screen is unreachable in this session's UI.

### Source review (implementation confirmed, not live-exercised)

`register_dpns_name_screen.rs` implements real client-side format validation before any network
call — `validate_dpns_name()` checks length (3–63 chars) and character set (letters, numbers,
hyphens only), with per-violation error text ("Name must be at least 3 characters long", "Invalid
character '{c}'. Only letters, numbers, and hyphens are allowed"), and a "Valid name format" /
"This is not a contested name." / "This is a contested name. Cost ≈ 0.2006 Dash" status line as
the user types. Separately, a general **"Estimated fee:"** line (via
`fee_estimator.estimate_document_create()`, formatted through
`model::fee_estimation::format_credits_as_dash`) is shown for every registration attempt
regardless of contested status, plus an inline "Insufficient identity balance for fee" check
before the button enables — satisfying the story's "cost estimate displayed before confirmation"
criterion structurally. None of this was exercised live; it is a static-code read, flagged the
same way `scenarios/IDN.md` flagged IDN-012.

One structural nit (not a functional bug): `validate_dpns_name()` lives in
`ui/identities/register_dpns_name_screen.rs`, not `model/` — the project's own `DET Module
Placement Policy` (CLAUDE.md) states pure format/length validation belongs in `model/` as a
stateless function. Worth a minor follow-up, not counted against this verdict.

**Verdict: BLOCKED** — reasoning: "blocked: no Platform identity reachable in this environment,
see scenarios/IDN.md — root cause is the known Testnet masternode-list/quorum-sync/wallet-storage
failure, see CAMPAIGN-CONTEXT.md".

---

## DPN-002: View owned usernames — **BLOCKED**

**Persona:** Alex, Priya. Acceptance criteria: "Lists all usernames tied to the current wallet's
identities."

Owned-username display lives in the Identity Hub's **Settings** tab (username + aliases panel per
`identity/settings.rs`'s module doc). Same gating as DPN-001: the Settings tab does not render
until `HubLanding::Home`/`Picker` (≥1 local identity), which never happens in this session.

**Verdict: BLOCKED** — same reasoning as DPN-001.

---

## DPN-003: View active name contests — **BLOCKED**

**Persona:** Priya. Acceptance criteria: "Lists all contests with status and vote counts."

Reachable only via a loaded masternode/evonode's detail screen (`masternodes/detail_screen.rs`),
which fetches `ContestedResourceTask::QueryDPNSContests` for that node's voter identity. Navigated
to Masternodes: confirmed **"No masternodes loaded"** empty state (matches `scenarios/DEV.md`'s
DEV-006 screenshot and `scenarios/IDN.md`'s IDN-003 finding) — "Load a masternode" is the only
path in, and IDN-003 already demonstrated that submitting a well-formed ProTxHash there hangs
silently with zero feedback in this environment. No masternode/evonode identity exists to view
contests for.

**Verdict: BLOCKED** — reasoning: "blocked: no Platform identity reachable in this environment,
see scenarios/IDN.md — root cause is the known Testnet masternode-list/quorum-sync/wallet-storage
failure, see CAMPAIGN-CONTEXT.md" (specifically, no masternode/evonode identity is loadable —
IDN-003's "Load masternode" submission hangs silently on this exact prerequisite).

---

## DPN-004: View past name contests — **BLOCKED**

**Persona:** Priya. Acceptance criteria: "Lists completed contests with results."

Same reachability path and root cause as DPN-003 (past-contest history is a sibling view under the
same masternode-detail-gated surface). No separate UI exists that would make past contests
reachable without a loaded masternode/evonode identity.

**Verdict: BLOCKED** — same reasoning as DPN-003.

---

## DPN-005: Vote on contested names — **BLOCKED**

**Persona:** Priya (masternode operator). Acceptance criteria: "Cast, change, or abstain votes
(max 4 vote changes per contest). Evonode/masternode identity required."

The story's own acceptance criteria states a masternode/evonode identity is required — confirmed
in source (`masternodes/detail_screen.rs`'s inline per-contest vote controls, gated on the node
having a voter identity with a loaded voting key). IDN-003 could not load a masternode/evonode
identity in this environment (silent hang on submission after passing ProTxHash format
validation). No voting surface is reachable.

**Verdict: BLOCKED** — same reasoning as DPN-003.

---

## DPN-006: Schedule votes — **BLOCKED**

**Persona:** Priya. Acceptance criteria: "Set vote to be cast at a future time. View and manage
scheduled votes."

Same masternode/evonode-identity prerequisite as DPN-005 (the Scheduled Votes surface is
referenced from the masternode detail screen per `detail_screen.rs`'s doc comment: "Scheduled
Votes screen (§10.7)"). No masternode/evonode identity reachable.

**Verdict: BLOCKED** — same reasoning as DPN-003.

---

## DPN-007: Batch voting across contests — **BLOCKED**

**Persona:** Priya. Acceptance criteria: "'Set all' option for batch vote assignment."

Same masternode/evonode-identity prerequisite and reachability path as DPN-005/006. No voting
surface reachable to exercise a "Set all" control.

**Verdict: BLOCKED** — same reasoning as DPN-003.

---

## Summary

| Story | Verdict |
|---|---|
| DPN-001 | BLOCKED (no identity reachable; client-side name-format validation + fee estimate confirmed implemented via source, not live-exercised) |
| DPN-002 | BLOCKED (no identity reachable) |
| DPN-003 | BLOCKED (no masternode/evonode identity reachable — IDN-003) |
| DPN-004 | BLOCKED (same as DPN-003) |
| DPN-005 | BLOCKED (same as DPN-003; story's own acceptance criteria requires a masternode/evonode identity) |
| DPN-006 | BLOCKED (same as DPN-003) |
| DPN-007 | BLOCKED (same as DPN-003) |

All seven DPN stories are BLOCKED on the same root cause already established in
`scenarios/IDN.md`: zero identities of any kind (user or masternode/evonode) can be loaded or
registered in this environment. This pass additionally confirmed via source + empirical clicking
that there is no dev-tool bypass or alternate nav path that sidesteps the identity requirement —
DPNS registration, username management, contest viewing, and voting are all gated behind the
Identity Hub or a loaded masternode's detail screen, both of which require an identity that
cannot currently be established. No PR892 application source was modified; no persistent state
was changed by this pass (read-only navigation only — confirmed no new rows possible since the
identity/masternode-loading screens were never reached).
