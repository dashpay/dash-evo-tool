# Wallet Screen Wireframe Mockups

ASCII wireframes for every key state of the redesigned wallet screen. These mockups show the desktop layout at a typical window width (1280px+). The left panel (Zone 1) is shown abbreviated.

---

## 1. Default View -- HD Wallet (Level 0, Alex's View)

This is what a user sees immediately upon selecting a wallet. No sections are expanded.

```
+------------------------------------------------------------------+
| DASH EVO TOOL    [HD] My Dash Wallet  1.2345 DASH  v   [Mainnet] |
+--------+---------------------------------------------------------+
|        |                                                         |
| Dash   |  My Dash Wallet                              [... menu] |
| Pay    |  ==================                                     |
|        |                                                         |
| Ident- |      1.2345 DASH                                        |
| ities  |      Total Balance                                      |
|        |                                                         |
| Con-   |  [Show breakdown v]                                     |
| tracts |                                                         |
|        |  -------------------------------------------------------+
| Tokens |                                                         |
|        |  [  Send  ]    [  Receive  ]    [  Refresh  ]           |
|>Wallet |                                                         |
|        |  -------------------------------------------------------+
| Tools  |                                                         |
|        |  Recent Transactions                                    |
| Set-   |  +---------+---------+----------+---------+----------+  |
| tings  |  | Date    | Type    | Amount   | Status  | TxID     |  |
|        |  +---------+---------+----------+---------+----------+  |
|        |  | Feb 10  |  v Recv | +0.5000  |  OK     | abc12..  |  |
|        |  | Feb 8   |  ^ Sent | -0.2500  |  OK     | def34..  |  |
|        |  | Feb 5   |  v Recv | +1.0000  |  ...    | ghi56..  |  |
|        |  +---------+---------+----------+---------+----------+  |
|        |  [Show All Transactions]                                |
|        |                                                         |
|        |  > Accounts & Addresses       4 accounts, 12 addresses  |
|        |                                                         |
+--------+---------------------------------------------------------+
```

Key points:
- Balance is the largest text element on the screen
- "Show breakdown" link is subtle -- Alex ignores it, Priya clicks it
- Transaction history is always visible (no longer gated behind developer mode)
- Accounts & Addresses section is collapsed, showing only a summary count
- Asset Locks and Platform Addresses sections are not visible at Level 0
- The overflow menu "..." provides: Rename, Remove, Lock/Unlock, Show Recovery Phrase

---

## 2. Expanded View -- HD Wallet (Level 1, Priya's View)

Priya has clicked "Show breakdown" and expanded the Accounts & Addresses section.

```
+------------------------------------------------------------------+
| DASH EVO TOOL    [HD] My Dash Wallet  1.2345 DASH  v   [Mainnet] |
+--------+---------------------------------------------------------+
|        |                                                         |
| ...    |  My Dash Wallet                              [... menu] |
| nav    |  ==================                                     |
|        |                                                         |
|>Wallet |      1.2345 DASH                                        |
|        |      Total Balance                                      |
|        |                                                         |
|        |      Core:     1.0000 DASH                              |
|        |      Platform: 0.2345 DASH                              |
|        |      [Hide breakdown ^]                                 |
|        |                                                         |
|        |  -------------------------------------------------------+
|        |                                                         |
|        |  [ Send ]  [ Receive ]  [ Refresh  [All (Auto) v] ]     |
|        |                                                         |
|        |  -------------------------------------------------------+
|        |                                                         |
|        |  Recent Transactions                       [Filter: All v]
|        |  +---------+---------+----------+------+---------+------+
|        |  | Date    | Type    | Amount   | Fee  | Status  | TxID |
|        |  +---------+---------+----------+------+---------+------+
|        |  | Feb 10  |  v Recv | +0.5000  | --   |  OK     |abc1..|
|        |  | Feb 8   |  ^ Sent | -0.2500  |0.0001|  OK     |def3..|
|        |  | Feb 5   |  v Recv | +1.0000  | --   |  1/6    |ghi5..|
|        |  +---------+---------+----------+------+---------+------+
|        |  [Show All Transactions]                                |
|        |                                                         |
|        |  v Accounts & Addresses                                 |
|        |  |                                                      |
|        |  | v Main Account                        1.0000 DASH    |
|        |  | +--------+--------+------+-------+------+---------+  |
|        |  | |Address |Balance |UTXOs |Received|Type |Actions  |  |
|        |  | +--------+--------+------+-------+------+---------+  |
|        |  | |Xo..e3m |0.5000  |  2   |0.5000 |Funds |[Key]   |  |
|        |  | |Xr..b2k |0.3000  |  1   |0.8000 |Funds |[Key]   |  |
|        |  | |Xp..c4n |0.2000  |  1   |0.2000 |Change|[Key]   |  |
|        |  | +--------+--------+------+-------+------+---------+  |
|        |  | [+ Add Receiving Address]                            |
|        |  |                                                      |
|        |  | v Platform Account                    0.2345 DASH    |
|        |  | +----------+----------+------+---------+             |
|        |  | |Address   |Balance   |Type  |Actions  |             |
|        |  | +----------+----------+------+---------+             |
|        |  | |tevo1..q2 |0.2345    |Pmt   |[Fund][Withdraw]|     |
|        |  | +----------+----------+------+---------+             |
|        |  | [+ New Platform Address]                             |
|        |  |                                                      |
|        |  | > CoinJoin                            0.0000 DASH    |
|        |  | > Identity Keys                       (keys only)    |
|        |  | > Masternode Voting                    (keys only)    |
|        |                                                         |
|        |  v Asset Locks                   [Create] [Search Used] |
|        |  +------------+---------+--------+--------+---------+   |
|        |  |TxID        |Amount   |IS Lock |Usable  |Actions  |   |
|        |  +------------+---------+--------+--------+---------+   |
|        |  |abc123..    |0.5000   |  Yes   |  Yes   |[View][Fund]||
|        |  +------------+---------+--------+--------+---------+   |
|        |                                                         |
+--------+---------------------------------------------------------+
```

Key points:
- Balance breakdown shows Core and Platform amounts separately
- Refresh mode selector appears next to the Refresh button (was dev-mode only)
- Transaction history gains Fee column, confirmation count, and filter dropdown
- Accounts section is expanded with per-account address tables
- Only accounts with balances or activity are shown expanded; others are collapsed
- Asset Locks section is now visible
- Platform addresses show Fund and Withdraw action buttons

---

## 3. Developer View -- HD Wallet (Level 2, Jordan's View)

Jordan has Developer Tools enabled in Settings. Viewing on Testnet.

```
+------------------------------------------------------------------+
| DASH EVO TOOL    [HD] Test Wallet  0.8000 tDASH  v   [Testnet]   |
+--------+---------------------------------------------------------+
|        |                                                         |
| ...    |  Test Wallet                                 [... menu] |
| nav    |  ==================                   [DEV]             |
|        |                                                         |
|>Wallet |      0.8000 tDASH                                       |
|        |      Total Balance                                      |
|        |                                                         |
|        |      Core:     0.5000 tDASH                             |
|        |      Platform: 0.3000 tDASH (300,000,000 credits)       |
|        |      Unconfirmed: +0.1000 tDASH (pending)               |
|        |                                                         |
|        |  -------------------------------------------------------+
|        |                                                         |
|        |  [Send] [Receive] [Refresh [Core Only v]] [Get Test Dash]|
|        |                                                         |
|        |  -------------------------------------------------------+
|        |                                                         |
|        |  Recent Transactions                    [Filter v] [CSV]|
|        |  +------+------+--------+----+------+------+-----------+|
|        |  |Date  |Type  |Amount  |Fee |Status|TxID  |           ||
|        |  +------+------+--------+----+------+------+-----------+|
|        |  | ...  | ...  | ...    |... | ...  | ...  |           ||
|        |  +------+------+--------+----+------+------+-----------+|
|        |                                                         |
|        |  v Accounts & Addresses        [Filter: Has Activity v] |
|        |  |                                                      |
|        |  | v Main Account                        0.5000 tDASH   |
|        |  | +------+-------+----+-------+----+---+--------+-----+|
|        |  | |Addr  |Bal    |UTXO|Recv   |Type|Idx|Path    |Key  ||
|        |  | +------+-------+----+-------+----+---+--------+-----+|
|        |  | |Xo..  |0.3000 | 2  |0.3000 |Fnd | 0 |m/44'/1| [V] ||
|        |  | |Xr..  |0.2000 | 1  |0.5000 |Fnd | 1 |m/44'/1| [V] ||
|        |  | +------+-------+----+-------+----+---+--------+-----+|
|        |  |                                                      |
|        |  | v Platform Account    0.3000 tDASH (300M credits)    |
|        |  | +----------+----------+----------+---------+         |
|        |  | |Address   |Credits   |DASH Equiv|Actions  |         |
|        |  | +----------+----------+----------+---------+         |
|        |  | |tevo1..q2 |300000000 |0.3000    |[F][W]   |         |
|        |  | +----------+----------+----------+---------+         |
|        |  |                                                      |
|        |                                                         |
|        |  v Asset Locks        [Create] [Bulk Create] [Search]   |
|        |  +----------+--------+--------+----------+--------+    |
|        |  |TxID      |DASH    |Credits |IS / Usabl|Actions |    |
|        |  +----------+--------+--------+----------+--------+    |
|        |  |abc12..   |0.5000  |500M    | Y / Y    |[V][F]  |    |
|        |  +----------+--------+--------+----------+--------+    |
|        |                                                         |
+--------+---------------------------------------------------------+
```

Key points:
- "[DEV]" badge next to wallet name indicates Developer Tools mode
- Platform balance shows both DASH and raw credits
- "Get Test Dash" faucet button appears in action bar (Testnet only)
- CSV export button on transaction history
- "Filter: Has Activity" toggle on accounts section
- Full derivation path column (Path) in address tables
- Asset locks show both DASH and credit amounts
- Bulk Create option for asset locks

---

## 4. SingleKey Wallet View (Level 0)

```
+------------------------------------------------------------------+
| DASH EVO TOOL    [SK] Cold Storage  0.8000 DASH  v   [Mainnet]   |
+--------+---------------------------------------------------------+
|        |                                                         |
| ...    |  Cold Storage                                [... menu] |
| nav    |  ==============                                         |
|        |  Imported Key                                           |
|        |                                                         |
|>Wallet |      0.8000 DASH                                        |
|        |      Total Balance                                      |
|        |                                                         |
|        |  Address: XoRQE8bHjEm3p7Yf2X9...cK4dF                   |
|        |  [Copy Address]                                         |
|        |                                                         |
|        |  -------------------------------------------------------+
|        |                                                         |
|        |  [  Send  ]    [  Receive  ]    [  Refresh  ]           |
|        |                                                         |
|        |  -------------------------------------------------------+
|        |                                                         |
|        |  Recent Transactions                                    |
|        |  +---------+---------+----------+---------+----------+  |
|        |  | Date    | Type    | Amount   | Status  | TxID     |  |
|        |  +---------+---------+----------+---------+----------+  |
|        |  | Feb 10  |  v Recv | +0.8000  |  OK     | xyz78..  |  |
|        |  +---------+---------+----------+---------+----------+  |
|        |                                                         |
|        |  > UTXOs (3)                                            |
|        |                                                         |
+--------+---------------------------------------------------------+
```

Key points:
- Type badge is "SK" (SingleKey) instead of "HD"
- "Imported Key" label below the wallet alias
- Single address shown inline in the balance header area (not in a table)
- Copy Address button next to the address
- No account categories, no asset locks, no Platform addresses
- UTXOs section is available as a collapsible section
- Same transaction history pattern as HD wallets

---

## 5. SingleKey Wallet -- Expanded UTXOs (Level 1)

```
|        |                                                         |
|        |  v UTXOs (3)                                            |
|        |  +-----------+----------+---------+                     |
|        |  | Amount    | Confirms | TxID    |                     |
|        |  +-----------+----------+---------+                     |
|        |  | 0.5000    |    42    | abc1..  |                     |
|        |  | 0.2000    |    15    | def3..  |                     |
|        |  | 0.1000    |     3    | ghi5..  |                     |
|        |  +-----------+----------+---------+                     |
|        |  Page 1 of 1                                            |
|        |                                                         |
```

---

## 6. No Wallets Loaded (Empty State)

```
+------------------------------------------------------------------+
| DASH EVO TOOL    [Select a Wallet v]                  [Mainnet]   |
+--------+---------------------------------------------------------+
|        |                                                         |
| ...    |                                                         |
| nav    |                                                         |
|        |                                                         |
|>Wallet |                   [Wallet Icon]                         |
|        |                                                         |
|        |               No Wallets Loaded                         |
|        |                                                         |
|        |     Create a new wallet to start holding and            |
|        |     transacting Dash, or import an existing             |
|        |     wallet using your recovery phrase.                  |
|        |                                                         |
|        |     [  Create Wallet  ]   [  Import Wallet  ]           |
|        |                                                         |
|        |                                                         |
+--------+---------------------------------------------------------+
```

---

## 7. Wallet Locked State

When a password-protected wallet is locked, the balance header shows a locked indicator and operations requiring the private key are disabled.

```
|        |                                                         |
|        |  My Dash Wallet                        [Lock] [... menu]|
|        |  ==================                                     |
|        |                                                         |
|        |      1.2345 DASH                                        |
|        |      Total Balance                           [Locked]   |
|        |                                                         |
|        |  -------------------------------------------------------+
|        |                                                         |
|        |  [  Send  ]    [  Receive  ]    [  Refresh  ]           |
|        |     (locked)                                            |
|        |                                                         |
```

- "Locked" badge appears next to the balance
- Send button shows "(locked)" subtitle and triggers unlock popup when clicked
- Receive and Refresh still work (they do not require the private key for basic operation)
- "View Key" buttons in address tables are disabled with tooltip "Unlock wallet to view keys"

---

## 8. Receive Dialog (Modal Overlay)

### Level 0 (Alex's View)

```
+----------------------------------------------+
|  Receive Dash                           [X]  |
|                                              |
|  [Core]  [Platform]                          |
|                                              |
|  +------------------+                        |
|  |                  |                        |
|  |   [QR CODE]      |                        |
|  |                  |                        |
|  +------------------+                        |
|                                              |
|  XoRQE8bHjEm3p7Yf2X9...cK4dF                |
|                                              |
|  [  Copy Address  ]  [  New Address  ]       |
|                                              |
+----------------------------------------------+
```

- Tabs for Core and Platform address types
- Single address displayed with QR code
- "New Address" generates the next unused address
- For Alex: Platform tab may be hidden if no Platform addresses exist

### Level 1 (Priya's View)

```
+----------------------------------------------+
|  Receive Dash                           [X]  |
|                                              |
|  [Core]  [Platform]                          |
|                                              |
|  Address: [XoRQE8bHjEm3p7Yf2X...cK4dF  v]   |
|                                              |
|  +------------------+                        |
|  |                  |                        |
|  |   [QR CODE]      |                        |
|  |                  |                        |
|  +------------------+                        |
|                                              |
|  XoRQE8bHjEm3p7Yf2X9...cK4dF                |
|  Balance: 0.0000 DASH | Path: m/44'/5'/0'/0/3|
|                                              |
|  [  Copy Address  ]  [  New Address  ]       |
|                                              |
+----------------------------------------------+
```

- Address selector dropdown to choose from existing addresses
- Derivation path shown below address (Level 1 and above)
- Balance of the selected address shown

---

## 9. Send Flow (Full Screen, Replaces Dialog)

### Step 1: Enter Details

```
+------------------------------------------------------------------+
| DASH EVO TOOL    [HD] My Dash Wallet  1.2345 DASH  v   [Mainnet] |
+--------+---------------------------------------------------------+
|        |                                                         |
| ...    |  Send Dash                                 [X Cancel]   |
| nav    |  =========                                              |
|        |                                                         |
|>Wallet |  From: [Core Wallet v]                                  |
|        |  Available: 1.0000 DASH                                 |
|        |                                                         |
|        |  To:   [________________________________]               |
|        |        Enter a Dash address or DPNS name                |
|        |                                                         |
|        |  Amount: [______________] DASH                          |
|        |          [x] Subtract fee from amount                   |
|        |                                                         |
|        |                                                         |
|        |  [  Continue  >>  ]                                     |
|        |                                                         |
+--------+---------------------------------------------------------+
```

- "From" selector: Core Wallet or Platform Addresses
- Address field auto-detects type (Core address, Platform address, DPNS name)
- Amount field with DASH label
- Checkbox for fee subtraction

### Step 2: Confirm Transaction

```
+------------------------------------------------------------------+
| DASH EVO TOOL    [HD] My Dash Wallet  1.2345 DASH  v   [Mainnet] |
+--------+---------------------------------------------------------+
|        |                                                         |
| ...    |  Confirm Send                           [<< Back]      |
| nav    |  ============                                           |
|        |                                                         |
|>Wallet |  +--------------------------------------------------+   |
|        |  |  Summary                                         |   |
|        |  |                                                  |   |
|        |  |  To:       XoRQE8bHjEm3p7Yf2X9...cK4dF          |   |
|        |  |  Amount:   0.5000 DASH                           |   |
|        |  |  Fee:      0.0001 DASH                           |   |
|        |  |  ----------------------------------------        |   |
|        |  |  Total:    0.5001 DASH                           |   |
|        |  |                                                  |   |
|        |  |  Remaining balance: 0.4999 DASH                  |   |
|        |  +--------------------------------------------------+   |
|        |                                                         |
|        |  [  << Back  ]              [  Confirm & Send  ]        |
|        |                                                         |
+--------+---------------------------------------------------------+
```

- Clear summary showing recipient, amount, fee, total, and remaining balance
- Back button to edit details
- Confirm button to broadcast

### Step 3: Result

```
|        |                                                         |
|        |  Transaction Sent                                       |
|        |  ================                                       |
|        |                                                         |
|        |      [Checkmark Icon]                                   |
|        |                                                         |
|        |  0.5000 DASH sent to XoRQE8bHjEm...                    |
|        |                                                         |
|        |  TxID: abc123def456...                    [Copy TxID]   |
|        |  Status: Confirmed (InstantSend)                        |
|        |                                                         |
|        |  [  Back to Wallet  ]                                   |
|        |                                                         |
```

---

## 10. Wallet Creation Flow (Key States)

### Step 1: Recovery Phrase Display

```
|        |                                                         |
|        |  Create Wallet - Step 1 of 3                            |
|        |  ===========================                            |
|        |                                                         |
|        |  Your Recovery Phrase                                   |
|        |                                                         |
|        |  Write down these 12 words in order.                    |
|        |  This is the ONLY way to recover your wallet.           |
|        |  Never share these words with anyone.                   |
|        |                                                         |
|        |  +------+------+------+------+                          |
|        |  | 1.   | 2.   | 3.   | 4.   |                         |
|        |  |word  |word  |word  |word  |                          |
|        |  +------+------+------+------+                          |
|        |  | 5.   | 6.   | 7.   | 8.   |                         |
|        |  |word  |word  |word  |word  |                          |
|        |  +------+------+------+------+                          |
|        |  | 9.   |10.   |11.   |12.   |                         |
|        |  |word  |word  |word  |word  |                          |
|        |  +------+------+------+------+                          |
|        |                                                         |
|        |  [  I Have Written These Down >>  ]                     |
|        |                                                         |
```

### Step 2: Backup Verification

```
|        |                                                         |
|        |  Create Wallet - Step 2 of 3                            |
|        |  ===========================                            |
|        |                                                         |
|        |  Verify Your Backup                                     |
|        |                                                         |
|        |  Enter the requested words to confirm                   |
|        |  you have written them down correctly.                  |
|        |                                                         |
|        |  Word #3:  [______________]                             |
|        |  Word #7:  [______________]                             |
|        |  Word #11: [______________]                             |
|        |                                                         |
|        |  [<< Back]           [  Continue >>  ]                  |
|        |                                                         |
```

### Step 3: Password and Alias

```
|        |                                                         |
|        |  Create Wallet - Step 3 of 3                            |
|        |  ===========================                            |
|        |                                                         |
|        |  Wallet Name                                            |
|        |  [My Dash Wallet____________]                           |
|        |                                                         |
|        |  Password (optional)                                    |
|        |  [________________________] [Show]                      |
|        |  Strength: [====--------] Medium                        |
|        |                                                         |
|        |  Confirm Password                                       |
|        |  [________________________]                             |
|        |                                                         |
|        |  [<< Back]           [  Create Wallet  ]                |
|        |                                                         |
```

---

## 11. Rename Dialog (Modal)

```
+----------------------------------------------+
|  Rename Wallet                          [X]  |
|                                              |
|  Current name: My Dash Wallet                |
|                                              |
|  New name: [My Main Wallet___________]       |
|            0-64 characters                   |
|                                              |
|  [  Cancel  ]         [  Save  ]             |
|                                              |
+----------------------------------------------+
```

---

## 12. Remove Wallet Confirmation (Modal)

```
+----------------------------------------------+
|  Remove Wallet                          [X]  |
|                                              |
|  Are you sure you want to remove             |
|  "My Dash Wallet"?                           |
|                                              |
|  This will delete:                           |
|  - All locally stored wallet data            |
|  - Address history and balances              |
|  - Asset lock records                        |
|                                              |
|  This will NOT delete:                       |
|  - On-chain identities                       |
|  - Funds (recoverable with recovery phrase)  |
|                                              |
|  ! Warning: 2 identities are linked to       |
|    this wallet. Identity operations will      |
|    fail until you re-import this wallet.      |
|                                              |
|  [Show Recovery Phrase]                       |
|                                              |
|  [  Cancel  ]    [  Remove Wallet  ]         |
|                                              |
+----------------------------------------------+
```

Key points:
- Clearly explains what is and is not deleted
- Warns about linked identities
- Offers to show recovery phrase one last time before removal
- "Remove Wallet" button uses destructive styling (red or warning color)

---

## 13. Private Key View (Modal)

```
+----------------------------------------------+
|  Private Key                            [X]  |
|                                              |
|  ! Keep your private key secure.             |
|    Never share it with anyone.               |
|                                              |
|  Address:                                    |
|  XoRQE8bHjEm3p7Yf2X9...cK4dF                |
|                                              |
|  Private Key (WIF):                          |
|  [**********************************]        |
|  [  Show Key  ]   [  Copy Key  ]             |
|                                              |
|  [  Close  ]                                 |
|                                              |
+----------------------------------------------+
```

- Key is hidden by default (asterisks)
- "Show Key" reveals the WIF string
- Security warning is always displayed prominently
- Requires wallet to be unlocked before this dialog opens

---

## 14. Fund Platform Address Dialog (Modal)

```
+----------------------------------------------+
|  Fund Platform Address                  [X]  |
|                                              |
|  From Asset Lock:                            |
|  [abc123...  0.5000 DASH  (Usable)     v]   |
|                                              |
|  To Platform Address:                        |
|  [tevo1...q2  (0.0000 DASH)           v]    |
|                                              |
|  Amount: [0.5000_______] DASH                |
|  [x] Deduct fees from amount                 |
|                                              |
|  Estimated fee: ~0.0001 DASH                 |
|  Platform address will receive: ~0.4999 DASH |
|                                              |
|  [  Cancel  ]         [  Fund  ]             |
|                                              |
+----------------------------------------------+
```

---

## 15. Wallet Unlock Popup (Modal)

```
+----------------------------------------------+
|  Unlock Wallet                          [X]  |
|                                              |
|  Enter your password to unlock               |
|  "My Dash Wallet"                            |
|                                              |
|  Password: [______________________] [Show]   |
|                                              |
|  [Incorrect password. Try again.]            |
|                                              |
|  [  Cancel  ]         [  Unlock  ]           |
|                                              |
+----------------------------------------------+
```

- Error message appears inline after failed attempt
- "Show" toggle to reveal password text
- Cancel returns the user to the previous state without completing the operation
