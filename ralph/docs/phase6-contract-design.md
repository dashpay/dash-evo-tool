# Phase 6: Contract & Document Screens — Design

## 6.1 Contract/Document Browser UX Design (Run 85)

### Complete Action Inventory (6 egui screens, ~50 user actions)

**Backend Status:** All Tauri IPC commands fully implemented (11 contract + 8 document commands).
All DTOs defined. TypeScript bindings auto-generated via tauri-specta. No backend work needed.

**Missing Frontend Infrastructure:**
- No `contractStore.ts` or `documentStore.ts` (Zustand stores)
- No contract/document components or screens (only DPNS screens exist under /contracts/)
- The `/contracts/` route is a placeholder

### UX Design

**Main Screen Layout (3-panel):**
- Left sidebar: Contract tree browser (collapsible) with search, showing contract -> document types -> indexes -> properties. Right-click context menu for copy hex/JSON. Remove button for user contracts.
- Center: SQL-like query input bar at top, document results below with JSON/YAML toggle, field selector, search filter, pagination
- Top bar: Action buttons (Load Contracts, Register, Update, Create/Delete/Replace/Transfer/Purchase/SetPrice Document, Group Actions)

**Sub-screens (each accessible from top bar buttons):**
1. Add Contracts — multi-ID input, fetch, set aliases
2. Register Contract — identity/key selection, JSON editor with auto-wrap detection, fee estimation, broadcast
3. Update Contract — contract selector, JSON editor, identity/key selection, fee estimation, broadcast
4. Document Actions (6 types sharing common layout) — contract/doc-type selection, identity/key selection, wallet unlock, type-specific inputs, fee estimation, broadcast
5. Group Actions — contract selector (filtered to group-enabled), identity selector, fetch & display table, "Take Action" routing to token screens

---

## 6.4 Contract/Document Screens Functionality Parity Audit (Run 117)

### Summary
**Overall grade: A-** — 2526 tests pass. All 10 top-bar action buttons present and functional. All 6 document action types implemented. Contract tree panel fully functional with search, expand/collapse, index selection, copy hex/JSON, remove. 3 minor gaps found (non-blocking, P3).

### Audit Methodology
- Read every egui source file in `src/ui/contracts_documents/` (~3,500 lines)
- Read every Tauri screen/component (~4,600 lines across 8 files)
- Compared action-by-action, element-by-element
- Ran full test suite (2526 pass, 0 fail)

### Top Bar Action Buttons
| egui Button | Tauri Button | Status |
|---|---|---|
| Load Contracts | Load Contracts → `/contracts/add-contracts` | Present |
| Register Contract | Register Contract → `/contracts/register` | Present |
| Update Contract | Update Contract → `/contracts/update-contract` | Present |
| Create Document | Create Document → `/contracts/create-document` | Present |
| Delete Document | Delete Document → `/contracts/delete-document` | Present |
| Replace Document | Replace Document → `/contracts/replace-document` | Present |
| Transfer Document | Transfer Document → `/contracts/transfer-document` | Present |
| Purchase Document | Purchase Document → `/contracts/purchase-document` | Present |
| Set Document Price | Set Document Price → `/contracts/set-document-price` | Present |
| Group Actions (non-Mainnet only) | Group Actions → `/contracts/group-actions` | Present (always visible — minor improvement over egui which hides it on Mainnet) |

### Contract Tree Panel
| Feature | Status | Notes |
|---|---|---|
| Contract list with search filter | Present | |
| Expand/collapse contracts | Present | |
| Document Types section | Present | |
| Indexes with properties (asc/desc) | Present | |
| Unique badge on indexes | Present | |
| Tokens section (base/max supply) | Present | |
| Contract JSON section | Present | |
| Copy Hex / Copy JSON context menu | Present | |
| Remove button (non-system only) | Present | With confirmation dialog |
| System contract protection | Present | dpns, keyword_search, token_history, withdrawals, dashpay |
| Empty state | Present | |
| Loading state | Present | |
| Tooltip with full ID + doc count | Present | |

### Document Query Screen
| Feature | Status | Notes |
|---|---|---|
| SQL-like query input | Present | |
| Fetch Documents button | Present | With loading spinner |
| Elapsed time counter | Present | |
| JSON/YAML display toggle | Present | |
| Field selection dialog | Present | With Select All / Deselect All |
| Document search filter | Present | |
| Pagination (Previous/Page N/Next) | Present | |
| Empty state | Present | |
| Error state | Present | |
| Auto-populate query from tree selection | Present | |

### Add Contracts Screen
| Feature | Status | Notes |
|---|---|---|
| Multi-field input (up to 10) | Present | |
| Hex + Base58 support | Present | |
| Add Another Field button | Present | |
| Remove field button | Present | |
| Fetch with progress | Present | With elapsed time |
| Success: found contracts list | Present | |
| Success: not found contracts list | Present | |
| Alias editing per contract | Present | With Set Alias button |
| Alias validation (non-empty) | Present | |
| Back to Contracts navigation | Present | |
| Error banner with dismiss | Present | |

### Register Contract Screen
| Feature | Status | Notes |
|---|---|---|
| Identity selector | Present | Auto-selects first |
| Key auto-selection (HIGH/CRITICAL) | Present | |
| Advanced: manual key selector | Present | Behind toggle |
| Alias input (optional) | Present | |
| JSON code editor | Present | Textarea |
| Auto-detect raw schemas + wrap | Present | |
| Link to dashpay.io | Present | |
| Fee estimation | Present | Client-side estimate |
| Wallet unlock gate | Present | |
| Broadcasting state with elapsed time | Present | |
| Success screen with actions | Present | Back + Register Another |
| Error handling with dismiss | Present | |
| Balance display | Present | |

### Update Contract Screen
| Feature | Status | Notes |
|---|---|---|
| Identity selector (CRITICAL keys only) | Present | |
| Contract dropdown (excludes system) | Present | |
| Auto-load selected contract JSON | Present | With loading spinner |
| JSON editor | Present | |
| Fee estimation | Present | |
| Wallet unlock gate | Present | |
| Broadcasting + success + error states | Present | |

### Document Action Screen (all 6 types)
| Feature | Create | Delete | Replace | Transfer | Purchase | SetPrice |
|---|---|---|---|---|---|---|
| Contract selector | Y | Y | Y | Y | Y | Y |
| Doc type selector | Y | Y | Y | Y | Y | Y |
| Identity selector | Y | Y | Y | Y | Y | Y |
| Key selector (advanced) | Y | Y | Y | Y | Y | Y |
| Wallet unlock | Y | Y | Y | Y | Y | Y |
| Fee estimation | Y | Y | Y | Y | Y | Y |
| Broadcasting state | Y | Y | Y | Y | Y | Y |
| Success screen | Y | Y | Y | Y | Y | Y |
| Dynamic form fields | Y | - | Y | - | - | - |
| Document ID input | - | Y | Y | Y | Y | Y |
| Fetch Owned Documents | - | Y | - | - | - | - |
| Fetched docs list + View/Select | - | Y | - | - | - | - |
| Fetch document for replace | - | - | Y | - | - | - |
| Recipient ID input | - | - | - | Y | - | - |
| Fetch Price button | - | - | - | - | Y | - |
| Price input | - | - | - | - | - | Y |
| Token cost info display | Y | Y | Y | Y | Y | Y |
| Boolean field (checkbox) | Y | - | Y | - | - | - |
| Object/Array field (textarea) | Y | - | Y | - | - | - |
| Required field validation | Y | - | Y | - | - | - |

### Group Actions Screen
| Feature | Status | Notes |
|---|---|---|
| Contract selector (filtered to tokens) | Present | Excludes system contracts |
| Identity selector | Present | Auto-selects if only one |
| Fetch Group Actions button | Present | |
| Elapsed time during fetch | Present | |
| Results table | Present | |
| Action ID column | Present | Truncated with full ID tooltip |
| Type column (badge) | Present | |
| Info column | Present | |
| Signers column (N/M format) | Present | |
| Take Action button per row | Present | Routes to token screen |
| Search filter for results | Present | |
| Empty state | Present | |
| Error banner with dismiss | Present | |

### Gaps Found (3 minor, all P3)

1. **Group Actions table missing "Note" column** (P3)
   The egui version has a separate "Note" column in the group actions table that shows the public note from token events. The Tauri version includes notes in the "Info" column via `formatActionInfo()` which checks `details.note`, but it's combined with other info rather than having its own column. Minor display difference; no functionality loss since the note IS displayed.

2. **Group Actions button visibility differs by network** (P3)
   The egui version hides the "Group Actions" button on Mainnet (only shows 9 buttons). The Tauri version always shows all 10 buttons. This is actually an improvement — the button is simply present but will show "No contracts with tokens found" if there are none, so no harm.

3. **AddContractsScreen inline error doesn't show recovery suggestion** (P3)
   The egui version calls `recovery_suggestion()` to show a secondary recovery hint below errors in the add contracts screen. The Tauri version uses `toastError()` which DOES include recovery suggestions in the toast notification, but the inline error banner on the screen just shows the raw error. Low impact since users see the toast with the suggestion.
