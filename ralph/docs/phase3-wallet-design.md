# Phase 3: Wallet Screens — Design & Audit

## 3.1 Wallet Screens UX Design (Run 45)

### Wallet Screens Architecture: 6 Route-Based Screens

The egui version crams all wallet functionality into a single 2,030-line file with
modal popups. The Tauri version splits into clean, focused route-based screens:

```
/wallets                    -> WalletsScreen (list + detail view)
/wallets/create             -> CreateWalletScreen (new HD wallet wizard)
/wallets/import             -> ImportWalletScreen (mnemonic + private key import)
/wallets/send/:type         -> SendScreen (HD and single-key send, unified)
/wallets/asset-locks/create -> CreateAssetLockScreen (registration + top-up flows)
/wallets/asset-locks/:id    -> AssetLockDetailScreen (lock details + private key)
```

### Wallet List & Detail View (WalletsScreen)

**Layout: Split-pane design**
- Left panel (300px): Wallet list with HD and single-key wallets in separate sections
- Right panel: Detail view for selected wallet
- Empty state when no wallets: Card with "No Wallets Loaded" + action buttons

**Wallet List Panel:**
- Section headers: "HD Wallets" and "Single-Key Wallets" with count badges
- Each wallet card: Alias, balance, pending indicator, lock icon (if password)
- Selected wallet highlighted with Dash Blue left border
- Context menu (right-click or kebab icon): Rename, Lock/Unlock, Remove

**HD Wallet Detail Panel:**
- Header: Wallet alias (editable inline), Core + Platform balance summary
- Action bar: Send, Receive, Refresh buttons + refresh mode dropdown (dev only)
- Tabs: Addresses | Transactions (dev) | Asset Locks
- Addresses tab: Account selector dropdown -> sortable address table with columns
  (Address, Balance, UTXOs, Total Received, Type, Index, Path, Actions)
- "Hide zero balances" toggle, "Add Receiving Address" button (when unlocked)
- Address row "View Key" button -> private key dialog (requires wallet unlock)
- Transactions tab (dev only): Sortable table (Date, Type, Amount, Status, TxID)
- Asset Locks tab: Table (TxID, Address, Amount, InstantLock, Usable, Actions)
  + "Create Asset Lock" and "Search for Unused" buttons

**Single-Key Wallet Detail Panel:**
- Header: Alias, address (monospace), balance + pending
- Action bar: Send, Receive
- UTXOs section with paginated cards (50 per page)

### UX Improvements Over egui

1. **Persistent wallet list**: No dropdown needed — see all wallets at a glance
2. **Inline rename**: Double-click alias to edit, Enter to save, Escape to cancel
3. **Context menus**: Right-click or kebab for wallet actions (cleaner than button row)
4. **Unified Send screen**: HD and single-key send merged into one screen with
   source type awareness (simpler navigation, less code duplication)
5. **Tabbed detail view**: Addresses/Transactions/Asset Locks as tabs instead of
   vertically stacked sections (reduces scrolling)
6. **Receive as Dialog**: Consistent modal dialog with tabs (Core/Platform) and QR
7. **Better empty states**: Illustrations + clear CTAs for each empty section
8. **Toast notifications**: Replace in-page message banners with toast system
9. **Stepper wizard for Create Wallet**: Clear progress indicator for multi-step flow
10. **Paste detection for Import**: Auto-detect full mnemonic paste and fill all fields

### Create Wallet Flow (CreateWalletScreen)

**Multi-step wizard with step indicator:**
1. Generate Entropy -> Shows entropy grid visualization
2. Configure -> Language + word count selection, Generate button
3. Backup -> Display seed words in numbered grid, "I wrote it down" checkbox
4. Name & Protect -> Wallet name input + optional password with strength meter
5. Success -> Wallet created, next steps (Fund, Create Identity, Go to Wallet)

**Success screen additions:**
- "Fund Wallet" button opens Receive dialog with QR
- Auto-detects incoming funds and updates UI
- "Create Platform Identity" navigates to identity creation with wallet pre-selected

### Import Wallet Flow (ImportWalletScreen)

**Two tabs at top: "Seed Phrase" | "Private Key"**

**Seed Phrase tab:**
- Word count selector (12/15/18/21/24)
- Word input grid (4 columns) with paste-to-fill support
- Real-time BIP39 validation with per-word error highlighting
- Identity auto-discovery config (collapsible advanced section)
- Name + password section

**Private Key tab:**
- Single input field (WIF or hex)
- Real-time parsing with address preview
- Name + password section

**Success screen**: Same as Create but with "Import Another" option

### Send Flow (SendScreen — Unified HD + Single-Key)

**Simple mode (default):**
- Source selector: Shows wallet info + available balance
- For HD wallets: Radio buttons for Core Wallet / Platform Addresses / Identity sources
- Destination address input with type detection badge (Core/Platform)
- Amount input with Max button
- Transaction type hint (auto-detected from source+dest combination)
- "Subtract fee from amount" checkbox (Core-to-Core only)
- Platform source breakdown panel (when applicable)
- Wallet unlock gate -> Send button

**Advanced mode (toggle):**
- HD: Source type selector (Core/Platform), manual address selection
- Single-key: Multiple recipients with add/remove, memo field
- Fee estimation display with UTXO count and tx size
- Large input warning (>100 UTXOs)

**States:** Form -> Wallet Unlock -> Sending (spinner + elapsed time) -> Success/Error
- Success: "Send Another" or "Back to Wallet"
- Error: Inline banner with dismiss + optional fee confirmation dialog

### Receive Dialog (Modal from Wallet Detail)

**Two tabs: Core | Platform**
- QR code display (220x220)
- Address selector dropdown (if multiple addresses)
- Full address display (monospace)
- Balance display
- Copy Address + New Address buttons
- Info text explaining what the address is for

### Asset Lock Screens

**Create Asset Lock (CreateAssetLockScreen):**
- Step 1: Purpose selection (Registration / Top Up) with info cards
- Step 2 (Top Up): Identity selector
- Step 3: Amount input with DASH display
- Step 4: QR code + funding address for receiving DASH
- Automatic progression: Funds received -> Asset lock creation -> Success
- Advanced options: Manual index selection

**Asset Lock Detail (AssetLockDetailScreen):**
- Transaction info section: TxID, Address, Amount (DASH + duffs)
- Proof status with color badges (Instant Send Locked/Chain Locked/Waiting)
- Proof details section (Instant or Chain variant)
- Proof hex with copy button
- Private key section (requires wallet unlock): WIF display with show/hide toggle
- Warning text about key security

### Dialog Components (Shared)

All wallet dialogs reuse components from Phase 2:
- ConfirmationDialog (remove wallet)
- WalletUnlockDialog (password entry)
- FeeConfirmationDialog (fee override)
- AmountInput (DASH/credits formatting)
- CopyButton (clipboard with feedback)
- Toast notifications (success/error/info)

---

## 3.6 Wallet Screens Functionality Parity Audit (Run 56)

### Overall Assessment: STRONG (A-)
811 tests pass, lint clean, typecheck clean. All major wallet workflows are present and
functional. The Tauri implementation covers all core functionality with significant UX
improvements (split-pane layout, inline rename, context menus, toast notifications,
tabbed detail view, step wizards).

### Features with FULL PARITY (confirmed present):
- Wallet list: HD + single-key sections, select, rename (inline), lock/unlock, remove with confirmation
- HD wallet detail: alias, Core+Platform balances, pending indicator, refreshing state
- Refresh modes: All 5 modes (All Auto, Core Only, Core+Platform Full/Terminal, Combined) in dev mode
- Addresses tab: account selector, sortable 7-column table, hide zero toggle, View Key, Add Address, CopyButton
- Transactions tab: dev-mode only, sorted by date, direction/amount/status/txid
- Asset Locks tab: table with all columns, Create/Search/View/Fund buttons
- Single-key detail: address+copy, balance+pending, paginated UTXOs (50/page)
- Receive dialog: Core/Platform tabs (HD), single tab (single-key), QR 220x220, address selector, New Address
- Private Key dialog: masked/revealed, Copy when revealed, security warning
- Create Wallet: 3-step wizard, word count 12-24, BIP39 generation, strength meter, success screen
- Import Wallet: Two tabs (Seed Phrase/Private Key), word grid with multi-word paste, BIP39 validation, advanced options (identity auto-discovery), strength meter
- HD Send: Simple (Core/Platform/Identity sources, address detection, Max+auto-subtract-fee, platform breakdown), Advanced (inputs/outputs, fee strategy), sending/complete/error states
- Single-key Send: Simple (address+amount+subtract fee), Advanced (multiple recipients, memo), sending/complete/error states
- Create Asset Lock: Purpose->Configure->Funding->Creating->Success, registration/top-up, identity selector, advanced options (identity/top-up index), auto-progression on fund receipt
- Asset Lock Detail: TX info, proof status badge, InstantLock/Usable badges, private key with unlock gate
- Identity withdrawal: source selection with key picker, correct IPC dispatch
- Fee strategy: 4-option dropdown for platform operations
- Max button: auto-enables subtract-fee for Core->Core

### Gaps Found (5 issues):

**1. Fee confirmation dialog not wired to send flows (P2)**
- `FeeConfirmationDialog` component exists and is fully implemented
- Neither SendScreen nor SingleKeySendScreen uses it
- Both pass `overrideFee: null` hardcoded
- The egui single-key send screen has `FeeConfirmationDialog` integration that intercepts min relay fee errors

**2. Transaction size estimation display missing from single-key send (P3)**
- egui shows: estimated fee, UTXO input count, tx byte size
- Also shows warning when >100 UTXOs needed

**3. Asset lock proof details missing from AssetLockDetailScreen (P2)**
- egui shows detailed proof information: Instant Send TxID + Output Index, Chain Lock Height + OutPoint
- egui shows proof hex with Copy button and collapsible "View Raw Proof Details" section

**4. BIP39 language selection missing from CreateWalletScreen (P3)**
- egui has Language dropdown: English, Spanish, French, Italian, Portuguese
- Tauri hardcodes English only

**5. Entropy grid visualization missing from CreateWalletScreen (P3)**
- egui has `U256EntropyGrid` component that shows randomness visualization
- Tauri uses `@scure/bip39` with WebCrypto (more secure)

### Test Coverage: 811 total Vitest tests (~365 wallet-related across 12 test files)
