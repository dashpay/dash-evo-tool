# Phase 7: Token Screens — Design

## 7.1 Token Screens UX Design (Run 86)

### Complete Action Inventory (15+ egui screens, ~120 user actions, 22 backend tasks)

**Backend Status:** All Tauri IPC commands fully implemented in `src-tauri/src/commands/token.rs`
(1,511 lines): 21 async dispatch commands + 2 direct database commands. All input DTOs defined.
TypeScript bindings auto-generated. 45+ backend unit tests. **No backend work needed.**

**Missing Frontend Infrastructure:**
- No `tokenStore.ts` (Zustand store)
- No token components or screens (only placeholder routes)

### Token Screens Architecture

**Main Screen: 3 tabs**
1. **My Tokens** — Portfolio view with sorting, detail expansion, per-row action menu
2. **Search Tokens** — Keyword search with pagination, contract detail expansion, "Add to My Tokens"
3. **Token Creator** — Multi-step wizard (7+ steps)

**Action Screens (13 separate routes, all share a common operation base pattern):**
- Transfer, Mint, Burn, Freeze, Unfreeze, Pause, Resume, Claim, View Claims,
  Set Price, Purchase, Update Config, Destroy Frozen Funds
- Each follows: token context -> input form -> advanced options (public note) ->
  key selection -> wallet unlock -> fee estimation -> confirmation -> broadcast -> result
- Group action support where applicable

**Supplementary Screens:**
- Add Token by ID — lookup by contract ID or token ID
- Token Info Popup — modal with full token metadata + JSON schema viewer
- Contract Details — expanded view from search results

### Store Design: `tokenStore.ts`

Following walletStore/identityStore/contestStore patterns:
- **State:** myTokens[], searchResults[], searchKeyword, searchCursor, tokenOrder[], selectedToken, loading, refreshing, error, sortColumn, sortOrder
- **Actions:** loadMyTokenBalances, searchByKeyword, clearSearch, fetchTokenByContractId, fetchTokenByTokenId, saveTokenLocally, removeToken, loadTokenOrder, saveTokenOrder, queryTokenPricing, queryFrozenIdentities, subscribeToUpdates
- **Event listeners:** TaskResultEvent filtered by "Token" result type

### Component Breakdown

- `MyTokensTable.tsx` — sortable table with per-row action dropdown (15 actions)
- `TokenInfoDialog.tsx` — modal with full metadata + "View Schema" JSON popup
- `TokenSearchPanel.tsx` — keyword input, results table, pagination, contract detail expansion
- `TokenCreatorWizard.tsx` — 7-step wizard:
  - Step 1: BasicInfoStep (name, plural, language, description, decimals, supply, options)
  - Step 2: DistributionStep (perpetual + pre-programmed, function selector with formula viz)
  - Step 3: ControlRulesStep (10 rule types with action taker combos)
  - Step 4: GroupsStep (groups with member/power grids)
  - Step 5: HistoryStep (keep history checkboxes)
  - Step 6: KeywordsStep (searchable keyword tags)
  - Step 7: ReviewStep (identity selection, summary, create)
- `TokenOperationForm.tsx` — shared layout for all 13 action screens
- Screen files for each action type

### Complexity Notes

- Token Creator is the single most complex screen (~112K lines in egui). Breaking into sub-components per step is critical.
- Distribution function visualization shows formula images (Linear, Polynomial, Exponential, Logarithmic, etc.)
- Control Rules have deeply nested configurations with ~10 rule types
- Group action support adjusts UI text/behavior for group-controlled tokens
- Pricing supports single price or tiered pricing (quantity thresholds to prices)

---

## 7.5 Token Screens Functionality Parity Audit (Run 136)

### Summary

**Grade: A-** — All 17 token routes implemented, all 13 action types functional, token creator wizard covers all 7 steps. 3,402 tests pass (109 test files). Distribution formula visualization implemented as SVG curves. Group action support present across all applicable screens. 8 gaps found (3 P2, 5 P3).

### What's Working Well

- **All 13 token action types implemented:** Transfer, Mint, Burn, Freeze, Unfreeze, Destroy Frozen Funds, Pause, Resume, Claim, View Claims, Set Price, Purchase, Update Config
- **Token Creator Wizard:** Full 7-step wizard with Basic Info, Distribution, Control Rules, Groups, History, Keywords, Review — all functioning with validation
- **Token Search:** Keyword search with pagination, contract detail expansion, "Add to My Tokens"
- **Add by ID:** Supports both contract ID and token ID lookup with fallback
- **TokenOperationForm:** Shared component used by all action screens with identity/key selection, wallet unlock, group action context, confirmation dialogs, broadcasting lifecycle, success/error screens
- **Group action support:** 7 screens support group signing mode with correct button labels, read-only pre-populated fields, group info banners
- **Distribution formula visualization:** SVG-based formula curves for all distribution function types
- **Set Price:** Full pricing support — single price, tiered pricing with dynamic tiers, remove pricing
- **Purchase:** Pricing schedule fetch, real-time price calculation with tier selection
- **Update Config:** 44 change item variants across 6 categories with per-variant editors

### Gaps Found

#### P2 — Important

1. **My Tokens table lacks two-level drill-down view** — The egui version has a two-level view: Level 1 shows token list (Token Name, Token ID, Description), and clicking a token drills into Level 2 showing per-identity balances with per-row action buttons and a Back button. The frontend shows a flat table with one row per identity-token combo. This means users can't see a token-level overview before diving into per-identity details. (egui: `my_tokens.rs` lines 159-161, 295-596, 944-991)

2. **My Tokens table missing Rewards column and estimation** — The egui version shows a "Rewards" column for tokens with perpetual distribution, with an "Estimate" button per row that calls `tokenEstimatePerpetualRewards`, and an info icon (ℹ) that opens a detailed reward explanation popup with Total Estimated Rewards, Basic Explanation, Detailed Explanation, and Step-by-Step Breakdown sections. None of this is present in the frontend table (only available in the separate Claim screen). (egui: `my_tokens.rs` lines 360-593)

3. **Unfreeze/Destroy screens use free-text input instead of frozen identity selector** — The egui version fetches the list of frozen identities from Platform on screen load (`TokenTask::QueryFrozenIdentities`) and populates an identity selector dropdown, so users pick from known frozen identities. The frontend uses a simple text input field with no fetch or dropdown. (egui: `unfreeze_tokens_screen.rs` lines 85-87, 221-228, 379-395)

#### P3 — Polish

4. **Token Creator missing Simple Mode toggle** — The egui version has a "Show Advanced Options" checkbox that switches between Simple Mode (fewer fields, token presets only) and Advanced Mode (full wizard). The frontend only has the 7-step wizard (advanced mode equivalent). Simple mode would be valuable for users who want quick token creation. (egui: `token_creator.rs` lines 144-147, 468-520)

5. **Token action screens missing Add Key / View Key Info buttons** — The egui version's Advanced Options section includes "Add Key" and "View Key Info" navigation buttons alongside the key selector dropdown. The frontend only has dropdown selectors without navigation. (egui: various token action screens)

6. **Token Creator Review step missing "View Data Contract JSON" preview** — The egui advanced creator has a "View Data Contract JSON" button that generates and displays the full contract JSON before creation, plus a separate "Calculate Fee" button. The frontend only shows fee estimation automatically without a JSON preview. (egui: `token_creator.rs`)

7. **Set Price screen doesn't show current pricing schedule** — The egui version displays the existing pricing schedule before allowing changes, giving users context about what they're changing from. The frontend only shows the new pricing input form. (egui: `set_token_price_screen.rs`)

8. **Mint screen doesn't respect minting destination configuration** — The egui version auto-populates the recipient based on token config (default destination = contract owner, "allow choosing destination" controls editability). The frontend uses a simple optional text input without reading the token's minting configuration. (egui: `mint_tokens_screen.rs`)
