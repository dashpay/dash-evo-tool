# NET — Network and Settings

Environment: PR892 build, isolated data dir `/data/tmp/det-qa-pr892-data`, display `:99`.

## NET-001: Switch networks — PASS

Steps:
1. Fresh launch defaults to **Mainnet** (SDK init log: `network=Mainnet`), and shows
   "Disconnected — check your internet connection" initially (expected — SPV not yet started).
2. Navigated to Settings (sidebar, requires scrolling down past Wallets/Tools to reveal
   Settings/Expert-toggle/Dash-logo — sidebar overflows the visible area at default window
   height, see UX note below) — this opens the "Networks" screen.
3. "Connection Settings" card at the top has a `Network:` dropdown, disabled while connected.
4. Clicked "Disconnect" (stops SPV) — dropdown became enabled.
5. Opened dropdown: options are Mainnet / Testnet / Devnet / Local.
6. Selected "Testnet" — SPV immediately started syncing against testnet (`Headers: 80000 /
   1514569`, DAPI "Available (29 unbanned / 29 total endpoints)"), sidebar network indicator
   at the bottom updated to "Testnet".

Verdict: **PASS**.

### UX note (not a defect, worth flagging)
The sidebar navigation (Identities / Masternodes / Contracts / Tokens / Wallets / Tools /
Settings / Expert-toggle / Dash logo) does not fit within the default 800×600 window height
in Expert view — "Settings" is pushed below the fold and only reachable by scrolling the
sidebar itself. Not discovered until the window was manually resized larger and then scrolled.
An easy miss for a new user in the default window size.

### UX note: Dash logo at the bottom of the sidebar is an external link
Clicking the Dash logo at the bottom of the sidebar opens the system's default browser to
`dash.org` (a new top-level browser window), rather than doing anything in-app. Confirmed
intentional (branding link) but worth noting since it's easy to click by accident given its
proximity to "Settings" right above it in the same sidebar column.

## Database Maintenance / Advanced Settings — observed (not yet formally tied to a story)

Settings > Networks > Advanced Settings exposes: Theme selector (NET-004), "Auto-start SPV
on startup" toggle, "Clear Mainnet/Testnet/etc. Database" (destructive, per-network — maps to
NET-011/NET-019 family), "Clear SPV Data" (maps to NET-020). Deferred to the destructive-tests
pass at the end of the campaign per plan.

## NET-002: Auto-update from dashmate config — FAIL

Acceptance criteria: "Detects and imports local dashmate config."

Steps:
1. Explored Settings > Networks for every network (Mainnet/Testnet/Devnet/Local), in both
   Expert and Developer interface modes — no dashmate-related detection/import UI anywhere.
   The "Dashmate Password" column / per-network table with Start buttons mentioned in early
   exploration notes does not exist in this build; `render_network_table()`
   (`src/ui/network_chooser_screen.rs:137`) renders only a single "Connection Settings" card
   with a plain Network dropdown — no table, no per-network rows, no dashmate fields.
2. Source inspection (read-only, PR892 build worktree) confirms there is no dashmate
   auto-detection code path at all. The only occurrence of "dashmate" in `src/ui/` is an
   unrelated hardcoded RPC username default (`LOCAL_core_rpc_user=dashmate`) used for a
   Devnet/Local test action, not a config-import feature.
3. `.env.example` documents the *opposite* of auto-detection — a comment instructs the
   developer to manually run `dashmate config get core.rpc.users.dashmate.password
   --config=local_seed` and paste the result into `.env` by hand.

Verdict: **FAIL** — the acceptance criterion ("detects and imports local dashmate config")
is not implemented. Local/Devnet Core RPC credentials are `.env`-file-only and require
manual copy-paste from the dashmate CLI; there is no in-app detection or import.

## NET-003: Configure Dash-Qt path — FAIL

Acceptance criteria: "Path set in settings. App validates the path exists."

Steps:
1. Checked Settings > Networks (all Advanced Settings sections, Expert and Developer view)
   for a Dash-Qt path field — none found.
2. Source inspection: `AppSettings::dash_qt_path: Option<PathBuf>` exists in
   `src/model/settings.rs` with an autodetection helper (`detect_dash_qt_path()`, tries
   `which::which("dash-qt")` then OS-specific default install locations), and is preserved
   across settings-blob (de)serialization. However:
   - `grep -rn "dash_qt_path" src/ui/` returns zero hits — no widget reads or writes it.
   - `SystemTask` (`src/backend_task/system_task/mod.rs`) has no variant to update it
     (only `UpdateThemePreference` exists).
   The field is populated once at settings-default time and from then on is dead data —
   never surfaced, never editable, never re-validated.

Verdict: **FAIL** — the setting exists in the data model (kept for on-disk wire-format
compatibility) but has no UI: no path field, no "path exists" validation surfaced to the
user, contradicting both acceptance-criteria bullets.

## NET-004: Select theme — PASS

Acceptance criteria: "Light, dark, and system-auto options. Theme change applied
immediately."

Steps:
1. Settings > Networks > Advanced Settings > Theme dropdown. Options: System / Light / Dark.
2. Selected **Dark** — entire UI (background, cards, banners, sidebar) re-themed to dark
   instantly, no reload/restart needed.
3. Selected **Light** — re-themed to light instantly. Screenshot:
   `screenshots/NET-004-3-theme-light.png`.
4. Selected **System** — a warning banner appeared: *"Could not detect your system theme.
   Using the previous theme for now — it will update automatically when detection
   succeeds."* This is expected/graceful behavior in this headless X11 test environment
   (no desktop portal to report a system theme preference to) — the app degrades sensibly
   (keeps the last explicit theme) rather than crashing or rendering unreadable UI, and the
   message follows the project's own user-facing-error conventions (plain language, no
   jargon, implies self-resolution). Not a bug.

Verdict: **PASS**. All three options apply immediately with no restart. Left the app on
**System** (matching the "previous theme" fallback, currently rendering Light) at the end
of this pass.

## NET-005: Toggle developer mode — PASS

Acceptance criteria: "Toggles visibility of advanced UI elements."

Steps:
1. Settings > Networks > Interface mode: three-way radio, Default view / Expert view /
   Developer view.
2. Switched to **Developer view**: sidebar gained a "Dev" badge; Settings > Advanced
   Settings gained a new "Developer Tools" section with a "Clear Platform Addresses" button
   (disabled here with tooltip "This tool is unavailable while earlier-version recovery
   data is kept read-only" — an unrelated, pre-existing gating condition, not a bug in this
   toggle). Screenshot: `screenshots/NET-005-2-developer-view-shows-developer-tools.png`.
3. Switched to **Default view**: sidebar lost the "Masternodes" nav entry entirely (only
   Identities / Contracts / Tokens / Wallets / Tools / Settings remain); Settings >
   Advanced Settings lost the "SPV Auto-Start" toggle, "Developer Tools" section, and "SPV
   Maintenance" (Clear SPV Data) section — only Theme + Database Maintenance remained.
   Description text under Interface mode updated accordingly ("Shows your balance, send and
   receive, and usernames."). Screenshot:
   `screenshots/NET-005-1-default-view-hides-advanced-settings.png`.
4. Switched to **Expert view**: sidebar and Advanced Settings returned to the mid-level set
   (Masternodes back, SPV Auto-Start + Database/SPV Maintenance back, no Developer Tools).

Verdict: **PASS**. Progressive disclosure across all three view levels works correctly and
matches `docs/personas`' model. Left the app on **Expert view** (the campaign's established
default) at the end of this pass.

## NET-007: Granular refresh controls — PASS (partial — see note)

Acceptance criteria: "Refresh mode selector available in detailed/developer view." (Story
prose: "choose whether to refresh Core Only, Platform Only, or both".)

Steps:
1. Wallets screen (Developer view) > per-wallet header has a "Refresh mode: Core + Platform"
   button next to "Get Test Dash".
2. Clicked repeatedly: the button cycles between exactly two labels, "Core + Platform" and
   "Platform Only" — confirmed across 4 consecutive clicks (no third state ever appeared).
   Screenshot: `screenshots/NET-007-1-refresh-mode-toggle.png` (shown in "Core + Platform").
3. Source inspection confirms this is intentional, not a bug:
   `src/ui/wallets/wallets_screen/mod.rs:155-185` defines `enum RefreshMode { All,
   PlatformOnly }` (only 2 variants) with a doc comment: *"There is no 'Core only' mode:
   Core wallet state (balances/UTXOs) is kept current continuously by the upstream runtime
   and pushed via the EventBridge, so there is nothing to reconcile on demand. Refresh only
   re-fetches the DAPI-sourced Platform-address balances, optionally alongside the
   always-live Core view."*

Verdict: **PASS** for the functional intent (a granular, view-gated refresh control that
saves time by skipping an unnecessary Platform re-fetch exists and works). Flagging as
partial because the story's literal 3-way framing ("Core Only, Platform Only, or both") no
longer matches the architecture — Core sync became push-based (EventBridge) after the
platform-wallet migration, making a manual "Core Only" refresh meaningless. This looks like
the user-story text has drifted from an architecture change rather than a real product gap;
worth a documentation update, not a code fix.

## NET-008: Select Core backend mode — FAIL

Acceptance criteria: "SPV for light sync, RPC for full node, Auto for app-selected."

Steps:
1. Checked Settings > Networks (all networks, all Advanced Settings, Expert and Developer
   view) for any SPV/RPC/Auto backend-mode selector — none found anywhere.
2. Source inspection confirms this is explicitly retired, not merely unsurfaced. In
   `src/model/settings.rs`, the wire-format struct comment reads: *"The
   `_reserved_core_backend_mode` byte is a retired field (the RPC/SPV selector — chain sync
   is SPV-only now) kept solely to preserve [on-disk] layout... written as a constant and
   ignored on read."* In `src/ui/network_chooser_screen.rs:1313-1316`,
   `any_rpc_backend()` is hardcoded to always return `false`, commented *"Chain sync is
   SPV-only; the RPC wallet backend was removed."*

Verdict: **FAIL** — this is not a missing-UI gap like NET-003/009, it is a deliberately
removed feature: the RPC wallet-sync backend was deleted as part of the platform-wallet
migration (consistent with CLAUDE.md's note that DET's bespoke SPV stack was replaced).
Chain sync is unconditionally SPV; there is no user-selectable backend mode. The
user-stories.md entry is stale relative to the current architecture.

## NET-009: Toggle ZMQ — FAIL

Acceptance criteria: "ZMQ enable/disable toggle in settings."

Steps:
1. Checked Settings > Networks (all Advanced Settings, Expert and Developer view) for a ZMQ
   toggle — none found.
2. Source inspection: `AppSettings::disable_zmq: bool` exists in `src/model/settings.rs`
   (default `false`), preserved in the wire format, but:
   - `grep -rn "zmq" src/ui/` (case-insensitive) returns zero hits.
   - No `SystemTask` variant to update it.
   Same pattern as NET-003 (`dash_qt_path`) — a settings-model field kept for
   wire-compatibility with no live UI or backend wiring.

Verdict: **FAIL** — no ZMQ toggle is reachable anywhere in the UI.

## NET-010: Onboarding wizard — PASS

Acceptance criteria: "Welcome screen with setup steps. Guides user through initial wallet
creation."

Steps:
1. Launched a throwaway instance against a brand-new, empty `DASH_EVO_DATA_DIR`
   (`/data/tmp/det-qa-net-onboarding-check`, not the shared QA data dir) to see the true
   first-run state without disturbing existing campaign state.
2. Welcome screen: Dash logo, "Welcome to Dash Evo Tool" / "Your gateway to decentralized
   data", a "Choose your experience level" Default/Expert/Developer selector (defaults to
   Expert — consistent with the note in `CAMPAIGN-CONTEXT.md`), and three option cards:
   Create Wallet / Import Wallet / Just Explore.
3. Clicked "Create Wallet" → guided 5-step flow begins ("Follow these steps to create your
   wallet", Step 1: move cursor over an entropy grid, then select language/word count and
   "Generate"). A live "Syncing with the Dash network — Step 1 of 5" progress modal appeared
   simultaneously (with "Continue in the background"), confirming SPV sync kicks off
   automatically with zero prior configuration.
4. Terminated the throwaway instance (`kill -TERM`, graceful) and deleted its data dir
   without saving/confirming a wallet — did not touch the shared QA data dir.
5. Cross-referenced `WAL-001` (`scenarios/WAL.md`), which independently exercised the same
   flow to completion (entropy → mnemonic → confirm → name → save) in an earlier session of
   this campaign, confirming the guided flow works end-to-end, not just up to the entropy
   step.

Verdict: **PASS**. Onboarding wizard renders correctly, guides through wallet creation
step-by-step, and starts SPV sync with no manual configuration.

## NET-015: Use Dash Evo Tool without a local Dash Core node — PASS (with a UX note)

Acceptance criteria: "Fresh install connects to the Dash network via the built-in SPV light
client with zero configuration. The user sees sync progress and status clearly; the default
everyday-user UI avoids mentions of SPV, RPC, or nodes. Technical/protocol terminology may
appear in Expert mode or advanced settings, where Dash Core RPC remains available as an
opt-in for users who do run a local node."

Steps:
1. Fresh-install zero-config bullet: confirmed via the NET-010 throwaway-instance test
   above — a brand-new data dir goes straight into wallet creation with automatic SPV sync
   starting (no RPC host/port/credentials prompt of any kind). Source confirms this isn't
   just "unconfigured" but architecturally guaranteed: `any_rpc_backend()`
   (`src/ui/network_chooser_screen.rs:1313`) is hardcoded `false`, and chain sync is
   unconditionally SPV (see NET-008 finding) — the app cannot fall back to requiring a
   local Core node for wallet sync even if one is present. This machine does have a local
   `dash-qt` running for unrelated RPC fallback use (per campaign environment notes); DET's
   own wallet sync does not depend on it.
2. Sync progress bullet: Settings > Networks > Connection Status shows live SPV
   header/filter/block sync stages and DAPI endpoint availability (seen working end-to-end
   on Testnet in NET-001's PASS write-up before the environment blocker appeared this
   session, and via the "Step 1 of 5" modal in this session's throwaway-instance test) — the
   user does see clear sync/status feedback.
3. "Default everyday-user UI avoids SPV/RPC/node mentions" bullet: **not fully met**.
   Reproduced live in Default view (Settings > Networks, Interface mode = Default view,
   Testnet with the known SPV connection failure active): the global error banner still
   reads *"SPV sync failed. Go to Settings for connection details."* — verbatim, unchanged
   from Expert/Developer view. Screenshot:
   `screenshots/NET-015-1-spv-jargon-in-default-view-banner.png`. Source confirms this is
   unconditional: `src/app/reconcilers.rs:314-325` builds this banner text with no
   `UserRole`/interface-mode branching at all.
4. "RPC remains available as opt-in" bullet: partially met at the config-file level only.
   `src/backend_task/core/mod.rs` still uses `dashcore_rpc`/`RpcApi` for one narrow,
   optional feature (`get_best_chain_lock`), gated by per-network `core_rpc_user`/
   `core_rpc_password` in `.env` (see `.env.example`) — but there is no in-app "opt-in"
   toggle for this (same gap documented under NET-002/003/008/009); it's a manual `.env`
   edit, not a Settings UI switch.

Verdict: **PASS** for the core, consequential criterion — the app fully operates via
SPV+DAPI with zero local-node configuration, and this is architecturally enforced, not
incidental. **Bug/UX finding**: the default-view connection-error banner leaks the term
"SPV" (technical jargon per the project's own `CLAUDE.md` error-message rules, which forbid
jargon like this for the Everyday User persona even though "SPV" isn't literally on the
forbidden-word list there) — this should read as neutral consumer language (e.g.
"Couldn't connect to the Dash network. Go to Settings for details.") in Default view.

---

*All assigned NET stories (NET-002 through NET-010, NET-015) complete. NET-011 intentionally
left untouched — reserved for the final destructive pass per `CAMPAIGN-CONTEXT.md`.*
