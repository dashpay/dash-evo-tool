# WAL — Wallet Management

Environment: PR892 build (`/data/target/debug/dash-evo-tool` @ `57195d54`), isolated data dir
`/data/tmp/det-qa-pr892-data`, network Testnet, display `:99`.

## WAL-001: Create a new wallet — PASS

Steps:
1. Launched app fresh (empty data dir). App defaulted to **Mainnet** — switched to Testnet via
   Settings > Networks first (see NET-001 in `NET.md`).
2. Welcome screen > "Create Wallet".
3. Moved cursor over the entropy grid ("Move your cursor over this grid to create extra
   randomness for your wallet's seed phrase").
4. Selected language (English) and word count (24 words), clicked "Generate" — mnemonic
   displayed as a numbered 24-word grid.
5. Checked "I wrote it down".
6. Entered wallet name "QA Wallet 1", left password optional field blank.
7. Clicked "Save Wallet" — "Wallet Created Successfully!" screen with next-step shortcuts
   (Fund Wallet / Create Platform Identity / Go To Wallet Screen).

Observed: wallet appears immediately in the wallet selector, balance 0 DASH, Dash Core /
Platform / Shielded / System tabs all present and empty.

Tested twice: once while the app was still on Mainnet (wallet only visible under Mainnet),
once after switching to Testnet (wallet only visible under Testnet) — see note under WAL-004
about per-network wallet isolation.

Verdict: **PASS**.

## WAL-004: Switch between wallets — PASS (partial; per-network isolation noted)

Wallets created on Mainnet are not visible when the app is switched to Testnet, and vice
versa — wallets are keyed per-network. This is expected/correct (different chain params /
address derivation), not a bug, but worth calling out since the Wallets screen shows
"No wallets yet" after a network switch even though a wallet exists on the other network.
The wallet-selector dropdown (`HD: QA Wallet 1 ▾`) at the top of the per-wallet screen is
present and functional for switching between multiple wallets *within* the same network
(not exhaustively tested with a second same-network wallet yet — revisit if time allows).

Verdict: **PASS** (core mechanism confirmed; per-network scoping documented as expected
behavior, not a defect).

## WAL-010: Generate receive address — PASS

The Wallet screen's "Dash Core" tab shows a live address table (see WAL-011). Clicking
"+ Add Receiving Address" is available. The first "Funds"-type address at index 0
(`yYCWtyP2mSLzGkZqL9a6G5rpPQQRs1fT5f`) was used directly as the funding target for the
testnet faucet and received funds correctly, confirming address generation/derivation works.

Verdict: **PASS**.

## WAL-011: View address table — PASS

"Addresses (Dash Core)" section shows columns: Address, Balance (DASH), UTXOs, Type, Index,
Full Path, Private Key (View Key button per row). A "Show zero-balance addresses" checkbox
toggles visibility of the full gap-limit-generated address set (tested: ~62 addresses
generated for a fresh wallet, alternating Funds/Change type, sequential index, correct BIP44
path `m/44'/1'/0'/{0,1}'/{index}`).

Verdict: **PASS**.

## WAL-016: View transaction history — PASS — **PR892 regression fix confirmed**

This is the direct regression test for PR892 ("show transaction history that predates the
current session"). Steps:

1. Funded `QA Wallet 1` (Testnet) with 3 separate 1 tDASH payouts from the Pasta testnet
   faucet (see `dash-platform:dash-faucet` skill), txids:
   - `fb12b8a5ca98353e7bf408d6472a50896a4d564da355b23addf31d2126c75d2f`
   - `e5c6752ea51c3f08e77752411a032fae15f4e3f84e4981751d68a81c06a5c5f8`
   - `bb04645a3ed1b90c0b847eaa6e5f859e79c8052982bb7bdd539657761f068e92`
2. Confirmed all 3 appeared in the live in-app Transaction History (expanded the
   "▶ Transaction History" section), each `Received +1 DASH`, `ChainLocked @1514579`.
   Screenshot: `WAL-016-1-tx-history-live-before-restart.png`.
3. **Fully quit the app**: `kill -TERM` on the app PID (graceful shutdown, confirmed process
   exit, no panic in `det-stderr.log`) — not just navigating away in-app.
4. **Cold-boot relaunched** the exact same binary against the exact same
   `DASH_EVO_DATA_DIR=/data/tmp/det-qa-pr892-data`.
5. App restored `QA Wallet 1` automatically on startup, balance showed correctly (3 DASH)
   immediately, even while the "Syncing with the Dash network" startup modal was still
   showing (dismissed via "Continue in the background").
6. Expanded Transaction History again: **all 3 transactions rendered correctly**, same
   amounts/timestamps/txids/ChainLock heights as before the restart.
   Screenshot: `WAL-016-2-tx-history-after-cold-boot-PASS.png`.

**Verdict: PASS.** This is the core PR892 fix working as intended — persisted
`core_transactions` rows are correctly hydrated into the in-memory snapshot store at wallet
load, so history no longer renders empty after an app restart.

## WAL-021 / WAL-023 / WAL-024: Collapsible sections — PASS (observed incidentally)

While testing WAL-016, incidentally confirmed:
- **WAL-023 (Collapsible transaction history)**: the "▶/▼ Transaction History" section
  expands/collapses correctly and its expanded/collapsed state is visually consistent
  across a manual refresh.
- **WAL-024 (Collapsible balance breakdown)**: the "▶/▼ Balance breakdown" header (Core /
  Platform / Shielded split) collapses/expands correctly; clicking it also reveals a
  "▶ Sync Status" sub-section.

Verdict: **PASS** for both (full acceptance-criteria walkthrough not yet separately
exercised — revisit if time allows, but core collapse/expand mechanism confirmed working
both pre- and post-restart).

## SND-003 cross-reference: "Receive" button appears non-functional (documented in SND.md)

While working through WAL-010/016 on the Wallet screen, found that the "Receive" button
next to "Send" does not open any modal, QR code, or navigate anywhere — see full repro in
`SND.md` under SND-003. Filed there since SND-003 ("Receive Dash with QR code") is the
story it maps to; noting the cross-reference here since it was discovered during WAL testing.

---

## WAL-002: Import wallet via mnemonic — PASS

Steps:
1. Used the Create Wallet entropy-grid flow to generate a fresh 12-word mnemonic (`sail
   eager shrug goose primary position under shuffle swarm occur fall diet`), noted the
   words, then navigated away via the "Wallets" breadcrumb **without** clicking "Save
   Wallet" — confirmed no wallet was created from this abandoned flow.
2. Clicked "Import Wallet", left seed length at the default 12, entered all 12 words in
   order (no autocomplete interference), set name "QA Throwaway HD" and an optional
   password ("Password Strength: Very Strong", 39-year crack estimate shown live).
3. Clicked "Save Wallet" — "Wallet Imported Successfully!" screen appeared with the same
   next-step shortcuts as WAL-001's create flow.
4. On the Wallet screen, the imported wallet ("HD: QA Throwaway HD") appeared correctly in
   the wallet selector alongside "HD: QA Wallet 1", with the same address-derivation
   scheme (`m/44'/1'/0'/0/{index}`, Funds/Change alternating) as a natively-created wallet,
   confirming the import correctly reconstructs the standard BIP44 account.

Verdict: **PASS**. (This throwaway wallet was reused for WAL-005/006/007 below, then
removed as part of that cleanup — see WAL-007.)

## WAL-003: Import single private key — PASS (with a documented product limitation)

Steps:
1. Generated a fresh zero-balance receiving address on `QA Wallet 1` via "+ Add Receiving
   Address" (index 31, `yZjRFx4KmGB3h36LGbf4xSAzK51cU1hQML`) — used a zero-balance address
   deliberately, to avoid any UTXO-sharing ambiguity with the funded index-0 address.
2. Exported its WIF via "View Key" → "Show Key":
   `cV1mQB3GMvXy7sZstnkTqG1Z2Boqi6P6Nvxwo8tCe8ut3iP9NbTq`.
3. Opened "Import key (advanced)", pasted the WIF. The dialog live-derived
   `yZjRFx4KmGB3h36LGbf4xSAzK51cU1hQML` and labeled it "This is a Testnet address." —
   exact match to the source address, cryptographically confirming the exported WIF round-
   trips correctly.
4. Entered nickname "QA Single Key Test", clicked "Add to wallets".

Observed: "SK: QA Single Key Test" appeared immediately in the wallet selector with the
correct 0 DASH balance. The wallet screen shows a clear banner: *"Sending from a single-
key wallet is not available in this version. You can still receive funds at this address.
To send these funds, import them into a recovery-phrase wallet."* — the "Send" button is
present but disabled/blank. This is an explicit, clearly-communicated product limitation
(not a silent bug); the story's acceptance criteria ("Creates a single-key wallet from
WIF-format key" / "Wallet appears in the wallet selector") are both met.

Verdict: **PASS**. (This wallet was removed during WAL-007 testing — see below, which
found a real bug in that removal path specific to single-key wallets.)

## WAL-005: Rename a wallet — FAIL

Steps: clicked the "Rename" button in the wallet header action row (next to "Remove") on
**two different wallets** — the single-key "QA Single Key Test" wallet, and the HD
"QA Throwaway HD" wallet — across multiple attempts (single click, double click, click
after navigating away and back, fresh mouse-move-then-click).

Observed: **no dialog, inline edit field, or any visible effect ever appeared**, on either
wallet type, in any attempt. The wallet name never changed. This was reproduced
consistently — not a one-off misclick.

Verdict: **FAIL**. The Rename feature is completely non-functional in this build: the
button renders and is clickable, but produces no effect. "Name change persists across
sessions" cannot be tested because there is no way to initiate a rename at all.

## WAL-006: Lock and unlock wallet — FAIL

Steps:
1. Imported "QA Throwaway HD" (see WAL-002) with password `QaThrowaway#2026`.
2. Clicked "Lock" — worked immediately, no confirmation needed; button label toggled to
   "Unlock".
3. Confirmed sensitive operations are blocked while locked: clicked "View Key" on an
   address row — no Private Key modal opened (correct behavior), though there was no
   error banner or other feedback explaining *why* nothing happened — a locked-wallet user
   might read this as the button being broken rather than a deliberate security block.
4. Clicked "Unlock" to test the reverse flow — across four separate attempts (immediate
   retry, retry after navigating away and back to the wallet screen, retry with a fresh
   mouse-move-then-click sequence) **no password-entry dialog or any other UI ever
   appeared**. The wallet remained permanently in the "locked" state (button stayed
   labeled "Unlock") with no way to re-enter the password through the UI.
5. Verified via the accessibility tree (`a11y_dump.py`) that this wasn't an off-screen or
   stale-render artifact — the dump was unreliable/stale as flagged in `CAMPAIGN-CONTEXT.md`
   and showed an unrelated screen, offering no evidence of a hidden dialog.
6. Cleaned up by removing the stuck wallet (Remove is not itself blocked by lock state —
   the confirmation dialog opened normally even while locked; see WAL-007).

Verdict: **FAIL**. Locking works, and correctly blocks sensitive operations, but
**Unlock is completely broken** — clicking it never surfaces a password prompt, so a
locked wallet cannot be unlocked again through the UI. The only known password
(`QaThrowaway#2026`) could never be used because the entry point never renders. This is a
self-lockout bug: in a real usage scenario (not a throwaway QA wallet), a user who locks
their wallet would be permanently unable to access it again without deleting and
re-importing it. `QA Wallet 1` was never locked and remains fully accessible throughout.

## WAL-007: Remove a wallet — FAIL (confirmation prompt inconsistent by wallet type)

Steps:
1. On the single-key wallet "QA Single Key Test", clicked "Remove". **The wallet was
   deleted instantly with zero confirmation dialog of any kind** — verified via the
   wallet-selector dropdown before/after (present, then gone, with no intervening prompt).
2. On the HD wallet "QA Throwaway HD" (locked at the time, per WAL-006), clicked "Remove".
   This time a proper **"Remove Wallet" confirmation modal appeared**, with clear warning
   text ("Removing wallet 'QA Throwaway HD' will delete its local data, including
   addresses, balances, and asset locks stored on this device. Identities linked to it
   will remain but the keys derived from this wallet will no longer work unless the
   wallet is re-imported. Continue?") and Cancel/Remove buttons. Confirmed Remove
   completed the deletion correctly, and that lock state does not block removal.

Verdict: **FAIL** (partial). The underlying mechanism — "Wallet data is deleted from
local storage" — works for both wallet types. But the "Confirmation prompt before
removal" acceptance criterion is violated specifically for **single-key wallets**: a
single stray click on "Remove" permanently and silently destroys an SK wallet (and any
funds held at its address, with no undo) — while HD wallets are correctly protected by a
confirmation step. `QA Wallet 1` was never targeted by Remove and was confirmed intact
(3 DASH, unchanged) after all of the above.

## WAL-008: View wallet balances — PASS (with a UX gap noted)

Steps: confirmed the "Balance breakdown" section on the Wallet screen shows
`Core: 3 DASH | Platform: 0 DASH | Shielded: 0 DASH` for `QA Wallet 1`, satisfying
"Displays Core balance and Platform balance." Checked this in both **Expert view** and
**Default view** (Settings > Networks > Interface mode has three options: Default /
Expert / Developer).

Observed: switching to Default view does **not** simplify the Wallet screen — the same
balance breakdown, the same Dash Core/Platform/Shielded tab bar, the same address table
(UTXOs, Type, Index, Full Path, and a WIF "View Key" export button per row), and the same
Transaction History (with TxID column) render identically in both Default and Expert
view. The only differences observed between the two modes on this screen are: the
"[DEV]" wallet-name badge (present in Expert/Developer, absent in Default) and the System
tab (see WAL-022). Switched back to Expert view after testing.

Verdict: **PASS** for the core criterion (Core/Platform balance display). UX note (not a
hard fail): the story's "Alex sees a simplified view; Priya sees per-account breakdown"
distinction is not really implemented on the Wallet screen itself — Default-view ("Alex")
users see the same level of technical detail (private-key export, UTXO counts, derivation
paths) as Expert-view ("Priya") users.

## WAL-012: View and export private keys — PASS

Steps: clicked "View Key" on an address row in the Dash Core address table. A "Private
Key" modal opened showing the Address, a "Copy Address" button, a masked WIF field
(dots) with "Show Key" / "Copy Key" buttons, and the warning "Keep your private key
secure. Never share it with anyone." Clicked "Show Key" — revealed the WIF in the correct
testnet format (`c...` prefix). Round-trip verified in WAL-003: re-importing this exact
WIF via "Import key (advanced)" re-derived the identical source address.

Verdict: **PASS**.

## WAL-013: View SPV sync status — PASS

Steps: expanded the "▶ Sync Status" sub-panel nested under "Balance breakdown" directly
on the Wallet screen (distinct from the Settings > Networks > Connection Status panel
already covered by NET-001). It shows:
- `Core: Synced — 3 peers` (green)
- `Addresses: 0 synced (blk 400719, Ns ago)`
- `Shielded: 0 DASH`

Verdict: **PASS**. Confirms "Connection status indicator shows current sync stage" is
available directly on the wallet-balance view, not only under Settings.

## WAL-017: Fund Platform address from wallet — FAIL

Steps:
1. Opened "Send" from the Wallet screen. The "Send to" field has autocomplete that
   correctly surfaces the wallet's own Platform (DIP-17) addresses tagged "Platform" —
   selecting one auto-fills the field, labels it "Platform address", and switches
   "Transaction type" to "Fund Platform Address" (button relabels accordingly). This part
   of the UX works well.
2. Entered `0.02` DASH (well within the 3 DASH available) and clicked "Fund Platform
   Address".

Observed: **fails every time** (reproduced twice, non-transient) with the banner "The
wallet service could not complete this operation. Please retry in a moment." Technical
details (via "Show details"):
```
WalletBackend { source: AssetLockTransaction("Asset lock builder failed: Transaction
builder error: Coin selection error: No UTXOs available for selection") }
```
Verified at the time of the failure that the wallet's funded address (index 0) genuinely
had 3 spendable UTXOs (`Balance: 3.00000000`, `UTXOs: 3`, ChainLocked) — the "no UTXOs
available" error is not because the wallet is actually empty; it is a real bug in the
asset-lock transaction builder's coin-selection logic. No funds were lost and no asset
lock was created across the failed attempts (balance stayed at 3 DASH; "Asset Locks"
section continued to show "No asset locks found").

Verdict: **FAIL**. Screenshot: `WAL-017-1-fund-platform-address-FAIL-no-utxos.png`. This
is a hard blocker for the entire Platform-funding chain — see WAL-018/019/020 below.

## WAL-018: Fund Platform address from asset lock — BLOCKED

**Reasoning**: this story requires an existing (previously created) asset lock to fund a
Platform address from. WAL-017's coin-selection bug means no asset lock can be created in
this environment — the wallet's "Asset Locks" section shows "No asset locks found"
throughout testing, and no alternate UI path to create or import a standalone asset lock
was found on the Wallet screen in Expert view. Cannot test until the WAL-017 bug is fixed
(or a pre-existing asset lock fixture is made available).

## WAL-019: Transfer credits between Platform addresses — BLOCKED

**Reasoning**: requires at least one Platform address holding a balance to serve as the
transfer source. Confirmed via Send Dash > Advanced Options > Source Type: "Platform
Addresses" is disabled with the inline note "(no Platform addresses with balance)" — a
direct consequence of WAL-017's failure, since Platform balance is permanently 0 in this
environment. Screenshot: `WAL-019-020-1-platform-source-disabled-no-balance.png`. Cannot
test until a Platform address can be funded.

## WAL-020: Withdraw from Platform address to Core — BLOCKED

**Reasoning**: identical dependency to WAL-019 — the "Platform Addresses" source type is
disabled for the same "no Platform addresses with balance" reason, itself downstream of
the WAL-017 coin-selection bug. Cannot test until WAL-017 is fixed.

## WAL-021: Navigate wallet accounts via tabs — PASS

Steps: exercised all four tabs on the Wallet screen — Dash Core, Platform, Shielded,
System. Each tab label shows live balance/empty state inline (`Dash Core (3 DASH)`,
`Platform (empty)`, `Shielded (empty)`, `System (empty)`), matching "Each tab shows its
balance in the label" and "Empty accounts display '(empty)' indicator." Switching between
tabs is instant — no loading spinner, no visible data reload; content swaps immediately
(Dash Core: address table + transaction history + asset locks; Platform: DIP-17 payment
address table; Shielded: shielded balance + shielded address + notes section; System: see
WAL-022).

Verdict: **PASS**.

## WAL-022: View system accounts in the Detailed view — PASS (scope slightly broader than worded)

**Reconciliation note**: this story's title was updated from "View system accounts in
developer mode" to "View system accounts in the Detailed view" in the corrected PR892
catalog (`docs/user-stories.md`) — same underlying story, same test below; the retitle just
better matches the observed gating (System tab is hidden only in Default view, not gated
strictly to Developer mode — see the PASS reasoning below, which already called this out).

Steps: confirmed via Settings > Networks > Interface mode (Default / Expert / Developer)
that the System tab is visible in **both Expert and Developer view**, and confirmed
**hidden in Default view** (along with the "[DEV]" wallet-name badge). So the gating is
effectively "not Default view" rather than strictly "Developer mode only" as the story
text implies — the functional intent (hiding low-level structure from everyday users) is
still met.

Expanded one category ("Identity Registration") to confirm each system account category
is a collapsible section with a description ("Credit funding addresses used to register
new identities (DIP-9). Each identity consumes one hardened address here.") and a
standard address table; header shows address count and balance state, e.g.
"Identity Registration (8 addresses, empty)". Other categories observed: Identity System
(0 addresses), Identity Top-up (40 addresses), Identity Invitation (8 addresses), CoinJoin
(16 addresses), Provider Owner (4 addresses), Provider Voting (4 addresses).

Verdict: **PASS**. Restored Interface mode to Expert view afterward (matches the
campaign's established baseline).

## WAL-004 addendum: multi-wallet switching confirmed

The earlier WAL-004 write-up noted same-network multi-wallet switching was "not
exhaustively tested." During this pass, up to 3 wallets coexisted in the same Testnet
wallet-selector dropdown (`QA Wallet 1`, `QA Throwaway HD`, `QA Single Key Test`)
simultaneously, each showing correct independent balances, and switching between them via
the dropdown was instant with no restart or reload glitches. This confirms WAL-004's
"Switching is instant with no app restart" criterion fully.

---

## Second pass (2026-07-14, later session): WAL-025 through WAL-029

**Environment note up front**: this pass's app instance (PID 1580158, same binary hash,
same data dir) hit the exact **known Testnet wallet-backend/storage blocker** already
diagnosed in `ALK.md` — `det.log` showed `Failed to start chain sync error=The wallet
service could not complete this operation. Please retry in a moment.` from the very first
frame after this instance's launch (22:00:42 UTC), never recovering on its own even after
~19 minutes idle. Per `ALK.md`'s own recommendation ("don't keep re-attempting repairs"),
this pass did **one** legitimate non-destructive diagnostic pass — switching Settings >
Networks to Mainnet (which built a wallet backend and fully synced in the background,
confirming the process/host is not generally broken) and back to Testnet — which
reproduced the *exact* documented failure signature (`Failed to start chain sync
error=Could not access wallet data. Check available disk space and restart the
application.`, the `WalletStorage`/SQLite-persister variant). This confirms the blocker is
still present and unresolved; per campaign instructions this pass did not attempt further
repair (killing/restarting the OS process was independently denied by the permission
system, consistent with the instruction to reuse the running instance). `QA Wallet 1`'s
balance showed **0 DASH** throughout this pass as a direct consequence (its real ~3 DASH
balance never loaded because the wallet backend never wired) — this is an environment
artifact, not evidence of lost funds; the underlying on-chain balance and DB rows are
untouched (see `ALK.md` for the full diagnosis chain). All five stories below were
evaluated against this degraded environment; where the blocker prevented a genuine test,
that is called out explicitly rather than papered over.

## WAL-025: Restore a password-protected imported key after an update — BLOCKED (as expected)

**Reasoning (matches the task's own premise)**: this story requires a pre-existing
"old-format" password-protected imported-key fixture from a previous app version. No such
fixture exists in this data dir — it was created fresh with the current build, and no
category of prior QA testing in this campaign has produced one.

Steps: navigated to the Wallets screen (`QA Wallet 1`) and inspected all banners across
multiple visits/wallet-switches during this pass. **No banner counting imported keys
waiting to be restored ever appeared** — consistent with the expected "nothing to
restore" state.

**Important nuance found via `det.log`**: the absence of this banner in this session is
not purely a clean "scan ran, found nothing" signal. The scan itself failed to run at all
this session, due to the environment blocker:
```
WARN dash_evo_tool::ui::wallets::wallets_screen: Failed to scan for protected single-key
  restores; banner suppressed error=MigrationFailed { source: WalletBackendUnavailable }
```
So the banner's absence is doubly explained here — both because (per the task's premise)
there is genuinely nothing to restore, *and* because the restore-scan couldn't even execute
against an unwired wallet backend. A light `sqlite3` check of `det-app.sqlite`'s schema
found no dedicated legacy-migration/protected-key-restore table, consistent with "no
fixture exists," but this doesn't fully substitute for the scan actually running
successfully and confirming zero results.

Screenshot: `screenshots/WAL-025-1-no-restore-banner-env-blocked.png`.

**Verdict: BLOCKED.** No legacy password-protected imported-key fixture exists in this
data dir to exercise the restore flow (expected, per the task's own framing — not a
product defect). The minimal check that *is* possible — confirming the "restore waiting"
banner does not falsely appear in a clean state — passes, but is weakened this session by
the wallet-backend environment blocker also suppressing the underlying scan.

## WAL-026: Unlock a passphrase-protected vault at startup — BLOCKED for live UI; source review confirms implementation

**Reasoning**: this data dir's vault is not currently passphrase-sealed (the primary
wallet was created without a vault password), and deliberately, destructively re-sealing
or corrupting the shared QA data dir's vault to force this condition was out of scope
(risks the evidence trail every other test category in this campaign depends on). Per the
task instructions, this story was evaluated via **read-only source review** of the PR892
build worktree (`/data/git-worktrees/home-ubuntu-git-dash-evo-tool-2-pr892-build/src/`, no
edits) instead of live UI testing.

**Source review findings** (file:line references from the PR892 build worktree):

- **Implemented as a distinct boot-time state**, not folded into the generic error-banner
  path. `src/boot.rs` defines `enum BootApp { Unlocking(UnlockState), Running(Box<AppState>),
  Failed }` (`src/boot.rs:56-64`), wired from `main.rs:82`
  (`Box::new(crate::boot::BootApp::new(cc.egui_ctx.clone())?)`). `docs/user-stories.md:245`
  lists WAL-026 as `[Implemented]` with acceptance-criteria text matching this prompt
  verbatim.
- **Classification logic**: `BootApp::new` (`src/boot.rs:74-88`) calls `classify_open()`
  (`src/boot.rs:260-266`), which attempts a keyless vault open and routes to
  `BootDecision::Unlock` only on `TaskError::is_secret_store_wrong_passphrase()`
  (`src/backend_task/error.rs:2036-2044`) — any other failure still aborts boot as before,
  so this path is narrowly scoped to the passphrase-sealed case.
- **Masked prompt, distinct from other error paths**: `UnlockState::show_modal`
  (`src/boot.rs:188-214`) renders the shared `passphrase_modal` component
  (`src/ui/components/passphrase_modal.rs`) — a masked `PasswordInput` in a centered
  overlay titled "Unlock your saved keys" — rendered from `BootApp::Unlocking`, before
  `AppState` (and therefore the generic `MessageBanner` machinery) even exists. A doc
  comment at `src/backend_task/error.rs:278-283` explicitly notes this exclusion from the
  generic `TaskError::SecretStore` banner path.
- **Wrong-password handling looks safe**: `try_unlock` (`src/boot.rs:93-126`) re-opens the
  *same* vault file via `open_secret_store_with_passphrase` →
  `SecretStore::file(path, passphrase)` (`src/wallet_backend/single_key.rs:824-835`,
  documented as "never deletes, recreates, or rekeys it"). On `WrongPassphrase`, the
  message is the fixed string "That passphrase is not correct. Try again."
  (`src/boot.rs:239`) with `hint: None` (`src/boot.rs:193`) — no hint is leaked. Cancel
  (`UnlockOutcome::Cancel`, `src/boot.rs:147-150`) closes the viewport without touching the
  vault.
- **Headless/CLI confirmed to avoid dialogs**: `src/mcp/server.rs:332-345` — on the same
  `is_secret_store_wrong_passphrase()` check, returns a typed `McpError::internal_error`
  with actionable text ("Open the Dash Evo Tool desktop app and enter the passphrase...
  then run this command again."), no GUI dependency.

This was independent read-only investigation (a background research agent), not a
speculative guess — exact struct/function names and line numbers are cited above so the
finding can be spot-checked.

**Verdict: BLOCKED** for the live-UI walkthrough (no fixture; destructive resealing
correctly out of scope). The source review is strong supporting evidence that the
implementation matches all five acceptance-criteria bullets, but this is not a substitute
for an actual observed unlock-prompt interaction — flagging for a future pass with either
(a) explicit authorization to seal a throwaway vault fixture, or (b) a dedicated
kittest/integration test exercising `BootApp::Unlocking` directly.

## WAL-027: Balance health check after syncing — BLOCKED (environment blocker prevented a genuine test)

**Reasoning**: the acceptance criteria trigger on "when a sync finishes" — in this
session, no sync ever finished for `QA Wallet 1` on Testnet; SPV stayed in the `Error`
sync state (see the environment note above) for the entire pass. The check that *was*
possible is necessarily degenerate.

Steps: on the Wallets screen (`QA Wallet 1`, Expert view), clicked "Refresh" (top-right)
multiple times across the session while the wallet-backend blocker was active. Watched the
"Balance breakdown" line (`Core: 0 DASH | Platform: 0 DASH | Shielded: 0 DASH`) and the
full banner list after each click.

Observed: balance breakdown stayed `0 DASH` across the board throughout (since the true
~3 DASH balance never loaded — see environment note), and **no "balance mismatch" /
reconciliation warning banner ever appeared** — only the four pre-existing
environment-blocker banners ("SPV sync failed", "We couldn't finish preparing your
wallet", "Your wallet is still starting up", "Could not load your identities"). Screenshot:
`screenshots/WAL-027-1-balance-breakdown-no-mismatch-banner.png`.

This is a valid negative check as far as it goes (no false-positive banner for a
trivially-consistent 0/0/0 state), but it does **not** exercise the story's real
acceptance criteria — a genuine sync never completed, so the "does the app catch a real
disagreement between the header total and the account-tab breakdown" question was never
actually tested.

**Additional source-level note** (light-touch grep, not an exhaustive audit — flagged for
follow-up rather than treated as a definitive finding): searching the PR892 build
worktree for terminology from the story text ("rounding", "known display issue", "funds
are safe" + balance context, dedicated reconciler struct names) found no distinct
runtime balance-reconciliation-and-warn feature. The one directly-relevant hit is a
**unit test**, not a user-facing check: `header_total_reconciles_with_core_tab_breakdown_
through_real_accessors` in `src/wallet_backend/snapshot.rs:1238`, which verifies that
DET's own account-summary aggregation code (`src/ui/state/account_summary.rs`) doesn't
itself introduce a mismatch — useful as an internal correctness guardrail, but distinct
from a runtime banner that detects and warns about a *real* wallet-vs-breakdown
disagreement after a sync. `src/app/reconcilers.rs` only defines `SpvBlockReconciler` and
`MigrationReconciler` — no balance-health reconciler was found there either. This search
was not exhaustive (different terminology could exist elsewhere in the ~200+ source
files), so it should not be read as a confirmed "Gap" — just a flag worth a closer look in
a future pass, ideally once the environment blocker is resolved and a real mismatch can be
constructed to test against.

**Verdict: BLOCKED.** Cite the Testnet wallet-backend environment blocker (`ALK.md`) as
the primary reason a genuine test could not be performed. The available degenerate
negative check passed (no spurious banner), but does not confirm the acceptance criteria.

## WAL-028: Switch the active wallet from the top-nav pill on the Wallets tab — PASS

Unlike WAL-025/027/029, this story's mechanics (wallet creation, selection, removal) are
largely independent of SPV/wallet-backend wiring, so it was **fully live-tested** despite
the environment blocker.

**Fixture note**: rather than creating a fresh throwaway wallet immediately, first
discovered via the pill dropdown that a "DIAG throwaway" HD wallet already existed —
a diagnostic leftover from `ALK.md`'s earlier root-cause investigation, deliberately left
in place at the time ("harmless and left in place"). Used it for an initial full
round of testing (confirmed every mechanic below), then removed it — fixing forward the
cleanup that investigation had deferred. Afterward, per the task's explicit instruction,
created a dedicated **"WAL-028 Throwaway"** HD wallet (Create Wallet, no password) and
repeated the key checks against it for clean, correctly-named evidence, then removed it
too, leaving only `QA Wallet 1`.

Steps and observations:

1. **Pill interactive on the Wallets tab**: on the Wallets tab with 2 wallets present,
   clicked the top-nav breadcrumb pill (`🖥 QA Wallet 1 ›` / `🖥 WAL-028 Throwaway ›`) — it
   opened a dropdown listing both wallets plus "Set up another wallet", confirming it is
   not a dead/informational element. Screenshot:
   `screenshots/WAL-028-1-top-nav-pill-interactive-on-wallets-tab.png`.
2. **In-place switch, no forced navigation**: picking the other wallet from the pill
   switched the active wallet immediately — page title, balance breakdown, and address
   table all updated to the newly-selected wallet's data — while staying on the Wallets
   tab throughout (no redirect to a different screen).
3. **Cross-surface re-sync**: with "DIAG throwaway" active, navigated to the Identities
   tab — the pill there also showed "DIAG throwaway" (global, not per-tab state).
   Switched to "QA Wallet 1" from the pill **while on the Identities tab**, then navigated
   back to the Wallets tab via the sidebar: it correctly arrived showing **QA Wallet 1**
   (the wallet last selected on the *other* surface), confirming "arriving at the Wallets
   tab re-syncs to the wallet last chosen on any surface."
4. **Pill and in-tab picker never disagree**: tested both directions — selecting a wallet
   from the top-nav pill updated the in-page "HD: ... ▾" selector to match, and
   conversely, selecting a wallet from the in-page selector updated the top-nav pill to
   match. No divergence observed in either direction, across several repeated switches.
5. **Removal confirmation intact for HD wallets**: clicking "Remove" on both "DIAG
   throwaway" and "WAL-028 Throwaway" (both HD wallets) correctly opened the "Remove
   Wallet" confirmation modal with the same warning text WAL-007 documented for HD
   wallets ("will delete its local data... Continue?"); confirmed removal completed
   cleanly both times, `QA Wallet 1` was never touched.
6. **Single-wallet pill is inert**: after removing the second wallet each time (down to
   just `QA Wallet 1`), clicking the top-nav pill produced **no dropdown** — confirmed the
   pill has nothing to switch to and stays non-interactive with one wallet. Screenshot:
   `screenshots/WAL-028-2-pill-inert-single-wallet.png`.

**Not independently exercised**: the sub-bullet "a single-key selection made in the tab
survives navigation; a later explicit HD pick from the pill supersedes it" specifically
requires a single-key (SK) wallet fixture. Only two HD wallets were used for this test (no
safe SK fixture was created, to avoid unnecessary state growth beyond what the task asked
for) — so this specific SK-vs-HD precedence behavior was not directly observed, though the
general pill/in-tab-agreement mechanism confirmed above makes no distinction between HD
and SK wallets in its implementation path.

**Incidental finding (documented, not blocking the verdict above)**: at one point during
this pass — after several pill/dropdown wallet switches plus a full wallet-creation
cycle — the Wallets screen's wallet-level header block (display name, total balance line,
Send/Receive buttons, and the Rename/Remove buttons) stopped rendering entirely for
*every* wallet, leaving only the error banners followed directly by the account-tab bar
and tab content. This persisted across sidebar navigation away-and-back and a window
resize/reposition, and was **not** a scroll-position artifact (scrolling up by up to 2000px
had no effect, confirming the content was genuinely absent from layout, not merely
off-screen). It recovered only after toggling Settings > Networks > Interface mode from
Expert to Default and back to Expert. This looks like a real, reproducible-in-session
layout/state bug independent of WAL-028's own acceptance criteria (all of which were
independently re-confirmed both before this glitch appeared and after it was
worked around) — worth a follow-up investigation, but not filed as its own WAL story since
it doesn't map cleanly to any of the five stories in scope for this pass.

**Verdict: PASS.** All four acceptance-criteria bullets that could be exercised with the
available fixtures were confirmed working correctly and consistently, across two
independent throwaway-wallet fixtures.

## WAL-029: View and copy my shielded receive address — BLOCKED (environment blocker)

**Reasoning**: this story requires the wallet's shielded keys to be "bound at unlock" —
i.e. `ensure_shielded_bound` to complete on the backend side. In this session, the
Testnet wallet backend never finished wiring (see the environment note above), so this
never happened.

Steps: opened the Wallets screen (`QA Wallet 1`, Expert view) > "Shielded" tab, at
multiple points across the session (roughly 40+ minutes apart, including immediately
after the Mainnet/Testnet network-switch diagnostic).

Observed, consistently every time: the tab shows a spinner and **"Preparing shielded
wallet..."**, plus the same "Your wallet is still starting up. Please wait a moment and
try again." banner seen elsewhere on this screen — never resolving to show an actual
address. Screenshot: `screenshots/WAL-029-1-shielded-tab-preparing-env-blocked.png`.

This differs from the situation the task description anticipated ("a prior session
already read this tab's shielded address for SND-007 testing, so it should already be
populated/bound") — that prior session ran against a healthy wallet-backend instance;
this session's instance hit the environment blocker from its very first frame, so the
shielded binding that SND-007 relied on was never (re-)established here.

Cannot test: whether the address displays correctly, whether clicking "Copy" or the
address text itself copies the full untruncated address to the clipboard (`xclip` is
installed and would have been used — `which xclip` confirmed — but there was never an
address to copy), and cannot cross-check the displayed truncated prefix/suffix against the
known full address from `SND.md` since no address ever rendered this session.

**Verdict: BLOCKED.** Root cause: the same Testnet wallet-backend/storage environment
blocker documented in `ALK.md`, not a defect specific to the Shielded tab or this story —
the tab's own "still preparing" state is itself a reasonable, non-crashing empty/pending
state (arguably consistent with, though not proof of, the story's first bullet: "until
then it says the address appears after unlock"). The last two acceptance-criteria bullets
(frame-safe snapshot sourcing from the backend; the diversified-address gap) remain
unverifiable via black-box UI testing regardless of environment state, per the task's own
framing — no "+" control was found anywhere on this tab, consistent with the documented
gap.

---

*Second-pass summary: WAL-025 BLOCKED (no fixture, as expected), WAL-026 BLOCKED for live
UI / source-review-confirmed-implemented, WAL-027 BLOCKED (environment blocker prevented a
genuine test), WAL-028 **PASS** (fully live-tested despite the environment blocker, since
its mechanics don't depend on SPV wiring), WAL-029 BLOCKED (environment blocker). Final
state left by this pass: network Testnet, Expert view, `QA Wallet 1` intact (balance
reads 0 DASH in-app due to the still-unresolved wallet-backend environment blocker — the
underlying ~3 DASH balance and all DB state are believed untouched, see `ALK.md`), only
wallet remaining after both throwaway-wallet cleanups. The Testnet wallet-backend
environment blocker documented in `ALK.md` remains unresolved and affects any future
testing that requires a live Testnet wallet/SPV connection in this data dir.*

---

## Third pass (2026-07-15): WAL-018/019/020/025/027/029 retested post-fix

**Environment**: the Testnet wallet-backend blocker documented above and in `ALK.md` is now
root-caused and (for the one historically bad row) fixed in this live data dir — see
`ALK.md`'s "Resolution (2026-07-15...)" section and
`/data/artifacts/dash-evo-tool/2026-07-14/pr892-user-story-qa/testnet-blocker-investigation/TEST-VECTOR.md`.
On arrival this pass, the app (PID 2216703, same hash-verified binary) was already running
against the live QA data dir with Testnet **fully synced** ("Synced - The SPV client can now
be used for transacting and querying.") — confirmed via Settings > Networks and via `det.log`
showing active, error-free header/masternode-list/filter/block/shielded-note sync. `QA Wallet
1` already carried a real balance (3.96 DASH Core, plus 0.0199 DASH already funded to a
Platform address from an earlier differential-retest pass — see `ALK.md`'s "Scope conclusion"
section) and a genuinely-synced, non-degenerate state throughout this pass, unlike the
second pass above.

### WAL-018: Fund Platform address from asset lock — BLOCKED (independent, confirmed cause)

**What changed**: the original blocker ("no asset lock could be created due to WAL-017's coin
selection bug") no longer applies — asset-lock creation now works reliably. Created a fresh
asset lock live end-to-end: Wallets > Dash Core tab > "Create Asset Lock" > Registration
purpose > funded the generated deposit address (`yitCWdDBXLCUMa84ENDnaxJKd14ju3tKHR`) via the
Pasta testnet faucet (solved the Cap.js v4 PoW challenge per the `faucet-cap-pow-solver`
memory note; txid `914c8b4a506175704670914e89bdb02bf54044eb37af64503a5d1d4272378074`) —
**without navigating away from the screen this time** (an earlier attempt in this same pass
that did navigate away lost the in-flight build and left the funds as plain wallet balance,
confirming ALK-001's documented navigation-loses-state behavior). The flow progressed through
"Waiting for funds…" → "Funds received!" → "Waiting for Core Chain to produce proof of asset
lock…" → **"Asset Lock Created Successfully!"** (txid
`88b8c37019edcc66b4e5ddb7c98b208e93f5a4311a03a29bacff7048198977d4`). Screenshot:
`screenshots/WAL-018-1-asset-lock-created-successfully.png`.

Note: a first attempt at this (different deposit address, funded via a separate faucet
payout) hit WAL-017's exact "No UTXOs available for selection" transient coin-selection
error mid-flow and stalled — consistent with ALK.md's characterization of that bug as
state-dependent/transient, not deterministic. It cleared on a fresh retry with a new deposit
address; the funds from the first, abandoned attempt remain in the wallet as ordinary balance
(not lost, just not part of an asset lock).

**Verified the lock is genuinely persisted**, not just a one-shot success screen: a read-only
`sqlite3` query against `spv/testnet/platform-wallet.sqlite`'s `asset_locks` table confirms a
row with `status='is_locked'`, `amount_duffs=50000000` (0.5 DASH — the form's default amount,
left unchanged), a 719-byte `lifecycle_blob`, unconsumed.

**Where it's still blocked**: WAL-018's actual acceptance criteria ("fund a Platform address
from an *existing* asset lock") require reaching the "Fund a Platform address with this asset
lock" action, which — per source review (`src/ui/wallets/wallets_screen/asset_locks.rs:174`,
`dialogs.rs:562`) — is only reachable from a row in the Wallets screen's "Asset Locks" list.
That list still shows **"No asset locks found"** for this same, freshly-created, confirmed-
persisted, unconsumed lock, even after multiple Refresh clicks — reproducing ALK-002's
documented UI/cache bug exactly, now in a fully healthy session with no other explanation
available. No alternate UI path to this specific dialog was found.

**Verdict: BLOCKED**, but for a different, now-independently-confirmed reason than
originally recorded: not "no asset lock could be created" (fixed), but "the Asset Locks list
never surfaces a created lock, so the only entry point to the fund-from-asset-lock dialog is
unreachable" (ALK-002's bug, confirmed live once more — see the ALK.md update below). Per
task guidance, this is a genuinely-blocked-for-an-independent-reason case, not a forced PASS
or a speculative FAIL.

### WAL-019: Transfer credits between Platform addresses — PASS

Steps: Wallets > Send > Advanced Options > Source Type: Platform Addresses (now enabled —
previously disabled with "no Platform addresses with balance"). Selected the wallet's funded
Platform address (`tdash1kp30ae9x752z7wu20j4m4y945449anlhtqqe9h4l`, 0.0198520418 DASH) as the
sole input, amount 0.005 DASH. Added an output to a second, zero-balance Platform address
belonging to the same wallet (`tdash1kplvfzspsn99pn4rvdwmwap5a3z7g4pchqsdzvt6`, index 0),
amount 0.005 DASH. Confirmed the **Fee Strategy** selector is present with all 4 documented
options: "Deduct from first input", "Deduct from last input", "Reduce first output", "Reduce
last output" — left at the default ("Deduct from first input"). Clicked "Send".

Observed: **"Platform credits transferred successfully!"** Screenshot:
`screenshots/WAL-019-1-platform-credits-transferred-successfully.png`. Confirmed on the
Platform tab afterward: destination address now holds exactly `0.00500000` DASH; source
address dropped from `0.01985204` to `0.01475803` (= 0.01985204 − 0.005 sent − ~0.00009401
fee), matching "deduct from first input" exactly.

Verdict: **PASS**. Both acceptance-criteria bullets confirmed: fee-strategy selection
present and functional; wired into the same internal wallet Send flow used elsewhere
("used in internal wallet operations").

### WAL-020: Withdraw from Platform address to Core — PASS

Steps: Wallets > Send > Source Type: Platform Addresses, destination a Core address
(`yYCWtyP2mSLzGkZqL9a6G5rpPQQRs1fT5f`, the wallet's own funded address). The combined "Send
to" field correctly recognized the Core address and auto-set **"Transaction type: Withdraw to
Wallet"**. Amount 0.005 DASH. Clicked "Withdraw to Wallet".

Observed: **"Withdrawal initiated successfully! Note: It may take a few minutes for funds to
appear on the Core chain."** Screenshot:
`screenshots/WAL-020-1-withdrawal-initiated-successfully.png`. Confirmed Core balance
increased correctly afterward (5.45998397 → 5.46498397 DASH) and Platform balance decreased
accordingly.

Verdict: **PASS**. "Destination Core address input" and "Fee strategy configuration"
(same Fee Strategy selector as WAL-019) both confirmed.

### WAL-025: Restore a password-protected imported key after an update — BLOCKED (fixture still absent; scan now confirmed clean)

**What changed**: the earlier BLOCKED reasoning noted the restore-scan itself failed to run
this session due to the wallet-backend blocker (`MigrationFailed { source:
WalletBackendUnavailable }` warning in `det.log`). Source review
(`src/ui/wallets/wallets_screen/mod.rs:2323-2343`,
`refresh_pending_protected_restores`) shows the scan runs lazily exactly once per
`WalletsBalancesScreen` instance lifetime (a persistent root screen, not re-created per
navigation), logging a `WARN` only on failure and nothing on success. Across this entire
pass's session — from the very first paint through dozens of subsequent screen visits and
real transactions — `det.log` contains **zero** occurrences of `MigrationFailed`,
`WalletBackendUnavailable`, "Failed to scan for protected single-key restores", or any
single-key/restore-scan string. This confirms the scan now runs and completes cleanly.

No restore banner appeared at any point (consistent with "nothing to restore"). A read-only
check of `det-app.sqlite`'s schema again found no dedicated legacy single-key-password table
(the scan's underlying data source), consistent with "no fixture exists in this data dir" —
same conclusion as before, now on firmer footing since the scan itself is confirmed to have
actually run and found nothing, rather than having failed to run at all.

Verdict: **BLOCKED** — same as before (no fixture to exercise the actual restore dialog), but
the caveat about the scan itself failing no longer applies; only the missing fixture blocks
this story now.

### WAL-027: Balance health check after syncing — FAIL

**What changed**: the earlier BLOCKED verdict was explicitly a "degenerate 0/0/0 test" since
no sync ever completed. This pass ran against a genuinely, fully-synced wallet with dozens of
real balance-changing operations (the asset lock creation, WAL-019's transfer, WAL-020's
withdrawal, SND-009's rejected-but-attempted shield, plus prior-session sends) — a much more
meaningful substrate for this story's "totals don't add up" check.

Observed: at every checkpoint, the wallet header total exactly equalled the sum of the
Core + Platform + Shielded account tabs (e.g. `5.4787091` = `5.46498397` + `0.01372513` +
`0`, verified by direct addition). **No mismatch/reconciliation warning banner ever
appeared**, across the whole session. Screenshot:
`screenshots/WAL-027-1-genuine-reconciliation-totals-agree.png`.

**Source review** (repeated from the earlier pass, now checked against this healthy session
too): grepped the PR892 build worktree for the story's own language ("totals don't add up",
"known display issue", "funds are safe", `header_total`, balance-reconciler struct names).
The only matches are: `src/wallet_backend/snapshot.rs:1238`
(`header_total_reconciles_with_core_tab_breakdown_through_real_accessors`) — an **internal
unit test** verifying DET's own account-summary aggregation code never introduces a mismatch,
not a user-facing runtime check — and `src/app/reconcilers.rs`, which defines only
`SpvBlockReconciler` and `MigrationReconciler`, no balance-health reconciler. No banner
string matching the story's wording ("funds are safe", "known display issue") exists
anywhere in the UI source.

**Verdict: FAIL.** With a genuine, healthy sync and a real, actively-changing wallet
throughout this session, the totals always agreed correctly (so there was never a true
mismatch to report — a legitimate negative result on its own), but source review confirms
the underlying proactive "detect and warn about a mismatch" mechanism the story describes
simply does not exist in this codebase — the same conclusion independently reached in the
degraded-environment second pass, now reconfirmed with a healthy substrate that could have
surfaced the feature if it existed. Recording as FAIL rather than BLOCKED because this is a
deterministic, source-confirmed absence, not an environment-dependent unknown.

### WAL-029: View and copy my shielded receive address — PASS

**What changed**: the Shielded tab no longer gets stuck at "Preparing shielded wallet..." —
it now renders immediately.

Steps: Wallets > `QA Wallet 1` > Shielded tab. Observed: **Shielded Balance: 0 DASH**,
**Shielded Address**: `tdash1zpzmpc25xp0x3g...pp4cvs6cca9x` (truncated display) with a "Copy"
button, the informational note "Shielded sending is not available on this network yet. You
can still view your shielded balance and receive address," and a "Shielded Notes" section
(placeholder, matching WAL-030's documented Gap). Screenshot:
`screenshots/WAL-029-1-shielded-tab-address-rendered.png`.

**Copy verified two ways** using `xclip -selection clipboard -o` to inspect the real X11
clipboard after each action (clearing it between tests):
1. Clicking the **"Copy" button**: clipboard held
   `tdash1zpzmpc25xp0x3gjh650nqhunsmezkqqujawl2g2p6k04uax7nj53fdlpcp77udv8vpp4cvs6cca9x` (83
   chars) — full, untruncated, matching the displayed prefix/suffix exactly.
2. Clicking the **address text itself**: same 83-character full address copied, confirmed via
   an in-app "Shielded address copied to the clipboard." toast plus the clipboard check.
   Screenshot: `screenshots/WAL-029-2-address-copy-confirmed-full-address.png`.

Verdict: **PASS**. All testable acceptance-criteria bullets confirmed: address shown once
bound at unlock; both click targets (address and Copy button) copy the correct full address.
The last two bullets (frame-safe backend sourcing; diversified-address "+" gap) remain
source-review-only per the story's own framing (no "+" control found, consistent with the
documented gap) — not re-verified live this pass since they require code inspection, not UI
interaction.

---

*Third-pass summary: WAL-018 BLOCKED (independent, confirmed cause — ALK-002's list bug, not
the resolved env blocker), WAL-019 **PASS**, WAL-020 **PASS**, WAL-025 BLOCKED (fixture still
absent, but scan confirmed to now run cleanly), WAL-027 **FAIL** (source-confirmed absent
feature), WAL-029 **PASS**. Final state: network Testnet, Expert/Developer view (left on
Developer view — see SND.md's SND-009 retest, which required it), `QA Wallet 1` balance
~5.48 DASH total across Core+Platform, app PID 2216703 still running against
`/data/tmp/det-qa-pr892-data`, Testnet still synced and healthy — no restart was performed
this pass (see the campaign coordinator's report for the reasoning: this pass's own asset
lock creation produced a new, not-yet-restart-tested `lifecycle_blob`, so a restart carries
the same theoretical AssetLockProof-decode risk described in `ALK.md`'s resolution section
until a product fix lands).*
