# Phase 4: Identity Screens — Design & Review

## 4.1 Identity Screens UX Design (Run 62)

### Identity Screens Architecture: Route-Based with Shared Store

The egui version uses ~7,000 lines across 12+ files with modal popups and screen stacks.
The Tauri version splits into clean, focused route-based screens with a shared Zustand store:

```
/identities                        -> IdentitiesScreen (list + detail split-pane)
/identities/create                 -> CreateIdentityScreen (new identity wizard)
/identities/load                   -> LoadIdentityScreen (add existing by ID/wallet/DPNS)
/identities/top-up/:id             -> TopUpIdentityScreen (top-up wizard)
/identities/withdraw/:id           -> WithdrawScreen (withdraw credits)
/identities/transfer/:id           -> TransferScreen (transfer credits)
/identities/register-dpns/:id      -> RegisterDpnsNameScreen (register DPNS name)
/identities/keys/:id               -> KeyManagementScreen (view/add/disable/replace keys)
/identities/keys/:id/:keyId        -> KeyInfoScreen (key detail + sign message)
```

### Identity List & Detail View (IdentitiesScreen)

**Layout: Split-pane design (similar to WalletsScreen pattern)**
- Left panel (320px): Sortable identity table
- Right panel: Detail view for selected identity
- Empty state when no identities: Card with "No Identities" + "Create" / "Load" buttons

**Identity Table:**
- Columns: Alias, Identity ID (truncated), In Wallet, Type (User/MN/Evonode), Balance (DASH)
- Sortable columns (click header: Alias, Identity ID, In Wallet, Type, Balance)
- Custom ordering: Up/Down buttons to reorder (persisted to DB via `identity_save_order`)
- Default: custom order if saved, else sort by Alias ascending
- Inline alias editing: click alias cell -> text input, Enter to save, Escape to cancel
- Row selection: click to select, highlights with Dash Blue left border
- Context menu (right-click or kebab): View Keys, Register DPNS Name, Top Up, Withdraw,
  Transfer, Update Alias, Remove
- Identity status colors: Active (green badge), Failed (red), Unknown (gray), Pending (yellow)
- Type display: "User" | "Masternode" | "Evonode" badge
- Balance: formatted in DASH, hover shows duffs (tooltip)

**Identity Detail Panel (when selected):**
- Header: Alias (large), Identity ID (full, monospace + copy), Type badge, Status badge
- Balance section: Credits balance in DASH, platform balance breakdown
- DPNS names section: List of registered names (if any)
- Associated wallet: Wallet name + link to wallet screen
- Action bar: Top Up, Withdraw, Transfer, Register DPNS, Refresh buttons
- Keys dropdown: Quick access to all keys with private key indicators
  (highlighted green if private key held, dim if not)
- For voter identity: separate section showing voter keys

**Top bar actions:**
- "Create Identity" button (navigates to /identities/create)
- "Load Identity" button (navigates to /identities/load)
- "Refresh All" button (bulk refresh all identities)
- If no wallets: shows "Import/Create Wallet" instead

### Create Identity Flow (CreateIdentityScreen)

**Multi-step wizard with step indicator (similar to CreateWalletScreen pattern):**

**Step 1: Select Wallet**
- Wallet selector dropdown (if multiple wallets)
- Auto-selects if only one wallet
- Wallet unlock gate if password-protected

**Step 2: Identity Index (advanced only, collapsible)**
- Dropdown showing indices 0-30
- "(used)" marker on already-claimed indices
- Info tooltip: "Identity index is an internal reference number"

**Step 3: Key Configuration (advanced only, collapsible)**
- Toggle: Default Keys / Advanced Configuration
- Default: platform auto-selects keys (explanation text shown)
- Advanced: Table with columns (Key #, WIF, Purpose, Type, Security Level, Delete)
  - Master key row (always present, not deletable)
  - Additional key rows with + Add Key button
  - Purpose dropdown: Authentication, Transfer, Voting, Owner
  - Type dropdown: ECDSA_SECP256K1, ECDSA_HASH160, BLS12_381, BIP13_SCRIPT_HASH
  - Security Level dropdown: Critical, High, Medium (auto-set based on purpose)

**Step 4: Local Alias**
- Text input for local alias (required)
- Info: "Stored only in Dash Evo Tool -- not broadcast to the network"

**Step 5: Funding Method**
- Selector with 4 options:
  1. "Unused Evo Funding Locks" (recommended, shown only if locks exist)
  2. "Wallet Balance" (shown only if wallet has sufficient balance)
  3. "Address with QR Code" (always available)
  4. "Platform Address" (shown only if wallet has platform balance)
- Each option has a brief description

**Step 6: Funding-specific UI (varies by method)**
- **Asset Lock:** Select from list of available locks, amount display
- **Wallet Balance:** Amount selector, auto-calculate from wallet balance
- **QR Code:** Generate receive address, show QR (220x220), address + copy button,
  waiting indicator, auto-detect incoming UTXO, progress through steps
  (WaitingOnFunds -> FundsReceived -> ReadyToCreate -> WaitingForAssetLock -> WaitingForPlatformAcceptance -> Success)
- **Platform Address:** Select platform address from wallet, amount input

**Step 7: Register**
- Review summary: wallet, alias, funding method, amount
- "Register Identity" button
- Progress: spinner + elapsed time

**Success Screen:**
- Identity ID display + copy
- Fee breakdown (base fee, processing fee, total in DASH)
- Action buttons: "Go to Identities", "Register DPNS Name", "Create Another"

### Load Existing Identity (LoadIdentityScreen)

**Three tabs at top: By Identity ID | By Wallet | By DPNS Name**

**By Identity ID tab:**
- Input field for Identity ID (accepts Hex and Base58)
- Advanced options (collapsible):
  - Identity Type selector (User/Masternode/Evonode)
  - Manual private keys section: list of key inputs with + Add / - Remove
  - Testnet only: "Fill Random HPMN" / "Fill Random Masternode" quick-fill buttons
- "Load Identity" button

**By Wallet tab:**
- Wallet selector dropdown
- Wallet unlock gate (if password-protected)
- Advanced options (collapsible):
  - Search mode: "Specific Index" (single input) or "Up to Index" (scan range)
- "Search" button

**By DPNS Name tab:**
- Username input (min 3 chars, ".dash" suffix shown)
- Advanced: wallet selector for key derivation
- "Search by Username" button

**All tabs share:**
- Error banner (dismissible) at top of content
- Loading state with elapsed time counter
- Success state with identity details + "Load Another" / "Back to Identities" buttons

### Top Up Identity (TopUpIdentityScreen)

**Same funding wizard pattern as Create, but for existing identity:**
- Identity header shows which identity is being topped up
- 4 funding methods (same as Create)
- Amount input for wallet balance and platform address methods
- QR code flow same as Create
- Success screen with fee breakdown

### Withdraw Credits (WithdrawScreen)

**Single-page form:**
- Key selector: dropdown of identity keys with TRANSFER purpose
- Available balance display (formatted DASH)
- Amount input with Max button (max = balance - 0.005 DASH fee reserve)
- Destination address input:
  - For owner key (masternode): auto-filled with payout address, read-only
  - For other keys: text input with address validation (network-specific)
  - Inline error if invalid address format
- Confirmation dialog (danger mode): "Are you sure you want to withdraw X DASH?"
- "Withdraw" button (disabled until valid)
- States: Form -> Wallet Unlock -> Sending (spinner + elapsed) -> Success/Error

### Transfer Credits (TransferScreen)

**Single-page form:**
- Key selector: dropdown of TRANSFER-purpose keys
- Transfer destination toggle: "To Identity" / "To Platform Address" buttons
- **To Identity:** Identity selector (search loaded identities) + receiver identity ID input
- **To Platform Address:** Platform address input field with validation
- Amount input with Max button
- Confirmation dialog: "Transfer X DASH credits?"
- "Transfer" button
- States: same as Withdraw

### Register DPNS Name (RegisterDpnsNameScreen)

**Single-page form:**
- Identity selector (if multiple user identities loaded)
- Key selector (advanced: choose specific key)
- Username input: text field + ".dash" suffix display (min 3 characters)
- "Register" button
- States: Form -> Wallet Unlock -> Registering (spinner + elapsed) -> Success/Error
- Success: shows whether name was registered normally or is contested

### Key Management (KeyManagementScreen)

**Keys list with actions:**
- Table: Key ID, Purpose, Security Level, Type, Status (Active/Disabled), Has Private Key
- Private key indicator: green highlight if private key held, dim if not
- Separate sections: Main Identity Keys / Voter Identity Keys
- Click key row -> navigate to KeyInfoScreen
- "+ Add Key" button

### Key Info (KeyInfoScreen)

**Key detail view:**
- Key metadata grid: Key ID, Purpose, Security Level, Type, Read Only, Active/Disabled
- Contract bounds (if set): Contract ID + Document Type
- Public key display: Hex + Base64, with copy buttons
- Private key section (varies by source)
- Message signing section: text area + "Sign Message" button + signed output
- Advanced actions: Disable Key, Replace Key, Remove Private Key

### Add Key Dialog

**Form:**
- Private key input (hex, 32 bytes)
- Key Type selector
- Purpose selector
- Security Level selector (auto-set based on purpose)
- Advanced: Contract bounds section
- Wallet unlock gate

### UX Improvements Over egui

1. Split-pane identity list
2. Inline alias editing
3. Route-based operations
4. Unified key management
5. Better empty states
6. Toast notifications
7. Breadcrumb navigation
8. Stepper wizards
9. Consistent status badges
10. Key indicators in list

### Backend Commands Available (27 commands)

All identity IPC commands are implemented and TypeScript bindings generated:
- Async: identity_load, identity_search_by_dpns_name, identity_search_from_wallet,
  identity_search_up_to_index, identity_register, identity_register_dpns_name,
  identity_refresh, identity_refresh_dpns_names, identity_withdraw, identity_transfer,
  identity_add_key, identity_disable_keys, identity_replace_key, identity_top_up,
  identity_top_up_from_platform_addresses, identity_transfer_to_addresses
- Sync: identity_list_local, identity_list_user, identity_list_voting, identity_get_by_id,
  identity_set_alias, identity_get_alias, identity_load_order, identity_save_order,
  identity_delete, identity_list_summaries, identity_local_dpns_names

---

## 4.5 Identity Screens Functionality Review (Run 71)

**Screens Reviewed:** IdentitiesScreen, IdentityListPanel, IdentityDetailPanel, CreateIdentityScreen, LoadIdentityScreen, TopUpIdentityScreen, WithdrawScreen, TransferScreen, KeyManagementScreen, KeyInfoScreen, AddKeyDialog
**Tests:** 1407 passing (48/49 test files pass; 1 pre-existing failure in NetworkChooserScreen unrelated)

### Fully Implemented (matching egui parity):
- Identity list with cards, context menus, inline alias editing
- Identity detail panel with balance, keys, DPNS names, wallets, type info
- Create identity with 4 funding methods + advanced options
- Load identity with 3 modes (by ID, by wallet, by DPNS name)
- Top up identity with 4 funding methods
- Withdraw with amount/address/key selection + confirmation dialog
- Transfer with identity/platform-address destinations + confirmation dialog
- Key management table with purpose/security/type/status/private indicators
- Key info with public key display, add/remove private key, disable key
- Add key dialog with purpose/security/type/private key + contract bounds UI
- Concurrent identity refresh (Promise.allSettled)
- Identity status display with color-coded badges
- Balance hover tooltip showing raw credits
- Copy to clipboard for IDs, keys, addresses
- Reorder identities up/down with persistence
- Fee estimation display

### Gaps Found:

**P1 — Functionality gaps:**
1. **DPNS name registration screen not implemented** — deferred to Phase 5
2. **Message signing not implemented** — backend IPC command missing
3. **Contract bounds not sent to backend in AddKey** — UI collects but IPC omits
4. **Master key replacement missing key generation UI** — no type selector or generate button
5. **Wallet unlock not integrated for identity operations** — dialog exists but unused
6. **QR code placeholder in CreateIdentityScreen** — dashed border box instead of actual QR
7. **UTXO monitoring for QR funding not implemented** — no active fund detection

**P2 — UX polish gaps:**
8. **No sortable table/columns for identity list** — store has infra but no UI
9. **No progress messages during wallet identity search**
10. **No testnet-only helper buttons** — "Fill Random HPMN" / "Fill Random Masternode" missing
11. **Identity encoding tooltip missing**
12. **No recovery suggestions for errors**
