# Dash Evo Tool — Smoke Test Cases

Weekly smoke test specification for Dash Evo Tool, covering implemented user stories in SPV mode on testnet.

## Prerequisites

See [test-prerequisites.md](test-prerequisites.md) for environment setup, `.env` configuration, secrets file, and the bank wallet concept.

All mnemonic and address placeholders (e.g., `${BANK_MNEMONIC}`, `${TC_SND_ADDRESS_0}`) refer to values defined in `~/.secrets/det-qa-mnemonics.env`.

---

## Test Case Format

| Field | Description |
|-------|-------------|
| **Use Case ID** | Reference to user stories (e.g., WAL-001) |
| **Test Case ID** | Unique test identifier (e.g., TC-WAL-001-01) |
| **Short Description** | What the test verifies |
| **Pre-Conditions** | State required before test execution |
| **Test Steps** | Numbered steps to execute |
| **Test Data** | Specific inputs or values used |
| **Expected Result** | Observable outcome on success |
| **Post-Condition** | State after test completes; cleanup actions |

---

## Session Setup

Perform these steps before running any test cases. If running TC-NET-010-01 (onboarding), do that first, then continue from step 1 below.

### 1. Configure SPV Mode

1. Click **"Settings"** in the left sidebar.
2. Check the **"Developer mode"** checkbox.
3. Stay on **Settings (Network Chooser)** and verify the **"Connection Type"** dropdown appears in **"Connection Settings"**.
4. In **"Connection Settings"**, open the **"Connection Type"** dropdown and select **"SPV Client"**.
5. Wait for the connection status indicator (top bar) to turn green.

### 2. Import the Bank Wallet

1. Click **"Wallets"** in the left sidebar.
2. Click **"Import Wallet"** (top-right).
3. The mnemonic seed phrase import screen is shown by default.
4. Select the seed phrase length matching `${BANK_MNEMONIC}`.
5. Enter each word into the numbered fields (**"1:"**, **"2:"**, ...).
6. In **"Name:"**, type `Bank`.
7. In **"Optional Password:"**, enter `${BANK_PASSWORD}`.
8. Click **"Import Wallet"**.
9. Wait for **"Core balance:"** to appear (should show >= 10 tDASH).

### 3. Import the Send-Test Wallet

1. On the **"Wallets"** screen, click **"Import Wallet"** (top-right).
2. The mnemonic seed phrase import screen is shown by default. Select the correct length.
3. Enter each word of `${TC_SND_MNEMONIC}` into the numbered fields.
4. In **"Name:"**, type `Send Test`.
5. Leave **"Optional Password:"** empty.
6. Click **"Import Wallet"**.

### 4. Import the Identity-Test Wallet

1. On the **"Wallets"** screen, click **"Import Wallet"** (top-right).
2. The mnemonic seed phrase import screen is shown by default. Select the correct length.
3. Enter each word of `${TC_IDN_MNEMONIC}` into the numbered fields.
4. In **"Name:"**, type `Identity Test`.
5. Leave **"Optional Password:"** empty.
6. Click **"Import Wallet"**.

### 5. Fund Test Wallets from Bank

1. Select the **Bank** wallet on the **"Wallets"** screen.
2. If the wallet shows **"Unlock"** instead of **"Lock"**, click **"Unlock"**, enter `${BANK_PASSWORD}` in the popup, and click **"Unlock"**.
3. Click **"Send"**.
4. In the **"To:"** field (hint: *"Enter Dash address"*), enter `${TC_SND_ADDRESS_0}`.
5. In the **"Amount:"** field (hint: *"0.00"*), enter `1`.
6. The **"Fee:"** label displays the estimated fee.
7. Click **"Send"** in the form.
8. A **"Fee Confirmation Required"** dialog appears — confirm the transaction.
9. Wait for the transaction to appear in the **"Transactions"** section.
10. Click **"Send"** again.
11. In **"To:"**, enter `${TC_IDN_ADDRESS_0}`.
12. In **"Amount:"**, enter `3`.
13. Click **"Send"** in the form.
14. Confirm in the **"Fee Confirmation Required"** dialog.
15. Wait for both transactions to confirm.
16. Select the **Send Test** wallet — verify **"Core balance:"** shows ~1 tDASH.
17. Select the **Identity Test** wallet — verify **"Core balance:"** shows ~3 tDASH.

---

## Network and Settings (NET)

### TC-NET-010-01: Onboarding wizard

| Field | Value |
|-------|-------|
| **Use Case ID** | NET-010 |
| **Test Case ID** | TC-NET-010-01 |
| **Short Description** | Verify onboarding wizard on fresh start |
| **Pre-Conditions** | DET data directory removed (fresh start — see [test-prerequisites.md](test-prerequisites.md#fresh-start-for-onboarding-tests)); `.env` file in place |
| **Test Steps** | 1. Launch DET.<br>2. Verify the welcome screen appears with three cards: **"Create Wallet"**, **"Import Wallet"**, and **"Just Explore"**.<br>3. Click **"Just Explore"**.<br>4. Verify the app loads — the left sidebar shows navigation items (**"Wallets"**, **"Identities"**, **"Contracts"**, **"Tokens"**, **"Tools"**, **"Settings"**, etc.) and no wallet is selected. |
| **Test Data** | N/A |
| **Expected Result** | Welcome screen shows all three options. Clicking **"Just Explore"** enters the app without creating a wallet. |
| **Post-Condition** | App is in explore mode. Proceed to [Session Setup](#session-setup). |

### TC-NET-001-01: Verify testnet network selection

| Field | Value |
|-------|-------|
| **Use Case ID** | NET-001 |
| **Test Case ID** | TC-NET-001-01 |
| **Short Description** | Verify DET is on testnet and network options are visible |
| **Pre-Conditions** | DET running, SPV synced |
| **Test Steps** | 1. Click **"Settings"** in the left sidebar.<br>2. Locate the network chooser at the top of the settings screen.<br>3. Verify **"Testnet"** is the currently selected network.<br>4. Verify the other chooser entry listed by the documented config is **"Mainnet"**.<br>5. Do NOT switch networks. |
| **Test Data** | N/A |
| **Expected Result** | **"Testnet"** is selected. The chooser shows the documented configured networks: **"Mainnet"** and **"Testnet"**. |
| **Post-Condition** | Remain on Testnet. |

### TC-NET-004-01: Toggle theme

| Field | Value |
|-------|-------|
| **Use Case ID** | NET-004 |
| **Test Case ID** | TC-NET-004-01 |
| **Short Description** | Switch between light and dark themes |
| **Pre-Conditions** | DET running |
| **Test Steps** | 1. Click **"Settings"** in the left sidebar.<br>2. Locate the **"Theme:"** selector.<br>3. Select **"🌙 Dark"**.<br>4. Verify the UI updates immediately to dark colors (dark background, light text).<br>5. Select **"☀ Light"**.<br>6. Verify the UI updates immediately to light colors (light background, dark text). |
| **Test Data** | N/A |
| **Expected Result** | Theme changes apply immediately when a new option is selected. UI is readable in both modes. |
| **Post-Condition** | Set theme to preferred value. |

### TC-NET-005-01: Toggle Developer mode

| Field | Value |
|-------|-------|
| **Use Case ID** | NET-005 |
| **Test Case ID** | TC-NET-005-01 |
| **Short Description** | Enable and disable Developer mode |
| **Pre-Conditions** | DET running |
| **Test Steps** | 1. Click **"Settings"** in the left sidebar.<br>2. Locate the **"Developer mode"** checkbox.<br>3. If unchecked, check **"Developer mode"**.<br>4. Stay on **Settings (Network Chooser)** and locate **"Connection Settings"**.<br>5. Verify the **"Connection Type"** selector appears (shows **"SPV Client"** or **"Dash Core RPC"**).<br>6. Navigate to **"Wallets"** and verify additional developer UI appears (for example, address tables and refresh controls).<br>7. Return to **"Settings"**.<br>8. Uncheck **"Developer mode"**.<br>9. Verify the **"Connection Type"** selector is hidden on **Settings (Network Chooser)**.<br>10. Navigate to **"Wallets"** and verify the additional developer UI is hidden. |
| **Test Data** | N/A |
| **Expected Result** | Checking **"Developer mode"** immediately reveals advanced controls, including the **"Connection Type"** selector on **Settings (Network Chooser)** and additional Wallets developer UI. Unchecking it hides those elements. |
| **Post-Condition** | Re-enable **"Developer mode"** if needed for SPV backend selection, then leave **"Connection Type"** set to **"SPV Client"**. |

---

## Wallet Management (WAL)

### TC-WAL-013-01: View SPV sync status

| Field | Value |
|-------|-------|
| **Use Case ID** | WAL-013 |
| **Test Case ID** | TC-WAL-013-01 |
| **Short Description** | Verify SPV sync status indicator |
| **Pre-Conditions** | DET running in SPV mode on testnet |
| **Test Steps** | 1. Observe the connection status indicator in the top bar.<br>2. During initial sync, verify the indicator shows a syncing state (orange or magenta color).<br>3. After sync completes, verify the indicator turns green.<br>4. Verify a peer count is displayed (> 0). |
| **Test Data** | N/A |
| **Expected Result** | Status indicator transitions from syncing (orange/magenta) to connected (green). Peer count > 0. |
| **Post-Condition** | SPV fully synced. |

### TC-WAL-001-01: Create a new HD wallet

| Field | Value |
|-------|-------|
| **Use Case ID** | WAL-001 |
| **Test Case ID** | TC-WAL-001-01 |
| **Short Description** | Create a new wallet with generated mnemonic |
| **Pre-Conditions** | DET running, SPV synced |
| **Test Steps** | 1. Click **"Wallets"** in the left sidebar.<br>2. Click **"Create Wallet"** in the top-right corner.<br>3. Verify the breadcrumb reads **Wallets > Create Wallet**.<br>4. **Step 1 — Entropy:** Move the mouse around the entropy grid until the progress bar fills.<br>5. **Step 2 — Generate:** Under the heading *"Select your desired seed phrase language and word count and press Generate"*, select **English** and **24 words**, then click **"Generate"**.<br>6. **Step 3 — Record:** Under the heading *"Write down the passphrase on a piece of paper"*, write down all 24 displayed words.<br>7. Check **"I wrote it down"**.<br>8. **Step 4 — Name:** Under the heading *"Enter a wallet name"*, type `Created Wallet` in the **"Wallet Name:"** field.<br>9. **Step 5 — Password:** Under the heading *"Add a password"*, leave the **"Optional Password:"** field empty.<br>10. Click **"Save Wallet"**. |
| **Test Data** | Wallet name: `Created Wallet`, no password |
| **Expected Result** | Wallet appears on the **"Wallets"** screen. **"Core balance:"** shows `0.0000 DASH`. The generated mnemonic was 24 English words. |
| **Post-Condition** | `Created Wallet` exists with zero balance. |

### TC-WAL-002-01: Import wallet via mnemonic

| Field | Value |
|-------|-------|
| **Use Case ID** | WAL-002 |
| **Test Case ID** | TC-WAL-002-01 |
| **Short Description** | Import a wallet from a fixed test mnemonic |
| **Pre-Conditions** | DET running, SPV synced, `${TC_WAL_MNEMONIC}` available from `~/.secrets/det-qa-mnemonics.env` |
| **Test Steps** | 1. Click **"Wallets"** in the left sidebar.<br>2. Click **"Import Wallet"** in the top-right corner.<br>3. Verify the breadcrumb reads **Wallets > Import Wallet** and the heading reads *"Follow these steps to import your wallet."*<br>4. Verify the mnemonic seed phrase import screen is shown by default.<br>5. Under **"Select the seed phrase length"**, choose the word count matching `${TC_WAL_MNEMONIC}`.<br>6. Enter each word of `${TC_WAL_MNEMONIC}` into the numbered fields (**"1:"**, **"2:"**, ...).<br>7. In the **"Name:"** field, type `Imported Wallet`.<br>8. In the **"Optional Password:"** field, enter `${TC_WAL_PASSWORD}`.<br>9. Click **"Import Wallet"**.<br>10. Wait for sync to complete. |
| **Test Data** | Mnemonic: `${TC_WAL_MNEMONIC}`, Name: `Imported Wallet`, Password: `${TC_WAL_PASSWORD}` |
| **Expected Result** | Wallet appears on the **"Wallets"** screen as **Imported Wallet**. Balance syncs (may be `0.0000 DASH` if never funded). |
| **Post-Condition** | `Imported Wallet` exists and is password-protected. |

### TC-WAL-004-01: Switch between wallets

| Field | Value |
|-------|-------|
| **Use Case ID** | WAL-004 |
| **Test Case ID** | TC-WAL-004-01 |
| **Short Description** | Switch between multiple wallets on the Wallets screen |
| **Pre-Conditions** | At least two wallets exist (e.g., **Bank** and **Created Wallet**) |
| **Test Steps** | 1. Click **"Wallets"** in the left sidebar.<br>2. Click on the **Created Wallet** entry in the wallet list.<br>3. Verify **"Core balance:"** shows `0.0000 DASH`.<br>4. Click on the **Bank** entry in the wallet list.<br>5. Verify **"Core balance:"** shows >= 10 tDASH. |
| **Test Data** | N/A |
| **Expected Result** | Selecting a different wallet updates the displayed balance immediately. No restart required. |
| **Post-Condition** | **Bank** wallet is selected. |

### TC-WAL-005-01: Rename a wallet

| Field | Value |
|-------|-------|
| **Use Case ID** | WAL-005 |
| **Test Case ID** | TC-WAL-005-01 |
| **Short Description** | Rename the Created Wallet |
| **Pre-Conditions** | `Created Wallet` exists |
| **Test Steps** | 1. Click **"Wallets"** in the left sidebar.<br>2. Locate **Created Wallet** in the wallet list.<br>3. Click the **"Rename"** button next to it.<br>4. A **"Rename Wallet"** dialog appears with a text field (hint: *"Enter wallet name"*).<br>5. Clear the existing name and type `Renamed Wallet`.<br>6. Confirm the rename.<br>7. Verify the wallet list now shows **Renamed Wallet** instead of **Created Wallet**.<br>8. Quit and re-launch DET.<br>9. Click **"Wallets"** — verify the wallet is still named **Renamed Wallet**. |
| **Test Data** | New name: `Renamed Wallet` |
| **Expected Result** | Wallet name updates immediately in the list. Name persists after app restart. |
| **Post-Condition** | Wallet is now named `Renamed Wallet`. |

### TC-WAL-008-01: View wallet balances

| Field | Value |
|-------|-------|
| **Use Case ID** | WAL-008 |
| **Test Case ID** | TC-WAL-008-01 |
| **Short Description** | Verify balance display for the bank wallet |
| **Pre-Conditions** | **Bank** wallet imported and synced |
| **Test Steps** | 1. Click **"Wallets"** in the left sidebar.<br>2. Select the **Bank** wallet.<br>3. Verify **"Core balance:"** is displayed with a value >= 10 tDASH (format: `X.XXXX DASH`).<br>4. Verify **"Platform balance:"** is displayed (may show `0.0000 DASH` if no credits loaded). |
| **Test Data** | N/A |
| **Expected Result** | Both **"Core balance:"** and **"Platform balance:"** are visible with non-negative numeric values in `X.XXXX DASH` format. |
| **Post-Condition** | N/A |

### TC-WAL-010-01: Generate receive address

| Field | Value |
|-------|-------|
| **Use Case ID** | WAL-010 |
| **Test Case ID** | TC-WAL-010-01 |
| **Short Description** | Generate a receive address with QR code |
| **Pre-Conditions** | **Bank** wallet selected |
| **Test Steps** | 1. Click **"Wallets"** in the left sidebar.<br>2. Select the **Bank** wallet.<br>3. Click the **"Receive"** button.<br>4. Verify a Dash address is displayed (starts with `y` on testnet).<br>5. Verify a QR code is rendered alongside the address.<br>6. Click the copy button to copy the address to the clipboard.<br>7. Paste into a text editor — verify the pasted address matches the displayed address and starts with `y`. |
| **Test Data** | N/A |
| **Expected Result** | Address starts with `y`. QR code is visible. Address is copyable to clipboard. |
| **Post-Condition** | N/A |

### TC-WAL-016-01: View transaction history

| Field | Value |
|-------|-------|
| **Use Case ID** | WAL-016 |
| **Test Case ID** | TC-WAL-016-01 |
| **Short Description** | View transaction history for the bank wallet |
| **Pre-Conditions** | **Bank** wallet selected and synced; wallet has prior transactions (e.g., from Session Setup fund transfers) |
| **Test Steps** | 1. Click **"Wallets"** in the left sidebar.<br>2. Select the **Bank** wallet.<br>3. Scroll down to the **"Transactions"** section.<br>4. Verify a table of transactions is displayed.<br>5. Verify each row shows at least: amount and direction (sent/received). |
| **Test Data** | N/A |
| **Expected Result** | **"Transactions"** table is populated. Each entry shows amount and direction. |
| **Post-Condition** | N/A |

### TC-WAL-006-01: Lock and unlock wallet

| Field | Value |
|-------|-------|
| **Use Case ID** | WAL-006 |
| **Test Case ID** | TC-WAL-006-01 |
| **Short Description** | Lock a password-protected wallet and unlock it |
| **Pre-Conditions** | `Imported Wallet` exists with password `${TC_WAL_PASSWORD}` (from TC-WAL-002-01) |
| **Test Steps** | 1. Click **"Wallets"** in the left sidebar.<br>2. Select **Imported Wallet**.<br>3. Click the **"Lock"** button.<br>4. Verify the button label changes to **"Unlock"**.<br>5. Attempt to click **"Send"** — verify the action is blocked or requires unlocking first.<br>6. Click **"Unlock"**.<br>7. In the popup *"Enter password to unlock Imported Wallet:"*, type `${TC_WAL_PASSWORD}`.<br>8. Click the **"Unlock"** button in the popup.<br>9. Verify the button label changes back to **"Lock"**.<br>10. Verify **"Send"** is now available. |
| **Test Data** | Password: `${TC_WAL_PASSWORD}` |
| **Expected Result** | Locking blocks sensitive operations. Correct password unlocks the wallet. Button toggles between **"Lock"** and **"Unlock"**. |
| **Post-Condition** | Wallet is unlocked. |

### TC-WAL-007-01: Remove a wallet

| Field | Value |
|-------|-------|
| **Use Case ID** | WAL-007 |
| **Test Case ID** | TC-WAL-007-01 |
| **Short Description** | Remove the Renamed Wallet |
| **Pre-Conditions** | `Renamed Wallet` exists, has no funds (or funds have been moved out) |
| **Test Steps** | 1. Click **"Wallets"** in the left sidebar.<br>2. Locate **Renamed Wallet** in the wallet list.<br>3. Click the red **"Remove"** button next to it.<br>4. A **"Remove Wallet"** confirmation dialog appears.<br>5. Click the **"Remove"** button in the dialog to confirm.<br>6. Verify **Renamed Wallet** no longer appears in the wallet list. |
| **Test Data** | N/A |
| **Expected Result** | Confirmation dialog shown. After confirming, wallet is removed from the list. |
| **Post-Condition** | `Renamed Wallet` no longer exists. |

---

## Send and Receive (SND)

### TC-SND-003-01: Receive Dash with QR code

| Field | Value |
|-------|-------|
| **Use Case ID** | SND-003 |
| **Test Case ID** | TC-SND-003-01 |
| **Short Description** | Verify QR code generation for a receive address |
| **Pre-Conditions** | **Send Test** wallet selected |
| **Test Steps** | 1. Click **"Wallets"** in the left sidebar.<br>2. Select the **Send Test** wallet.<br>3. Click the **"Receive"** button.<br>4. Verify a QR code is displayed.<br>5. Verify a text address is shown alongside the QR code (starts with `y`).<br>6. Click the copy button.<br>7. Paste into a text editor — verify the pasted address matches the displayed address. |
| **Test Data** | N/A |
| **Expected Result** | QR code renders correctly. Text address matches QR content. Copy to clipboard works. |
| **Post-Condition** | N/A |

### TC-SND-005-01: See fee estimate before confirming

| Field | Value |
|-------|-------|
| **Use Case ID** | SND-005 |
| **Test Case ID** | TC-SND-005-01 |
| **Short Description** | Verify fee estimate is shown in the send confirmation dialog |
| **Pre-Conditions** | **Send Test** wallet selected with >= 1 tDASH |
| **Test Steps** | 1. Click **"Wallets"** in the left sidebar.<br>2. Select the **Send Test** wallet.<br>3. Click **"Send"**.<br>4. In the **"To:"** field (hint: *"Enter Dash address"*), enter `${BANK_ADDRESS_0}`.<br>5. In the **"Amount:"** field (hint: *"0.00"*), enter `0.01`.<br>6. Verify the **"Fee:"** label shows a non-zero fee estimate.<br>7. Click **"Send"** in the form.<br>8. A **"Fee Confirmation Required"** dialog appears.<br>9. Verify the dialog shows the amount, fee, and total deduction.<br>10. Cancel or close the dialog — do NOT confirm the send. |
| **Test Data** | Amount: `0.01` tDASH, Destination: `${BANK_ADDRESS_0}` |
| **Expected Result** | **"Fee:"** estimate and total deduction are clearly visible before confirming. |
| **Post-Condition** | No funds moved. |

### TC-SND-001-01: Send Dash to an address

| Field | Value |
|-------|-------|
| **Use Case ID** | SND-001 |
| **Test Case ID** | TC-SND-001-01 |
| **Short Description** | Send tDASH from the Send Test wallet to the bank wallet |
| **Pre-Conditions** | **Send Test** wallet selected with >= 0.5 tDASH |
| **Test Steps** | 1. Click **"Wallets"** in the left sidebar.<br>2. Select the **Send Test** wallet.<br>3. Note the current **"Core balance:"** value.<br>4. Click **"Send"**.<br>5. In the **"To:"** field (hint: *"Enter Dash address"*), enter `${BANK_ADDRESS_0}`.<br>6. In the **"Amount:"** field (hint: *"0.00"*), enter `0.1`.<br>7. Verify the **"Fee:"** label shows a fee estimate.<br>8. Click **"Send"** in the form.<br>9. A **"Fee Confirmation Required"** dialog appears — confirm the transaction.<br>10. Wait for the transaction to appear in the **"Transactions"** section.<br>11. Verify **"Core balance:"** decreased by approximately 0.1 tDASH plus the fee.<br>12. Select the **Bank** wallet.<br>13. Verify the **"Transactions"** section shows the incoming 0.1 tDASH transaction. |
| **Test Data** | Amount: `0.1` tDASH, Destination: `${BANK_ADDRESS_0}` |
| **Expected Result** | Transaction broadcasts. Sender balance decreases. Receiver shows the incoming transaction. |
| **Post-Condition** | 0.1 tDASH transferred from **Send Test** to **Bank**. |

---

## Asset Locks (ALK)

### TC-ALK-001-01: Create an asset lock

| Field | Value |
|-------|-------|
| **Use Case ID** | ALK-001 |
| **Test Case ID** | TC-ALK-001-01 |
| **Short Description** | Create an asset lock from the Identity Test wallet |
| **Pre-Conditions** | **Identity Test** wallet selected with >= 2 tDASH |
| **Test Steps** | 1. Click **"Wallets"** in the left sidebar.<br>2. Select the **Identity Test** wallet.<br>3. In the **"Asset Locks"** section, click **"Create Asset Lock"**.<br>4. Verify the breadcrumb/heading shows **Wallets > Create Asset Lock** / **"Create Asset Lock"**.<br>5. Under **"Select Asset Lock Purpose"**, click **"Registration"**.<br>6. In **"Select how much you would like to transfer?"**, enter `0.5` tDASH.<br>7. Review the generated funding address/QR code and the estimated fee.<br>8. Send funds from the **Identity Test** wallet to the displayed address if prompted, then wait through **"Waiting for funds..."** and **"Waiting for Core Chain to produce proof of asset lock..."**.<br>9. Verify **"Asset Lock Created Successfully!"** and wait for the asset lock transaction/proof to confirm. |
| **Test Data** | Amount: `0.5` tDASH |
| **Expected Result** | Asset lock created. Transaction ID is shown. **"Core balance:"** decreases by ~0.5 tDASH + fee. |
| **Post-Condition** | Asset lock available for identity registration in TC-IDN-001-01. |

---

## Identity Operations (IDN)

### TC-IDN-001-01: Register a new identity

| Field | Value |
|-------|-------|
| **Use Case ID** | IDN-001 |
| **Test Case ID** | TC-IDN-001-01 |
| **Short Description** | Register a new identity funded by asset lock |
| **Pre-Conditions** | **Identity Test** wallet selected; asset lock from TC-ALK-001-01 is confirmed |
| **Test Steps** | 1. Click **"Identities"** in the left sidebar.<br>2. Click **"Create Identity"** and verify the breadcrumb shows **Identities > Create Identity**.<br>3. Select the **Identity Test** wallet as the funding source (unlock it if prompted).<br>4. Review the generated identity keys and optional **"Alias:"** field.<br>5. Open the **"Select funding method"** combobox and choose **"Unused Asset Lock (recommended)"**.<br>6. Under **"Select an unused asset lock:"**, choose the asset lock created in TC-ALK-001-01 and click **"Select"**.<br>7. Verify the **"Estimated Fee:"** summary, then click **"Create Identity"**.<br>8. Wait through **"=> Waiting for Platform acknowledgement <="** until the success screen appears.<br>9. Verify the new identity appears in the identities list with an identity ID (Base58 string). |
| **Test Data** | Funding: asset lock from TC-ALK-001-01 |
| **Expected Result** | Identity registered. Identity ID displayed. Credits balance > 0. |
| **Post-Condition** | New identity available for DPNS, DashPay, and token tests. |

### TC-IDN-004-01: Top up identity credits

| Field | Value |
|-------|-------|
| **Use Case ID** | IDN-004 |
| **Test Case ID** | TC-IDN-004-01 |
| **Short Description** | Add credits to an existing identity |
| **Pre-Conditions** | Identity from TC-IDN-001-01 exists; **Identity Test** wallet has remaining funds |
| **Test Steps** | 1. Click **"Identities"** in the left sidebar.<br>2. Select the identity created in TC-IDN-001-01.<br>3. Open top-up and verify the breadcrumb shows **Top Up Identity**.<br>4. Under **"Choose your funding method"**, choose **"Wallet Balance"** or **"Unused Asset Locks"**, whichever is available.<br>5. Enter amount: `0.1` tDASH worth of credits.<br>6. Confirm the top-up transaction.<br>7. Wait for Platform confirmation.<br>8. Verify the identity's credit balance increased. |
| **Test Data** | Top-up amount: `0.1` tDASH |
| **Expected Result** | Credits increase. Transaction confirmed on Platform. |
| **Post-Condition** | Identity has sufficient credits for DPNS registration, DashPay profile, etc. |

### TC-IDN-008-01: View identity keys and details

| Field | Value |
|-------|-------|
| **Use Case ID** | IDN-008 |
| **Test Case ID** | TC-IDN-008-01 |
| **Short Description** | Inspect identity key list and details |
| **Pre-Conditions** | Identity from TC-IDN-001-01 exists |
| **Test Steps** | 1. Click **"Identities"** in the left sidebar.<br>2. Select the identity from TC-IDN-001-01.<br>3. Click the **"Keys"** button for that identity.<br>4. Verify a key list is displayed.<br>5. Verify each key shows: type, purpose (e.g., AUTHENTICATION), and status. |
| **Test Data** | N/A |
| **Expected Result** | At least one key listed with AUTHENTICATION purpose. Key type, purpose, and status are visible for each key. |
| **Post-Condition** | N/A |

### TC-IDN-009-01: Refresh identity state

| Field | Value |
|-------|-------|
| **Use Case ID** | IDN-009 |
| **Test Case ID** | TC-IDN-009-01 |
| **Short Description** | Manually refresh identity data from the network |
| **Pre-Conditions** | Identity from TC-IDN-001-01 exists |
| **Test Steps** | 1. Click **"Identities"** in the left sidebar.<br>2. Select the identity from TC-IDN-001-01.<br>3. Click the **"Refresh"** button in the top panel.<br>4. Wait for the refresh to complete.<br>5. Verify the credit balance and key state update without error. |
| **Test Data** | N/A |
| **Expected Result** | Identity data refreshed. Updated balance and keys reflected. No error messages. |
| **Post-Condition** | N/A |

---

## DPNS (DPN)

### TC-DPN-001-01: Register a DPNS username

| Field | Value |
|-------|-------|
| **Use Case ID** | DPN-001 |
| **Test Case ID** | TC-DPN-001-01 |
| **Short Description** | Register a unique DPNS username for the test identity |
| **Pre-Conditions** | Identity from TC-IDN-001-01 with sufficient credits |
| **Test Steps** | 1. Click **"Identities"** in the left sidebar.<br>2. Select the identity from TC-IDN-001-01.<br>3. Navigate to DPNS registration — the heading reads **"Register DPNS Name"**.<br>4. Enter a unique username: `detsmoke-{timestamp}` (e.g., `detsmoke-1710770000`).<br>5. Review the cost estimate.<br>6. Click **"Register Name"**.<br>7. Wait for Platform confirmation. |
| **Test Data** | Username: `detsmoke-{timestamp}` (use current Unix timestamp for uniqueness) |
| **Expected Result** | Username registered. Appears in the identity's owned names. Credits decreased by the registration cost. |
| **Post-Condition** | Identity has a DPNS username for DashPay tests. |

### TC-DPN-002-01: View owned usernames

| Field | Value |
|-------|-------|
| **Use Case ID** | DPN-002 |
| **Test Case ID** | TC-DPN-002-01 |
| **Short Description** | Verify registered username appears in the owned names list |
| **Pre-Conditions** | Username registered in TC-DPN-001-01 |
| **Test Steps** | 1. Click **"Tools"** in the left sidebar.<br>2. In the tools subscreen chooser, click **"DPNS"**.<br>3. In the DPNS subscreen chooser, click **"My usernames"** (the owned names view).<br>4. Verify the username from TC-DPN-001-01 is listed. |
| **Test Data** | N/A |
| **Expected Result** | The registered username appears in the owned names list. |
| **Post-Condition** | N/A |

### TC-DPN-003-01: View active name contests

| Field | Value |
|-------|-------|
| **Use Case ID** | DPN-003 |
| **Test Case ID** | TC-DPN-003-01 |
| **Short Description** | View the DPNS name contests screen |
| **Pre-Conditions** | DET running on testnet, SPV synced |
| **Test Steps** | 1. Click **"Tools"** in the left sidebar.<br>2. In the tools subscreen chooser, click **"DPNS"**.<br>3. Verify the **"Active contests"** view is selected.<br>4. Verify the screen loads without error.<br>5. If contests exist, verify each shows status and vote information. |
| **Test Data** | N/A |
| **Expected Result** | Contests screen loads. Any listed contests show status and vote counts. |
| **Post-Condition** | N/A |

---

## DashPay (DPY)

### TC-DPY-001-01: Create a DashPay profile

| Field | Value |
|-------|-------|
| **Use Case ID** | DPY-001 |
| **Test Case ID** | TC-DPY-001-01 |
| **Short Description** | Create a DashPay profile for the test identity |
| **Pre-Conditions** | Identity from TC-IDN-001-01 with DPNS username and sufficient credits |
| **Test Steps** | 1. Click **"Dashpay"** in the left sidebar.<br>2. Click **"My Profile"** in the DashPay subscreen chooser.<br>3. Click **"Refresh"** in the top panel and wait for the profile view to load.<br>4. If the **"No DashPay Profile"** card appears, click **"Create Profile"**.<br>5. In the **"Display Name:"** field (hint: *"Enter your display name (required)"*), type `Smoke Test Bot`.<br>6. In the **"Bio/Status:"** field, type `Automated QA testing`.<br>7. Leave the **"Avatar URL:"** field empty.<br>8. Click **"Save Profile"**.<br>9. Wait for Platform confirmation.<br>10. Verify the profile screen shows **Display Name:** `Smoke Test Bot` and **Bio/Status:** `Automated QA testing`. |
| **Test Data** | Display Name: `Smoke Test Bot`, Bio: `Automated QA testing` |
| **Expected Result** | Profile created. Display name and bio visible on the profile screen. |
| **Post-Condition** | Profile exists for search test. |

### TC-DPY-002-01: Search DashPay profiles

| Field | Value |
|-------|-------|
| **Use Case ID** | DPY-002 |
| **Test Case ID** | TC-DPY-002-01 |
| **Short Description** | Search for a known DashPay profile by username |
| **Pre-Conditions** | DashPay profile created in TC-DPY-001-01 |
| **Test Steps** | 1. Click **"Dashpay"** in the left sidebar.<br>2. Click **"Search Profiles"** in the DashPay subscreen chooser.<br>3. Locate the search field (hint: *"Enter DPNS username..."*).<br>4. Type the username registered in TC-DPN-001-01.<br>5. Verify search results appear.<br>6. Verify the result shows **Display Name:** `Smoke Test Bot` and **Bio/Status:** `Automated QA testing`. |
| **Test Data** | Search query: username from TC-DPN-001-01 |
| **Expected Result** | Profile found in search results. Display name and bio match what was set in TC-DPY-001-01. |
| **Post-Condition** | N/A |

---

## Token Operations (TOK)

### TC-TOK-001-01: View token balances

| Field | Value |
|-------|-------|
| **Use Case ID** | TOK-001 |
| **Test Case ID** | TC-TOK-001-01 |
| **Short Description** | View the My Tokens tab |
| **Pre-Conditions** | Identity from TC-IDN-001-01 exists (may hold 0 tokens) |
| **Test Steps** | 1. Click **"Tokens"** in the left sidebar.<br>2. Select the **"My Tokens"** tab.<br>3. Verify the screen loads without error.<br>4. If tokens are held, verify each shows a name and balance.<br>5. If no tokens are held, verify an empty state is displayed gracefully. |
| **Test Data** | N/A |
| **Expected Result** | **"My Tokens"** tab loads. Held tokens show name and balance, or a clean empty state is shown. |
| **Post-Condition** | N/A |

### TC-TOK-002-01: Search and discover tokens

| Field | Value |
|-------|-------|
| **Use Case ID** | TOK-002 |
| **Test Case ID** | TC-TOK-002-01 |
| **Short Description** | Search for tokens by keyword |
| **Pre-Conditions** | DET running on testnet, SPV synced |
| **Test Steps** | 1. Click **"Tokens"** in the left sidebar.<br>2. Select the **"Search Tokens"** tab.<br>3. Enter the keyword `test` in the search field.<br>4. Execute the search.<br>5. Verify results appear, or a "no results" message is shown.<br>6. If results appear, verify each entry shows a token name and metadata. |
| **Test Data** | Keyword: `test` |
| **Expected Result** | Search executes without error. Results or empty-state message displayed. |
| **Post-Condition** | N/A |

### TC-TOK-003-01: Add a token by contract ID

| Field | Value |
|-------|-------|
| **Use Case ID** | TOK-003 |
| **Test Case ID** | TC-TOK-003-01 |
| **Short Description** | Add a token using a contract or token ID |
| **Pre-Conditions** | DET running on testnet, a known token contract ID available |
| **Test Steps** | 1. Click **"Tokens"** in the left sidebar.<br>2. Click the **"Add Token"** button.<br>3. Verify the heading reads **"Add Token"**.<br>4. In the **"Contract or Token ID:"** field, enter a known testnet token contract ID.<br>5. Click **"Search"** and select the desired token if multiple results are shown.<br>6. Click **"Add Token"** and verify the **"Token Added Successfully"** screen appears.<br>7. Click **"Back to Tokens screen"**, open **"Search Tokens"**, and verify the added token/contract can be found by its name or ID. |
| **Test Data** | A valid token contract ID from testnet |
| **Expected Result** | Token is saved/tracked locally and searchable by name or ID. **"My Tokens"** only shows the token if one of the local identities holds a non-zero balance. |
| **Post-Condition** | Token available for viewing. |

---

## Contracts and Documents (DOC)

### TC-DOC-003-01: Import a contract by ID

| Field | Value |
|-------|-------|
| **Use Case ID** | DOC-003 |
| **Test Case ID** | TC-DOC-003-01 |
| **Short Description** | Import the DPNS system contract by ID |
| **Pre-Conditions** | DET running on testnet |
| **Test Steps** | 1. Click **"Contracts"** in the left sidebar.<br>2. Click the **"Add Contracts"** button.<br>3. Verify the breadcrumb reads **Contracts > Add Contracts**.<br>4. Enter the DPNS contract ID in the input field.<br>5. Confirm the import.<br>6. Verify the contract appears in the contracts list with its name and available document types. |
| **Test Data** | DPNS contract ID (system contract, available on testnet) |
| **Expected Result** | Contract imported. Shows contract name and document types in the list. |
| **Post-Condition** | DPNS contract available for document browsing. |

### TC-DOC-004-01: Query and browse documents

| Field | Value |
|-------|-------|
| **Use Case ID** | DOC-004 |
| **Test Case ID** | TC-DOC-004-01 |
| **Short Description** | Query documents from the imported DPNS contract |
| **Pre-Conditions** | DPNS contract imported from TC-DOC-003-01 |
| **Test Steps** | 1. Click **"Contracts"** in the left sidebar.<br>2. Select the DPNS contract from the list.<br>3. Select a document type (e.g., `domain`).<br>4. Execute the query.<br>5. Verify document results are displayed, each showing its properties. |
| **Test Data** | Contract: DPNS, Document type: `domain` |
| **Expected Result** | Query returns documents. Each document displays its properties (e.g., name, records). |
| **Post-Condition** | N/A |

---

## Developer and Power Tools (DEV)

### TC-DEV-005-01: View Platform info

| Field | Value |
|-------|-------|
| **Use Case ID** | DEV-005 |
| **Test Case ID** | TC-DEV-005-01 |
| **Short Description** | View Platform network status and info |
| **Pre-Conditions** | DET running on testnet, SPV synced |
| **Test Steps** | 1. Click **"Tools"** in the left sidebar.<br>2. If needed, click **"Platform info"** in the tools subscreen chooser.<br>3. Click **"Fetch Current Epoch Info"** and verify the result pane shows current epoch data (for example, epoch number and start time).<br>4. Click **"Fetch Total Credits on Platform"** and verify the result pane shows the total credits value.<br>5. Click **"Fetch Validator Set Info"** and verify the result pane shows validator set information. |
| **Test Data** | N/A |
| **Expected Result** | **"Platform Info"** screen loads and each fetch action returns current network data in the result pane without error. |
| **Post-Condition** | N/A |

---

## Session Cleanup

Perform these steps after all tests are complete to return funds and remove test wallets.

### 1. Return Funds from Send Test Wallet

1. Click **"Wallets"** in the left sidebar.
2. Select the **Send Test** wallet.
3. Click **"Send"**.
4. In the **"To:"** field, enter `${BANK_ADDRESS_0}`.
5. Click **"Max"** to send the entire remaining balance minus fee.
6. Click **"Send"** in the form.
7. Confirm in the **"Fee Confirmation Required"** dialog.
8. Wait for the transaction to confirm.

### 2. Return Funds from Identity Test Wallet

1. Select the **Identity Test** wallet.
2. If any **"Core balance:"** remains (> 0), click **"Send"**.
3. In the **"To:"** field, enter `${BANK_ADDRESS_0}`.
4. Click **"Max"**.
5. Click **"Send"** in the form.
6. Confirm in the **"Fee Confirmation Required"** dialog.
7. Wait for the transaction to confirm.

### 3. Remove Test Wallets

For each test wallet (**Send Test**, **Identity Test**, **Imported Wallet**, **Renamed Wallet** if it still exists):

1. Locate the wallet in the wallet list.
2. Click the red **"Remove"** button next to it.
3. In the **"Remove Wallet"** confirmation dialog, click **"Remove"**.

Only the **Bank** wallet should remain after cleanup.

> **Note:** Identities and DPNS names created on Platform persist on-chain. They do not need local cleanup but should use unique, timestamp-based names to avoid collisions with future runs.

---

## Recommended Execution Order

Execute tests in this order to build on each other and minimize setup/cleanup:

| # | Test Case | Description |
|---|-----------|-------------|
| — | **Pre-session** | Fresh start (only if testing onboarding) |
| 1 | TC-NET-010-01 | Onboarding wizard |
| — | **[Session Setup](#session-setup)** | Configure SPV, import wallets, fund test wallets |
| 2 | TC-WAL-013-01 | SPV sync status |
| 3 | TC-WAL-001-01 | Create wallet |
| 4 | TC-WAL-002-01 | Import wallet (`${TC_WAL_MNEMONIC}`) |
| 5 | TC-WAL-004-01 | Switch wallets |
| 6 | TC-WAL-005-01 | Rename wallet |
| 7 | TC-WAL-008-01 | View balances |
| 8 | TC-WAL-010-01 | Receive address |
| 9 | TC-WAL-016-01 | Transaction history |
| 10 | TC-WAL-006-01 | Lock/unlock wallet |
| 11 | TC-SND-003-01 | Receive with QR |
| 12 | TC-SND-005-01 | Fee estimate |
| 13 | TC-SND-001-01 | Send tDASH |
| 14 | TC-ALK-001-01 | Create asset lock |
| 15 | TC-IDN-001-01 | Register identity |
| 16 | TC-IDN-004-01 | Top up credits |
| 17 | TC-IDN-008-01 | View identity keys |
| 18 | TC-IDN-009-01 | Refresh identity |
| 19 | TC-DPN-001-01 | Register DPNS name |
| 20 | TC-DPN-002-01 | View owned names |
| 21 | TC-DPN-003-01 | View contests |
| 22 | TC-DPY-001-01 | Create DashPay profile |
| 23 | TC-DPY-002-01 | Search profiles |
| 24 | TC-TOK-001-01 | View tokens |
| 25 | TC-TOK-002-01 | Search tokens |
| 26 | TC-TOK-003-01 | Add token by ID |
| 27 | TC-DOC-003-01 | Import contract |
| 28 | TC-DOC-004-01 | Query documents |
| 29 | TC-DEV-005-01 | Platform info |
| 30 | TC-NET-001-01 | Network selection |
| 31 | TC-NET-004-01 | Theme toggle |
| 32 | TC-NET-005-01 | Developer mode |
| 33 | TC-WAL-007-01 | Remove wallet (cleanup) |
| — | **[Session Cleanup](#session-cleanup)** | Return funds to bank, remove test wallets |
