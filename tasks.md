# Dash Evo Tool — Tauri Migration Task Backlog

> **Branch:** `react-native` (from `ralph/improvements`)
> **Goal:** Full migration from egui to Tauri 2.0 + web frontend
> **Convention:** `[META]` = research/design tasks (produce sub-tasks, no code). `[REVIEW]` = audit tasks (verify work, may produce fix tasks). All other tasks produce code + tests + commits.
> **Priority:** P0 = blocks everything, P1 = critical path, P2 = important, P3 = polish

---

## Phase 0: Project Foundation

- [x] **0.1 [META] Choose web frontend framework and UI component library** (P0)
  Evaluate React vs Svelte vs Vue for the DET frontend. Consider:
  - Component library ecosystem (we need tables, forms, modals, tabs, trees, code viewers)
  - Testing story (Testing Library support, Playwright integration)
  - TypeScript support quality
  - Community size, hiring pool, long-term viability
  - Bundle size and performance
  - Accessibility (WCAG 2.1 AA) out of the box
  - State management options that pair well with Tauri's async IPC
  Also evaluate UI component libraries for the chosen framework (e.g., Radix/shadcn, Mantine, Ant Design for React; Skeleton, shadcn-svelte for Svelte). The library must support: sortable tables, tree views, tabs, modals/dialogs, form inputs with validation, dark/light themes, toast notifications, dropdown menus, and code/JSON display.
  Produce sub-tasks for project initialization based on the choice.

  > **Decision (Run 1):**
  >
  > **Framework: React 19 + TypeScript + Vite**
  > - Largest component ecosystem — critical for 75+ screens needing sortable tables, tree views, code viewers, drag-and-drop, QR codes, formula visualization
  > - Gold-standard testing: Vitest + React Testing Library + jsdom
  > - Tauri's official docs and examples primarily use React; `@tauri-apps/api` works naturally with React hooks
  > - Largest community for troubleshooting a project of this complexity
  >
  > **Component Library: shadcn/ui (Radix primitives + Tailwind CSS)**
  > - Components are copied into the project (full ownership and customization)
  > - Built on Radix UI — best-in-class accessibility (WCAG 2.1 AA) with focus management, keyboard nav, ARIA labels
  > - Tailwind CSS theming with CSS variables — dark/light mode built-in
  > - Covers: tables, tabs, dialogs, forms (react-hook-form + zod), dropdowns, toasts, badges, cards
  > - Supplementary libraries: `@tanstack/react-table` (sortable data tables), `@tanstack/react-virtual` (virtual scrolling), `@dnd-kit` (drag-and-drop), `react-json-view-lite` (JSON viewer), `qrcode.react` (QR codes), `react-arborist` (tree views)
  >
  > **State Management: Zustand**
  > - Minimal boilerplate, TypeScript-native, async-friendly (perfect for Tauri IPC)
  > - Slices pattern for domain organization (wallets, identities, tokens, etc.)
  > - Middleware: persistence, devtools, immer
  >
  > **Build Tool: Vite**
  > - Tauri 2.0 recommended, fast HMR, excellent TypeScript/React support

  **Sub-tasks produced for task 0.2 refinement:**
  - [x] **0.1a** Install Node.js dependencies: react, react-dom, typescript, vite, @vitejs/plugin-react, tailwindcss, postcss, autoprefixer, zustand
  - [x] **0.1b** Initialize shadcn/ui: `npx shadcn@latest init`, configure components.json, install initial components (button, dialog, dropdown-menu, input, label, select, tabs, table, sonner, card, badge, separator, sheet, tooltip, popover)
  - [x] **0.1c** Install supplementary libraries: @tanstack/react-table, @tanstack/react-virtual, @tanstack/react-router (file-based routing), @dnd-kit/core, @dnd-kit/sortable, react-json-view-lite, qrcode.react, lucide-react (icons), clsx, tailwind-merge
  - [x] **0.1d** Install dev dependencies: vitest, @testing-library/react, @testing-library/jest-dom, @testing-library/user-event, jsdom, @playwright/test, eslint, prettier, eslint-plugin-react-hooks, @typescript-eslint/eslint-plugin
  - [x] **0.1e** Configure Tauri 2.0: install @tauri-apps/cli, @tauri-apps/api, create tauri.conf.json, set up src-tauri/ Cargo.toml with workspace deps

- [x] **0.2 Initialize Tauri 2.0 project with chosen frontend framework** (P0)
  Create the Tauri project scaffolding inside this worktree. Set up:
  - `src-tauri/` with Cargo.toml that depends on the existing DET crates (backend_task, model, database, context, spv)
  - Frontend project (package.json, tsconfig, framework config)
  - Tauri configuration (tauri.conf.json) with proper app metadata, window config, CSP
  - Basic "Hello World" that compiles and launches both Rust backend and web frontend
  - `.gitignore` for node_modules, target, dist
  Verify: `npx tauri dev` launches a window with the frontend visible.

- [x] **0.3 Set up testing infrastructure** (P0)
  Configure the full testing stack:
  - **Component tests:** Vitest + Testing Library (for the chosen framework) + jsdom
  - **Playwright:** Install, configure `playwright.config.ts` with the Tauri dev server, set up project for Chromium + WebKit
  - **Rust tests:** Ensure `cargo test` works in src-tauri/
  - Write one trivial test in each layer (component, Playwright, Rust) to verify the pipeline works
  - Add npm scripts: `test`, `test:e2e`, `test:components`, `lint`, `typecheck`
  Verify: `npm run test` and `npx playwright test` both pass.

- [x] **0.4 Set up CI pipeline configuration** (P2)
  Create GitHub Actions workflow (or equivalent) that runs on PR:
  - Rust: fmt check, clippy, cargo test
  - Frontend: lint, typecheck, component tests
  - Playwright: headless E2E tests
  - Build: verify `npx tauri build` succeeds (at least on one platform)
  This doesn't need to be merged yet but should be ready for when we do.

---

## Phase 1: Backend Bridge (Tauri IPC Layer)

- [x] **1.1 [META] Design the Tauri IPC command API surface** (P0)
  Map every `BackendTask` variant and `AppContext` direct-call method to Tauri IPC commands. The existing backend has:
  - 13 BackendTask domains (Identity, Document, Contract, ContestedResource, Core, DashPay, Token, Wallet, System, MnList, PlatformInfo, GroveSTARK, BroadcastStateTransition)
  - ~70 distinct operations
  - ~22 result types
  - Direct database read methods (load_local_user_identities, get_contracts, etc.)
  - Direct context methods (fee_estimator, platform_version, network, etc.)
  Design the command grouping, naming convention, serialization strategy (serde JSON), and error handling pattern. Define TypeScript type generation strategy (manual types vs. ts-rs vs. specta). Produce sub-tasks for implementing each command group.

  > **Decision (Run 10):**
  >
  > ### Complete Backend Inventory
  >
  > **13 BackendTask domains, ~120 task variants, ~100 result variants:**
  > - IdentityTask: 16 variants → IdentityResult: 10 variants
  > - DocumentTask: 8 variants → DocumentResult: 9 variants
  > - ContractTask: 7 variants → ContractResult: 11 variants
  > - ContestedResourceTask: 7 variants → ContestResult: 6 variants
  > - CoreTask: 10 variants → CoreResult: 1 variant (wraps CoreItem: 5 variants)
  > - WalletTask: 6 variants → WalletResult: 8 variants
  > - DashPayTask: 14 variants → DashPayResult: 14 variants
  > - TokenTask: 23 variants → TokenResult: 20 variants
  > - SystemTask: 2 variants → SystemResult: 1 variant
  > - MnListTask: 5 variants → MnListResult: 4 variants
  > - PlatformInfoTask: 8 variants → PlatformResult: 1 variant (wraps 3 sub-variants)
  > - GroveSTARKTask: 2 variants → GroveSTARKResult: 2 variants
  > - BroadcastStateTransition: 1 (no separate enum)
  >
  > **18 direct database methods called by UI** (bypassing BackendTask):
  > - Identity ordering, alias management, deletion
  > - Token ordering
  > - DashPay contacts, profiles, payments (save/load/update)
  > - Contract listing
  > - Onboarding completion, SPV auto-start, wallet address management
  >
  > **~76 AppContext public methods** across 6 modules:
  > - Core: initialization, animation, developer mode, fee management, platform info
  > - Settings: get/update settings, password, Dash Core config, ZMQ config
  > - Identity DB: CRUD for identities, aliases, DPNS names, scheduled votes
  > - Contract/Token DB: CRUD for contracts, tokens, balances
  > - Wallet lifecycle: SPV management, wallet bootstrapping, address registration
  > - Transaction processing: finality, asset locks
  >
  > ### TypeScript Type Generation Strategy: tauri-specta
  >
  > **Chosen: `tauri-specta` v2 (Specta + Tauri 2.0 native integration)**
  > - Automatic dependency resolution — when a parent type is registered, all dependent types are included automatically (critical for 100+ types)
  > - Native Tauri 2.0 integration — Tauri added a `specta` feature flag for AppHandle, State, Window types
  > - Generates both TypeScript types AND type-safe command wrapper functions
  > - Event type generation supported in v2
  > - `#[specta::specta]` decorator on each `#[tauri::command]` function
  > - `tauri_specta::collect_commands!` macro gathers all commands
  > - Types exported to `src/frontend/bindings.ts` automatically during dev builds
  >
  > ### IPC Command Design Decisions
  >
  > **1. Command Grouping:** One Rust module per domain (13 modules in `src-tauri/src/commands/`)
  >
  > **2. Naming Convention:** `snake_case` Rust functions, auto-converted to `camelCase` by tauri-specta for TypeScript. Prefix with domain: `identity_load`, `wallet_send_payment`, `token_mint`, etc.
  >
  > **3. Serialization:** serde JSON (Tauri default). All command args and return types derive `Serialize + Deserialize + specta::Type`. Complex Rust types (Arc, RwLock) replaced with serializable DTOs at the IPC boundary.
  >
  > **4. Error Handling:** All commands return `Result<T, String>` at the Tauri boundary. Domain errors mapped to descriptive strings. Frontend receives structured error objects.
  >
  > **5. Async Pattern:** Commands that dispatch BackendTasks are `async`. Long-running operations use Tauri events for progress/results rather than blocking the IPC call. Short reads (database queries, config) return directly.
  >
  > **6. State Access:** `tauri::State<AppState>` injected into every command. AppState wraps `Arc<AppContext>` per network + active network selection.
  >
  > **7. Direct DB Access:** The 18 UI-direct database calls will be exposed as dedicated Tauri commands (not routed through BackendTask). Grouped into `commands/db.rs` or folded into the relevant domain module.
  >
  > **8. Wallet References:** The egui code passes `Arc<RwLock<Wallet>>` to tasks. For IPC, commands accept `WalletSeedHash` (a serializable identifier) and look up the wallet from AppState.
  >
  > **9. Events (Backend → Frontend):**
  > - `task-completed` — BackendTaskSuccessResult payloads
  > - `task-error` — BackendTaskError payloads
  > - `zmq-instant-lock` — InstantLock transaction data
  > - `zmq-chain-lock` — ChainLock data
  > - `zmq-chain-locked-block` — Block + ChainLock data
  > - `zmq-connection-status` — ZMQ connection state changes
  > - `spv-status` — SPV sync progress
  > - `wallet-updated` — Wallet balance/state changes
  > - `scheduled-vote-executed` — Vote casting results

  **Sub-tasks produced:**
  - [x] **1.1a** Install and configure tauri-specta v2: Add `specta`, `tauri-specta` to src-tauri/Cargo.toml. Set up the `tauri_specta::ts::builder()` in main.rs to export bindings to `src/frontend/bindings.ts`. Add `specta` feature to the `tauri` dependency. Create a minimal test command with `#[specta::specta]` and verify TypeScript types are generated.
  - [x] **1.1b** Create IPC DTO types module: Create `src-tauri/src/dto/` with serializable Data Transfer Objects for all complex Rust types that cross the IPC boundary. These replace `Arc<RwLock<T>>`, `Arc<DataContract>`, etc. with owned, serializable structs. Start with: `WalletDto`, `SingleKeyWalletDto`, `QualifiedIdentityDto`, `DataContractDto`, `DocumentDto`, `TokenInfoDto`, `FeeResultDto`. All derive `Serialize, Deserialize, specta::Type, Clone`.

- [ ] **1.2 Implement Tauri app state and initialization** (P0)
  Create `src-tauri/src/state.rs` that:
  - Wraps `AppContext` creation for all 4 networks (reuse existing initialization from `app.rs`)
  - Manages the active network selection
  - Creates the Tokio runtime (12 workers)
  - Initializes database, SDK, system contracts, Core RPC client, wallets
  - Handles SPV manager creation
  - Provides `tauri::State<AppState>` for all commands to access
  Verify: Tauri app starts, creates AppContexts, connects to database.

- [ ] **1.3 Implement async result event system** (P0)
  Replace `egui_mpsc` channels with Tauri's event system:
  - Backend tasks emit results via `app_handle.emit("task-result", payload)`
  - ZMQ events (instant-locked transactions, chain-locked blocks) forwarded as Tauri events
  - SPV status updates forwarded as Tauri events
  - Frontend listens with `listen("task-result", callback)`
  - TypeScript types auto-generated by tauri-specta for all event payloads
  - Handle the scheduled vote polling (every 60s) in the Tauri backend
  Verify: Can dispatch a backend task and receive the result in the frontend via event.

- [ ] **1.4 Implement Identity IPC commands** (P1)
  Create `src-tauri/src/commands/identity.rs` with Tauri commands for all IdentityTask variants (16 operations):
  - load_identity, search_identity_from_wallet, search_identities_up_to_index, search_identity_by_dpns_name
  - register_identity, top_up_identity, top_up_identity_from_platform_addresses
  - add_key_to_identity, disable_keys, replace_key
  - withdraw_from_identity, transfer_credits, transfer_to_addresses
  - register_dpns_name, refresh_identity, refresh_loaded_identities_owned_dpns_names
  Also: load_local_user_identities, load_local_voting_identities, get_identity_by_id, set_identity_alias, get_identity_alias, load_identity_order, save_identity_order, delete_identity (direct DB methods)
  Each command: `#[tauri::command] #[specta::specta]`, accepts DTO args, constructs BackendTask, dispatches, returns Result<DTO, String>.
  Write Rust unit tests for serialization/deserialization of command args and results.

- [ ] **1.5 Implement Wallet & Core IPC commands** (P1)
  Create `src-tauri/src/commands/wallet.rs` and `commands/core.rs` with commands for:
  - **CoreTask (10 ops):** get_best_chain_lock, get_best_chain_locks, refresh_wallet_info, refresh_single_key_wallet_info, send_wallet_payment, send_single_key_wallet_payment, create_registration_asset_lock, create_top_up_asset_lock, recover_asset_locks, start_dash_qt
  - **WalletTask (6 ops):** generate_receive_address, fetch_platform_address_balances, transfer_platform_credits, fund_platform_address_from_asset_lock, fund_platform_address_from_wallet_utxos, withdraw_from_platform_address
  - **Direct reads:** get_wallets, get_wallet, get_selected_wallet_hash, select_wallet, wallet balance queries, remove_wallet, add_wallet_address
  - **SPV:** start_spv, stop_spv, clear_spv_data, spv_status
  All commands accept `WalletSeedHash` identifiers (not Arc<RwLock<Wallet>>).
  Write Rust unit tests.

- [ ] **1.6 Implement Contract, Document & Token IPC commands** (P1)
  Create commands for:
  - **ContractTask (7 ops):** fetch_contracts, fetch_contracts_with_descriptions, fetch_active_group_actions, remove_contract, register_data_contract, update_data_contract, save_data_contract
  - **DocumentTask (8 ops):** broadcast_document, delete_document, replace_document, transfer_document, purchase_document, set_document_price, fetch_documents, fetch_documents_page
  - **TokenTask (23 ops):** register_token_contract, query_my_token_balances, query_identity_token_balance, query_frozen_identities, query_descriptions_by_keyword, fetch_token_by_contract_id, fetch_token_by_token_id, save_token_locally, query_token_pricing, mint_tokens, transfer_tokens, burn_tokens, destroy_frozen_funds, freeze_tokens, unfreeze_tokens, pause_tokens, resume_tokens, claim_tokens, estimate_perpetual_rewards, update_token_config, purchase_tokens, set_direct_purchase_price, load_token_order, save_token_order
  - **Direct DB:** get_contracts (local), get_contract_by_id, set_contract_alias, remove_token, identity_token_balances
  Write Rust unit tests.

- [ ] **1.7 Implement DashPay, DPNS & remaining IPC commands** (P1)
  Create commands for:
  - **DashPayTask (14 ops):** load_profile, update_profile, load_contacts, load_contact_requests, fetch_contact_profile, search_profiles, send_contact_request, send_contact_request_with_proof, accept_contact_request, reject_contact_request, load_payment_history, send_payment_to_contact, update_contact_info, register_dashpay_addresses
  - **DashPay direct DB (10 ops):** save_dashpay_profile, save_dashpay_contact, save_contact_request, load_contact_private_info, save_contact_private_info, set_contact_hidden, load_pending_contact_requests, load_payment_history (local), save_payment, save_dashpay_profile_avatar_bytes
  - **ContestedResourceTask (7 ops):** query_dpns_contests, vote_on_dpns_names, schedule_dpns_votes, cast_scheduled_vote, clear_all_scheduled_votes, clear_executed_scheduled_votes, delete_scheduled_vote
  - **DPNS direct DB:** all_contested_names, ongoing_contested_names, local_dpns_names, get_scheduled_votes
  - **PlatformInfo (8 ops):** current_epoch_info, total_credits_on_platform, current_version_voting_state, current_validator_set_info, current_withdrawals_in_queue, recently_completed_withdrawals, basic_platform_info, fetch_address_balance
  - **SystemTask (2 ops):** wipe_platform_data, update_theme_preference
  - **MnListTask (5 ops):** fetch_end_dml_diff, fetch_end_qr_info, fetch_end_qr_info_with_dmls, fetch_chain_locks, fetch_diffs_chain
  - **GroveSTARKTask (2 ops):** generate_proof, verify_proof
  - **BroadcastStateTransition (1 op):** broadcast_state_transition
  - **Settings (6 ops):** get_settings, update_settings, update_main_password, update_dash_core_execution_settings, update_disable_zmq, update_onboarding_completed
  - **Context (5 ops):** get_network, get_fee_multiplier, set_fee_multiplier, is_developer_mode, enable_developer_mode
  Write Rust unit tests.

- [ ] **1.8 Configure tauri-specta TypeScript type generation** (P1)
  After all commands are implemented, verify that tauri-specta generates complete TypeScript bindings:
  - All command functions exported with correct parameter and return types
  - All DTO types exported as TypeScript interfaces
  - All event payload types exported
  - Bindings file at `src/frontend/bindings.ts` is complete and valid
  - Frontend can import and use the generated types
  Write a TypeScript test that imports the bindings and verifies key types exist.
  (Replaces manual TypeScript type definitions — tauri-specta auto-generates everything.)

- [ ] **1.9 [REVIEW] Backend bridge completeness audit** (P1)
  Systematically compare every BackendTask variant, every AppContext method called by UI screens, and every database query used by the egui UI against the implemented Tauri commands. Catalog any gaps. Check that:
  - Every operation the egui UI performs has a corresponding Tauri command
  - All TypeScript types accurately mirror Rust types (via tauri-specta generation)
  - Error handling is consistent and informative
  - Event payloads contain all necessary data
  Create fix tasks for any gaps found.

---

## Phase 2: Design System & App Shell

- [ ] **2.1 [META] Design the overall app layout, navigation, and visual language** (P0)
  Study the current egui UI (screenshots or running app) and design the new layout:
  - Left sidebar navigation (Dashpay, Identities, Contracts, Tokens, Wallets, Tools, Settings)
  - Top bar (breadcrumbs, connection status, context actions)
  - Content area layout patterns (list views, detail views, forms, wizards)
  - Modal/dialog patterns (confirmation, wallet unlock, fee review)
  - Color palette (dark + light mode), typography scale, spacing scale, border radii
  - Loading states, error states, empty states, success feedback
  - Mobile-responsive considerations (even though desktop-first)
  Document decisions. Produce sub-tasks for implementing the design system.

- [ ] **2.2 Implement design system foundation** (P0)
  Based on 2.1's decisions, create the design system:
  - CSS variables / theme tokens for colors, spacing, typography, shadows
  - Dark and light theme definitions
  - Base component styles (buttons, inputs, cards, tables, badges, tabs)
  - Layout primitives (stack, grid, sidebar layout)
  - Utility classes or styled components as appropriate
  - Theme toggle mechanism (persisted to backend settings)
  Write component tests for theme switching. Write Playwright test verifying dark/light mode.

- [ ] **2.3 Implement app shell: sidebar navigation + top bar** (P0)
  Build the persistent app chrome:
  - **Left sidebar:** Navigation items with icons for each main section (Dashpay, Identities, Contracts, Tokens, Wallets, Tools, Settings). Active state highlighting. Collapsible on narrow viewports.
  - **Top bar:** Breadcrumb trail showing current location. Connection status indicator (pulsating dot: green=connected, red=disconnected, fed by ZMQ status events). Network badge showing current network (Mainnet/Testnet/Devnet/Local).
  - **Content area:** Router outlet for screen components
  - Set up client-side routing for all main sections
  Write component tests for navigation state. Write Playwright test for navigating between sections.

- [ ] **2.4 Implement shared dialog and feedback components** (P1)
  Build reusable components used across many screens:
  - **Confirmation dialog:** Generic yes/no with customizable title, message, and button labels
  - **Wallet unlock popup:** Password entry with validation, loading state, error display
  - **Fee confirmation dialog:** Shows fee breakdown, confirm/cancel
  - **Toast/notification system:** Success, error, warning, info messages with auto-dismiss
  - **Amount input:** Numeric input with Dash/credit formatting, min/max validation, unit switching
  - **Identity selector:** Dropdown to select from loaded identities
  - **Loading overlay:** Spinner/skeleton for async operations
  - **Copy-to-clipboard button:** With visual feedback
  - **JSON/YAML viewer:** Syntax-highlighted, collapsible, with copy button
  Reference egui components: `src/ui/components/amount_input.rs` (581 lines), `src/ui/components/confirmation_dialog.rs`, `src/ui/components/wallet_unlock_popup.rs`, `src/ui/components/identity_selector.rs`
  Write component tests for each.

- [ ] **2.5 Implement welcome/onboarding screen** (P1)
  Port the welcome screen that appears on first launch:
  - Action cards: "Load Wallet", "Create Wallet", "Import Identity", "Just Browse"
  - Each action navigates to the appropriate screen
  - "Don't show again" persisted to backend settings
  Reference: `src/ui/welcome_screen.rs` or the welcome logic in `src/app.rs`
  Write component test. Write Playwright test for onboarding flow.

- [ ] **2.6 Implement network chooser/settings screen** (P1)
  Port the network configuration screen:
  - Network selection (Mainnet, Testnet, Devnet, Local) with visual cards
  - Connection status per network
  - Core RPC configuration (host, port, auth)
  - SPV mode toggle (developer mode only)
  - SPV sync progress visualization
  - Start/stop Dash Core button
  - Settings: theme, developer mode, ZMQ toggle, overwrite dash.conf, auto-start SPV, close Dash-Qt on exit
  Reference: `src/ui/network_chooser_screen.rs` (1,916 lines)
  Write component tests. Write Playwright test for network switching.

- [ ] **2.7 [REVIEW] App shell and design system quality audit** (P1)
  Review the implemented shell, design system, and shared components:
  - Visual consistency across light/dark themes
  - Accessibility: keyboard navigation, screen reader labels, focus management, color contrast
  - Responsive behavior at different window sizes
  - Component API consistency (props patterns, event handling)
  - Test coverage completeness
  Create fix tasks for any issues.

---

## Phase 3: Wallet Screens

- [ ] **3.1 [META] Design wallet screens UX** (P1)
  Review all wallet functionality in the egui version and design improved UX:
  - Wallet list/portfolio view (HD + single-key wallets together)
  - Wallet detail view (accounts, addresses, balances, UTXOs)
  - Send flow (simple + advanced modes, multiple recipients, fee selection)
  - Receive flow (address display, QR code)
  - Asset lock creation and management
  - Platform address operations (funding, withdrawal, transfer)
  Files to review: `src/ui/wallets/wallets_screen/mod.rs` (2,030 lines), `src/ui/wallets/send_screen/mod.rs` (1,725 lines), all files in `src/ui/wallets/`
  Identify UX improvements over current implementation. Produce implementation sub-tasks.

- [ ] **3.2 Implement wallet list and detail screens** (P1)
  Build the main wallet management interface:
  - Wallet list showing all HD and single-key wallets with balances
  - Wallet selector dropdown
  - HD wallet detail: account tree, address list with balances, sortable columns, hide zero-balance toggle
  - Single-key wallet detail: address, balance, UTXO list with pagination
  - Wallet actions: rename, delete (with confirmation), refresh
  - Platform address section with balances
  - Private key display (behind wallet unlock)
  Reference: `wallets_screen/mod.rs` — catalog EVERY action button, menu item, and display field
  Write component tests. Write Playwright tests for wallet selection and detail viewing.

- [ ] **3.3 Implement add wallet and import mnemonic screens** (P1)
  Build wallet creation flows:
  - **New wallet:** Generate mnemonic, display seed words, set password, name wallet
  - **Import mnemonic:** Enter 12/24 words, set password, name wallet, import
  - Input validation, error handling, loading states
  Reference: `add_new_wallet_screen.rs`, `import_mnemonic_screen.rs`
  Write component tests. Write Playwright test for full create/import flow.

- [ ] **3.4 Implement HD wallet send screen** (P1)
  Build the send transaction flow for HD wallets:
  - Simple mode: single recipient, amount, fee level
  - Advanced mode: multiple recipients, manual UTXO selection, custom fee
  - Address validation
  - Amount validation (balance check, dust limit)
  - Fee estimation and display
  - Wallet unlock step
  - Fee confirmation dialog
  - Transaction broadcast and result display
  Reference: `send_screen/mod.rs` (1,725 lines) — this is complex, trace every code path
  Write component tests for form validation. Write Playwright test for send flow.

- [ ] **3.5 Implement single-key wallet send and asset lock screens** (P1)
  Build remaining wallet transaction screens:
  - **Single-key send:** Similar to HD send but with UTXO selection from single-key wallet
  - **Create asset lock:** Wallet selection, amount, script type, unlock conditions
  - **Asset lock detail:** View lock details, status, use for identity creation
  Reference: `single_key_send_screen.rs`, `create_asset_lock_screen.rs`, `asset_lock_detail_screen.rs`
  Write component tests. Write Playwright tests.

- [ ] **3.6 [REVIEW] Wallet screens functionality parity** (P1)
  Exhaustive comparison of every wallet action in egui vs Tauri:
  - Open `wallets_screen/mod.rs` and trace every button, menu item, dialog, and display element
  - Verify each has a corresponding UI element and IPC command in the Tauri version
  - Check: wallet refresh modes (Core only, Platform full/terminal, Combined), address operations (copy, view key, fund platform), asset lock recovery, platform address funding
  - Verify test coverage for critical paths
  Create fix tasks for gaps.

---

## Phase 4: Identity Screens

- [ ] **4.1 [META] Design identity screens UX** (P1)
  Review all identity functionality and design improved UX:
  - Identity list with sortable columns and drag-drop reordering
  - Identity creation wizard (4 funding methods, key configuration)
  - Key management (view, add, disable, replace)
  - Identity operations (top-up, withdraw, transfer, register DPNS name)
  Files to review: All files in `src/ui/identities/` — catalog every screen, dialog, and user action
  Produce implementation sub-tasks.

- [ ] **4.2 Implement identity list screen** (P1)
  Build the main identity management interface:
  - Table with columns: Alias, Identity ID, In Wallet, Type, Balance
  - Sortable columns (ascending/descending)
  - Drag-and-drop reordering (persisted to database)
  - Inline alias editing
  - Context menu per identity: View Keys, Register DPNS Name, Top Up, Withdraw, Transfer, Delete
  - Refresh identity balances (individual and bulk)
  - Add identity buttons: "Add New" and "Add Existing"
  Reference: `identities_screen.rs` (1,258 lines)
  Write component tests. Write Playwright test for identity list interactions.

- [ ] **4.3 Implement add new identity screen** (P1)
  Build the identity creation flow with all 4 funding methods:
  - **By Wallet QR Code:** Generate receive address, display QR, wait for funding
  - **By Unused Balance:** Select wallet with balance, specify amount
  - **By Unused Asset Lock:** Select from existing asset locks
  - **By Platform Address:** Fund via platform address
  - Key configuration step: select key types (EC/BLS), purposes (AUTH/VOTING/TRANSFER/SUPPORT), security levels
  - Identity index selection
  - Confirmation and creation
  Reference: `add_new_identity_screen/mod.rs` and all sub-files (by_wallet_qr_code.rs, by_unused_balance.rs, by_unused_asset_lock.rs, by_platform_address.rs)
  Write component tests. Write Playwright test for at least one funding method.

- [ ] **4.4 Implement identity keys, top-up, withdraw, and transfer screens** (P1)
  Build remaining identity operation screens:
  - **Keys screen:** List all identity keys with metadata, add new key, view key details, disable/enable key
  - **Key info:** View key metadata, export private key
  - **Add existing identity:** Input identity ID (Base58/Hex), load from blockchain
  - **Top up:** 4 funding methods (same as creation), amount input
  - **Withdraw:** Destination address, amount, fee
  - **Transfer:** Destination identity ID, amount
  Reference: `keys/keys_screen.rs`, `keys/key_info_screen.rs`, `keys/add_key_screen.rs`, `add_existing_identity_screen.rs`, `top_up_identity_screen/`, `withdraw_screen.rs`, `transfer_screen.rs`
  Write component tests. Write Playwright tests for key operations.

- [ ] **4.5 [REVIEW] Identity screens functionality parity** (P1)
  Exhaustive comparison against all egui identity screens. Verify every action, dialog, and display element is present and working. Check DPNS name registration flow (which bridges identities and DPNS). Create fix tasks for gaps.

---

## Phase 5: DPNS Contest & Voting Screens

- [ ] **5.1 [META] Design DPNS contest/voting UX** (P2)
  Review the DPNS contested names functionality and design improved UX. This is one of the most complex screens in DET:
  - Active contests table with real-time data (locked votes, abstain votes, ending time, top contender)
  - Single vote flow (immediate cast or schedule for later)
  - Bulk voting (select multiple names, apply same vote/schedule to all)
  - Past contests (historical view)
  - My usernames (owned names, renewal, transfer)
  - Scheduled votes management (view, cancel, execute early)
  Reference: `dpns_contested_names_screen.rs` (2,173 lines) — this file handles ALL 4 tabs
  Produce implementation sub-tasks.

- [ ] **5.2 Implement DPNS active contests and voting screens** (P2)
  Build the contest viewing and voting interface:
  - Sortable table: Contested Name, Locked Votes, Abstain Votes, Ending Time, Last Updated, Awarded To
  - Click to expand contest details and vote
  - Vote options: vote for specific contender, abstain, schedule for later
  - Bulk voting: checkbox selection, bulk action bar, apply vote/schedule to all selected
  - Register new DPNS name button
  Reference: `dpns_contested_names_screen.rs` — trace the Active and voting code paths
  Write component tests. Write Playwright test for viewing contests and casting a vote.

- [ ] **5.3 Implement DPNS past contests, my usernames, and scheduled votes** (P2)
  Build the remaining DPNS tabs:
  - **Past contests:** Historical view of completed contests with winners
  - **My usernames:** Names owned by loaded identities, renewal info, transfer
  - **Scheduled votes:** List of pending scheduled votes, cancel, execute early, status tracking
  Reference: `dpns_contested_names_screen.rs` — trace the Past, Owned, and ScheduledVotes code paths
  Write component tests. Write Playwright tests.

- [ ] **5.4 [REVIEW] DPNS screens functionality parity** (P2)
  Verify all DPNS actions work. The egui implementation is 2,173 lines of dense logic — ensure nothing was missed. Create fix tasks for gaps.

---

## Phase 6: Contract & Document Screens

- [ ] **6.1 [META] Design contract/document browser UX** (P2)
  Review the contract and document management functionality:
  - Contract browser with system contracts and user contracts
  - Document querying with index selection, pagination, JSON/YAML display
  - Contract registration, update, and management
  - Document CRUD operations (create, delete, replace, transfer, purchase, set price)
  - Group actions
  Files to review: All files in `src/ui/contracts_documents/`
  Produce implementation sub-tasks.

- [ ] **6.2 Implement contract browser and document query screen** (P2)
  Build the main contract/document interface:
  - Contract list (system contracts + user-added contracts)
  - Contract search/add by ID
  - Document type selector per contract
  - Query index selection
  - Document query execution with results display
  - Pagination (next/previous)
  - Toggle between JSON and YAML display
  - Copy document data
  Reference: `contracts_documents_screen.rs` — very complex, trace carefully
  Write component tests. Write Playwright test for querying documents.

- [ ] **6.3 Implement contract registration, update, and document action screens** (P2)
  Build the remaining contract/document screens:
  - **Register contract:** JSON input, identity selection, key selection, wallet unlock, fee confirmation, broadcast
  - **Update contract:** Select existing contract, edit JSON, identity/key selection, broadcast
  - **Add contracts:** Input contract ID, fetch from platform
  - **Document actions:** Create, delete, replace, transfer, purchase, set price — each is a form with identity/key selection, wallet unlock, and fee confirmation
  - **Group actions:** Batch operations on documents
  Reference: `register_contract_screen.rs`, `update_contract_screen.rs`, `add_contracts_screen.rs`, `document_action_screen.rs`, `group_actions_screen.rs`
  Write component tests. Write Playwright tests.

- [ ] **6.4 [REVIEW] Contract/document screens functionality parity** (P2)
  Verify all contract and document operations work. Create fix tasks for gaps.

---

## Phase 7: Token Screens

- [ ] **7.1 [META] Design token screens UX** (P2)
  Review ALL token functionality — this is the largest screen group with 15+ sub-screens:
  - Token portfolio view (My Tokens tab)
  - Token search and discovery
  - Token creator (extremely complex: name/symbol/decimals, supply, distribution functions with formula visualization, freezing rules, pausing, claims, access control)
  - 12+ token action screens (transfer, mint, burn, freeze, unfreeze, pause, resume, claim, view claims, update config, purchase, set price, add by ID)
  Files to review: All files in `src/ui/tokens/`
  Produce implementation sub-tasks.

- [ ] **7.2 Implement token portfolio and search screens** (P2)
  Build the main token interface:
  - **My Tokens tab:** List all tokens owned by loaded identities with name, symbol, balance, decimals. Token actions accessible per row.
  - **Search Tokens tab:** Keyword search, browse results, view details, purchase
  - **Add Token by ID:** Input contract ID, fetch and track
  - Token detail expansion with full metadata
  Reference: `tokens_screen/mod.rs` (2,187 lines) — MyTokens and SearchTokens subscreens
  Write component tests. Write Playwright test for token portfolio viewing.

- [ ] **7.3 Implement token creator screen** (P2)
  Build the token creation wizard — one of the most complex screens:
  - Basic info: name, symbol, description, decimals
  - Supply: initial mint, maximum supply, mint cap per transaction
  - Distribution: function selection (Linear, Log, Polynomial, Exponential, Inverse Log), parameter configuration, formula visualization/preview
  - Perpetual distributions configuration
  - Freezing: enable/disable, requirements, conditions
  - Pausing: enable/disable, requirements
  - Claims: enable/disable, amounts, intervals
  - Access control: action takers configuration (who can mint, freeze, pause, etc.)
  - Review and create: identity selection, wallet unlock, fee confirmation, broadcast
  Reference: `tokens_screen/mod.rs` — TokenCreator subscreen, `token_creator.rs`, `distributions.rs`
  Write component tests for each wizard step. Write Playwright test for token creation flow.

- [ ] **7.4 Implement token action screens** (P2)
  Build all token operation screens:
  - Transfer, Mint, Burn, Freeze, Unfreeze, Pause, Resume tokens
  - Claim tokens and view claims
  - Update token configuration
  - Purchase tokens and set token price
  Each follows a similar pattern: select token, input parameters, select identity/key, wallet unlock, fee confirmation, broadcast
  Reference: All token action screen files in `src/ui/tokens/` (transfer_tokens_screen.rs, mint_tokens_screen.rs, burn_tokens_screen.rs, freeze_tokens_screen.rs, unfreeze_tokens_screen.rs, pause_tokens_screen.rs, resume_tokens_screen.rs, claim_tokens_screen.rs, view_token_claims_screen.rs, update_token_config.rs, direct_token_purchase_screen.rs, set_token_price_screen.rs)
  Write component tests. Write Playwright tests for transfer and mint flows.

- [ ] **7.5 [REVIEW] Token screens functionality parity** (P2)
  Exhaustive comparison. Token screens are the most feature-rich area — verify ALL 12+ action types work, token creator wizard has all options, portfolio displays correctly. Create fix tasks.

---

## Phase 8: DashPay Screens

- [ ] **8.1 [META] Design DashPay social/payments UX** (P2)
  Review all DashPay functionality and design improved UX:
  - Profile management (display name, avatar, bio)
  - Contact list with search, add, accept/reject requests
  - Payment sending to contacts
  - Payment history
  - Profile search and discovery
  - QR code generation
  Files to review: All files in `src/ui/dashpay/`
  Produce implementation sub-tasks.

- [ ] **8.2 Implement DashPay profile and contacts screens** (P2)
  Build the DashPay social interface:
  - **Profile tab:** View/edit display name, avatar, bio. Publish profile to blockchain.
  - **Contacts tab:** Contact list with search. Add contact by ID. Accept/reject pending requests. Contact detail view with edit alias, view profile, send payment, remove.
  - **Contact profile viewer:** Read-only view of another user's profile
  - **Contact info editor:** Edit contact metadata
  Reference: `dashpay_screen.rs`, `profile_screen.rs`, `contacts_list.rs`, `add_contact_screen.rs`, `contact_details.rs`, `contact_profile_viewer.rs`, `contact_info_editor.rs`
  Write component tests. Write Playwright tests.

- [ ] **8.3 Implement DashPay payments, search, and QR screens** (P2)
  Build remaining DashPay screens:
  - **Send payment:** Select sender identity, recipient contact, amount, optional memo, confirm
  - **Payment history:** List with filtering, detail view, retry failed payments
  - **Profile search:** Search public profiles, view results, add as contact or send payment
  - **QR code generator:** Generate/display QR for identity sharing
  Reference: `send_payment.rs`, `profile_search.rs`, `qr_code_generator.rs`
  Write component tests. Write Playwright tests.

- [ ] **8.4 [REVIEW] DashPay screens functionality parity** (P2)
  Verify all DashPay social features work. Create fix tasks.

---

## Phase 9: Tools Screens

- [ ] **9.1 [META] Design tools screens UX** (P2)
  Review all developer/power-user tools and design improved UX:
  - Platform info queries
  - State transition visualizer and broadcaster
  - Proof log and proof visualizer
  - Contract and document visualizers
  - Masternode list diff viewer (4 complex tabs)
  - GroveSTARK proof viewer
  - Address balance lookup
  Files to review: All files in `src/ui/tools/`
  Produce implementation sub-tasks.

- [ ] **9.2 Implement platform info and address balance tools** (P2)
  Build the simpler tools:
  - **Platform info:** Buttons for each query type (basic info, epoch, credits, version voting, validators, withdrawals). Display results in expandable panels.
  - **Address balance:** Input address, query Core/Platform balance, display results
  Reference: `platform_info_screen.rs`, `address_balance_screen.rs`
  Write component tests. Write Playwright tests.

- [ ] **9.3 Implement transition, proof, contract, and document visualizers** (P2)
  Build the data visualization tools:
  - **Transition visualizer:** Paste hex/base64/CSV data, parse, display formatted JSON, broadcast to platform
  - **Proof log:** View SPV proof history, filter, export
  - **Proof visualizer:** Input proof data, parse structure, validate, display verification status
  - **Contract visualizer:** Paste JSON, visualize schema, data types, indexes
  - **Document visualizer:** Paste JSON/CBOR, parse, validate against contract schema
  Reference: `transition_visualizer_screen.rs`, `proof_log_screen.rs`, `proof_visualizer_screen.rs`, `contract_visualizer_screen.rs`, `document_visualizer_screen.rs`
  Write component tests. Write Playwright tests.

- [ ] **9.4 Implement masternode list diff and GroveSTARK screens** (P2)
  Build the most complex tool screens:
  - **Masternode list diff (4 tabs):**
    - Core Items: chain-locked blocks, instant-send transactions via ZMQ
    - MNList Diffs: query diffs between block heights, view entries
    - Quorum Viewer: quorum snapshots, composition, member details
    - QR Info: quorum rotation info messages
  - **GroveSTARK:** STARK proof input, analysis, verification
  Reference: `masternode_list_diff_screen/` (mod.rs + 3 tab files, 2,376 lines total), `grovestark_screen.rs`
  Write component tests. Write Playwright tests.

- [ ] **9.5 [REVIEW] Tools screens functionality parity** (P2)
  Verify all tools work correctly. The masternode list diff screen is particularly complex — verify all 4 tabs. Create fix tasks.

---

## Phase 10: Integration, Polish & Final Audit

- [ ] **10.1 [META] Full functionality audit — complete action inventory comparison** (P0)
  Systematically go through EVERY screen in the egui version and the Tauri version side by side. For each screen:
  1. List every user action in the egui version (buttons, menus, dialogs, keyboard shortcuts)
  2. Verify the action exists and works in the Tauri version
  3. Note any differences in behavior
  This is the definitive "zero functionality loss" verification. Produce fix tasks for every gap found.

- [ ] **10.2 Implement drag-and-drop reordering** (P2)
  Add drag-and-drop for lists that support reordering:
  - Identity list reordering (persisted to database)
  - Token list reordering (persisted to database)
  - Any other lists that support custom ordering in the egui version
  Write component tests. Write Playwright test for drag-and-drop.

- [ ] **10.3 Implement keyboard shortcuts and accessibility** (P2)
  Add keyboard navigation and accessibility features:
  - Tab navigation through all interactive elements
  - Focus management for dialogs and modals
  - ARIA labels for all interactive elements
  - Screen reader compatibility
  - Keyboard shortcuts for common actions (if the egui version had any)
  - Color contrast verification (WCAG 2.1 AA)
  Write accessibility tests (axe-core via Playwright).

- [ ] **10.4 Cross-platform build and testing** (P1)
  Verify the Tauri app builds and works on all target platforms:
  - macOS (x86_64 and aarch64)
  - Windows (x86_64)
  - Linux (x86_64 and aarch64)
  - Test: app launches, connects to database, loads wallets, navigates between screens
  - Verify platform-specific paths (app data directory, .env file location)
  - Check code signing configuration for macOS
  Document any platform-specific issues. Create fix tasks.

- [ ] **10.5 Performance optimization and bundle size** (P2)
  Measure and optimize:
  - Frontend bundle size (target: under 2MB)
  - App startup time (target: under 2 seconds)
  - Screen transition smoothness
  - Large list rendering (100+ identities, 1000+ transactions)
  - Memory usage during extended sessions
  - IPC latency for common operations
  Apply optimizations: code splitting, lazy loading, virtual scrolling for large lists, memoization.

- [ ] **10.6 [META] Final UX polish and edge case review** (P1)
  Review the complete app for polish:
  - Consistent loading states everywhere
  - Error handling for all failure modes (network down, invalid input, permission denied, etc.)
  - Empty states for all lists ("No wallets yet — create one to get started")
  - Confirmation before destructive actions (delete wallet, remove identity)
  - Form validation feedback (inline errors, disabled submit until valid)
  - Responsive behavior at common window sizes
  - Animation and transitions (subtle, professional)
  Produce fix/polish tasks.

- [ ] **10.7 [REVIEW] Final comprehensive review** (P0)
  The ultimate quality gate before considering the migration complete:
  - Functionality: every egui action works in Tauri
  - Tests: every screen has component tests and critical paths have E2E tests
  - UI/UX: professional, consistent, accessible
  - Performance: responsive, reasonable startup time and memory usage
  - Cross-platform: builds and runs on macOS, Windows, Linux
  - Code quality: no lint warnings, no type errors, clean architecture
  This review may produce a final batch of fix tasks.

---

## Progress Tracking

| Metric | Count |
|---|---|
| Total tasks (top-level) | 60 |
| META tasks | 13 |
| REVIEW tasks | 11 |
| Implementation tasks | 36 |
| Completed | 12 |
| Remaining | 50 |

*Note: META tasks will expand into sub-tasks. The actual task count will grow significantly as META tasks are completed. Estimated total including sub-tasks: 150-250.*
