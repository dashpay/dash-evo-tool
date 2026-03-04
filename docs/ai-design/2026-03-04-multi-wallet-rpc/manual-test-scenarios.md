# Manual Test Scenarios: Multi-Wallet RPC Support

## Overview

When Dash Core (dash-qt) has multiple wallets loaded, wallet-specific RPC calls fail with error code -19 ("Wallet file not specified"). This feature:

1. Detects the error on first occurrence
2. Calls `listwallets` RPC to enumerate loaded wallets
3. Auto-selects if only one wallet exists, or presents a `SelectionDialog` for the user to choose
4. Persists the selected wallet name to `.env` as `{PREFIX}_core_wallet_name`
5. Uses the `/wallet/<name>` URL path for all subsequent RPC calls
6. On subsequent error -19 (e.g., wallet removed from Core), re-triggers the detection flow

Key components:
- `SelectionDialog` (`src/ui/components/selection_dialog.rs`) -- modal ComboBox dialog with Confirm/Cancel, Escape/Enter/X support
- `AppState` (`src/app.rs`) -- owns `core_wallet_dialog` and `core_wallet_names`, handles `CoreWalletSelectionNeeded` result
- `AppContext` (`src/context/mod.rs`) -- `set_core_wallet_name()` persists to `.env` and reinits RPC client
- `core_rpc_url()` (`src/backend_task/core/mod.rs`) -- appends `/wallet/<name>` when configured
- `NetworkConfig.core_wallet_name` (`src/config.rs`) -- per-network config field

---

## MWT-001: Single Core wallet -- auto-select on first RPC call

| Field | Value |
|---|---|
| **ID** | MWT-001 |
| **Title** | Single Core wallet is auto-selected transparently |
| **Priority** | P0 |

### Prerequisites
- Dash Core running with exactly **one** wallet loaded
- No `{PREFIX}_core_wallet_name` set in `.env`
- Application configured for any network with RPC backend

### Steps
1. Launch the application
2. Navigate to the Wallets screen
3. Trigger any wallet-related RPC operation (e.g., refresh balance)

### Expected Result
- The RPC call initially triggers error -19, but the application handles it transparently
- No `SelectionDialog` is shown to the user
- The application calls `listwallets`, discovers exactly one wallet, auto-selects it
- The original operation completes successfully
- `{PREFIX}_core_wallet_name=<wallet_name>` is persisted to `.env`
- All subsequent RPC calls use `/wallet/<name>` URL path without further prompts

---

## MWT-002: Multiple Core wallets -- user selects via dialog

| Field | Value |
|---|---|
| **ID** | MWT-002 |
| **Title** | SelectionDialog appears when multiple Core wallets exist |
| **Priority** | P0 |

### Prerequisites
- Dash Core running with **two or more** wallets loaded (e.g., `wallet.dat`, `savings`, `trading`)
- No `{PREFIX}_core_wallet_name` set in `.env`

### Steps
1. Launch the application
2. Trigger any wallet-related RPC operation
3. Observe the SelectionDialog that appears
4. Open the ComboBox dropdown and review the listed wallet names
5. Select a wallet (e.g., `savings`)
6. Click the "Confirm" button

### Expected Result
- Step 3: A modal overlay appears with a SelectionDialog titled "Select Dash Core Wallet"
- Step 4: The ComboBox lists all wallet names returned by `listwallets`
- Step 6: The dialog closes; the original RPC operation retries and succeeds
- `{PREFIX}_core_wallet_name=savings` is persisted to `.env`
- All subsequent RPC calls use `/wallet/savings`
- The selection persists across application restarts

---

## MWT-003: Multiple Core wallets -- user cancels selection

| Field | Value |
|---|---|
| **ID** | MWT-003 |
| **Title** | Canceling the SelectionDialog shows info banner |
| **Priority** | P0 |

### Prerequisites
- Dash Core running with two or more wallets loaded
- No `{PREFIX}_core_wallet_name` set in `.env`

### Steps
1. Trigger a wallet-related RPC operation
2. Observe the SelectionDialog
3. Click the "Cancel" button

### Expected Result
- The dialog closes
- An info banner appears: "Dash Core wallet not selected. Set manually in .env with {NETWORK}_core_wallet_name=<wallet>"
- The original RPC operation fails gracefully -- no crash or hang
- No wallet name is persisted to `.env`
- Subsequent RPC operations re-trigger error -19 and re-prompt the dialog

---

## MWT-004: DET wallet creation with single Core wallet

| Field | Value |
|---|---|
| **ID** | MWT-004 |
| **Title** | Creating a new DET wallet with single Core wallet works seamlessly |
| **Priority** | P0 |

### Prerequisites
- Dash Core running with exactly one wallet loaded
- No existing DET wallets in the application

### Steps
1. Navigate to Wallets screen and click "Create Wallet"
2. Complete the entropy grid, generate seed phrase, write it down
3. Enter a wallet name (e.g., "My Test Wallet")
4. Click "Save Wallet"
5. Observe the wallet creation success screen

### Expected Result
- The wallet is created successfully without any Core wallet selection dialog
- The Core wallet is auto-selected in the background (MWT-001 behavior)
- The success screen shows "Wallet Created Successfully!" with next steps
- The receive address is displayed and QR code can be generated
- `{PREFIX}_core_wallet_name` is persisted to `.env`

---

## MWT-005: DET wallet creation with multiple Core wallets

| Field | Value |
|---|---|
| **ID** | MWT-005 |
| **Title** | Creating a new DET wallet triggers Core wallet selection when multiple exist |
| **Priority** | P0 |

### Prerequisites
- Dash Core running with two or more wallets loaded
- No `{PREFIX}_core_wallet_name` set in `.env`

### Steps
1. Navigate to Wallets screen and click "Create Wallet"
2. Complete the wallet creation form (entropy, seed phrase, name, password)
3. Click "Save Wallet"
4. When the SelectionDialog appears, select the desired Core wallet
5. Click "Confirm"

### Expected Result
- The wallet creation succeeds locally (seed stored in SQLite)
- When a subsequent RPC call is made (e.g., balance refresh), error -19 triggers the SelectionDialog
- After selecting a Core wallet, the DET wallet connects to the correct Core wallet for RPC operations
- The selected Core wallet name is persisted to `.env`

---

## MWT-006: DET wallet import triggers Core wallet selection

| Field | Value |
|---|---|
| **ID** | MWT-006 |
| **Title** | Importing a wallet by seed phrase works with multi-wallet Core |
| **Priority** | P1 |

### Prerequisites
- Dash Core running with multiple wallets loaded
- No `{PREFIX}_core_wallet_name` set in `.env`
- A valid BIP39 seed phrase available for import

### Steps
1. Navigate to Wallets screen and import a wallet using a seed phrase
2. Complete the import process
3. When a wallet-related RPC call triggers error -19, observe the SelectionDialog
4. Select the appropriate Core wallet and confirm

### Expected Result
- The wallet is imported successfully (local data stored in SQLite)
- The SelectionDialog appears on the first RPC call requiring Core interaction
- After selection, the imported wallet's balance and transactions are fetched correctly
- The Core wallet selection is persisted and not re-prompted

---

## MWT-007: Error -19 runtime recovery re-prompts selection

| Field | Value |
|---|---|
| **ID** | MWT-007 |
| **Title** | Re-prompts SelectionDialog when configured Core wallet is no longer available |
| **Priority** | P0 |

### Prerequisites
- Application running with `{PREFIX}_core_wallet_name=old_wallet` in `.env`
- Dash Core initially had `old_wallet` loaded but it was unloaded mid-session
- At least one other wallet is now loaded in Core

### Steps
1. Confirm the application was working with `old_wallet` (RPC calls succeeding)
2. Unload `old_wallet` from Dash Core (via `dash-cli unloadwallet old_wallet`)
3. Trigger a wallet-related RPC operation in the application

### Expected Result
- The RPC call to `/wallet/old_wallet` fails
- The application detects the failure and re-triggers the wallet detection flow
- If exactly one wallet remains loaded, it auto-selects (MWT-001 behavior)
- If multiple wallets remain, the SelectionDialog appears (MWT-002 behavior)
- `.env` is updated with the newly selected wallet name, replacing `old_wallet`
- Subsequent RPC calls succeed with the new wallet

---

## MWT-008: Pre-configured Core wallet name in .env

| Field | Value |
|---|---|
| **ID** | MWT-008 |
| **Title** | Application uses pre-configured wallet name without prompting |
| **Priority** | P1 |

### Prerequisites
- `.env` contains `TESTNET_core_wallet_name=savings`
- Dash Core running with the `savings` wallet loaded

### Steps
1. Launch the application
2. Navigate to the Wallets screen
3. Trigger a wallet-related RPC operation

### Expected Result
- The RPC call uses `/wallet/savings` URL path from the start
- No error -19 occurs
- No SelectionDialog is shown
- The operation completes successfully on the first attempt

---

## MWT-009: SelectionDialog UI interactions (Escape, X button, Enter)

| Field | Value |
|---|---|
| **ID** | MWT-009 |
| **Title** | SelectionDialog supports keyboard shortcuts and X close button |
| **Priority** | P1 |

### Prerequisites
- Dash Core running with two or more wallets loaded
- No `{PREFIX}_core_wallet_name` set in `.env`
- A wallet-related RPC operation triggers the SelectionDialog

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
- Step 7: Enter key confirms the current ComboBox selection -- wallet is selected and persisted

---

## MWT-010: Network switch -- independent wallet selection per network

| Field | Value |
|---|---|
| **ID** | MWT-010 |
| **Title** | Each network maintains its own Core wallet name independently |
| **Priority** | P1 |

### Prerequisites
- Application configured with both Testnet and Mainnet RPC backends
- Testnet Core has wallets: `testnet_wallet_1`, `testnet_wallet_2`
- Mainnet Core has wallets: `mainnet_wallet_A`, `mainnet_wallet_B`
- No `{PREFIX}_core_wallet_name` set in `.env` for either network

### Steps
1. Connect to Testnet
2. Trigger a wallet-related RPC operation
3. In the SelectionDialog, select `testnet_wallet_1` and confirm
4. Switch to Mainnet
5. Trigger a wallet-related RPC operation
6. In the SelectionDialog, select `mainnet_wallet_A` and confirm
7. Switch back to Testnet
8. Trigger a wallet-related RPC operation

### Expected Result
- Step 3: `TESTNET_core_wallet_name=testnet_wallet_1` is persisted to `.env`
- Step 5: A new SelectionDialog appears listing Mainnet wallets (not Testnet wallets)
- Step 6: `MAINNET_core_wallet_name=mainnet_wallet_A` is persisted to `.env`
- Step 7-8: No dialog appears; Testnet uses `testnet_wallet_1` from persisted config
- Both entries coexist in `.env`

---

## MWT-011: Multiple DET wallets share the same Core wallet selection

| Field | Value |
|---|---|
| **ID** | MWT-011 |
| **Title** | Core wallet selection applies globally to all DET wallets on the same network |
| **Priority** | P1 |

### Prerequisites
- Two or more DET wallets already created in the application
- Dash Core running with multiple wallets loaded
- `{PREFIX}_core_wallet_name` already set (e.g., `savings`)

### Steps
1. Select DET Wallet A in the UI
2. Trigger an RPC operation (e.g., refresh balance)
3. Select DET Wallet B in the UI
4. Trigger an RPC operation

### Expected Result
- Both DET wallets use the same Core wallet (`savings`) for RPC calls
- No SelectionDialog appears for either wallet
- The Core wallet name is a per-network setting, not per-DET-wallet
- Both wallets' balances and transactions are fetched correctly via the same Core RPC endpoint

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
- No `{PREFIX}_core_wallet_name` set in `.env`

### Steps
1. Trigger a wallet-related RPC operation
2. Observe the SelectionDialog ComboBox
3. Select `my wallet.dat`
4. Click "Confirm"
5. Verify the RPC call succeeds
6. Restart the application
7. Trigger another wallet-related RPC operation

### Expected Result
- Step 2: All wallet names including those with special characters are displayed correctly
- Step 4: Wallet name persisted as `{PREFIX}_core_wallet_name=my wallet.dat`
- Step 5: The RPC URL path handles the wallet name correctly (e.g., `/wallet/my wallet.dat`)
- Step 7: After restart, the persisted name is read correctly and used without re-prompting

---

## MWT-013: Dash Core not running -- no false wallet selection trigger

| Field | Value |
|---|---|
| **ID** | MWT-013 |
| **Title** | Connection errors do not trigger the wallet selection flow |
| **Priority** | P2 |

### Prerequisites
- Dash Core is **not running** or RPC endpoint is unreachable
- No `{PREFIX}_core_wallet_name` set in `.env`

### Steps
1. Launch the application
2. Attempt a wallet-related RPC operation

### Expected Result
- A normal connection error is displayed (e.g., "Could not connect to Dash Core")
- The SelectionDialog is **not** triggered -- the wallet selection flow only activates on error code -19
- No wallet name is persisted to `.env`

---

## MWT-014: Concurrent RPC calls during wallet detection

| Field | Value |
|---|---|
| **ID** | MWT-014 |
| **Title** | Only one SelectionDialog appears even with concurrent error -19 triggers |
| **Priority** | P2 |

### Prerequisites
- Dash Core running with multiple wallets loaded
- No `{PREFIX}_core_wallet_name` set in `.env`
- Multiple background tasks may trigger RPC calls simultaneously

### Steps
1. Trigger multiple wallet-related operations in quick succession (e.g., navigate to wallet screen which triggers refresh + balance check concurrently)

### Expected Result
- Only **one** SelectionDialog appears (not multiple stacked dialogs)
- The first error -19 triggers the detection; subsequent errors are handled after selection
- After selection, all pending and subsequent operations succeed
- No race condition causes duplicate `.env` writes or conflicting wallet names
