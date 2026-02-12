# UX Todo: Future Improvements Beyond Wallet Screen

Items discovered during the wallet screen redesign that apply to other areas of the app. Organized by priority and area. Each item includes a brief rationale.

---

## High Priority

### 1. Apply Progressive Disclosure Model App-Wide

**Area**: All screens
**Rationale**: The wallet redesign introduces a three-level disclosure model (Default / Expanded / Developer Tools). This same pattern should be applied consistently to Identity, Token, Contract, and DashPay screens. Currently, every screen shows all information at once regardless of user expertise.

- Identity screen: hide key details, security levels, and raw identity IDs behind expandable sections
- Token screen: hide contract details and raw token IDs behind expandable sections
- Contract screen: hide raw document schemas behind expandable sections
- Tools screens: consider gating behind Developer Tools setting entirely

### 2. Standardize Error Message Patterns

**Area**: All screens, all backend task results
**Rationale**: Backend task errors currently surface as raw Rust error strings (e.g., `"Error: connection timeout"` or `"Error: insufficient credits"`). The wallet redesign specifies human-readable error messages with suggested next steps. This pattern needs to be applied to all screens.

- Create a centralized error message mapping (error code/type to user-facing text)
- Every error should follow the template: "[What happened]. [What to do next]."
- Developer mode should allow expanding errors to see raw technical details
- Affected screens: identity registration, DPNS registration, token operations, contract deployment

### 3. Transaction History Visible by Default

**Area**: Wallet screen (immediate)
**Rationale**: Transaction history is currently hidden behind developer mode. This was identified as a critical usability issue by all three personas. This is the single most impactful quick-win from the redesign.

### 4. Rename "Developer Mode" to "Developer Tools"

**Area**: Settings screen, left panel indicator
**Rationale**: The current "developer mode" label implies instability or unsupported features. "Developer Tools" is more accurate and less intimidating. The setting should only gate genuinely developer-specific features (faucet, bulk ops, raw credit values, Devnet config), not power-user features like refresh controls and address tables.

### 5. Consistent Message Banner Component

**Area**: All screens
**Rationale**: The wallet redesign specifies a `MessageBanner` component with consistent styling (Success/Info/Warning/Error), auto-dismiss behavior, and action buttons. Currently, each screen handles messages differently. Extract the message banner as a shared component in `ui/components/` and use it everywhere.

---

## Medium Priority

### 6. Identity Screen UX Alignment

**Area**: `identities_screen.rs`, `add_new_identity_screen/`, `top_up_identity_screen/`
**Rationale**: The identity screen uses the same table-based layout as the current wallet screen. It should adopt the wallet redesign's patterns:

- Balance display: show identity credit balance prominently (not just in a table cell)
- Key management: use collapsible sections for identity keys, not a flat table
- Top-up flow: should be a guided multi-step flow like the redesigned send flow
- Identity creation: should integrate with wallet context (auto-select funding wallet, streamline asset lock creation)

### 7. Token Screen UX Alignment

**Area**: `tokens/tokens_screen/`, `mint_tokens_screen.rs`, `transfer_tokens_screen.rs`, etc.
**Rationale**: Token screens have many operations (mint, burn, freeze, unfreeze, pause, resume, transfer, claim, set price, purchase). These should follow the wallet redesign's multi-step flow pattern:

- Enter details -> Confirm summary -> Result (success/failure)
- Fee estimates shown before confirmation
- Consistent action bar pattern

### 8. Global Wallet Selector in Top Panel

**Area**: `top_panel.rs`, `wallets_screen/mod.rs`
**Rationale**: The wallet redesign specifies a global wallet selector in the top panel, visible on all screens. Currently, wallet selection is only on the wallet screen. Other screens (identities, tokens) that depend on the selected wallet should show which wallet is active. This requires refactoring wallet selection from a screen-level concern to an app-level concern.

### 9. DashPay Screen Integration

**Area**: `dashpay/dashpay_screen.rs`, `profile_screen.rs`, `add_contact_screen.rs`
**Rationale**: DashPay social features (contacts, profiles) should surface wallet context:

- Show contact's DPNS name as a valid send recipient in the send flow
- Profile screen should show the user's wallet balance and recent activity
- Contact list should integrate with address book for send operations

### 10. Network Switching UX

**Area**: `network_chooser_screen.rs`
**Rationale**: Network switching currently happens on a dedicated settings screen. The wallet redesign shows the network indicator in the top panel. Consider:

- Network indicator in top panel should be clickable to quick-switch
- Switching should preserve wallet selections per-network
- On Testnet/Devnet, show a prominent colored banner so users cannot mistake test networks for Mainnet
- Devnet configuration (currently .env editing) should move to in-app UI (gated behind Developer Tools)

### 11. Onboarding / Welcome Screen Improvements

**Area**: `welcome_screen.rs`
**Rationale**: The welcome screen offers four actions: Load Wallet, Create Wallet, Import Identity, Just Browse. This should be simplified based on persona analysis:

- Primary path: "Create Wallet" or "Import Wallet" (wallet-first, since all other features require a wallet)
- Secondary path: "Just Browse" for exploring Platform without a wallet
- "Import Identity" is confusing for new users -- this action should be available from the Identities screen after a wallet is loaded
- Add brief explanatory text for each option

---

## Lower Priority

### 12. Consistent Collapsible Sections

**Area**: All screens with detail sections
**Rationale**: The wallet redesign introduces a `CollapsibleSection` component. Many other screens have sections that should use this same component for visual consistency:

- Identity key sections
- Contract document property sections
- Token detail sections
- Tool output sections

### 13. Terminology Consistency Audit

**Area**: All user-facing text
**Rationale**: The wallet redesign creates a terminology guide. An audit should verify that terminology is consistent across the entire app:

- "Recovery phrase" vs "seed phrase" vs "mnemonic" (should always be "recovery phrase")
- "Credits" vs "Platform balance" (should be "Platform" for Level 0/1, "credits" for Level 2)
- "Asset lock" visibility (hidden at Level 0 across all screens, not just wallet)
- Button label consistency (always "Cancel" not "Close" for abort actions; always "Confirm" for final actions)

### 14. Keyboard Navigation

**Area**: All screens
**Rationale**: egui supports basic keyboard navigation but the app does not define consistent shortcuts. Consider:

- Tab navigation through interactive elements
- Escape to close modals consistently
- Enter to confirm primary actions
- Global shortcuts: Ctrl+R (refresh), Ctrl+S (send from wallet context)

### 15. Loading and Empty State Consistency

**Area**: All screens
**Rationale**: The wallet redesign defines clear empty states ("No Wallets Loaded") and loading states (spinners in collapsible sections). Other screens should follow the same patterns:

- Identity screen empty state: "No identities registered. Create or import an identity."
- Token screen empty state: "No tokens found. Add a token by contract ID."
- Contract screen empty state: "No contracts loaded. Add a contract to inspect."
- All loading states should use the same spinner component with descriptive text

### 16. Confirmation Dialogs for Destructive Actions

**Area**: All screens with delete/remove operations
**Rationale**: The wallet redesign specifies detailed confirmation dialogs that explain consequences. This pattern should apply to:

- Removing an identity from the app
- Removing a contract from the app
- Removing a contact
- Any operation that deletes local data

### 17. Color and Theme Consistency

**Area**: All screens, `theme.rs`
**Rationale**: The wallet redesign uses `DashColors` semantic colors (SUCCESS, WARNING, ERROR, INFO) for all status indicators. Verify that:

- All status indicators across the app use the same color mapping
- Glass morphism effects (`glass_white()`, `glass_blue()`) are used consistently
- Dark mode and light mode both look correct on all screens

### 18. Tools Screens Visibility

**Area**: `tools/`
**Rationale**: The Tools section contains developer-oriented screens (transition visualizer, proof log, document query, contract visualizer, GroveStark, platform info, masternode list diff). Consider:

- Gating the entire Tools section behind Developer Tools setting
- Or: showing a simplified "Platform Info" view for all users and gating the rest
- Tools screens should benefit from the same error handling and loading patterns

### 19. DPNS Contested Names Screen

**Area**: `dpns/dpns_contested_names_screen.rs`
**Rationale**: The DPNS contested names (voting) screen is a specialized feature. Consider:

- Progressive disclosure: show voting status simply, expand for vote details
- Integration with wallet: voting requires identity credit balance, show relevant wallet/identity context
- Terminology: ensure voting-related terms are documented in a future terminology guide extension

### 20. Accessibility Audit

**Area**: All screens
**Rationale**: egui has limited accessibility support compared to web frameworks, but basic improvements are possible:

- Ensure all interactive elements have meaningful tooltips
- Use sufficient color contrast (test with simulated color vision deficiency)
- Avoid conveying information solely through color (add text labels or icons)
- Test with screen magnification
- Document known accessibility limitations of egui for the project

---

## Cross-Cutting Patterns to Extract from Wallet Redesign

These are reusable patterns defined during the wallet redesign that should become shared components:

| Pattern | Current Location | Should Become |
|---|---|---|
| CollapsibleSection | Wallet-specific | `ui/components/collapsible_section.rs` |
| MessageBanner | Wallet-specific | `ui/components/message_banner.rs` |
| Multi-step flow (Enter -> Confirm -> Result) | Send screen | `ui/components/guided_flow.rs` or pattern doc |
| Balance display with expand | Wallet balance header | `ui/components/balance_display.rs` |
| Sortable table | Address table | Already partially generic via `egui_extras::Table` |
| Overflow menu (...) | Wallet overflow | `ui/components/overflow_menu.rs` |
| Empty state | No-wallet view | `ui/components/empty_state.rs` |
