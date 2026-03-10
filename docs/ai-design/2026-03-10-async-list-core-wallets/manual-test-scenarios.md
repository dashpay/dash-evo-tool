# Manual Test Scenarios: Async list_core_wallets() (Issue #700)

## Overview

`list_core_wallets()` was previously called synchronously on the UI thread in three locations.
This change moves all three call sites to an async backend task (`CoreTask::ListCoreWallets`) and
returns the result via `BackendTaskSuccessResult::CoreWalletsList`. The affected screens are:

- **AddNewWalletScreen** — Core wallet ComboBox populated asynchronously on first frame
- **ImportMnemonicScreen** — Same pattern
- **WalletsBalancesScreen** — `CoreWalletNotConfigured` error now sets a flag and dispatches
  the task on the next frame instead of blocking

Cross-cutting behaviors:
- If the fetch fails, `core_wallets_loading` resets so the next frame retries automatically
- `change_context()` (network switch) clears the cached wallet list and loading flag via
  `reset_core_wallets_cache()`, forcing a fresh fetch for the new network
- The UI renders without blocking even while the task is in flight

---

## ACW-001: Create Wallet screen -- UI renders immediately before wallet list arrives

| Field | Value |
|---|---|
| **ID** | ACW-001 |
| **Title** | AddNewWalletScreen is responsive on the first frame |
| **Priority** | P0 |

### Preconditions
- Dash Core RPC is configured (any network)
- Application is running

### Steps
1. Navigate to Wallets screen
2. Click "Create Wallet"
3. Observe the Create Wallet screen at the moment it appears (before any async response arrives)

### Expected Results
- Step 3: The Create Wallet screen renders immediately without any visible freeze or hang
- The UI is interactive on the first frame -- the entropy grid, seed phrase controls, and other
  form elements respond to interaction
- No loading spinner or blank screen is shown while the wallet list is being fetched
- The "Dash Core Wallet" ComboBox section is absent initially (the form renders without waiting
  for the RPC call result)

---

## ACW-002: Create Wallet screen -- ComboBox appears after wallet list arrives (multiple Core wallets)

| Field | Value |
|---|---|
| **ID** | ACW-002 |
| **Title** | Core wallet ComboBox populates asynchronously when multiple Core wallets exist |
| **Priority** | P0 |

### Preconditions
- Dash Core running with **two or more** wallets loaded (e.g., `wallet.dat`, `savings`)
- Application connected to RPC

### Steps
1. Navigate to "Create Wallet"
2. Complete all required steps (entropy grid, generate seed phrase, write it down, enter name)
3. Scroll down to observe the area before the "Save Wallet" button
4. Wait for the async wallet list fetch to complete (typically less than 1 second on local RPC)
5. Observe the ComboBox labeled "Dash Core Wallet"
6. Open the ComboBox dropdown
7. Select a non-default wallet (e.g., `savings`)
8. Click "Save Wallet"

### Expected Results
- Step 3: If the list has not yet arrived, step "6. Select the Dash Core wallet..." and the
  ComboBox are absent; the step numbering shows "6. Save the wallet."
- Step 4-5: Once the task completes, a new step "6. Select the Dash Core wallet to use for RPC
  operations." and an inline ComboBox appear above "Save the wallet." (step renumbers to "7.")
- Step 6: The dropdown lists all Core wallet names returned by `listwallets`
- Step 8: The wallet is saved with `core_wallet_name` set to the selected wallet name in SQLite
- All subsequent RPC calls for this wallet use `/wallet/<selected_name>`

---

## ACW-003: Create Wallet screen -- ComboBox absent with zero or one Core wallet

| Field | Value |
|---|---|
| **ID** | ACW-003 |
| **Title** | No Core wallet ComboBox shown when zero or one Core wallet exists |
| **Priority** | P0 |

### Preconditions
- Dash Core running with **zero or one** wallet loaded

### Steps
1. Navigate to "Create Wallet"
2. Complete all required steps
3. Observe the form before the "Save Wallet" button

### Expected Results
- Step 3: No "Dash Core Wallet" ComboBox appears at any point during the session
- Step numbering remains at "6. Save the wallet." (no extra Core wallet step)
- Clicking "Save Wallet" saves the wallet with the sole Core wallet auto-assigned (or `core_wallet_name = NULL` if zero wallets loaded)

---

## ACW-004: Import Wallet screen -- UI renders immediately before wallet list arrives

| Field | Value |
|---|---|
| **ID** | ACW-004 |
| **Title** | ImportMnemonicScreen is responsive on the first frame |
| **Priority** | P0 |

### Preconditions
- Dash Core RPC is configured (any network)
- Application is running

### Steps
1. Navigate to Wallets screen
2. Click "Import Wallet"
3. Observe the Import Wallet screen at the moment it appears

### Expected Results
- The Import Wallet screen renders immediately -- seed phrase input fields, length selector, and
  other controls are interactive from the first frame
- No freeze or hang occurs while the Core wallet list RPC is in flight
- The form renders and accepts input while waiting for the async result

---

## ACW-005: Import Wallet screen -- ComboBox appears asynchronously (multiple Core wallets)

| Field | Value |
|---|---|
| **ID** | ACW-005 |
| **Title** | Core wallet ComboBox populates asynchronously in the import form |
| **Priority** | P0 |

### Preconditions
- Dash Core running with **two or more** wallets loaded
- A valid BIP39 seed phrase is available for import

### Steps
1. Navigate to "Import Wallet"
2. Enter all seed phrase words and complete the password/name fields
3. Observe the area before the "Save Wallet" / "Import Key" button before and after the fetch
4. Open the ComboBox dropdown
5. Select a Core wallet
6. Click "Save Wallet"

### Expected Results
- After the async task completes, the "Dash Core Wallet" ComboBox is visible in the form, before
  the save button
- The dropdown lists all Core wallet names
- Saving associates the imported wallet with the selected `core_wallet_name` in SQLite
- No post-import SelectionDialog modal appears

---

## ACW-006: Import Wallet -- private key import also receives the ComboBox asynchronously

| Field | Value |
|---|---|
| **ID** | ACW-006 |
| **Title** | Core wallet ComboBox appears in private key import mode as well |
| **Priority** | P1 |

### Preconditions
- Dash Core running with **two or more** wallets loaded
- A valid WIF or hex private key is available
- "Show Advanced Options" is enabled in the import form

### Steps
1. Navigate to "Import Wallet"
2. Enable "Show Advanced Options" and select "Private Key (Single Address)" import type
3. Enter a valid private key
4. Observe the form before the "Import Key" button
5. Select a Core wallet from the ComboBox
6. Click "Import Key"

### Expected Results
- The Core wallet ComboBox appears before the "Import Key" button (same as mnemonic import)
- The single-key wallet is saved with `core_wallet_name` set in the `single_key_wallet` table
- All subsequent RPC calls for this single-key wallet route to the selected Core wallet

---

## ACW-007: Wallets screen -- CoreWalletNotConfigured triggers async fetch, not UI freeze

| Field | Value |
|---|---|
| **ID** | ACW-007 |
| **Title** | WalletsBalancesScreen handles CoreWalletNotConfigured without blocking the UI |
| **Priority** | P0 |

### Preconditions
- Dash Core running with **two or more** wallets loaded
- A DET wallet with `core_wallet_name = NULL` in SQLite (legacy wallet)
- The user is on the Wallets screen

### Steps
1. Navigate to the Wallets screen
2. Select the legacy wallet (or wait for the auto-refresh on arrival)
3. Trigger an RPC operation that causes a `CoreWalletNotConfigured` error
   (e.g., click Refresh)
4. Observe the UI immediately after the error is received

### Expected Results
- Step 4: The UI remains responsive -- no freeze, hang, or blank frame
- The `refreshing` flag clears and the Refresh button is no longer shown as in-progress
- Within the same frame or the next, a `ListCoreWallets` backend task is dispatched
- After the task completes, the outcome follows ACW-008, ACW-009, or ACW-010 depending on
  how many wallets are loaded

---

## ACW-008: Wallets screen -- single Core wallet auto-selected after CoreWalletNotConfigured

| Field | Value |
|---|---|
| **ID** | ACW-008 |
| **Title** | When exactly one Core wallet exists, it is auto-selected silently after the async fetch |
| **Priority** | P0 |

### Preconditions
- Dash Core running with **exactly one** wallet loaded
- A DET wallet with `core_wallet_name = NULL` in SQLite

### Steps
1. Navigate to the Wallets screen
2. Trigger any wallet RPC operation (e.g., Refresh)
3. Wait for the `CoreWalletNotConfigured` error and the subsequent async wallet list fetch

### Expected Results
- No SelectionDialog modal appears
- A success banner is shown: "Auto-selected Core wallet '<name>' — refreshing wallet. If you were
  performing another operation, please retry it."
- `core_wallet_name` is persisted to SQLite for this specific wallet
- The Wallets screen refreshes and RPC operations succeed using `/wallet/<name>`

---

## ACW-009: Wallets screen -- SelectionDialog appears after async fetch with multiple Core wallets

| Field | Value |
|---|---|
| **ID** | ACW-009 |
| **Title** | SelectionDialog appears after async list when multiple Core wallets are loaded |
| **Priority** | P0 |

### Preconditions
- Dash Core running with **two or more** wallets loaded
- A DET wallet with `core_wallet_name = NULL` in SQLite

### Steps
1. Navigate to the Wallets screen
2. Trigger a wallet RPC operation that causes a `CoreWalletNotConfigured` error
3. Wait for the async `ListCoreWallets` task to complete
4. Observe the SelectionDialog modal that appears
5. Select a wallet from the ComboBox
6. Click "Confirm"

### Expected Results
- Step 3-4: The SelectionDialog appears after the async task result arrives (not synchronously
  in the error handler) -- the UI was not blocked while the list was fetching
- Step 4: The dialog lists all wallet names from the async response
- Step 6: `core_wallet_name` is persisted; a success banner appears; the Wallets screen refreshes
- Subsequent RPC calls for this wallet use the selected `/wallet/<name>`

---

## ACW-010: Wallets screen -- no Core wallets loaded shows error banner

| Field | Value |
|---|---|
| **ID** | ACW-010 |
| **Title** | An error banner appears when the async fetch returns an empty wallet list |
| **Priority** | P1 |

### Preconditions
- Dash Core running with **no wallets loaded** (or all wallets unloaded after startup)
- A DET wallet with `core_wallet_name = NULL` in SQLite

### Steps
1. Navigate to the Wallets screen
2. Trigger a wallet RPC operation

### Expected Results
- The `CoreWalletNotConfigured` error is received, triggering the async fetch
- The async `ListCoreWallets` task returns an empty list
- An error banner appears: "No wallets loaded in Dash Core"
- No SelectionDialog modal is shown
- No `core_wallet_name` is persisted to SQLite

---

## ACW-011: Error recovery -- failed fetch stops retry loop, user navigates away and back to retry

| Field | Value |
|---|---|
| **ID** | ACW-011 |
| **Title** | Failed ListCoreWallets sets an empty list to prevent infinite retry; user can retry by reopening the screen |
| **Priority** | P1 |

### Preconditions
- Dash Core RPC is **unreachable or returns an error** when `listwallets` is called
- Create Wallet or Import Wallet screen is open

### Steps
1. Open the Create Wallet screen while Core RPC is unavailable
2. Wait several seconds for the async wallet list fetch to attempt and fail
3. Observe the screen -- no repeated RPC calls are made
4. Restore Core RPC connectivity
5. Navigate away (back to the Wallets screen) and reopen "Create Wallet"

### Expected Results
- Step 2: The fetch fails; an error banner is shown; `core_wallets` is set to an empty list
  (`Some(vec![])`) which stops the dispatch guard from re-firing
- Step 3: No infinite retry loop occurs -- the screen remains stable with no further
  `ListCoreWallets` tasks dispatched
- Step 5: Reopening the screen creates a fresh instance with `core_wallets = None`, which
  triggers a new `ListCoreWallets` fetch. After it succeeds, the ComboBox appears if multiple
  Core wallets are loaded (ACW-002 behavior) or the form proceeds without it if zero/one wallet
  exists (ACW-003 behavior)

---

## ACW-012: Network switch -- Create Wallet screen retains stale Core wallet list

| Field | Value |
|---|---|
| **ID** | ACW-012 |
| **Title** | Create Wallet modal screen retains stale Core wallet data after network switch because change_network() only iterates main_screens |
| **Priority** | P2 |

### Preconditions
- Application configured with at least two networks (e.g., Mainnet and Testnet) each backed by
  a different Dash Core instance
- Mainnet Core has wallets: `mainnet_a`, `mainnet_b`
- Testnet Core has wallets: `testnet_x`
- The Create Wallet screen is open on Mainnet

### Steps
1. Navigate to "Create Wallet" on Mainnet
2. Wait for the async wallet list to arrive -- observe the ComboBox listing `mainnet_a`,
   `mainnet_b`
3. Switch to Testnet using the network selector
4. Observe the Create Wallet screen

### Expected Results
- The Create Wallet screen is a modal screen on `screen_stack`, not a root screen in
  `main_screens`
- `change_network()` only calls `change_context()` on `main_screens`, not on `screen_stack`
  items
- Although `Screen::AddNewWalletScreen` has a `change_context()` handler that calls
  `reset_core_wallets_cache()` (defined in `src/ui/mod.rs`), it is NOT invoked during a
  network switch
- Step 4: The stale Mainnet wallet list (`mainnet_a`, `mainnet_b`) remains visible in the
  ComboBox
- The user must dismiss the Create Wallet screen and reopen it on Testnet to get the correct
  wallet list

### Notes
This is a known limitation of the current architecture. Modal screens on `screen_stack` do not
receive `change_context()` calls during network switches. A future improvement could extend
`change_network()` to also iterate `screen_stack` items.

---

## ACW-013: Network switch -- Import Wallet screen retains stale Core wallet list

| Field | Value |
|---|---|
| **ID** | ACW-013 |
| **Title** | Import Wallet modal screen retains stale Core wallet data after network switch because change_network() only iterates main_screens |
| **Priority** | P2 |

### Preconditions
- Same as ACW-012

### Steps
1. Navigate to "Import Wallet" on Mainnet -- let the wallet list arrive (ComboBox shows
   `mainnet_a`, `mainnet_b`)
2. Switch to Testnet
3. Observe the Import Wallet screen

### Expected Results
- Same behavior as ACW-012: the Import Wallet screen is a modal on `screen_stack` and does not
  receive `change_context()` during network switches
- Step 3: The stale Mainnet wallet list remains visible in the ComboBox
- The user must dismiss and reopen the Import Wallet screen on Testnet to get the correct list

### Notes
Same architectural limitation as ACW-012. Both `AddNewWalletScreen` and `ImportMnemonicScreen`
have `change_context()` handlers with `reset_core_wallets_cache()`, but these handlers are only
reachable if `change_network()` iterates `screen_stack`, which it currently does not.

---

## ACW-014: Concurrent frames -- only one ListCoreWallets task is dispatched per screen open

| Field | Value |
|---|---|
| **ID** | ACW-014 |
| **Title** | Task is dispatched exactly once; subsequent frames do not re-dispatch while loading |
| **Priority** | P1 |

### Preconditions
- Dash Core RPC is functioning normally
- Create Wallet screen is open

### Steps
1. Open the Create Wallet screen
2. Interact with the UI during the first few frames while the wallet list is being fetched
   (e.g., click on the entropy grid)

### Expected Results
- The `ListCoreWallets` backend task is dispatched exactly once (on the first frame where
  `core_wallets.is_none() && !core_wallets_loading`)
- `core_wallets_loading` is set to `true` immediately on dispatch, preventing re-dispatch on
  subsequent frames
- After the result arrives, `core_wallets` is populated and no further tasks are dispatched
- No duplicate RPC calls to `listwallets` occur during normal use

---

## ACW-015: Task dispatched and ComboBox absent -- Save Wallet still works

| Field | Value |
|---|---|
| **ID** | ACW-015 |
| **Title** | User can save a wallet even if the Core wallet list fetch is still in flight |
| **Priority** | P2 |

### Preconditions
- Dash Core RPC is very slow or the user acts quickly before the result arrives
- Create Wallet screen is open; `core_wallets` is still `None`

### Steps
1. Navigate to "Create Wallet"
2. Rapidly complete all required steps and click "Save Wallet" before the async fetch completes

### Expected Results
- The wallet is saved successfully
- `core_wallet_name` is set to `None` in SQLite (since no ComboBox selection was made)
- No crash or error occurs due to the in-flight task
- Once the task completes, the `display_task_result()` receives the wallet list but this only
  updates the now-unused `core_wallets` field (the wallet has already been saved)

---

## ACW-016: Wallets screen -- pending_list_* state is cleaned up after task completes

| Field | Value |
|---|---|
| **ID** | ACW-016 |
| **Title** | Pending state fields are cleared after CoreWalletsList result is processed |
| **Priority** | P2 |

### Preconditions
- Dash Core running with any number of wallets
- A `CoreWalletNotConfigured` error has been triggered on the Wallets screen

### Steps
1. Trigger the `CoreWalletNotConfigured` path (see ACW-007)
2. Allow the `ListCoreWallets` task to complete (auto-select or dialog path)
3. Complete the Core wallet assignment
4. Trigger another RPC error on a different wallet (if applicable)

### Expected Results
- After the first `CoreWalletsList` result is processed:
  - `pending_list_wallet_hash` is `None` (consumed by `take()`)
  - `pending_list_is_single_key` is reset to `false`
  - `pending_list_core_wallets` is `false`
- A subsequent `CoreWalletNotConfigured` error for a different wallet correctly sets new values
  in these fields and dispatches a fresh task
- No stale state from the previous fetch interferes with the new one
