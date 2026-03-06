# Manual Test Scenarios

End-to-end manual test scenarios for Dash Evo Tool. Each scenario exercises
multiple features across screens and backend systems. Designed for Testnet
unless stated otherwise.

---

## Global Preconditions

1. Dash Evo Tool is installed and launches successfully.
2. At least one network (Testnet or Regtest) is configured with valid DAPI endpoints.
3. Tester has a wallet mnemonic with known UTXO distribution and at least one Platform identity.
4. Developer mode is enabled in Settings (required for refresh mode dropdown and sync status details).

---

## Scenario 1: Fresh Wallet Setup & SPV Sync Lifecycle

**Goal:** Verify wallet creation, SPV connection state progression, and sync
status panel from cold start.

**Preconditions:** SPV backend mode. No prior wallet loaded.

### Steps

1. Launch the application with normal network connectivity.
2. Observe the connection indicator immediately — it should show **orange fast pulse** (Connecting). Hover to confirm tooltip: "Connecting...".
3. Import a wallet using a known mnemonic.
4. Select the wallet on the Wallets screen. Confirm the **sync status panel** appears between wallet selector and detail panel.
5. Monitor the Core line in the sync status panel as SPV progresses through phases:
   - "Connecting..." (with blue spinner)
   - "Syncing -- Headers: C / T (NN%)"
   - "Syncing -- Masternodes: C / T (NN%)"
   - "Syncing -- Filter Headers: C / T (NN%)"
   - "Syncing -- Filters: C / T (NN%)"
   - "Syncing -- Blocks: C / T (NN%)"
   - "Synced -- N peers" (green, no spinner)
6. Observe the connection indicator transitions: orange fast pulse -> orange slow pulse -> green.
7. Hover over the green indicator. Confirm tooltip: "Ready\nSPV: Synced\nDAPI: Available (N unbanned / M total endpoints)".

### Expected Results

- Wallet imports without error; addresses appear in address table.
- Connection indicator follows the Connecting -> Syncing -> Synced progression.
- Sync status panel shows each phase with progress percentages that increase over time.
- Blue spinner visible during all syncing phases, disappears when synced.
- Final state: green indicator, "Synced -- N peers" in status panel.

---

## Scenario 2: Platform Address Sync & Address Table

**Goal:** Verify Platform address balance sync, refresh modes, address table
column switching between Core and Platform accounts.

**Preconditions:** Wallet loaded and SPV synced (Scenario 1 complete). Wallet has
both Core (BIP44) and Platform Payment accounts.

### Steps

1. On the Wallets screen, select the **BIP44 Core account**. Confirm the address table headers include: Address, Balance, UTXOs, Total Received (DASH), Type, Index, Derivation Path.
2. Click "UTXOs" header to sort ascending. Confirm sort indicator shows "UTXOs ^".
3. Switch to the **Platform Payment account**. Confirm headers change to: Address, Balance, Nonce, Type, Index, Derivation Path. The "UTXOs" and "Total Received" columns must not appear.
4. Verify nonce values: addresses used for Platform transactions show nonce > 0; unused addresses show 0.
5. Switch back to BIP44 account. Confirm "UTXOs" and "Total Received" columns return. No crash or visual glitches.
6. Using the developer mode refresh dropdown, select **"Platform Only"** and click Refresh.
7. Wait for refresh to complete. Confirm Platform address balances update (spinner on Platform line in sync status panel during refresh).
8. Select **"Core Only"** and click Refresh. Confirm only Core data refreshes — Platform balances remain unchanged.
9. Select **"Core + Platform"** (default) and click Refresh. Confirm both Core and Platform data refresh.

### Expected Results

- Column headers switch correctly between Core and Platform account types.
- Sort state does not cause crashes when switching account types.
- Each refresh mode triggers only the expected sync operations (verify via sync status panel spinners and log output).
- Platform sync status line shows "Addresses: N synced (blk H, T ago)" after sync with accurate time-ago.

---

## Scenario 3: Identity Registration with Dynamic Fee

**Goal:** Verify identity registration flow including dynamic asset lock fee
calculation and transaction broadcast.

**Preconditions:** Wallet loaded with sufficient balance (at least 0.001 DASH).
Connected to Testnet or Devnet.

### Steps

1. Navigate to the Identity screen.
2. Initiate a new identity registration with an amount of 50,000 duffs.
3. Confirm the transaction.
4. Observe the asset lock transaction creation and broadcast.
5. Wait for identity registration to complete (or await proof confirmation).
6. Check wallet balance decrease: should be approximately 50,000 + fee duffs.

### Expected Results

- Asset lock transaction is created and broadcast successfully (no "min relay fee not met" error).
- With 1 input: fee = max(3000, estimated_size). For typical single-input tx, minimum 3000 applies.
- If the wallet has many small UTXOs requiring multiple inputs, fee scales upward (e.g., 21 inputs -> ~3246 duffs fee).
- Change output is present if input total > amount + fee; absent if exact match.
- Identity appears in the identity list after confirmation.

---

## Scenario 4: Identity Top-Up & Nonce Tracking

**Goal:** Verify identity top-up with dynamic fee, and that Platform Payment
address nonces are preserved across refreshes.

**Preconditions:** Identity exists (Scenario 3 complete). Wallet has Platform
Payment addresses with known nonce values.

### Steps

1. On the Wallets screen, select Platform Payment account. Note nonce values for addresses with nonce > 0.
2. Navigate to the identity detail screen.
3. Initiate a top-up of 20,000 duffs. Confirm the transaction.
4. Wait for the top-up to complete.
5. Verify identity balance increases by approximately 20,000 duffs (in credits).
6. Return to Wallets screen, Platform Payment account. Verify nonce for the used address has incremented.
7. Click Refresh ("Core + Platform"). Wait for completion.
8. Verify all nonce values are preserved — addresses that had nonce > 0 still show the same (or incremented) nonce. No nonces reset to 0.
9. Find a Platform address with 0 balance but nonce > 0 (if available). Confirm nonce is retained even for zero-balance addresses.

### Expected Results

- Top-up transaction broadcast succeeds with dynamically calculated fee.
- Identity balance reflects the top-up amount.
- Nonces are never reset to 0 by refresh operations.
- Zero-balance addresses retain their nonce values.

---

## Scenario 5: Mining Blocks on Regtest

**Goal:** Verify the Mine Blocks dialog functionality, visibility rules, and
post-mining balance refresh.

**Preconditions:** Connected to **Regtest** network. Developer mode enabled.
Core backend mode set to **RPC** (not SPV). At least one wallet loaded.

### Steps

1. On the Wallets screen, confirm a **"Mine"** button is visible in the toolbar alongside Send and Receive.
2. Click "Mine". Confirm the dialog opens with: address dropdown (showing wallet addresses with balances), block count field defaulting to "1", Cancel and Mine buttons.
3. Leave block count as "1". Click "Mine" in the dialog.
4. Confirm: dialog closes, success banner "Mined 1 block(s)" appears, wallet balance updates.
5. Open Mine dialog again. Change block count to "10". Select a different address from the dropdown. Click "Mine".
6. Confirm success banner: "Mined 10 block(s)".
7. Test validation: open Mine dialog, enter "0" as block count, click Mine. Confirm error: "Enter a valid number of blocks (> 0)". Repeat with "abc" and empty string — same error.
8. Switch network to **Testnet**. Confirm Mine button disappears.
9. Switch back to Regtest. Disable developer mode in Settings. Return to Wallets screen. Confirm Mine button disappears.
10. Re-enable developer mode. Confirm Mine button reappears.

### Expected Results

- Mine button visible only on Regtest/Devnet + Developer mode + RPC backend.
- Mining succeeds and triggers automatic wallet balance refresh.
- Invalid inputs show inline error without closing dialog.
- Visibility toggles correctly with network switches and developer mode changes.

---

## Scenario 6: Network Switching & State Isolation

**Goal:** Verify that network switches reset connection state, isolate
per-network data, and SPV restarts cleanly.

**Preconditions:** SPV mode. Wallet loaded on Testnet with known Platform balances.
A second network (Mainnet or Regtest) configured.

### Steps

1. On Testnet, confirm SPV is synced (green indicator). Note wallet balances (Core and Platform).
2. Note sync status panel: Platform line shows "Addresses: N synced (blk H, T ago)".
3. Switch to a different network via the network chooser.
4. Observe the connection indicator immediately: should turn **red** (Disconnected) momentarily as `ConnectionStatus::reset()` clears all state.
5. Observe indicator transition to orange (Connecting) as SPV starts on the new network.
6. Wait for sync to complete on the new network.
7. Confirm wallets/balances on the new network are independent — Testnet balances are not displayed.
8. Switch back to Testnet.
9. Confirm Testnet balances are restored from database and match values from Step 1.
10. Confirm sync status panel shows the previously stored Platform sync info (not "never synced").

### Expected Results

- Connection state resets cleanly on switch: indicator goes red -> orange -> green.
- Per-network balance isolation: no cross-contamination between networks.
- Previously synced data persists in database and restores on return.
- SPV restarts without manual intervention on each network switch.

---

## Scenario 7: SPV Error Handling & Degraded Mode

**Goal:** Verify SPV behavior under network failures: degraded warning on peer
loss, error state visualization, and automatic recovery.

**Preconditions:** SPV mode. Network controls available (firewall or VPN toggle).

### Steps

1. Launch and wait for SPV to sync fully (green indicator, "Synced -- N peers").
2. **Block peer connections** (firewall on port 9999/19999). Keep DAPI reachable.
3. Observe the indicator over the next 10 seconds: should change from green to **orange fast pulse** (Connecting) as peer count drops to 0.
4. Wait approximately 30 seconds with zero peers.
5. Hover over indicator. Confirm tooltip includes: "Having trouble finding peers. Check your connection." A warning banner should also appear.
6. Confirm SPV has **not** stopped — no "SPV disconnected" error, no `stop_spv()` in logs.
7. **Restore connectivity** (remove firewall rule). Observe automatic recovery: indicator transitions from orange -> syncing -> green without manual restart.
8. Confirm the degraded warning banner clears automatically on recovery.
9. If SPV sync encounters a fatal error (e.g., masternode sync failure on a problematic network):
   - Observe the indicator turns **magenta** with slow pulse and "!" glyph.
   - Hover to confirm tooltip: "SPV sync error: {specific error message}".
   - Verify this is visually distinct from red/Disconnected (which is static, no glyph).

### Expected Results

- SPV stays running on peer loss — never auto-stops.
- Degraded warning appears after ~30s with zero peers.
- Recovery is automatic when peers reconnect.
- Error state (magenta) is visually distinct from Disconnected (red).
- All DAPI endpoints banned -> indicator goes red regardless of SPV peer status.

---

## Scenario 8: Error Banners & Details Display

**Goal:** Verify error banner stacking, expandable details, and independent
scroll areas when multiple errors occur.

**Preconditions:** Developer mode enabled. Ability to trigger multiple backend
errors (e.g., invalid network config, disconnected network, expired operations).

### Steps

1. Trigger 2+ error banners with technical details (e.g., attempt operations on a disconnected network, then perform a different failing action).
2. Verify all banners appear stacked vertically without overlap.
3. Click "Show details" on the **first** banner. Confirm details section expands inline, pushing subsequent banners down.
4. Click "Show details" on the **second** banner. Confirm its details expand without overlapping the first.
5. Scroll within each details section independently. Confirm scroll positions are isolated (no shared state).
6. Click "Hide details" on one banner. Confirm only that banner's details collapse; the other remains expanded.
7. Dismiss one banner entirely. Confirm remaining banners reflow correctly with no gaps or overlap.
8. Verify a single banner with "Show details" still works normally (regression check).
9. Verify banners without details are unaffected by the presence of detail-enabled banners.

### Expected Results

- Each banner's details section occupies its own vertical space.
- No visual overlap between expanded details of different banners.
- Scroll areas are independent per banner.
- Dismiss and collapse operations affect only the targeted banner.

---

## Scenario 9: Developer Mode & Local Network Configuration

**Goal:** Verify developer-only features: dashmate auto-update, refresh mode
dropdown, and feature visibility toggling.

**Preconditions:** Dashmate installed with `~/.dashmate/config.json` containing
a valid `local_seed` configuration. Local Regtest network configured.

### Steps

1. Navigate to Network Chooser > **Local Network** (Regtest) tab.
2. Locate the RPC password text field. Confirm both **"Save"** and **"Auto Update"** buttons are visible next to it.
3. Click **"Auto Update"**. Confirm the password field is populated with the value from `~/.dashmate/config.json` (`configs.local_seed.core.rpc.users.dashmate.password`).
4. Close and reopen the application. Navigate back to Local Network. Confirm the auto-filled password persists.
5. Verify "Auto Update" button is **not present** on Mainnet, Testnet, or Devnet tabs.
6. Navigate to Wallets screen. Confirm the refresh mode dropdown is visible (developer mode is on).
7. Confirm exactly three options: "Core + Platform", "Core Only", "Platform Only". No old options (Auto, Full, Terminal).
8. Disable developer mode in Settings. Return to Wallets screen.
9. Confirm: refresh mode dropdown is hidden. Mine button (if on Regtest) is hidden.
10. Re-enable developer mode. Confirm both reappear.

### Expected Results

- Auto Update reads dashmate config and persists the password automatically.
- Auto Update button only appears on Local Network tab.
- Refresh dropdown shows exactly 3 simplified options.
- Developer-only UI elements hide/show correctly with the toggle.

---

## Scenario 10: Locked Wallet, Insufficient Funds & Visual Edge Cases

**Goal:** Verify error handling for locked wallets, insufficient funds scenarios,
and visual correctness across themes.

**Preconditions:** Wallet loaded. Platform Payment addresses with known nonces.

### Steps

1. **Locked wallet refresh:** Lock the wallet (close it / remove seed phrase). Attempt Refresh ("Core + Platform"). Confirm error: "Wallet is locked. Please unlock it first to refresh." Nonces from before locking remain unchanged in the UI.
2. Unlock the wallet. Confirm refresh now succeeds.
3. **Insufficient funds (no fee deduction):** With a wallet holding exactly 10,000 duffs in one UTXO, attempt to fund a platform address for 10,000 duffs with "deduct fee from amount" **disabled**. Confirm error with specific amounts (need 10,000 + fee, have 10,000). No transaction broadcast.
4. **Insufficient funds (amount too small for fee):** With a wallet holding only 2,500 duffs, attempt identity registration with fee deduction enabled. Confirm failure — entire balance consumed by minimum 3,000 fee.
5. **Theme check:** Switch to dark mode. Observe sync status panel, address table, and connection indicator. Confirm colors are readable: green for connected, red for error, blue for syncing, secondary text legible against dark background.
6. Switch to light mode. Repeat visual check. Confirm same color semantics with appropriate contrast.
7. **Address table edge case:** If wallet has key-only addresses (Identity Registration, Identity Topup accounts), confirm they show "N/A" for UTXOs and Total Received columns.

### Expected Results

- Locked wallet produces a clear user-facing error; no data corruption.
- Insufficient funds errors include specific amounts and prevent broadcast.
- Light and dark mode both render status colors (green/red/blue/magenta) with readable contrast.
- Key-only addresses display "N/A" for inapplicable columns.
