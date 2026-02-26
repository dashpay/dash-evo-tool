# Manual Test Scenarios: Address Nonce Column (PR #637)

**Feature:** Replace UTXOs and Total Received columns with a single Nonce column for Platform Payment accounts.
**File changed:** `src/ui/wallets/wallets_screen/address_table.rs`

---

## Preconditions

- Dash Evo Tool is built and running against Testnet (or a network with Platform enabled).
- At least one wallet is loaded that contains:
  - A **Core account** (BIP44 or BIP32) with at least one address that has received funds (non-zero UTXOs and Total Received).
  - A **Platform Payment account** with at least one address that has a known nonce value on Platform.
- The wallet balances screen is accessible from the main navigation.

---

## Test Scenario 1: Platform Payment Account Shows Nonce Column

**Objective:** Verify that when a Platform Payment account is selected, the table header displays "Nonce" instead of "UTXOs" and "Total Received (DASH)".

### Steps

1. Open the Wallets screen.
2. Select a wallet that has a Platform Payment account.
3. In the account selector, choose the **Platform Payment** account.
4. Observe the address table headers.

### Expected Results

- The table header row contains: **Address**, **Balance**, **Nonce**, **Type**, **Index**, **Derivation Path**.
- The columns "UTXOs" and "Total Received (DASH)" are **not** present.
- The "Nonce" header is rendered as a plain label (not a sortable button).

---

## Test Scenario 2: Core Account Shows UTXOs and Total Received Columns

**Objective:** Verify that non-Platform accounts retain the original two-column layout.

### Steps

1. Open the Wallets screen.
2. Select the same wallet (or any wallet with a Core account).
3. In the account selector, choose a **BIP44** (or BIP32) account.
4. Observe the address table headers.

### Expected Results

- The table header row contains: **Address**, **Balance**, **UTXOs**, **Total Received (DASH)**, **Type**, **Index**, **Derivation Path**.
- The "Nonce" column is **not** present.
- Both "UTXOs" and "Total Received (DASH)" headers are sortable buttons that toggle ascending/descending indicators (^ / v) when clicked.

---

## Test Scenario 3: Switching Between Account Types Updates Columns

**Objective:** Verify that switching between a Platform Payment account and a Core account dynamically updates the table columns.

### Steps

1. Open the Wallets screen and select a wallet.
2. Choose the **Platform Payment** account.
3. Confirm the table shows the "Nonce" column (per Scenario 1).
4. Switch to a **BIP44** Core account.
5. Confirm the table shows "UTXOs" and "Total Received (DASH)" columns (per Scenario 2).
6. Switch back to the **Platform Payment** account.
7. Confirm the table shows the "Nonce" column again.

### Expected Results

- Each switch correctly replaces the columns without visual glitches, overlapping headers, or missing columns.
- The remaining columns (Address, Balance, Type, Index, Derivation Path) remain unchanged across switches.
- No crash or panic occurs during rapid switching.

---

## Test Scenario 4: Nonce Value Accuracy

**Objective:** Verify that the nonce displayed in the table matches the actual nonce from Platform.

### Steps

1. Open the Wallets screen and select the **Platform Payment** account.
2. Identify an address that has been used for Platform state transitions (non-zero nonce).
3. Note the nonce value displayed in the table for that address.
4. Independently verify the nonce using the Dash Platform API or another tool (e.g., `get_platform_address_info` output, or a Platform explorer if available).

### Expected Results

- The nonce value in the table matches the value returned by `get_platform_address_info()` for that address.
- Addresses that have never been used for Platform transactions show a nonce of **0**.

---

## Test Scenario 5: Nonce Display for Unused Platform Payment Addresses

**Objective:** Verify that Platform Payment addresses with no transaction history display nonce as 0.

### Steps

1. Open the Wallets screen and select the **Platform Payment** account.
2. Locate an address that has never been used (no Platform state transitions).
3. Observe the nonce value.

### Expected Results

- The nonce column shows **0** for unused addresses (the `unwrap_or_default()` fallback).

---

## Test Scenario 6: Table Layout and Alignment

**Objective:** Verify that the table renders correctly with proper alignment for both account types.

### Steps

1. Open the Wallets screen.
2. Select the **Platform Payment** account and observe the table layout.
3. Resize the application window to a narrow width.
4. Resize the application window to a wide width.
5. Switch to a **Core (BIP44)** account and repeat steps 3-4.

### Expected Results

- For Platform Payment accounts: the table has 6 columns (Address, Balance, Nonce, Type, Index, Derivation Path). Column widths are reasonable -- Nonce column starts at approximately 80px.
- For Core accounts: the table has 7 columns (Address, Balance, UTXOs, Total Received, Type, Index, Derivation Path). UTXOs starts at approximately 70px, Total Received at approximately 150px.
- All columns are resizable.
- Text does not overflow or clip unexpectedly at reasonable window sizes.
- Cell content is left-aligned and vertically centered.

---

## Test Scenario 7: Core Account Row Values -- Key-Only Addresses

**Objective:** Verify that key-only addresses in Core accounts display "N/A" for UTXOs and Total Received (regression check for behavior change in the diff).

### Steps

1. Open the Wallets screen and select a Core account that contains key-only addresses (e.g., Identity Registration, Identity Topup, or other special-purpose accounts).
2. Observe the UTXOs and Total Received columns for key-only addresses.

### Expected Results

- Key-only addresses show **"N/A"** in both the UTXOs and Total Received columns.
- Non-key-only addresses show numeric values (UTXO count and DASH amount formatted to 8 decimal places).

---

## Test Scenario 8: Sorting Behavior After Account Switch

**Objective:** Verify that sort state behaves correctly when switching between account types with different column layouts.

### Steps

1. Select a **Core (BIP44)** account.
2. Click the "UTXOs" column header to sort by UTXOs (ascending).
3. Confirm the sort indicator shows "UTXOs ^".
4. Switch to the **Platform Payment** account.
5. Observe the table (the "UTXOs" column should no longer be visible).
6. Switch back to the **Core (BIP44)** account.
7. Observe the sort indicator on the UTXOs column.

### Expected Results

- After switching back, the sort state (column and direction) is preserved or reset gracefully -- no crash or incorrect column association.
- The sort indicator is either maintained (showing "UTXOs ^") or reset to the default sort. Either is acceptable; it must not cause a panic or display artifacts.

---

## Edge Cases

| Case | Expected Behavior |
|------|-------------------|
| Wallet with only Platform Payment accounts (no Core accounts) | Table always shows the Nonce column layout; no way to trigger UTXOs/Total Received columns. |
| Wallet with only Core accounts (no Platform Payment account) | Table always shows UTXOs and Total Received; Nonce column never appears. |
| Platform address info unavailable (network error during fetch) | Nonce defaults to 0 (via `unwrap_or_default()`). No crash. |
| Very large nonce value (u32::MAX = 4,294,967,295) | Displays as a plain integer without formatting issues. |
| No wallet loaded / no account selected | `selected_account` is `None`, `is_platform_account` evaluates to `false`, standard columns shown. |
