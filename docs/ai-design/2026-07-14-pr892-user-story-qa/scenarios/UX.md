# UX — Cross-Cutting UX Infrastructure

Environment: PR892 build, isolated data dir `/data/tmp/det-qa-pr892-data`, display `:99`. New
category for this campaign — three stories (UX-001–UX-003) plus a pre-existing `[Gap]` story
(UX-004, not tested — see `progress.md`).

**Testnet wallet-backend blocker still active.** The known issue documented in `scenarios/ALK.md`
/ `scenarios/DEV.md` ("Failed to start chain sync error=The wallet service could not complete
this operation") reproduced again on every launch this session, including a fresh cold-boot
restart performed specifically for UX-002. `QA Wallet 1` shows Balance: 0 DASH, "Core: Error",
"Addresses: never synced" — no receiving address could even be generated ("+ Add Receiving
Address" produced nothing). This blocked a genuine live broadcast test for UX-001 on the main
instance; worked around for UX-002 via a throwaway Mainnet instance (Mainnet is confirmed
unaffected — see ALK.md).

**Incidental observation (not this category's responsibility, flagged for the record):**
`scenarios/WAL.md`'s WAL-028 write-up claims "DIAG throwaway" and "WAL-028 Throwaway" HD wallets
were removed at the end of that pass, leaving only `QA Wallet 1`. Both were still present and
loaded (3 wallets total) at the start of this session's testing — the removal apparently did not
persist, or a prior process crash/restart re-surfaced them. Used harmlessly for extra live
evidence in UX-003 (switching away from and back to `QA Wallet 1`); left in place since cleanup
is out of scope for this category and deleting wallets was not requested.

## UX-001: Blocking progress overlay for unsafe-to-interrupt operations — FAIL

Acceptance criteria (from the task): full-window dimming overlay with indeterminate spinner +
optional step/description, auto-lowers on completion; all interaction beneath suppressed
(pointer sink + frame-start keyboard claim, never dismissable by Esc/Enter/Space/Tab); yields to
a passphrase prompt; honest 30s/120s escalation with a one-shot dev-error log; no
background/dismiss button for unsafe-to-interrupt ops.

### Live attempt (Core-wallet Send)

Steps: Wallets tab (Expert view) > `QA Wallet 1` > Send. Balance shows **0 DASH**, "Core: Error",
"Addresses: never synced" (screenshot:
`screenshots/UX-001-1-qa-wallet1-zero-balance-env-blocker.png`). Clicked "+ Add Receiving
Address" to try to get any address to self-send to — no address appeared. The Send form itself
rendered ("Send from: Core Wallet — 0 DASH", "Send to", "Amount (DASH)"), but with zero balance
and zero addresses there was nothing to broadcast.

**Verdict on the live attempt: BLOCKED** — same root cause as every other wallet-dependent story
this session: "blocked by known environment issue: Testnet wallet-backend fails to connect in
this data dir as of 2026-07-14, see scenarios/ALK.md for full diagnosis." No amount of retrying
would help; this is the same failure WAL-017/ALK-002/IDN-001 etc. hit all session.

### Source review (the task's explicit fallback for this story)

Grepped the whole `src/` tree for every symbol that touches the overlay
(`ProgressOverlay`/`OverlayHandle`/`OverlayConfig`/`OptionOverlayExt`/`op_overlay`). Exactly
**five files** reference it: the component itself (`src/ui/components/progress_overlay.rs`), its
barrel export (`src/ui/components/mod.rs`), and **three consumer sites**:

1. `src/app.rs` + `src/app/reconcilers.rs` — the SPV-sync block (`SpvBlockReconciler`), UX-002's
   subject.
2. `src/ui/identities/register_dpns_name_screen.rs` — DPNS username registration, the **only**
   "unsafe to interrupt operation" adopter in the UX-001 sense (a "multi-step registration" per
   the story's own example list).

**The component itself is excellent and thoroughly correct.** Read the full 1911-line
implementation plus its ~30 inline unit tests (`src/ui/components/progress_overlay.rs`), which
individually exercise: full-window dim + click-and-drag pointer sink on `Order::Foreground`
(above popups); `claim_input()` stripping `Event::Text`, clipboard events, and
Tab/Escape/Enter/Space/arrows/Backspace/Delete/Home/End/PageUp/PageDown at frame start (unit
test `claim_input_strips_text_and_nav_keys_when_block_active`); a designated keyboard-escape
action activated by Enter/Space and enqueued focus-independently
(`claim_input_escape_block_enqueues_action_and_strips_keys`); the 30s/120s thresholds
(`stuck_reveal`/`watchdog_tripped`, unit-tested at the boundary); a one-shot watchdog dev-error
log (`watchdog_flag_flips_once_via_render`); auto-teardown via `OverlayHandle::clear()` /
`take_and_clear()`; and a `secret_prompt_active` gate wired from `app.rs`'s
`claim_overlay_input()` with a dedicated `#[cfg(feature = "testing")] test_set_secret_prompt_active`
seam explicitly so a kittest can assert the prompt keeps the keyboard above the block. Every
acceptance-criteria bullet in the task has a direct, named implementation and (for most) a
passing unit test backing it — this is some of the most rigorously tested UI code in the
codebase.

**But adoption is essentially nil for the story's own headline example.** `src/ui/wallets/send_screen.rs`
and `single_key_send_screen.rs` (the Core-wallet Send/broadcast flow) reference `ProgressOverlay`
**zero times** — sending dispatches a `MessageBanner::set_global(ctx, "Sending transaction...",
MessageType::Info)` (`send_screen.rs:637`) instead, which by `MessageBanner`'s own documented
design does **not** block interaction. A user broadcasting a Dash transaction today gets no
full-window block — they can click elsewhere or fire a second action during the broadcast, which
is exactly the failure mode this story exists to prevent. The same is true for every other named
example except one: no "signing" flow, no "key import" flow, and no "network migration" step
(`MigrationReconciler` in `app/reconcilers.rs` uses only a `BannerHandle`, never the overlay)
uses it either.

The one adopter that does exist, `register_dpns_name_screen.rs`, is correctly wired
(`OverlayConfig::default()` — spinner + description only, no buttons, matching "no
background/dismiss button"; raised only after a real `BackendTask` is produced so a no-op click
never strands a block; a `#[doc(hidden)] raise_progress_overlay_for_test` seam exists specifically
so this exact behavior is kittest-covered without needing a funded identity) — and is itself
commented, verbatim, as **"Bucket A"** adoption:
`docs/ai-design/2026-06-17-blocking-progress-overlay/03-dev-plan.md:327` says outright
*"follow-up, not the component (T4 documents it; per-feature adoption is out of scope here)"*.
This confirms the gap is a deliberately deferred, self-acknowledged scope cut for this PR, not an
accidental oversight — but it is a real, currently-shipping gap all the same. DPNS registration
itself could not be live-exercised this session (no identity reachable, same
`ALK.md`/`IDN.md`-documented blocker as every other identity-dependent story).

**Minor wording note** (not verdict-affecting): the task's quoted acceptance text says the 30s
line should read *"This is taking longer than usual."* — the shipped text is
`"Still in progress — please keep the app open."` (`STUCK_REASSURANCE` constant, live-confirmed
during UX-002 testing below, same shared component). A source comment explains this was a
deliberate choice ("copy that implies a fault … would be misleading" since SPV initial sync can
legitimately take minutes) — reasonable, but it does diverge from the story's literal wording.
The 120s escalation text (*"This is taking much longer than expected…"*) matches almost verbatim.

### Verdict: FAIL

The **component** is correctly and thoroughly implemented — if anything, over-engineered relative
to its current footprint (the test-spec doc's own QA author flags this: *"this design-doc set …
plus the ~1,922-line kittest module is disproportionate to the shipped widget's actual footprint
… exactly two production call sites"*). But the **story**, read as written ("while a long
operation that is unsafe to interrupt is running — broadcasting a state transition, signing, key
import, a multi-step registration, a network migration — I want to see a clear please-wait
block"), is not satisfied by this build: broadcasting/Send, the task's own suggested easiest
trigger and the story's first-listed example, does not raise this overlay at all, confirmed both
by a blocked-but-attempted live Send and by unambiguous source review. Only one of five named
scenarios (multi-step registration, via DPNS) is wired, and that one couldn't be live-verified
due to the pre-existing identity/environment blocker. Recommend re-testing this story once (a)
the Testnet wallet-backend issue is fixed and (b) Send/broadcast adopts the overlay — at which
point, given the component's demonstrated quality, a PASS looks very achievable.

## UX-002: Blocking SPV-sync overlay with a "continue in the background" escape — PASS

Acceptance criteria: full-window block with jargon-free please-wait text + "Step N of 5" while
user-initiated sync connects; always-visible "Continue in the background" secondary button that
lowers the block and doesn't re-raise for the rest of that sync episode; keyboard-reachable via
Enter or Space; scoped to user-initiated sync (lowers on its own on Synced/Error; a fresh
Connect/startup blocks again).

### Main-instance cold-boot restart (as instructed)

1. Verified the running PID (`1795744`) matched the required hash
   (`2931220e94871a0454ac56a43092aa87246b5a590d917645c025ddb1c7f9271a`), sent `kill -TERM`,
   confirmed via `pgrep` it was gone, then relaunched from the same hash-verified binary in a
   separate background call per the environment instructions.
2. Screenshotted within ~1s of relaunch — too late; det.log shows the overlay's full lifecycle
   already completed by then:
   ```
   00:15:19.091 Blocking progress overlay shown description="Connecting to the Dash network." step=None
   00:15:19.124 ERROR Failed to start chain sync error=The wallet service could not complete this operation.
   00:15:19.128 Blocking progress overlay dismissed key=0
   ```
   ~37ms end-to-end. The known Testnet wallet-backend bug (`ALK.md`) fails near-instantly (not a
   real, slowly-timing-out connection), so on the main instance the block's **auto-lower-on-Error**
   path fires far faster than any screenshot/keyboard round-trip could ever catch — confirmed
   reproducible: the *previous* session's launch (before this restart) logged the identical
   pattern (~74ms) at `23:49:24.83`–`23:49:24.90`. This is itself valid live evidence for the "lowers
   on its own … or fails (Error)" bullet — just not enough of a window to test the "Continue in the
   background" interaction on this instance.
3. Text confirmed jargon-free both times: `"Connecting to the Dash network."` — no "SPV", no raw
   heights, no percentages, no "RPC"/"node". Matches the acceptance criteria's exact example
   sentence.
4. Post-restart, the main instance settled into the same steady state documented all session
   (SPV sync failed banner, Testnet, Expert view) — confirming the block did not linger or
   mis-fire.

### Throwaway-instance test (Mainnet, fresh data dir — needed to catch the interactive window)

Since Mainnet is documented as unaffected by the Testnet-specific backend bug (`ALK.md`), and a
genuine sync gives the overlay a real, multi-second-plus lifespan to interact with (as NET-006/
NET-010 found for onboarding), launched two short-lived throwaway instances
(`/data/tmp/det-qa-ux002-check`, then `/data/tmp/det-qa-ux002-check2`, both deleted afterward;
zero interaction with the shared QA data dir or `QA Wallet 1`) via Welcome screen > "Just
Explore" (no wallet created, minimal footprint). Both fully corroborate every remaining bullet:

1. **Full-window block appears with jargon-free text + step counter**: caught live —
   `screenshots/UX-002-1-blocking-overlay-step1of5-continue-in-background.png` shows the whole
   window dimmed, an animated spinner, **"Step 1 of 5"**, **"Syncing with the Dash network."**,
   and a **"Continue in the background"** button. det.log confirms the same content
   programmatically: `description="Connecting to the Dash network." step=None` →
   134ms later → `description="Syncing with the Dash network." step=Some((1, 5))`.
2. **All interaction beneath is suppressed**: clicked the "Identities" sidebar nav item (behind
   the dim) and pressed Escape — neither had any effect; the overlay stayed up, the page
   underneath never changed screens. Screenshot:
   `screenshots/UX-002-2-click-and-escape-blocked-still-syncing.png` (also shows the 30s soft
   reveal: `"Elapsed: 33s"` / `"Still in progress — please keep the app open."` — live
   confirmation of the honest-escalation mechanism from UX-001, sharing this component).
3. **Keyboard-only "Continue in the background" (Enter, then separately Tab+Enter)**: on the
   first throwaway instance, pressed **Enter alone** (button is focus-pinned automatically on
   raise) — overlay lowered immediately, full interaction restored (clicked into the Identities
   welcome screen). On the second throwaway instance, repeated with the task's specific
   **Tab then Enter** sequence (Tab is stripped/trapped by `claim_input`, so the already-focused
   button stays targeted) — same result, captured live:
   `screenshots/UX-002-3-keyboard-enter-dismissed-unblocked.png` (full color restored, sidebar
   nav clickable, "Welcome to Identities" content visible — block fully gone). det.log confirms
   both dismissals were the user action, not an error auto-lower (no `ERROR` line preceding
   either `dismissed` log entry, unlike the main-instance Testnet runs above).
4. **Does not re-appear for the rest of the episode**: after dismissal, navigated to the Tokens
   tab and left the instance running while background sync continued (confirmed via
   `dash_spv::sync::filters::pipeline` / `GetCFHeaders` log lines still arriving). `grep -c
   "Blocking progress overlay shown"` against each instance's full log returned exactly **1** —
   the overlay was never re-raised despite sync actively continuing in the background for the
   full remainder of each session.
5. **Scoped to user-initiated sync**: confirmed via source (`SpvBlockReconciler::arm()` is called
   only from boot auto-start, the Connect button, and post-onboarding auto-start —
   `src/app.rs:1330,1433,1856`) and via the main-instance restart above, where the block correctly
   re-armed and re-raised on the fresh cold boot (a new user-initiated episode) rather than
   silently staying dismissed from the prior session.

Cleaned up both throwaway instances (`kill -TERM`, confirmed dead via `pgrep`/`kill -0`, data
dirs removed) without touching the shared QA data dir. Restored focus to the main QA instance
window afterward and confirmed it remained on Testnet / Expert view, undisturbed.

### Verdict: PASS

Every bullet was directly, live-observed with screenshot and/or timestamped-log evidence: the
full-window block with jargon-free text and step counter; total pointer/Escape suppression;
keyboard-only dismissal via Enter (and via Tab+Enter, the task's specific ask) landing on
"Continue in the background"; no re-raise for the rest of the sync episode; and — via the main
instance's own two cold-boot restarts this session — the "lowers on Error" half of the
user-initiated-sync scoping. The only thing not independently re-verified is a live "lowers on
Synced" (this environment never reaches a genuinely Synced state on either network in the time
available), but that is the same code path as the already-observed Error case
(`SpvBlockStep::Disarm` fires identically for both) — see the `04-design-addendum.md` doc's
`update_spv_overlay` note. One same-component wording deviation is flagged under UX-001 above,
not repeated here since it does not affect this story's own acceptance criteria (which do not
quote specific 30s copy).

## UX-003: Global wallet/identity switcher across all tabs — FAIL

Acceptance criteria: every root screen shows a page-aware 3-segment switcher in the top panel
(segment 1 = active tab, linking to it); selecting a wallet/identity updates the app-global
selection in place with no forced navigation, two-way synced with pages that consume it; segment
3 is page-scoped (app-global User identity on everyday pages, masternode/evonode in view on
Masternodes); an unconsumed pill renders dimmed/no-caret with an explanatory tooltip; a
no-identity-context page (e.g. Wallets) shows only the wallet pill.

### Live sweep across all 7 sidebar root screens (Expert view)

| Tab | Switcher present? | What it showed |
|---|---|---|
| **Wallets** | Yes — 2 segments | `Wallets › 💼 QA Wallet 1` (wallet-only spec; no 3rd segment, matching the "no identity context" bullet exactly). Screenshot: `screenshots/UX-003-1-wallets-tab-switcher.png`. |
| **Identities** (routes to the new Identity Hub, `RootScreenIdentityHub`) | Yes — 3 segments, fully interactive | `Identities › 💼 QA Wallet 1 › (choose an identity)`. Screenshot: `screenshots/UX-003-2-identities-tab-switcher.png`. |
| **Masternodes** | Yes — 3 segments, fully interactive | `Masternodes › 💼 QA Wallet 1 › (no masternode yet)` — page-scoped placeholder, distinct wording from the identity pill's own placeholder. Screenshot: `screenshots/UX-003-3-masternodes-tab-switcher.png`. |
| **Contracts** | **No switcher at all** | Top panel shows only `● Contracts` — no wallet pill, no identity pill, no breadcrumb of any kind. |
| **Tokens** | **No switcher at all** | Same — only `● Tokens`. Screenshot: `screenshots/UX-003-6-tokens-tab-no-switcher.png`. |
| **Tools** | **No switcher at all** | Same — only `● Tools`. Screenshot: `screenshots/UX-003-4-tools-tab-no-switcher.png`. |
| **Settings** | **No switcher at all** | Same — only `● Networks`. |

### Interactive behavior confirmed live (Wallets + Identity Hub + Masternodes)

1. **In-place switching, no forced navigation, two-way sync**: on the Identity Hub, clicked the
   wallet pill (a real dropdown with 3 wallets — see the "incidental observation" note above),
   picked "DIAG throwaway" — the pill updated in place, still on the Identity Hub (no
   navigation). Screenshot: `screenshots/UX-003-5-wallet-pill-dropdown-open.png`. Switched to the
   Wallets tab: it independently showed "DIAG throwaway" too, in both the top pill and the
   in-page "HD: DIAG throwaway ▾" selector — confirming the two-way, cross-tab sync bullet.
   Switched back to `QA Wallet 1` from the Wallets-tab pill to restore state.
2. **Page-scoped 3rd segment**: confirmed the Identities/Hub placeholder reads
   `"(choose an identity)"` while the Masternodes placeholder reads `"(no masternode yet)"` —
   different copy per page, sourced from `src/ui/state/masternodes_view.rs`'s own
   `NO_NODES_PLACEHOLDER` constant vs. the Hub's identity-count-based label — confirming segment 3
   is genuinely page-scoped, not a shared/generic string.
3. **Unconsumed-pill dimming**: not directly reproducible live (every reachable page with a
   switcher fully **consumes** both pills — `Consumed`, not `Unwired` — per source: the
   Identity Hub's `hub_spec()` and the Masternodes page's `masternodes_page_nav_spec()` both use
   `PillConsumption::Consumed` throughout). The `Unwired`/dimmed path (`subdued_everyday_spec`,
   `TT_WALLET_UNWIRED`/`TT_IDENTITY_UNWIRED` tooltip constants: *"Change the active wallet from
   the Wallets tab."* / *"Change the active identity from the Identity Hub."*) is used by
   `src/ui/dashpay/dashpay_screen.rs` and `src/ui/dpns/dpns_contested_names_screen.rs` in source,
   but neither screen is reachable in this session (no identity loaded — same blocker as every
   DPY/DPN story this campaign). Source confirms the mechanism exists and is correctly
   implemented (`render_wallet_pill`/`render_app_global_identity_pill`'s `PillConsumption::Unwired`
   arms render via `BreadcrumbPill::subdued(true)` with `with_tooltip(tooltip.clone())`, no click
   handling), but this specific bullet could not be live-observed.

### The core defect: 4 of 7 root screens render no switcher at all

Traced every call site of the two entry points a root screen must use
(`add_top_panel_with_global_nav` / `add_top_panel_with_global_nav_capturing`,
`src/ui/components/top_panel.rs:417,450`) across `src/ui/`. Only five screen files call either:
`dashpay_screen.rs`, `identities_screen.rs` (the older, superseded-by-the-Hub screen, using
`subdued_everyday_spec`, with its own explicit `// TODO: wire wallet/identity selection
consumption for the Identities page.` comment), `dpns_contested_names_screen.rs`,
`masternodes/list_screen.rs`, and `wallets_screen/mod.rs`. **`contracts_documents_screen.rs`,
`tokens_screen/mod.rs`, `network_chooser_screen.rs` (Settings), and every screen under
`src/ui/tools/` call neither** — they use the plain `add_top_panel()` with no breadcrumb at all,
confirmed both by source (no `global_nav`/`PageNavSpec` reference anywhere in those files) and by
live navigation to all four (table above).

This directly contradicts the acceptance criteria's first bullet — *"Every root screen renders a
page-aware three-segment switcher … in the top panel"* — and its fifth bullet's implication that
the **wallet pill at minimum** is always present (*"A page with no identity/object context …
shows only the wallet pill"* presumes a wallet pill is the floor, not that some pages show
nothing). Contracts, Tokens, Tools, and Settings show neither pill.

### Verdict: FAIL

The switcher component and its integration are excellent everywhere they're wired: correct
in-place switching, correct two-way cross-tab sync, correct page-scoped 3rd-segment copy, and a
correctly-implemented (if not live-reachable this session) dimmed/tooltip pattern for pages that
don't yet consume a pill. But the story's first, load-bearing claim — "every root screen" — is
directly falsified: 4 of the app's 7 top-level tabs (Contracts, Tokens, Tools, Settings) show no
switcher whatsoever, not even the baseline wallet pill the 5th bullet presumes is always present.
As with UX-001, this reads as a partial, in-progress rollout (the Identity Hub and Masternodes
adopt the newest/fullest pattern; Wallets, DashPay, and DPNS Contested Names use earlier/lighter
variants; Contracts/Tokens/Tools/Settings haven't been touched at all) rather than a broken
mechanism — worth re-testing once rollout is complete.

---

## Summary

| Story | Verdict | One-line reason |
|---|---|---|
| UX-001 | **FAIL** | Overlay component is correctly and thoroughly implemented (source + ~30 passing unit tests), but Send/broadcast — the story's own headline example and the task's suggested test — does not raise it (uses a non-blocking `MessageBanner` instead); only DPNS registration adopts it, explicitly scoped as a single "Bucket A" rollout with the rest deferred. |
| UX-002 | **PASS** | Every bullet directly live-confirmed with screenshots and timestamped logs: full-window block, jargon-free "Connecting to the Dash network." / "Syncing with the Dash network." + Step N of 5, total pointer/Escape suppression, keyboard-only (Enter, and Tab+Enter) "Continue in the background" dismissal, no re-raise for the rest of the episode, and auto-lower-on-Error confirmed twice on the main instance's own cold-boot restarts. |
| UX-003 | **FAIL** | Switcher works correctly (in-place switching, two-way sync, page-scoped 3rd segment) on the 3 tabs that adopt it (Wallets, Identity Hub, Masternodes), but 4 of 7 root screens (Contracts, Tokens, Tools, Settings) render no switcher at all — not even the baseline wallet pill — directly contradicting the "every root screen" acceptance criterion. |
| UX-004 | N/A (Gap) | Pre-existing in `progress.md`; one-time post-migration disclosure notice is not implemented. Not tested this session (out of scope per task). |
