# MN — Masternodes

Environment: PR892 build, isolated data dir `/data/tmp/det-qa-pr892-data`, display `:99`. Brand
new category for this campaign — 12 stories (MN-001–MN-012), all newly written around the
Masternodes tab introduced in this PR. `progress.md`/the reconciled catalog reclassifies the old
IDN-003 ("Load evonode/masternode identity") as `[Superseded by MN-001]`; that story's prior FAIL
finding (silent hang on "Load masternode" with a well-formed ProTxHash) is treated here as prior
context, re-verified fresh rather than assumed unchanged.

**Zero masternodes/evonodes are loaded in this environment for the whole session** — no
`.testnet_nodes.yml` dev fixture exists (confirmed again this pass, consistent with `DEV.md`'s
DEV-006 finding), and real registration needs ~1000 tDASH collateral this environment doesn't
have. Combined with MN-001's re-confirmed load hang below, no masternode/evonode ever becomes
loadable, so every story that requires an *already-loaded* node (MN-003, MN-004, MN-006 through
MN-009, MN-011's core behavior) is transitively BLOCKED. Five stories have testable surface that
does not require a loaded node — MN-001, MN-002, MN-005, MN-010, MN-012 — and were exercised
live; the rest received a quick read-only source review as supporting context only, per the
task's own guidance.

**Testnet wallet-backend blocker still active throughout** — the same known issue documented in
`scenarios/ALK.md`/`scenarios/DEV.md`: 3-4 red banners present all session ("SPV sync failed",
"We couldn't finish preparing your wallet", "Your wallet is still starting up", "Could not load
your identities from this device"). Cited, not re-diagnosed.

---

## MN-001: Load a masternode by keys — FAIL (silent hang on submit re-confirmed, with new
## evidence pointing at the wallet-backend blocker as a likely contributing cause)

**Persona:** masternode operator. Acceptance criteria: dedicated ProTxHash + Masternode/Evonode
+ alias + optional VOP-key load form; "Load masternode" disabled+tooltip until a ProTxHash is
entered; malformed/already-loaded ProTxHash rejected with a specific message; non-blocking
unencrypted-storage note; Testnet-only "Fill Random" dev convenience when a fixture is present.

### Steps and observed result

1. Masternodes tab (empty state) > "Load a masternode" opened the form: Masternode/Evonode
   toggle, ProTxHash field, Alias (optional), Voting/Owner/Payout private key fields, and a new
   **"Encryption password (optional)"** field (see MN-006 below) with helper text "Set a password
   to encrypt these keys on this device. Leave it blank to store them unencrypted and add
   protection later." plus an always-visible warning-toned note: *"Set an optional password to
   encrypt these keys on this device. Without one, they are stored unencrypted and you can add
   protection later from the key screen."* — matches bullet 3 exactly (non-blocking, informative,
   not alarming). Screenshot:
   `screenshots/MN-001-1-load-form-empty-disabled-button-unencrypted-note.png`.
2. **Disabled+tooltip (bullet 2, first half)**: with ProTxHash empty, "Load masternode" renders
   visibly greyed-out/disabled — confirmed live. Hovering it to capture the tooltip text was
   attempted repeatedly (5+ attempts, dwell times up to 5s, window-focus explicitly confirmed via
   `xdotool windowactivate`/`windowfocus`) but the tooltip did not render on screen in any
   attempt, despite the exact same technique successfully capturing tooltip text elsewhere this
   session (a sanity-check hover over a "Show details" link registered normally, and `NET.md`'s
   NET-017 captured a tooltip on the connection-status dot with this same method). Source review
   resolved the ambiguity: `src/ui/masternodes/load_form.rs:450` wires
   `.disabled_tooltip(LOAD_DISABLED_TOOLTIP)` where `LOAD_DISABLED_TOOLTIP = "Enter a ProTxHash to
   continue."` (`load_form.rs:23`), and `ResponseExt::disabled_tooltip` in `src/ui/theme.rs:1082`
   correctly calls `on_disabled_hover_text(text)` plus `on_hover_cursor(CursorIcon::NotAllowed)` —
   the standard, correctly-used pattern for this exact purpose. Disabled-state: **live-confirmed
   PASS**. Tooltip text: **source-confirmed correct**, live visual capture inconclusive (treated
   as an automation/timing limitation, not a functional defect, given the unambiguous source
   wiring).
3. **Malformed ProTxHash (bullet 2, second half)**: typed `not-a-valid-protxhash`, clicked "Load
   masternode" (now enabled). Got a clean inline validation error: **"This doesn't look like a
   valid ProTxHash. Enter a hex or Base58 ProTxHash from your masternode configuration."** —
   identical wording to IDN-003's prior finding, re-confirmed fresh. Screenshot:
   `screenshots/MN-001-2-malformed-protxhash-validation.png`.
4. **Well-formed but fake ProTxHash — the core re-test**: replaced the input with a freshly
   generated random 64-hex-char string (`a1568cfaaec73f539c91a452cde8a7998765b0230619f1e29fa1aecf59bbf288`
   — passes `is_valid_pro_tx_hash`'s format check, no such ProTxHash exists on-chain). No
   validation error shown; "Load masternode" enabled. Clicked it, with `det.log` line-count and a
   wall-clock timestamp captured immediately beforehand for precise before/after comparison.
   - **New this pass**: ~6s after the click, a fresh log line appeared —
     `WARN dash_evo_tool::backend_task: Wallet backend initialization deferred error=Could not
     access wallet data. Check available disk space and restart the application.` — the exact
     same `WalletBackendNotYetWired`-class error already showing as one of the persistent red
     banners on this screen. IDN-003's prior pass explicitly reported *zero* log activity after
     its equivalent click; this pass got one. This is a genuine behavioral difference worth
     flagging, even though the end-user-visible outcome is unchanged (see below).
   - **User-facing outcome, unchanged from IDN-003**: no banner, no navigation, no "Loading…"
     state on the button (it stayed as plain "Load masternode", never switched to the
     disabled+spinner submitting state the source shows exists for this exact case —
     `load_form.rs:438-442`), and the ProTxHash field still held the entered value. Reconfirmed
     at 3s, 15s, and 20s after the click — no further log lines, no UI change. Screenshot:
     `screenshots/MN-001-3-wellformed-fake-protxhash-silent-hang-20s.png`.
   - Source review of `backend_task/mod.rs:558-606` (`run_backend_task`) shows the deferred-init
     warning is logged and then **execution continues** into the task match arm rather than
     returning early (only a `TerminalStorageOpenError` short-circuits with a surfaced error, and
     a separate migration-in-progress gate doesn't apply here) — meaning the load logic proceeds
     into wallet-dependent code paths whose prerequisites were never actually initialized, and
     apparently hangs there with no path back to a user-visible error or the `submitting` UI
     state ever engaging. Consistent with the environment's known wallet-backend blocker being a
     likely (if not fully root-caused here) contributing factor, not a masternode-load-specific
     regression in isolation — though from a user's perspective the net effect is identical to
     IDN-003: click the button, get nothing.
5. Navigated back to "‹ All masternodes" — confirmed the list is still the empty "No masternodes
   loaded" state (the load never succeeded), and the header pill still correctly reads "(no
   masternode yet)" — no residual UI corruption from the stuck attempt.
6. **"Fill Random Masternode/Evonode" (bullet 4)**: no such button/row is present anywhere in the
   form — confirmed live by viewing the full form top-to-bottom (screenshot:
   `screenshots/MN-001-4-no-fill-random-button-no-fixture.png`, showing the space directly between
   the Masternode/Evonode toggle and the ProTxHash field where the row would render). Source
   confirms why: `load_form.rs:307-320` gates the entire row on `dev_mode && self.testnet_nodes.
   is_some()` — "Entire row is absent otherwise — never shown-disabled" per its own comment — and
   no `.testnet_nodes.yml` fixture exists in this environment (re-confirmed, matching DEV-006's
   prior finding). Absence here is the expected, correct behavior for a fixture-less environment,
   not a defect.

### Verdict: FAIL

Re-verified IDN-003's finding fresh rather than assuming it's unchanged, and it **is** unchanged
in the way that matters to a user: clicking "Load masternode" with a well-formed, syntactically
valid ProTxHash still produces total silence — no banner, no loading indicator, no navigation,
no eventual timeout/error, reconfirmed across 20s of waiting. What's new this pass is a `det.log`
line pointing at the same wallet-backend-not-ready condition already surfaced as a red banner
elsewhere on this exact screen — suggesting the masternode-load hang may be a downstream symptom
of the environment's pre-existing wallet-backend blocker rather than an independent bug, though
this pass cannot fully separate the two without a healthy wallet backend to test against. Every
other bullet in this story passes cleanly: the disabled-button gate and malformed-hash rejection
both work correctly and are precisely worded; the unencrypted-storage note is present, correctly
non-blocking, and well-worded; and the Fill-Random button's absence is expected and
correctly-gated given no fixture exists. Re-test once the wallet-backend blocker is resolved.

---

## MN-002: See my masternodes at a glance — mostly PASS on the directly-testable half (empty
## state + interface-mode gating); card-list-with-real-nodes half untested (no nodes loaded)

**Persona:** masternode operator. Acceptance criteria: card list (shortened ProTxHash/alias, type
badge, voter readiness, key-status, DPNS-voting status, identity status dot+label); empty state
explains the concept + offers "Load a masternode"; tab/nav entry visible only at Detailed
(Expert) view or above, with live fallback to Identities if the role drops while the tab is
active.

### Steps and observed result

1. **Empty state**: Masternodes tab (0 nodes loaded) shows "No masternodes loaded", body copy
   *"Load a masternode or evonode to vote on DPNS name contests and manage its owner and payout
   keys."*, a primary blue "Load a masternode" button, and a helper line *"Have your node's
   ProTxHash to hand. Keys are optional — a node loads read-only without them."* — clearly
   explains what a masternode identity is for and offers the primary CTA, matching bullet 2.
   Screenshot: `screenshots/MN-002-1-empty-state-and-header-pill.png`.
2. **Interface-mode gating, live**: from Expert view on the Masternodes tab, navigated to
   Settings > Interface mode > "Default view". Sidebar immediately dropped the Masternodes entry
   entirely (Identities, Contracts, Tokens, Wallets, Tools, Settings only — no Masternodes, no
   "Expert" role indicator at the sidebar foot either). Screenshot:
   `screenshots/MN-002-2-default-view-no-masternodes-nav-entry.png`. Confirms the nav-entry
   visibility half of bullet 3 directly.
3. **Live de-gating fallback (the harder half of bullet 3)**: changing interface mode is only
   reachable via the Settings screen, and navigating to Settings is itself a root-screen switch
   (`RootScreenType::RootScreenNetworkChooser`) that moves `selected_main_screen` off Masternodes
   *before* the mode toggle is ever clicked — confirmed via source
   (`src/app.rs:1173-1186`, `active_root_screen_mut()`): the live re-gate check is
   `if self.selected_main_screen == RootScreenType::RootScreenMasternodes &&
   !FeatureGate::Masternodes.is_available(...) { self.select_main_screen(FALLBACK_ROOT_SCREEN)
   }`, and `FALLBACK_ROOT_SCREEN = RootScreenType::RootScreenIdentityHub` (`app.rs:102`) — i.e.
   exactly "falls back to the Identities screen" as the story specifies. This single-window app
   has no UI path to flip interface mode *without* first leaving the Masternodes screen, so the
   literal "role drops while Masternodes is the on-screen tab" sequence could not be triggered
   with mouse-only interaction this pass — the guard is defensive-in-depth for edge cases (e.g. a
   future multi-surface trigger of role changes) beyond what the nav-hiding alone already
   prevents in every reachable, real user flow. Restored Expert view afterward and confirmed the
   Masternodes tab reappeared and rendered correctly (clean empty state, correct header pill).
4. **Card-list content** (type badge, voter readiness, key-status glyphs, DPNS-voting status,
   identity status dot+label): cannot be exercised live — 0 nodes loaded all session (MN-001's
   hang prevents ever loading one). Source review of `src/ui/masternodes/card.rs` confirms the
   structural elements exist: `card_heading`/`card_sub_line` (alias-or-shortened-ProTxHash),
   `draw_type_badge`, `voter_readiness_label` ("Voting ready" / "No voting key"),
   `key_status_tokens`, and `platform_identity_status_label` with a dedicated status-dot rect —
   consistent with the story's claims, but this is source-only corroboration, not a live
   confirmation.

### Verdict: PASS for the directly-testable empty-state and nav-visibility halves; the card-list
### content and the literal same-frame de-gating trigger are untested/unreachable this pass — noted
### as untested scope, not failures

The empty state is well-written and matches the acceptance criteria precisely. The Masternodes
tab and its sidebar entry are confirmed gated to Detailed (Expert) view and above — dropping to
Default view live-hides the tab immediately, and Expert view live-restores it with the screen
intact. The "falls back to Identities" mechanism is confirmed correct and precisely-targeted in
source, but the app's own navigation model (must leave Masternodes to reach the Settings toggle)
makes the literal live trigger unreachable via normal UI interaction — this is an architectural
observation, not a defect. The card-list-with-real-nodes half remains genuinely untested pending
a loaded node.

---

## MN-003: Open a masternode and vote — BLOCKED (no loaded masternode reachable)

**Reasoning**: requires an already-loaded masternode/evonode to open a detail view and vote —
unreachable this session because MN-001's "Load masternode" hangs silently on every well-formed
ProTxHash (re-confirmed above), the same defect class IDN-003 first found. No fixture exists to
bypass the load flow (see MN-001 bullet 6 / `DEV.md`'s DEV-006).

Quick read-only source review (`src/ui/masternodes/detail_screen.rs`) as supporting context only:
the file implements a full DPNS-voting section — `dpns_section_header()` ("DPNS name contests to
vote on (N)"), a `MasternodeContestSummary`/`ContestedName` model, per-contest candidate/vote-
count rendering (`candidate_choice_label`), and a framing line shown once above the vote controls
plus a nudge for contests with no vote picked yet — dispatching through
`ContestedResourceTask`. Structurally consistent with the story's claims; not independently
live-verified.

**Verdict: BLOCKED.**

---

## MN-004: Remove a masternode — BLOCKED (no loaded masternode reachable)

**Reasoning**: same as MN-003 — no node ever loads this session.

Quick read-only source review as supporting context: `detail_screen.rs:925-935` implements
`render_remove_section()` with a "Remove masternode" button that opens a confirmation dialog
(`.confirm_text(Some("Remove masternode"))`) rather than removing immediately — matches the
expectation of a confirm-before-destructive-action pattern. Not independently live-verified.

**Verdict: BLOCKED.**

---

## MN-005: Keep the everyday surface clean — PASS (live-confirmed on the directly-testable half)

**Persona:** everyday user. Acceptance criteria: masternode/evonode identities filtered out of
the Identity Hub picker (still visible on the Masternodes tab); the legacy "Load Existing
Identity" screen's Identity Type selector now offers User only.

### Steps and observed result

1. Identities tab (Identity Hub) > "I already have an identity — load it" opened the legacy
   `Load Existing Identity` screen. Default view (Advanced Options collapsed) shows three tabs:
   **"Identity ID & private key" | "From my wallet" | "My username"** — no fourth "ProTxHash"/
   masternode tab exists here at all (contrast with IDN-003's prior finding, which found a
   Masternode/Evonode node-type toggle directly on this screen).
2. Clicked "Show Advanced Options" to reveal the full field set (matches the checkbox seen in
   IDN-002/003's prior passes) — this revealed an **"Identity Type:"** dropdown, currently reading
   "User". Clicked it open: the dropdown lists **exactly one option, "User"** — no Masternode or
   Evonode entry present. Screenshot:
   `screenshots/MN-005-1-legacy-load-identity-type-user-only.png`.

### Verdict: PASS (for the directly-testable second bullet)

This is a clean, precise regression fix relative to IDN-003's prior finding: the legacy Load
Existing Identity screen's Identity Type selector now offers **User only**, and there is no
separate ProTxHash-loading tab left on this screen either — masternode/evonode loading has been
fully relocated to the dedicated Masternodes tab, exactly as this story specifies. The first
bullet (Identity Hub picker filtering out masternode identities) could not be directly tested — no
masternode identity was ever loaded this session to verify it gets filtered — but the Identity
Hub's picker only ever showed the "(choose an identity)" placeholder with 0 identities present,
consistent with (though not proof of) the filtering claim; no contradicting evidence was found.

---

## MN-006: Encrypt my node keys at load time — BLOCKED (cannot observe an actual load with a
## password; load form itself already seen and reported under MN-001)

**Reasoning**: requires actually loading a node with a password set to observe the resulting
protection tier — unreachable this session because loading never completes (MN-001).

The load form was already examined live under MN-001 (step 1): it has an **"Encryption password
(optional)"** field with helper text "Set a password to encrypt these keys on this device. Leave
it blank to store them unencrypted and add protection later," plus the always-visible warning-
toned unencrypted-storage note. This directly matches the story's premise that at-load encryption
is a real, present feature — not a gap.

Quick source review of the Tier-1/Tier-2 sealing logic as supporting context: `detail_screen.rs`'s
`render_keys_section()` reads a `protection_tier()` and conditionally shows an "Add password
protection…" CTA via `tier.offers_add_protection()`, routing into the same `KeyInfoScreen` seal
flow (`IdentityTask::ProtectIdentityKeys`) documented in `CLAUDE.md`'s secret-storage-seam section
for identity keys generally. Consistent with masternode keys following the same Tier-1 (keyless)
→ Tier-2 (per-identity password-sealed) model as other identity key types, with the load-time
password field as an alternate, earlier entry point into Tier-2. Not independently live-verified.

**Verdict: BLOCKED.**

---

## MN-007: Withdraw a node's credits — BLOCKED (no loaded masternode reachable)

**Reasoning**: same as MN-003/MN-004.

Quick read-only source review as supporting context: `detail_screen.rs:511-512` has a "Withdraw"
button that pushes `ScreenType::WithdrawalScreen(self.identity.clone())` — reusing the same
withdrawal screen as a regular identity, scoped to the masternode's own identity. Not
independently live-verified.

**Verdict: BLOCKED.**

---

## MN-008: Manage a node's keys — BLOCKED (no loaded masternode reachable)

**Reasoning**: same as MN-003/MN-004/MN-007.

Quick read-only source review as supporting context:

- `detail_screen.rs`'s `render_keys_section()` lists each held key (main identity + voter
  identity) with role labels resolved via `role_label_and_tip()` — Voting/Owner/Payout
  address/Authentication — each opening the real, interactive `KeyInfoScreen` (view/sign/seal),
  not a static read-only table.
- The **add-key purpose selector** (`src/ui/identities/keys/add_key_screen.rs`, the generic
  screen shared across identity types) offers exactly four selectable purposes in its UI:
  `ENCRYPTION`, `DECRYPTION`, `AUTHENTICATION`, `TRANSFER` (`add_key_screen.rs:460-510`) — `OWNER`
  and `VOTING` never appear as options anywhere in that match, for any identity type. This
  structurally satisfies the story's "correctly excludes OWNER/VOTING" requirement, though it does
  so by those purposes never being generically addable at all (they're DIP3-specific, protocol-
  assigned roles) rather than via a masternode-specific runtime filter — the practical guarantee
  (a user can never add an OWNER/VOTING key through this flow) holds either way. Not independently
  live-verified.

**Verdict: BLOCKED.**

---

## MN-009: Claim an evonode's token rewards — BLOCKED (no loaded Evonode reachable)

**Reasoning**: same as MN-003/MN-004/MN-007/MN-008, and additionally requires the Evonode variant
specifically (not just any masternode).

Quick read-only source review as supporting context: `detail_screen.rs:514-522` shows a "Claim
token rewards ›" button with hover text "Claim this evonode's token rewards," gated to render
only for the Evonode identity type ("Evonode-only token-rewards cross-link (FR-11); absent for a
plain [masternode]" per its own comment), routing via `claim_token_rewards_action()`. Not
independently live-verified.

**Verdict: BLOCKED.**

---

## MN-010: Keep the Masternodes tab consistent across a network switch — PASS

**Persona:** masternode operator. Acceptance criteria: switching networks while on the List view
(including with a filled-but-unsubmitted Load form) returns to the empty List view for the newly
active network with no leftover ProTxHash/alias/key input; error/status banners from the previous
network are cleared by the switch.

### Steps and observed result

1. On Testnet, Masternodes tab > "Load a masternode" > switched the toggle to **Evonode**, typed
   a fake 64-hex ProTxHash (`deadbeef` × 8) into the ProTxHash field, and
   `"MN-010 leftover alias test"` into Alias — **did not submit**. Screenshot:
   `screenshots/MN-010-1-form-filled-before-network-switch.png`.
2. Settings > Networks > switched Network from Testnet to Mainnet. Observed the Testnet-specific
   banner set (4 banners: SPV sync failed / wallet still starting / couldn't finish preparing
   wallet / couldn't load identities) was immediately replaced by a fresh, distinct Mainnet
   banner set (1 red banner + a temporary "SPV sync in progress…" toast) — confirms bullet 2
   (stale banners cleared by the switch) directly, before even reaching the Masternodes tab.
3. Navigated to the Masternodes tab on the now-active Mainnet network: showed the clean **"No
   masternodes loaded"** empty List view — not the Load form, and critically not the Load form
   pre-filled with the Evonode/ProTxHash/alias entered in step 1. Screenshot:
   `screenshots/MN-010-2-mainnet-clean-list-view-no-leftover.png`. The header pill correctly read
   `Masternodes › 💼 QA Wallet 1 › (no masternode yet)`, and only 1 (Mainnet-relevant) error
   banner remained.
4. Switched back to Testnet (Settings > Networks; had to click "Disconnect" first — the network
   dropdown is disabled while a connection is actively `Synced`/`Connecting`, only enabled once
   disconnected, a sensible guard unrelated to this story). Confirmed the app returned to the
   familiar Testnet known-blocker state (SPV sync failed / wallet starting banners) and the
   Masternodes tab rendered its normal clean empty state afterward — app left healthy on Testnet.

### Verdict: PASS

Both bullets are directly, live-confirmed. Switching networks while the Load form held
unsubmitted Evonode-type input (ProTxHash + alias) returned to the clean, empty List view for the
new network — the Load form itself was discarded entirely, not just cleared field-by-field, which
satisfies the "no leftover ProTxHash/alias/key input" requirement about as strongly as possible.
Stale per-network error/status banners were also confirmed cleared on the switch, both on the
Settings screen itself and again on arrival at Masternodes. The app was left back on Testnet,
Expert view, in its normal known-blocker state, ready for the next test.

---

## MN-011: Refresh masternode and voting state — BLOCKED overall (core node-refresh behavior
## needs a loaded node), with a small positive no-op-safety data point

**Persona:** masternode operator. Acceptance criteria: a Refresh control re-queries node state and
DPNS voting status for loaded nodes; refresh is a no-op when no node is loaded.

### Steps and observed result

1. On the Masternodes tab's empty-list toolbar (0 nodes loaded), an orange **"Refresh"** button is
   present next to "+ Load", even with an empty list. Clicked it.
2. Observed result: no crash, no new error banner, no change to the empty-state screen, no
   spinner or visible activity of any kind. Waited 3s and re-screenshotted — no change. Screenshot:
   `screenshots/MN-011-1-refresh-clicked-no-op.png`.
3. Source review of `src/ui/masternodes/list_screen.rs:433-446` (`refresh_from_network()`)
   confirms this is the intended, coded behavior, not a silent failure: it loads local masternode
   identities, and `if identities.is_empty() { return AppAction::None; }` — an explicit early
   return producing no backend dispatch at all when the list is empty, matching the story's own
   second bullet verbatim ("Refresh is a no-op when no node is loaded").

### Verdict: BLOCKED (core node-refresh functionality — re-querying state/voting status for an
### actually-loaded node — is untestable this session; no node ever loads). The no-op-safety
### sub-check **passes**: the Refresh control exists even with zero nodes loaded, and clicking it
### is confirmed safe (no crash, no error, matches the source-coded no-op path) — a small positive
### data point, not a substitute for the blocked core behavior.

---

## MN-012: Switch wallet/identity from the Masternodes header — PASS (on the directly-testable
## presence + empty-state-text half)

**Persona:** masternode operator. Acceptance criteria: page-aware breadcrumb with interactive
wallet pill; third segment is a page-scoped node pill listing every loaded masternode/evonode,
reading `(no masternode yet)` when none is loaded; picking a node there never changes the
identity shown on everyday-user pages.

### Steps and observed result

1. Masternodes tab header (Testnet, Expert view, 0 nodes loaded) reads exactly: **`Masternodes ›
   💼 QA Wallet 1 › (no masternode yet)`** — three segments: the page-link ("Masternodes"), an
   interactive wallet pill ("💼 QA Wallet 1"), and the page-scoped node-pill placeholder. Screenshot:
   `screenshots/MN-012-1-header-no-masternode-yet.png`. This is consistent with `UX.md`'s UX-003
   prior finding on the same build, which independently confirmed the Masternodes tab has a
   3-segment, fully-interactive switcher with this exact placeholder text distinct from the
   Identity Hub's own `(choose an identity)` placeholder — corroborating evidence from a separate
   test pass, not just this session's single observation.
2. **Exact placeholder text match**: the third segment reads precisely `(no masternode yet)` —
   character-for-character what the acceptance criteria specifies — confirmed both in this
   session's live screenshot and by UX-003's independent prior pass.
3. **Wallet pill interactivity**: not re-exercised in depth this pass (already covered by UX-003's
   live switching test on this same header); its presence and correct label ("💼 QA Wallet 1") are
   re-confirmed here.
4. **Node-pill picking behavior** ("Picking a node there never changes the identity shown on
   everyday-user pages"): untestable — no masternode/evonode is loaded this session (MN-001's
   hang), so there is nothing in the node pill to pick.

### Verdict: PASS (for the directly-testable presence + exact-placeholder-text sub-parts).
### The interactive node-picking / cross-page-identity-isolation sub-part is untested, not
### failed — no loaded node exists to pick.

The header switcher is present, correctly structured (3 segments), and the empty-state node-pill
text is an exact match to the story's specified copy. The behavioral guarantee about picking a
node never leaking into everyday-user pages could not be exercised — there's no node to pick —
and is noted as untested scope rather than assumed.

---

# Retest — 2026-07-15 (recurrence-2 environment fix: does anything change for MN?)

Environment: same PR892 build/hash, running instance PID 527888, data dir
`/data/tmp/det-qa-pr892-data`, Testnet. The Testnet wallet-backend blocker is fixed (upstream
`dashpay/platform#4133`). Task guidance: MN was expected to likely still be blocked on the "no
masternode fixture" constraint (real registration needs ~1000 tDASH collateral), but to check
first whether anything changed given MN-001's original finding suspected the wallet-backend
blocker as a contributing cause to its silent hang.

**Something did change.** Re-tested MN-001 fresh: Masternodes tab (still empty, "No masternodes
loaded") > "Load a masternode" > entered a freshly-generated, well-formed-but-nonexistent 64-hex
ProTxHash. Previously this silently hung forever with zero feedback. **Now**: clicking "Load
masternode" returns, within a couple seconds, a clean, correctly-worded red banner —
**"No masternode or evonode was found on the network for this ProTxHash. Check the ProTxHash and
try again, or confirm the node is registered on this network."** — with "Show details" revealing
a proper typed error, `MasternodeNotFound { identity_id: Identifier(...) }`, not a raw string.
Screenshots: `screenshots/MN-001-1-load-wellformed-fake-protxhash-now-clean-error.png`,
`screenshots/MN-001-2-typed-error-details-masternodenotfound.png`. Navigating back to the list
confirmed it's still cleanly empty — no residual UI corruption from the attempt. Screenshot:
`screenshots/MN-001-1-load-wellformed-fake-protxhash-now-clean-error.png`.

This confirms MN-001's original suspicion was correct: the silent hang **was** a downstream
symptom of the wallet-backend blocker, not an independent masternode-load bug. **MN-001 is
upgraded from FAIL to PASS** — every acceptance-criteria bullet (form fields, disabled-gate,
malformed-hash rejection, unencrypted-storage note, Fill-Random gating, and now also the
not-found-on-network path) works correctly.

**What this does *not* change**: there is still no *real* masternode/evonode registered on
Testnet that this environment can load — the fake ProTxHash correctly comes back "not found"
because it genuinely isn't registered, and getting a real one still requires ~1000 tDASH
collateral this environment doesn't have (per `CAMPAIGN-CONTEXT.md`). A `memcan:recall` search
for a pre-existing masternode/evonode fixture (project `dash-evo-tool`) turned up nothing usable.
A brief external search for a public, ownership-free Testnet ProTxHash to load **read-only**
(the form explicitly supports keys-optional read-only loading) did not turn up a live, fetchable
source in the time budget for this pass. So MN-003/004/006/007/008/009/011 — every story that
needs an *actually-loaded* node — remain genuinely BLOCKED, but the reasoning is now narrower and
more precise: purely "no real fixture available," not "the load mechanism itself might be
broken." MN-002/005/010/012 were not re-tested (unaffected by either environment fix — their
prior PASS verdicts don't depend on the wallet-backend blocker).

## MCP-003/MCP-004 cross-reference (same underlying code path)

`scenarios/MCP.md`'s MCP-003/004 share the identical masternode/evonode-identity-fixture
dependency. Re-verified the CLI tool schemas are unchanged via a freshly rebuilt, hash-noted
`det-cli` (`tool-describe name=masternode_identity_load` / `masternode_credits_withdraw`) —
identical shape to the original pass. Did not re-run a live fake-ProTxHash dispatch through the
CLI this pass (that requires a full from-scratch SPV sync in a throwaway dir, disproportionate
for what would only re-confirm what MN-001 already proved for the same underlying identity-fetch
code path) — but MN-001's live confirmation above is strong indirect evidence the CLI's
SPV-gated dispatch now behaves the same way (clean "not found" instead of an indefinite hang).
MCP-003/004 remain BLOCKED — same fixture-availability constraint, unaffected by either fix.

---

## Summary

| Story | Verdict | One-line reason |
|---|---|---|
| MN-001 | **PASS** (2026-07-15, upgraded from FAIL) | Disabled-button gate, malformed-hash rejection, unencrypted-storage note, and Fill-Random gating all correct (unchanged); the silent hang on a well-formed nonexistent ProTxHash is now FIXED — returns a clean, fast, correctly-worded "not found" error with a proper typed `MasternodeNotFound` in details, confirming it was a downstream symptom of the (now-fixed) wallet-backend blocker. |
| MN-002 | **PASS** (directly-testable scope) | Empty state and Expert-view-only nav gating (incl. live restore) both confirmed; card-list-with-real-nodes and the literal same-frame de-gating trigger are untested/architecturally unreachable, not failed. Not retested 2026-07-15 (unaffected by the env fix). |
| MN-003 | **BLOCKED** (2026-07-15: narrower reasoning) | MN-001's load flow is now confirmed working end-to-end; no loaded masternode reachable purely because no real fixture is registered on Testnet for this environment (~1000 tDASH collateral required) — not because loading hangs. DPNS-voting UI structurally confirmed via source only. |
| MN-004 | **BLOCKED** (2026-07-15: narrower reasoning) | Same as MN-003; confirm-before-remove dialog structurally confirmed via source only. |
| MN-005 | **PASS** | Legacy "Load Existing Identity" screen's Identity Type selector now offers User only, and its ProTxHash-loading tab is gone entirely — clean regression fix vs. IDN-003's prior finding. Not retested 2026-07-15 (unaffected by the env fix). |
| MN-006 | **BLOCKED** (2026-07-15: narrower reasoning) | Same as MN-003 — MN-001's load flow works, but no real fixture exists to observe an actual encrypted load; load-time password field already confirmed present and correctly worded under MN-001. |
| MN-007 | **BLOCKED** (2026-07-15: narrower reasoning) | Same as MN-003; Withdraw button routing to the shared withdrawal screen confirmed via source only. |
| MN-008 | **BLOCKED** (2026-07-15: narrower reasoning) | Same as MN-003; add-key purpose selector structurally excludes OWNER/VOTING (never offered to any identity type) confirmed via source only. |
| MN-009 | **BLOCKED** (2026-07-15: narrower reasoning) | Same as MN-003, plus requires the Evonode variant specifically; Evonode-only "Claim token rewards" gating confirmed via source only. |
| MN-010 | **PASS** | Network switch with an unsubmitted, filled Load form (Evonode + fake ProTxHash + alias) returns to a clean empty List view with zero leftover input, and stale per-network banners are cleared — both live-confirmed; app restored to Testnet afterward. Not retested 2026-07-15 (unaffected by the env fix). |
| MN-011 | **BLOCKED** (2026-07-15: narrower reasoning) (core), no-op-safety sub-check passes | Core node-refresh behavior needs a loaded node — same fixture-availability constraint as MN-003, not a load-mechanism issue; Refresh button exists and is a confirmed-safe no-op with zero nodes loaded, matching the story's own no-op requirement. |
| MN-012 | **PASS** (directly-testable scope) | Header renders the 3-segment switcher with the exact `(no masternode yet)` placeholder text, corroborated by UX-003's independent prior finding; node-picking / cross-page-isolation behavior untested — no node to pick. Not retested 2026-07-15 (unaffected by the env fix). |
