# Manual Test Scenarios: Multi-Wallet RPC Support

## Overview

When Dash Core (dash-qt) has multiple wallets loaded, wallet-specific RPC calls fail with error code -19 ("Wallet file not specified"). This feature handles Core wallet association at two levels:

1. **Wallet creation and import (inline selection):** When creating or importing a DET wallet, if Dash Core has more than one wallet loaded, an inline ComboBox labeled "Dash Core Wallet" appears in the form UI before the Save/Import button. The user selects the desired Core wallet before saving. The `core_wallet_name` is stored per-wallet in SQLite. If Dash Core has 0 or 1 wallets loaded, the ComboBox is not shown and `core_wallet_name` is set to `None`.

2. **Runtime recovery (modal dialog):** For legacy wallets created before the inline selection feature (with `NULL` `core_wallet_name` in the database), an RPC error -19 at runtime triggers a `SelectionDialog` modal in `app.rs`. The user selects a Core wallet, and the association is persisted to SQLite. If only one Core wallet is loaded, it is auto-selected without showing a dialog.

3. **Per-wallet storage:** Each DET wallet (HD or single-key) stores its own `core_wallet_name` in SQLite (`wallet` and `single_key_wallet` tables). Different DET wallets can be associated with different Core wallets. The `/wallet/<name>` URL path is used for all RPC calls for that wallet.

Key components:
- **Inline ComboBox** (`add_new_wallet_screen.rs`, `import_mnemonic_screen.rs`) -- appears in the creation/import form when Dash Core has >1 wallets loaded
- **SelectionDialog** (`src/ui/components/selection_dialog.rs`) -- modal dialog used only in `app.rs` for runtime recovery of legacy wallets hitting error -19
- **AppState** (`src/app.rs`) -- handles `CoreWalletSelectionNeeded` result for runtime recovery modal
- **SQLite persistence** (`src/database/wallet.rs`, `src/database/single_key_wallet.rs`) -- `core_wallet_name` column per wallet
- **`core_client_for_wallet()`** (`src/context/mod.rs`) -- builds RPC client with `/wallet/<name>` path
- **`check_wallet_not_specified()`** (`src/backend_task/core/mod.rs`) -- detects error -19 and triggers wallet selection

---

## MWT-001: Single Core wallet -- auto-select on first RPC call

| Field | Value |
|---|---|
| **ID** | MWT-001 |
| **Title** | Single Core wallet is auto-selected transparently |
| **Priority** | P0 |

### Prerequisites
- Dash Core running with exactly **one** wallet loaded
- A DET wallet with `core_wallet_name = NULL` in SQLite (legacy wallet or newly created when only 1 Core wallet existed)
- Application configured for any network with RPC backend

### Steps
1. Launch the application
2. Navigate to the Wallets screen
3. Trigger any wallet-related RPC operation (e.g., refresh balance)

### Expected Result
- The RPC call initially triggers error -19, but the application handles it transparently
- No SelectionDialog modal is shown to the user
- The application calls `listwallets`, discovers exactly one wallet, auto-selects it
- The original operation completes successfully
- `core_wallet_name` is persisted to SQLite in the `wallet` or `single_key_wallet` table for this specific wallet
- All subsequent RPC calls for this wallet use `/wallet/<name>` URL path without further prompts

---

## MWT-002: Multiple Core wallets -- runtime recovery via modal dialog

| Field | Value |
|---|---|
| **ID** | MWT-002 |
| **Title** | SelectionDialog modal appears at runtime when legacy wallet hits error -19 |
| **Priority** | P0 |

### Prerequisites
- Dash Core running with **two or more** wallets loaded (e.g., `wallet.dat`, `savings`, `trading`)
- A DET wallet with `core_wallet_name = NULL` in SQLite (legacy wallet)

### Steps
1. Launch the application
2. Trigger any wallet-related RPC operation for the legacy wallet
3. Observe the SelectionDialog modal that appears
4. Open the ComboBox dropdown and review the listed wallet names
5. Select a wallet (e.g., `savings`)
6. Click the "Confirm" button

### Expected Result
- Step 3: A modal overlay appears with a SelectionDialog titled "Select Dash Core Wallet"
- Step 4: The ComboBox lists all wallet names returned by `listwallets`
- Step 6: The dialog closes; the original RPC operation retries and succeeds
- `core_wallet_name=savings` is persisted to SQLite for this specific DET wallet
- All subsequent RPC calls for this wallet use `/wallet/savings`
- The selection persists across application restarts

---

## MWT-003: Multiple Core wallets -- user cancels runtime recovery dialog

| Field | Value |
|---|---|
| **ID** | MWT-003 |
| **Title** | Canceling the runtime SelectionDialog shows info banner |
| **Priority** | P0 |

### Prerequisites
- Dash Core running with two or more wallets loaded
- A DET wallet with `core_wallet_name = NULL` in SQLite

### Steps
1. Trigger a wallet-related RPC operation for the legacy wallet
2. Observe the SelectionDialog modal
3. Click the "Cancel" button

### Expected Result
- The dialog closes
- An info banner appears: "Dash Core wallet not selected. Some operations may fail until a wallet is assigned."
- The original RPC operation fails gracefully -- no crash or hang
- No `core_wallet_name` is persisted to SQLite
- Subsequent RPC operations for this wallet re-trigger error -19 and re-prompt the dialog

---

## MWT-004: DET wallet creation with single Core wallet

| Field | Value |
|---|---|
| **ID** | MWT-004 |
| **Title** | Creating a new DET wallet with single Core wallet works seamlessly |
| **Priority** | P0 |

### Prerequisites
- Dash Core running with exactly one wallet loaded (or no wallets loaded)
- No existing DET wallets in the application

### Steps
1. Navigate to Wallets screen and click "Create Wallet"
2. Complete the entropy grid, generate seed phrase, write it down
3. Enter a wallet name (e.g., "My Test Wallet")
4. Observe the form -- verify no "Dash Core Wallet" ComboBox is visible
5. Click "Save Wallet"
6. Observe the wallet creation success screen

### Expected Result
- Step 4: No inline Core wallet ComboBox is shown because Dash Core has 0 or 1 wallets loaded
- Step 5: The wallet is created successfully
- Step 6: The success screen shows "Wallet Created Successfully!" with next steps
- The wallet is saved to SQLite with `core_wallet_name = NULL`
- When a subsequent RPC call occurs and Dash Core has one wallet, it is auto-selected (MWT-001 behavior)

---

## MWT-005: DET wallet creation with multiple Core wallets

| Field | Value |
|---|---|
| **ID** | MWT-005 |
| **Title** | Inline ComboBox appears in creation form when multiple Core wallets exist |
| **Priority** | P0 |

### Prerequisites
- Dash Core running with two or more wallets loaded (e.g., `wallet.dat`, `savings`)
- No existing DET wallets in the application

### Steps
1. Navigate to Wallets screen and click "Create Wallet"
2. Complete the wallet creation form (entropy, seed phrase, name, password)
3. Observe the form UI before the "Save Wallet" button -- a ComboBox labeled "Dash Core Wallet" should be visible
4. Open the ComboBox dropdown and verify all Core wallet names are listed
5. Select the desired Core wallet (e.g., `savings`)
6. Click "Save Wallet"

### Expected Result
- Step 3: An inline ComboBox labeled "Dash Core Wallet" is visible in the form, before the Save button
- Step 4: The dropdown lists all wallet names returned by Dash Core's `listwallets` RPC
- Step 6: The wallet is created with `core_wallet_name=savings` already set in SQLite -- no post-creation modal dialog appears
- All subsequent RPC calls for this wallet use `/wallet/savings`
- The association persists across application restarts

---

## MWT-006: DET wallet import with multiple Core wallets

| Field | Value |
|---|---|
| **ID** | MWT-006 |
| **Title** | Inline ComboBox appears in import form when multiple Core wallets exist |
| **Priority** | P1 |

### Prerequisites
- Dash Core running with multiple wallets loaded
- A valid BIP39 seed phrase available for import

### Steps
1. Navigate to Wallets screen and click "Import Wallet"
2. Enter the seed phrase and complete other import fields
3. Observe the form UI before the "Save" / "Import" button -- a ComboBox labeled "Dash Core Wallet" should be visible
4. Open the ComboBox dropdown and verify all Core wallet names are listed
5. Select the appropriate Core wallet
6. Click Save / Import

### Expected Result
- Step 3: An inline ComboBox labeled "Dash Core Wallet" is visible in the import form, before the Save button
- Step 4: The dropdown lists all Core wallets
- Step 6: The wallet is imported with `core_wallet_name` already set in SQLite -- no post-import modal dialog
- The imported wallet's balance and transactions are fetched correctly via the selected Core wallet
- The association persists across application restarts

---

## MWT-007: Error -19 runtime recovery re-prompts selection

| Field | Value |
|---|---|
| **ID** | MWT-007 |
| **Title** | Re-prompts SelectionDialog when configured Core wallet is no longer available |
| **Priority** | P0 |

### Prerequisites
- A DET wallet with `core_wallet_name=old_wallet` in SQLite
- Dash Core initially had `old_wallet` loaded but it was unloaded mid-session
- At least one other wallet is now loaded in Core

### Steps
1. Confirm the application was working with `old_wallet` (RPC calls succeeding)
2. Unload `old_wallet` from Dash Core (via `dash-cli unloadwallet old_wallet`)
3. Trigger a wallet-related RPC operation in the application for that DET wallet

### Expected Result
- The RPC call to `/wallet/old_wallet` fails
- The application detects the failure and re-triggers the wallet detection flow
- If exactly one wallet remains loaded, it auto-selects (MWT-001 behavior)
- If multiple wallets remain, the SelectionDialog modal appears in `app.rs` (MWT-002 behavior)
- SQLite is updated with the newly selected `core_wallet_name`, replacing `old_wallet`
- Subsequent RPC calls for this wallet succeed with the new wallet

---

## MWT-008: Pre-configured Core wallet name in SQLite

| Field | Value |
|---|---|
| **ID** | MWT-008 |
| **Title** | Application uses per-wallet Core wallet name from SQLite without prompting |
| **Priority** | P1 |

### Prerequisites
- A DET wallet in SQLite with `core_wallet_name=savings`
- Dash Core running with the `savings` wallet loaded

### Steps
1. Launch the application
2. Navigate to the Wallets screen
3. Select the wallet that has `core_wallet_name=savings`
4. Trigger a wallet-related RPC operation

### Expected Result
- The RPC call uses `/wallet/savings` URL path from the start
- No error -19 occurs
- No SelectionDialog modal is shown and no inline ComboBox prompt appears
- The operation completes successfully on the first attempt

---

## MWT-009: SelectionDialog UI interactions (Escape, X button, Enter)

| Field | Value |
|---|---|
| **ID** | MWT-009 |
| **Title** | Runtime recovery SelectionDialog supports keyboard shortcuts and X close button |
| **Priority** | P1 |

### Prerequisites
- Dash Core running with two or more wallets loaded
- A legacy DET wallet with `core_wallet_name = NULL` in SQLite
- A wallet-related RPC operation triggers the runtime recovery SelectionDialog in `app.rs`

**Note:** This scenario applies only to the runtime recovery modal in `app.rs`, not to the inline ComboBox in wallet creation/import screens.

### Steps
1. Observe the dialog overlay -- verify it dims/blocks the background UI
2. Observe dialog contents: title, instruction message, ComboBox, Confirm/Cancel buttons
3. Press the **Escape** key
4. Re-trigger the dialog (perform another RPC operation)
5. Click the **X** (close) button on the dialog window
6. Re-trigger the dialog again
7. Use the ComboBox to select a wallet, then press **Enter**

### Expected Result
- Step 1: A semi-transparent overlay covers the main UI; only the dialog is interactive
- Step 2: Dialog shows "Select Dash Core Wallet" title, instruction text, ComboBox with wallet names, Confirm and Cancel buttons
- Step 3: Escape closes the dialog (equivalent to Cancel) -- info banner appears (MWT-003 behavior)
- Step 5: X button closes the dialog (equivalent to Cancel) -- info banner appears
- Step 7: Enter key confirms the current ComboBox selection -- wallet is selected and persisted to SQLite for this wallet

---

## MWT-010: Network switch -- independent wallet selection per network

| Field | Value |
|---|---|
| **ID** | MWT-010 |
| **Title** | Each DET wallet on each network maintains its own Core wallet name in SQLite |
| **Priority** | P1 |

### Prerequisites
- Application configured with both Testnet and Mainnet RPC backends
- Testnet Core has wallets: `testnet_wallet_1`, `testnet_wallet_2`
- Mainnet Core has wallets: `mainnet_wallet_A`, `mainnet_wallet_B`
- DET wallets exist on both networks with `core_wallet_name = NULL`

### Steps
1. Connect to Testnet
2. Create a new DET wallet -- observe the inline ComboBox, select `testnet_wallet_1`, save
3. Switch to Mainnet
4. Create a new DET wallet -- observe the inline ComboBox, select `mainnet_wallet_A`, save
5. Switch back to Testnet
6. Trigger a wallet-related RPC operation for the Testnet wallet

### Expected Result
- Step 2: Testnet wallet is saved with `core_wallet_name=testnet_wallet_1` in SQLite
- Step 4: A new inline ComboBox appears listing Mainnet wallets (not Testnet wallets); Mainnet wallet is saved with `core_wallet_name=mainnet_wallet_A`
- Step 5-6: No dialog or ComboBox appears; Testnet wallet uses `testnet_wallet_1` from SQLite
- Each DET wallet's `core_wallet_name` is stored independently in its own row in the `wallet` table

---

## MWT-011: Multiple DET wallets with different Core wallet associations

| Field | Value |
|---|---|
| **ID** | MWT-011 |
| **Title** | Different DET wallets can be associated with different Core wallets |
| **Priority** | P1 |

### Prerequisites
- Dash Core running with multiple wallets loaded (e.g., `personal`, `business`)
- Two or more DET wallets in the application

### Steps
1. Create DET Wallet A -- in the inline ComboBox, select `personal`, save
2. Create DET Wallet B -- in the inline ComboBox, select `business`, save
3. Select DET Wallet A in the UI and trigger an RPC operation (e.g., refresh balance)
4. Select DET Wallet B in the UI and trigger an RPC operation

### Expected Result
- Step 3: RPC calls for Wallet A use `/wallet/personal`
- Step 4: RPC calls for Wallet B use `/wallet/business`
- Each DET wallet has its own `core_wallet_name` stored in SQLite (per-wallet, not global)
- No SelectionDialog appears for either wallet
- Both wallets' balances and transactions are fetched correctly via their respective Core wallet endpoints

---

## MWT-012: Wallet name with special characters

| Field | Value |
|---|---|
| **ID** | MWT-012 |
| **Title** | Core wallet names with spaces, dots, and special characters are handled correctly |
| **Priority** | P2 |

### Prerequisites
- Dash Core running with wallets having special characters in names:
  - `my wallet.dat` (space and dot)
  - `test-wallet_2` (hyphen and underscore)
  - `wallet (backup)` (parentheses and space)

### Steps
1. Create a new DET wallet
2. Observe the inline ComboBox listing the Core wallets
3. Select `my wallet.dat`
4. Click "Save Wallet"
5. Verify the RPC call succeeds (e.g., balance refresh)
6. Restart the application
7. Trigger another wallet-related RPC operation for this wallet

### Expected Result
- Step 2: All wallet names including those with special characters are displayed correctly in the ComboBox
- Step 4: Wallet is saved with `core_wallet_name=my wallet.dat` in SQLite
- Step 5: The RPC URL path handles the wallet name correctly (e.g., `/wallet/my wallet.dat`)
- Step 7: After restart, the persisted name is read from SQLite correctly and used without re-prompting
- Names containing `/` or `..` are rejected by `core_client_for_wallet()` as invalid

---

## MWT-013: Dash Core not running -- no false wallet selection trigger

| Field | Value |
|---|---|
| **ID** | MWT-013 |
| **Title** | Connection errors do not trigger the wallet selection flow |
| **Priority** | P2 |

### Prerequisites
- Dash Core is **not running** or RPC endpoint is unreachable
- A DET wallet with `core_wallet_name = NULL` in SQLite

### Steps
1. Launch the application
2. Attempt a wallet-related RPC operation

### Expected Result
- A normal connection error is displayed (e.g., "Could not connect to Dash Core")
- The SelectionDialog is **not** triggered -- the wallet selection flow only activates on error code -19 ("Wallet file not specified")
- No `core_wallet_name` is persisted to SQLite

---

## MWT-014: Concurrent RPC calls during wallet detection

| Field | Value |
|---|---|
| **ID** | MWT-014 |
| **Title** | Only one SelectionDialog appears even with concurrent error -19 triggers |
| **Priority** | P2 |

### Prerequisites
- Dash Core running with multiple wallets loaded
- A legacy DET wallet with `core_wallet_name = NULL` in SQLite
- Multiple background tasks may trigger RPC calls simultaneously

### Steps
1. Trigger multiple wallet-related operations in quick succession (e.g., navigate to wallet screen which triggers refresh + balance check concurrently)

### Expected Result
- Only **one** SelectionDialog modal appears (not multiple stacked dialogs)
- The first error -19 triggers the detection; subsequent errors are handled after selection
- After selection, all pending and subsequent operations succeed
- No race condition causes duplicate SQLite writes or conflicting wallet names

---

## MWT-015: Legacy wallet without core_wallet_name triggers runtime modal

| Field | Value |
|---|---|
| **ID** | MWT-015 |
| **Title** | Legacy wallet with NULL core_wallet_name triggers runtime recovery modal on error -19 |
| **Priority** | P1 |

### Prerequisites
- A DET wallet created before the inline selection feature was added (has `core_wallet_name = NULL` in SQLite `wallet` or `single_key_wallet` table)
- Dash Core running with **two or more** wallets loaded
- The legacy wallet has never had a Core wallet associated

### Steps
1. Launch the application
2. Select the legacy DET wallet in the UI
3. Trigger an RPC operation that requires Core interaction (e.g., refresh balance, send payment, create asset lock)
4. Observe the SelectionDialog modal that appears (rendered by `app.rs` runtime recovery)
5. Select the desired Core wallet from the ComboBox dropdown
6. Click "Confirm"
7. Verify the original operation completes or can be retried successfully
8. Restart the application
9. Trigger another RPC operation for the same wallet

### Expected Result
- Step 3: The RPC call fails with error -19 ("Wallet file not specified")
- Step 4: `app.rs` receives `CoreWalletSelectionNeeded` result and shows a SelectionDialog modal with all loaded Core wallets
- Step 5: The ComboBox lists wallet names from `listwallets` RPC
- Step 6: The dialog closes; `core_wallet_name` is persisted to SQLite for this specific wallet; a success banner appears ("Dash Core wallet '<name>' assigned")
- Step 7: The screen refreshes and subsequent RPC operations succeed using `/wallet/<name>`
- Step 8-9: After restart, the wallet's `core_wallet_name` is loaded from SQLite and used automatically -- no modal appears

### Edge Cases
- If the legacy wallet is a single-key wallet, verify `single_key_wallet.core_wallet_name` is updated in SQLite
- If the user cancels the modal, the info banner appears and no association is stored; the next RPC operation re-triggers error -19
- If Dash Core is restarted and only 1 wallet remains, auto-selection occurs without showing the modal
