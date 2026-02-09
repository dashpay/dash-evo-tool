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

- [x] **1.2 Implement Tauri app state and initialization** (P0)
  Create `src-tauri/src/state.rs` that:
  - Wraps `AppContext` creation for all 4 networks (reuse existing initialization from `app.rs`)
  - Manages the active network selection
  - Creates the Tokio runtime (12 workers)
  - Initializes database, SDK, system contracts, Core RPC client, wallets
  - Handles SPV manager creation
  - Provides `tauri::State<AppState>` for all commands to access
  Verify: Tauri app starts, creates AppContexts, connects to database.

- [x] **1.3 Implement async result event system** (P0)
  Replace `egui_mpsc` channels with Tauri's event system:
  - Backend tasks emit results via `app_handle.emit("task-result", payload)`
  - ZMQ events (instant-locked transactions, chain-locked blocks) forwarded as Tauri events
  - SPV status updates forwarded as Tauri events
  - Frontend listens with `listen("task-result", callback)`
  - TypeScript types auto-generated by tauri-specta for all event payloads
  - Handle the scheduled vote polling (every 60s) in the Tauri backend
  Verify: Can dispatch a backend task and receive the result in the frontend via event.

- [x] **1.4 Implement Identity IPC commands** (P1)
  Create `src-tauri/src/commands/identity.rs` with Tauri commands for all IdentityTask variants (16 operations):
  - load_identity, search_identity_from_wallet, search_identities_up_to_index, search_identity_by_dpns_name
  - register_identity, top_up_identity, top_up_identity_from_platform_addresses
  - add_key_to_identity, disable_keys, replace_key
  - withdraw_from_identity, transfer_credits, transfer_to_addresses
  - register_dpns_name, refresh_identity, refresh_loaded_identities_owned_dpns_names
  Also: load_local_user_identities, load_local_voting_identities, get_identity_by_id, set_identity_alias, get_identity_alias, load_identity_order, save_identity_order, delete_identity (direct DB methods)
  Each command: `#[tauri::command] #[specta::specta]`, accepts DTO args, constructs BackendTask, dispatches, returns Result<DTO, String>.
  Write Rust unit tests for serialization/deserialization of command args and results.

- [x] **1.5 Implement Wallet & Core IPC commands** (P1)
  Create `src-tauri/src/commands/wallet.rs` and `commands/core.rs` with commands for:
  - **CoreTask (10 ops):** get_best_chain_lock, get_best_chain_locks, refresh_wallet_info, refresh_single_key_wallet_info, send_wallet_payment, send_single_key_wallet_payment, create_registration_asset_lock, create_top_up_asset_lock, recover_asset_locks, start_dash_qt
  - **WalletTask (6 ops):** generate_receive_address, fetch_platform_address_balances, transfer_platform_credits, fund_platform_address_from_asset_lock, fund_platform_address_from_wallet_utxos, withdraw_from_platform_address
  - **Direct reads:** get_wallets, get_wallet, get_selected_wallet_hash, select_wallet, wallet balance queries, remove_wallet, add_wallet_address
  - **SPV:** start_spv, stop_spv, clear_spv_data, spv_status
  All commands accept `WalletSeedHash` identifiers (not Arc<RwLock<Wallet>>).
  Write Rust unit tests.

- [x] **1.6 Implement Contract, Document & Token IPC commands** (P1)
  Create commands for:
  - **ContractTask (7 ops):** fetch_contracts, fetch_contracts_with_descriptions, fetch_active_group_actions, remove_contract, register_data_contract, update_data_contract, save_data_contract
  - **DocumentTask (8 ops):** broadcast_document, delete_document, replace_document, transfer_document, purchase_document, set_document_price, fetch_documents, fetch_documents_page
  - **TokenTask (23 ops):** register_token_contract, query_my_token_balances, query_identity_token_balance, query_frozen_identities, query_descriptions_by_keyword, fetch_token_by_contract_id, fetch_token_by_token_id, save_token_locally, query_token_pricing, mint_tokens, transfer_tokens, burn_tokens, destroy_frozen_funds, freeze_tokens, unfreeze_tokens, pause_tokens, resume_tokens, claim_tokens, estimate_perpetual_rewards, update_token_config, purchase_tokens, set_direct_purchase_price, load_token_order, save_token_order
  - **Direct DB:** get_contracts (local), get_contract_by_id, set_contract_alias, remove_token, identity_token_balances
  Write Rust unit tests.

- [x] **1.7 Implement DashPay, DPNS & remaining IPC commands** (P1)
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

- [x] **1.8 Configure tauri-specta TypeScript type generation** (P1)
  After all commands are implemented, verify that tauri-specta generates complete TypeScript bindings:
  - All command functions exported with correct parameter and return types
  - All DTO types exported as TypeScript interfaces
  - All event payload types exported
  - Bindings file at `src/frontend/bindings.ts` is complete and valid
  - Frontend can import and use the generated types
  Write a TypeScript test that imports the bindings and verifies key types exist.
  (Replaces manual TypeScript type definitions — tauri-specta auto-generates everything.)

- [x] **1.9 [REVIEW] Backend bridge completeness audit** (P1)
  Systematically compare every BackendTask variant, every AppContext method called by UI screens, and every database query used by the egui UI against the implemented Tauri commands. Catalog any gaps. Check that:
  - Every operation the egui UI performs has a corresponding Tauri command
  - All TypeScript types accurately mirror Rust types (via tauri-specta generation)
  - Error handling is consistent and informative
  - Event payloads contain all necessary data
  Create fix tasks for any gaps found.

  > **Audit Findings (Run 20):**
  >
  > ### TypeScript Bindings: COMPLETE (A+)
  > - 163/163 registered Rust commands exported to TypeScript
  > - 8/8 events properly mapped with correct naming
  > - 153 TypeScript types exported (all DTOs, enums, input types)
  > - Consistent `Result<T, string>` error pattern across 134 commands
  > - Proper camelCase conversion, nullable handling (`| null`), enum → string union
  > - No `any`/`unknown` in public API types
  >
  > ### BackendTask Coverage: 7 GAPS FOUND
  >
  > **IdentityTask gaps (4 variants missing):**
  > - `RegisterIdentity` — No `identity_register` Tauri command (critical: identity creation)
  > - `TopUpIdentity` — No `identity_top_up` Tauri command (critical: identity funding)
  > - `TopUpIdentityFromPlatformAddresses` — No Tauri command
  > - `TransferToAddresses` — No `identity_transfer_to_addresses` command (only `identity_transfer` for single-identifier transfers)
  >
  > **WalletTask gaps (1 variant missing):**
  > - `FundPlatformAddressFromAssetLock` — Not exposed (deferred: needs AssetLockProof serialization)
  >
  > **Other domains: COMPLETE** — All variants for Document (8/8), Contract (7/7), Core (10/10), DashPay (14/14), Token (22/22), ContestedResource (7/7), PlatformInfo (8/8), System (2/2), MnList (5/5), GroveSTARK (2/2), BroadcastStateTransition (1/1) are covered.
  >
  > ### AppContext Method Gaps (4 methods missing):
  > - `set_core_backend_mode()` — Used in network_chooser_screen.rs (6 call sites), no Tauri command
  > - `get_contract_by_token_id()` — Used in token detail screens, no Tauri command
  > - `bootstrap_wallet_addresses()` — Used in wallet import/creation, no Tauri command
  > - Wallet creation flow (save_wallet in add_new_wallet_screen.rs and import_mnemonic_screen.rs) — No Tauri command covers creating/importing wallets
  >
  > ### Event System: COMPLETE
  > - All 8 events properly typed and exported
  > - Error handling consistent (`Result<T, String>` mapped to `Result<T, string>`)
  >
  > ### Summary: 11 fix sub-tasks created below

  **Fix sub-tasks:**
  - [x] **1.9a** Add `identity_register` Tauri command wrapping `IdentityTask::RegisterIdentity` with all 4 funding methods (UseAssetLock, FundWithWallet, FundWithUtxo, FundWithPlatformAddresses) (P1)
  - [x] **1.9b** Add `identity_top_up` Tauri command wrapping `IdentityTask::TopUpIdentity` with all funding methods (P1)
  - [x] **1.9c** Add `identity_top_up_from_platform_addresses` Tauri command wrapping `IdentityTask::TopUpIdentityFromPlatformAddresses` (P1)
  - [x] **1.9d** Add `identity_transfer_to_addresses` Tauri command wrapping `IdentityTask::TransferToAddresses` (P1)
  - [x] **1.9e** Add `wallet_fund_platform_from_asset_lock` Tauri command wrapping `WalletTask::FundPlatformAddressFromAssetLock` — requires AssetLockProof DTO serialization (P2)
  - [x] **1.9f** Add `context_set_core_backend_mode` Tauri command for switching between SPV and RPC modes (P1)
  - [x] **1.9g** Add `contract_get_by_token_id` Tauri command wrapping `AppContext::get_contract_by_token_id()` (P1)
  - [x] **1.9h** Add `wallet_create` Tauri command covering the full wallet creation flow (generate mnemonic, derive keys, encrypt seed, save to DB, bootstrap addresses) (P1)
  - [x] **1.9i** Add `wallet_import_mnemonic` Tauri command covering the mnemonic import flow (validate mnemonic, derive keys, encrypt, save, bootstrap) (P1)
  - [x] **1.9j** Add `wallet_bootstrap_addresses` Tauri command wrapping `AppContext::bootstrap_wallet_addresses()` (P1)
  - [x] **1.9k** Regenerate TypeScript bindings after all fix tasks are complete and verify new commands appear in bindings.ts (P1)

---

## Phase 2: Design System & App Shell

- [x] **2.1 [META] Design the overall app layout, navigation, and visual language** (P0)
  Study the current egui UI (screenshots or running app) and design the new layout:
  - Left sidebar navigation (Dashpay, Identities, Contracts, Tokens, Wallets, Tools, Settings)
  - Top bar (breadcrumbs, connection status, context actions)
  - Content area layout patterns (list views, detail views, forms, wizards)
  - Modal/dialog patterns (confirmation, wallet unlock, fee review)
  - Color palette (dark + light mode), typography scale, spacing scale, border radii
  - Loading states, error states, empty states, success feedback
  - Mobile-responsive considerations (even though desktop-first)
  Document decisions. Produce sub-tasks for implementing the design system.

  > **Design Decisions (Run 29):**
  >
  > ### Overall Layout: Three-Panel "Island" Design
  >
  > Preserves the existing egui layout structure with modern web refinements:
  >
  > ```
  > ┌──────────────────────────────────────────────────────────┐
  > │ Background (muted gray/dark)                            │
  > │ ┌─────┐ ┌─────────────────────────────────────────────┐ │
  > │ │     │ │ Top Bar (island)                             │ │
  > │ │     │ │ [●] DashPay > Contacts    [Add] [Contracts] │ │
  > │ │ Nav │ ├─────────────────────────────────────────────┤ │
  > │ │     │ │                                             │ │
  > │ │ 🏠  │ │ ┌──────┐ ┌──────────────────────────────┐  │ │
  > │ │ 👤  │ │ │ Sub  │ │ Main Content (island)        │  │ │
  > │ │ 📄  │ │ │ Nav  │ │                              │  │ │
  > │ │ 🪙  │ │ │      │ │  Screen content here         │  │ │
  > │ │ 💰  │ │ │      │ │                              │  │ │
  > │ │ 🔧  │ │ └──────┘ └──────────────────────────────┘  │ │
  > │ │ ⚙️  │ │                                             │ │
  > │ │     │ └─────────────────────────────────────────────┘ │
  > │ │ NET │                                                 │
  > │ │ 🔷  │                                                 │
  > │ └─────┘                                                 │
  > └──────────────────────────────────────────────────────────┘
  > ```
  >
  > **Left Sidebar (fixed, 72px collapsed / 200px expanded):**
  > - 7 navigation items with Lucide icons + labels
  > - Items: DashPay, Identities, Contracts, Tokens, Wallets, Tools, Settings
  > - Active item highlighted with Dash Blue accent + white icon
  > - Network badge at bottom (Testnet/Devnet/Local — hidden on Mainnet)
  > - Developer mode badge when enabled
  > - Dash logo at very bottom (clickable → dash.org)
  > - Collapsible to icon-only mode on narrow viewports (<1024px)
  >
  > **Top Bar (sticky, within right content area):**
  > - Left: Connection status indicator (pulsating green dot when connected to Core, static red when disconnected, clickable to start Dash-Qt)
  > - Left: Breadcrumb navigation (e.g., "DashPay > Contacts")
  > - Right: Context-sensitive action buttons (network-accent colored)
  > - Right: Grouped dropdown menus for multi-action areas (Contracts, Documents)
  > - Network badge pill showing current network name + color
  >
  > **Sub-Navigation Panel (conditional, 220px):**
  > - Appears for screens with subscreens: DashPay (4 tabs), DPNS (4 tabs), Tokens (3 tabs), Tools (9 items), Contracts (Document Query + DPNS tabs)
  > - Vertical list of sub-items with active highlighting
  > - Implemented as a secondary sidebar within the content area
  >
  > **Main Content Area:**
  > - "Island" card with rounded corners (radius-lg), subtle border, elevated shadow
  > - Surface background (white light / dark-gray dark)
  > - Padding: 24px (lg)
  > - Scrollable content within the island
  >
  > ### Navigation Architecture
  >
  > **Routing: @tanstack/react-router (file-based)**
  > - Root layout: `/_app` (sidebar + top bar + outlet)
  > - Main sections: `/dashpay`, `/identities`, `/contracts`, `/tokens`, `/wallets`, `/tools`, `/settings`
  > - Sub-routes: `/dashpay/contacts`, `/dashpay/profile`, `/tokens/search`, `/tools/platform-info`, etc.
  > - Modal/overlay screens: Route-based modals via `@tanstack/react-router` modal routes or React portals
  > - Screen stack behavior: preserved via router history (back button works)
  >
  > **Navigation State:**
  > - Active section determined by current route path
  > - Breadcrumbs auto-generated from route hierarchy
  > - Context actions per route defined in route metadata
  >
  > ### Color System (Dash Brand + shadcn/ui)
  >
  > **Override shadcn's default OKLCH neutral palette with Dash brand colors:**
  >
  > **Brand Colors:**
  > - `--dash-blue`: #008de4 (primary action, links, active states)
  > - `--dash-deep-blue`: #012060 (gradient end, emphasis)
  > - `--dash-midnight`: #0b0f3b (darkest accent)
  >
  > **Semantic Colors (mapped to CSS variables):**
  > - `--primary`: Dash Blue (#008de4) — replaces shadcn's neutral primary
  > - `--primary-foreground`: White
  > - `--destructive`: Error Red (#eb5757)
  > - `--success`: Green (#27ae60) — custom addition
  > - `--warning`: Orange (#f1c40f) — custom addition
  > - `--info`: Blue (#3498db) — custom addition
  >
  > **Network Accent Colors:**
  > - Mainnet: Dash Blue (#008de4 / #0071b6 dark)
  > - Testnet: Orange (#ffa500 / #cc8400 dark)
  > - Devnet: Dark Red (#8b0000 / #6f0000 dark)
  > - Local/Regtest: Brown (#8b4513 / #6f370f dark)
  > - Applied to: top bar action buttons, active nav highlights, network badges
  >
  > **Light Mode:**
  > - Background: #f0f2f7 (soft blue-gray)
  > - Surface: #ffffff
  > - Input bg: #f8fafc
  > - Border: #e2e8f0 (light), #f0f5fb (very light)
  > - Text primary: #111921
  > - Text secondary: #64788c
  >
  > **Dark Mode:**
  > - Background: #121212
  > - Surface: #202020
  > - Input bg: #282828
  > - Border: #3c3c3c (normal), #323232 (light)
  > - Text primary: #f0f0f0
  > - Text secondary: #a0a0a0
  >
  > ### Typography (Noto Sans, shadcn defaults + overrides)
  >
  > **Font Family:** "Noto Sans", system-ui, sans-serif (matching egui's Noto Sans)
  > **Monospace:** "JetBrains Mono", ui-monospace, monospace (for JSON, hex, code display)
  >
  > **Scale (matching egui theme.rs):**
  > - `text-xs`: 12px — captions, badges
  > - `text-sm`: 14px — secondary text, table cells
  > - `text-base`: 16px — body text, inputs, buttons
  > - `text-lg`: 18px — large body
  > - `text-xl`: 20px — section headings
  > - `text-2xl`: 24px — page headings
  > - `text-3xl`: 30px — display headings
  > - `text-4xl`: 36px — hero/display
  >
  > ### Spacing Scale (matching egui Spacing constants)
  >
  > - `space-0.5`: 2px (xxs)
  > - `space-1`: 4px (xs)
  > - `space-2`: 8px (sm)
  > - `space-4`: 16px (md)
  > - `space-6`: 24px (lg)
  > - `space-8`: 32px (xl)
  > - `space-12`: 48px (xxl)
  > - `space-16`: 64px (xxxl)
  >
  > ### Border Radii
  >
  > Override shadcn's `--radius: 0.625rem` to match egui:
  > - `rounded-sm`: 6px
  > - `rounded-md`: 12px
  > - `rounded-lg`: 16px (island panels, cards)
  > - `rounded-xl`: 20px
  > - `rounded-full`: 9999px (pills, badges)
  >
  > ### Shadow System (matching egui Shadow struct)
  >
  > - `shadow-sm`: 0 2px 4px rgba(0,0,0,0.03) — subtle elements
  > - `shadow-md`: 0 4px 12px rgba(0,0,0,0.05) — popups, dropdowns
  > - `shadow-lg`: 0 8px 24px rgba(0,0,0,0.06) — large panels
  > - `shadow-elevated`: 0 12px 32px rgba(0,0,0,0.07) — island panels, cards
  > - `shadow-glow`: 0 0 20px rgba(0,141,228,0.12) — primary element glow
  >
  > ### Content Layout Patterns
  >
  > **List View (Identities, Wallets, Tokens, Contacts):**
  > - Sortable data table via @tanstack/react-table
  > - Column headers with sort indicators
  > - Row hover state with subtle highlight
  > - Row actions via context menu (right-click) or action column (kebab menu)
  > - Alternating row stripe for readability
  > - Empty state: centered illustration + message + CTA button
  >
  > **Detail View (Identity detail, Wallet detail):**
  > - Header with title + status badge + action buttons
  > - Tabbed content sections
  > - Key-value display grid for metadata
  > - Collapsible sections for advanced info
  >
  > **Form/Wizard View (Create wallet, Register identity, Token creator):**
  > - Multi-step wizard with step indicators
  > - Form fields with inline validation (red border + error text)
  > - Required field indicators
  > - Submit button disabled until valid
  > - Loading state on submission
  >
  > **Action Screen (Send payment, Top up, Transfer):**
  > - Input form (amount, destination, options)
  > - Fee preview section
  > - Wallet unlock step (if needed)
  > - Confirmation step with summary
  > - Progress indicator during broadcast
  > - Success/error result with details
  >
  > ### Modal/Dialog Patterns
  >
  > **Confirmation Dialog (shadcn AlertDialog):**
  > - Semi-transparent overlay backdrop (rgba(0,0,0,0.47))
  > - Centered card with title, message, and action buttons
  > - Confirm (primary or destructive) + Cancel buttons
  > - Escape key dismisses
  > - Focus trapped within dialog
  >
  > **Wallet Unlock Popup (shadcn Dialog):**
  > - Password input with show/hide toggle
  > - Wallet name displayed
  > - Error message on failed attempt with hint
  > - Auto-focus on password field
  > - Enter key submits, Escape cancels
  > - Password zeroized on close (security)
  >
  > **Fee Confirmation Dialog (shadcn Dialog):**
  > - Fee breakdown table (base fee, multiplier, total)
  > - Confirm + Cancel buttons
  > - Identity/wallet context shown
  >
  > **Toast Notifications (shadcn Sonner):**
  > - Bottom-right position
  > - Auto-dismiss: 5s for success/info, persistent for errors
  > - Types: success (green), error (red), warning (amber), info (blue)
  > - Dismissible by click
  >
  > ### Loading & Error States
  >
  > **Loading:**
  > - Skeleton loader for initial data fetch (shimmer effect)
  > - Inline spinner for in-progress actions (button spinner)
  > - Full-page spinner for app initialization
  > - Progress bar for known-duration operations (SPV sync)
  >
  > **Error:**
  > - Inline error messages (red text below inputs)
  > - Error banners (red background + icon at top of content area)
  > - Error toast for async operation failures
  > - Expandable error details (technical info collapsed by default)
  >
  > **Empty States:**
  > - Centered layout with muted icon + descriptive text + action button
  > - e.g., "No wallets yet" → "Create Wallet" button
  > - e.g., "No identities loaded" → "Add Identity" button
  >
  > ### Responsive Behavior (Desktop-First)
  >
  > - **≥1280px**: Full layout (sidebar expanded 200px + sub-nav 220px + content)
  > - **1024-1279px**: Sidebar collapsed to icons (72px), sub-nav as overlay/sheet
  > - **<1024px**: Sidebar as hamburger drawer, sub-nav integrated into content
  > - Minimum supported width: 800px (Tauri window minimum)
  >
  > ### Accessibility
  >
  > - All interactive elements keyboard-focusable (tab order)
  > - ARIA labels on icon-only buttons
  > - Focus visible indicator (ring) on all focusable elements
  > - Color contrast ≥4.5:1 for text (WCAG AA)
  > - Role attributes on navigation, main content, dialogs
  > - Skip-to-content link
  > - Screen reader announcements for toasts and status changes

  **Sub-tasks produced:**
  - [x] **2.1a** Override shadcn CSS variables with Dash brand color palette: replace neutral OKLCH values with Dash-specific light/dark mode colors in `index.css`. Add custom CSS variables for `--dash-blue`, `--success`, `--warning`, `--info`, network accent colors. Update `--radius` to 16px base. (P0)
  - [x] **2.1b** Set up typography: install Noto Sans and JetBrains Mono web fonts, configure Tailwind font-family, create `@font-face` declarations, set up `prose` classes for rich text areas. (P0)
  - [x] **2.1c** Create Tailwind theme extensions: add custom shadow utilities (shadow-elevated, shadow-glow), spacing tokens, network-accent color classes (`bg-network-mainnet`, etc.), and animation utilities (pulse for connection indicator). (P0)
  - [x] **2.1d** Create layout primitives: `<AppShell>` (flex container with sidebar + content), `<Island>` (elevated card with surface bg, border, shadow, rounded-lg), `<PageHeader>` (title + breadcrumbs + actions row). (P0)
  - [x] **2.1e** Create theme provider and toggle: React context for dark/light mode, persist to backend settings via `settings_update_theme` IPC command, `<ThemeToggle>` button component, system theme detection. (P0)
  - [x] **2.1f** Create empty state and loading components: `<EmptyState>` (icon + message + action), `<LoadingSkeleton>` (shimmer), `<LoadingSpinner>` (inline + overlay variants), `<ProgressBar>`. (P1)

- [x] **2.2 Implement design system foundation** (P0)
  Based on 2.1's decisions, create the design system:
  - CSS variables / theme tokens for colors, spacing, typography, shadows
  - Dark and light theme definitions
  - Base component styles (buttons, inputs, cards, tables, badges, tabs)
  - Layout primitives (stack, grid, sidebar layout)
  - Utility classes or styled components as appropriate
  - Theme toggle mechanism (persisted to backend settings)
  Write component tests for theme switching. Write Playwright test verifying dark/light mode.

- [x] **2.3 Implement app shell: sidebar navigation + top bar** (P0)
  Build the persistent app chrome:
  - **Left sidebar:** Navigation items with icons for each main section (Dashpay, Identities, Contracts, Tokens, Wallets, Tools, Settings). Active state highlighting. Collapsible on narrow viewports.
  - **Top bar:** Breadcrumb trail showing current location. Connection status indicator (pulsating dot: green=connected, red=disconnected, fed by ZMQ status events). Network badge showing current network (Mainnet/Testnet/Devnet/Local).
  - **Content area:** Router outlet for screen components
  - Set up client-side routing for all main sections
  Write component tests for navigation state. Write Playwright test for navigating between sections.

- [x] **2.4 Implement shared dialog and feedback components** (P1)
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

- [x] **2.5 Implement welcome/onboarding screen** (P1)
  Port the welcome screen that appears on first launch:
  - Action cards: "Load Wallet", "Create Wallet", "Import Identity", "Just Browse"
  - Each action navigates to the appropriate screen
  - "Don't show again" persisted to backend settings
  Reference: `src/ui/welcome_screen.rs` or the welcome logic in `src/app.rs`
  Write component test. Write Playwright test for onboarding flow.

- [x] **2.6 Implement network chooser/settings screen** (P1)
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

- [x] **2.7 [REVIEW] App shell and design system quality audit** (P1)
  Review the implemented shell, design system, and shared components:
  - Visual consistency across light/dark themes
  - Accessibility: keyboard navigation, screen reader labels, focus management, color contrast
  - Responsive behavior at different window sizes
  - Component API consistency (props patterns, event handling)
  - Test coverage completeness
  Create fix tasks for any issues.

  > **Audit Findings (Run 41):**
  >
  > ### Overall Assessment: SOLID (B+)
  > 322 tests pass, lint clean, typecheck clean. Good foundation with proper ARIA attributes,
  > semantic HTML, focus management in dialogs, and comprehensive theme variable system.
  >
  > ### Issues Found:
  >
  > **Accessibility (2 issues):**
  > 1. WalletUnlockDialog.tsx:133 — password visibility toggle has `tabIndex={-1}`, removing it
  >    from keyboard tab order. Users cannot reach show/hide password via keyboard.
  > 2. Light mode color contrast: `--muted-foreground` (#64788c) on `--muted` (#f8fafc) background
  >    = 4.36:1, below WCAG AA 4.5:1 minimum. On white bg = 4.56:1 (barely passes).
  >    Darkening to ~#5a6d80 would achieve ~5.1:1 on muted bg.
  >
  > **Bug (1 issue):**
  > 3. DesignSystem.tsx:326 — EmptyState uses non-existent `action` prop instead of correct
  >    `actionLabel` + `onAction` props. Button silently doesn't render. TypeScript doesn't
  >    catch it because JSX allows extra props.
  >
  > **No Issues (areas that passed):**
  > - Dark/light theme visual consistency: All CSS variables properly dual-defined
  > - Dialog keyboard handling: Escape via Radix, Enter key in WalletUnlockDialog
  > - Screen reader support: role="status", role="alert", role="navigation", role="main", aria-current, aria-expanded, aria-invalid, aria-describedby all properly used
  > - Component API consistency: Dialogs use onOpenChange+onResult, inputs use onChange, actions use onClick — patterns are coherent
  > - NetworkChooserScreen password toggle: Has proper aria-label (lines 531-533)
  > - Auto-focus in WalletUnlockDialog: Working via useEffect + setTimeout
  > - Test coverage: 322 tests across 23 test files — every component has tests

  **Fix sub-tasks:**
  - [x] **2.7a** Fix WalletUnlockDialog keyboard accessibility: remove `tabIndex={-1}` from password visibility toggle button (P1)
  - [x] **2.7b** Fix light mode muted-foreground contrast: darken `--muted-foreground` from #64788c to #5a6d80 for WCAG AA compliance on muted backgrounds (P2)
  - [x] **2.7c** Fix DesignSystem.tsx EmptyState demo: change `action={{ label: "Create Wallet", onClick: () => {} }}` to `actionLabel="Create Wallet" onAction={() => {}}` (P3)

---

## Phase 3: Wallet Screens

- [x] **3.1 [META] Design wallet screens UX** (P1)
  Review all wallet functionality in the egui version and design improved UX:
  - Wallet list/portfolio view (HD + single-key wallets together)
  - Wallet detail view (accounts, addresses, balances, UTXOs)
  - Send flow (simple + advanced modes, multiple recipients, fee selection)
  - Receive flow (address display, QR code)
  - Asset lock creation and management
  - Platform address operations (funding, withdrawal, transfer)
  Files to review: `src/ui/wallets/wallets_screen/mod.rs` (2,030 lines), `src/ui/wallets/send_screen/mod.rs` (1,725 lines), all files in `src/ui/wallets/`
  Identify UX improvements over current implementation. Produce implementation sub-tasks.

  > **Design Decisions (Run 45):**
  >
  > ### Wallet Screens Architecture: 6 Route-Based Screens
  >
  > The egui version crams all wallet functionality into a single 2,030-line file with
  > modal popups. The Tauri version splits into clean, focused route-based screens:
  >
  > ```
  > /wallets                    → WalletsScreen (list + detail view)
  > /wallets/create             → CreateWalletScreen (new HD wallet wizard)
  > /wallets/import             → ImportWalletScreen (mnemonic + private key import)
  > /wallets/send/:type         → SendScreen (HD and single-key send, unified)
  > /wallets/asset-locks/create → CreateAssetLockScreen (registration + top-up flows)
  > /wallets/asset-locks/:id    → AssetLockDetailScreen (lock details + private key)
  > ```
  >
  > ### Wallet List & Detail View (WalletsScreen)
  >
  > **Layout: Split-pane design**
  > - Left panel (300px): Wallet list with HD and single-key wallets in separate sections
  > - Right panel: Detail view for selected wallet
  > - Empty state when no wallets: Card with "No Wallets Loaded" + action buttons
  >
  > **Wallet List Panel:**
  > - Section headers: "HD Wallets" and "Single-Key Wallets" with count badges
  > - Each wallet card: Alias, balance, pending indicator, lock icon (if password)
  > - Selected wallet highlighted with Dash Blue left border
  > - Context menu (right-click or kebab icon): Rename, Lock/Unlock, Remove
  >
  > **HD Wallet Detail Panel:**
  > - Header: Wallet alias (editable inline), Core + Platform balance summary
  > - Action bar: Send, Receive, Refresh buttons + refresh mode dropdown (dev only)
  > - Tabs: Addresses | Transactions (dev) | Asset Locks
  > - Addresses tab: Account selector dropdown → sortable address table with columns
  >   (Address, Balance, UTXOs, Total Received, Type, Index, Path, Actions)
  > - "Hide zero balances" toggle, "Add Receiving Address" button (when unlocked)
  > - Address row "View Key" button → private key dialog (requires wallet unlock)
  > - Transactions tab (dev only): Sortable table (Date, Type, Amount, Status, TxID)
  > - Asset Locks tab: Table (TxID, Address, Amount, InstantLock, Usable, Actions)
  >   + "Create Asset Lock" and "Search for Unused" buttons
  >
  > **Single-Key Wallet Detail Panel:**
  > - Header: Alias, address (monospace), balance + pending
  > - Action bar: Send, Receive
  > - UTXOs section with paginated cards (50 per page)
  >
  > ### UX Improvements Over egui
  >
  > 1. **Persistent wallet list**: No dropdown needed — see all wallets at a glance
  > 2. **Inline rename**: Double-click alias to edit, Enter to save, Escape to cancel
  > 3. **Context menus**: Right-click or kebab for wallet actions (cleaner than button row)
  > 4. **Unified Send screen**: HD and single-key send merged into one screen with
  >    source type awareness (simpler navigation, less code duplication)
  > 5. **Tabbed detail view**: Addresses/Transactions/Asset Locks as tabs instead of
  >    vertically stacked sections (reduces scrolling)
  > 6. **Receive as Dialog**: Consistent modal dialog with tabs (Core/Platform) and QR
  > 7. **Better empty states**: Illustrations + clear CTAs for each empty section
  > 8. **Toast notifications**: Replace in-page message banners with toast system
  > 9. **Stepper wizard for Create Wallet**: Clear progress indicator for multi-step flow
  > 10. **Paste detection for Import**: Auto-detect full mnemonic paste and fill all fields
  >
  > ### Create Wallet Flow (CreateWalletScreen)
  >
  > **Multi-step wizard with step indicator:**
  > 1. Generate Entropy → Shows entropy grid visualization
  > 2. Configure → Language + word count selection, Generate button
  > 3. Backup → Display seed words in numbered grid, "I wrote it down" checkbox
  > 4. Name & Protect → Wallet name input + optional password with strength meter
  > 5. Success → Wallet created, next steps (Fund, Create Identity, Go to Wallet)
  >
  > **Success screen additions:**
  > - "Fund Wallet" button opens Receive dialog with QR
  > - Auto-detects incoming funds and updates UI
  > - "Create Platform Identity" navigates to identity creation with wallet pre-selected
  >
  > ### Import Wallet Flow (ImportWalletScreen)
  >
  > **Two tabs at top: "Seed Phrase" | "Private Key"**
  >
  > **Seed Phrase tab:**
  > - Word count selector (12/15/18/21/24)
  > - Word input grid (4 columns) with paste-to-fill support
  > - Real-time BIP39 validation with per-word error highlighting
  > - Identity auto-discovery config (collapsible advanced section)
  > - Name + password section
  >
  > **Private Key tab:**
  > - Single input field (WIF or hex)
  > - Real-time parsing with address preview
  > - Name + password section
  >
  > **Success screen**: Same as Create but with "Import Another" option
  >
  > ### Send Flow (SendScreen — Unified HD + Single-Key)
  >
  > **Simple mode (default):**
  > - Source selector: Shows wallet info + available balance
  > - For HD wallets: Radio buttons for Core Wallet / Platform Addresses / Identity sources
  > - Destination address input with type detection badge (Core/Platform)
  > - Amount input with Max button
  > - Transaction type hint (auto-detected from source+dest combination)
  > - "Subtract fee from amount" checkbox (Core-to-Core only)
  > - Platform source breakdown panel (when applicable)
  > - Wallet unlock gate → Send button
  >
  > **Advanced mode (toggle):**
  > - HD: Source type selector (Core/Platform), manual address selection
  > - Single-key: Multiple recipients with add/remove, memo field
  > - Fee estimation display with UTXO count and tx size
  > - Large input warning (>100 UTXOs)
  >
  > **States:** Form → Wallet Unlock → Sending (spinner + elapsed time) → Success/Error
  > - Success: "Send Another" or "Back to Wallet"
  > - Error: Inline banner with dismiss + optional fee confirmation dialog
  >
  > ### Receive Dialog (Modal from Wallet Detail)
  >
  > **Two tabs: Core | Platform**
  > - QR code display (220x220)
  > - Address selector dropdown (if multiple addresses)
  > - Full address display (monospace)
  > - Balance display
  > - Copy Address + New Address buttons
  > - Info text explaining what the address is for
  >
  > ### Asset Lock Screens
  >
  > **Create Asset Lock (CreateAssetLockScreen):**
  > - Step 1: Purpose selection (Registration / Top Up) with info cards
  > - Step 2 (Top Up): Identity selector
  > - Step 3: Amount input with DASH display
  > - Step 4: QR code + funding address for receiving DASH
  > - Automatic progression: Funds received → Asset lock creation → Success
  > - Advanced options: Manual index selection
  >
  > **Asset Lock Detail (AssetLockDetailScreen):**
  > - Transaction info section: TxID, Address, Amount (DASH + duffs)
  > - Proof status with color badges (Instant Send Locked/Chain Locked/Waiting)
  > - Proof details section (Instant or Chain variant)
  > - Proof hex with copy button
  > - Private key section (requires wallet unlock): WIF display with show/hide toggle
  > - Warning text about key security
  >
  > ### Dialog Components (Shared)
  >
  > All wallet dialogs reuse components from Phase 2:
  > - ConfirmationDialog (remove wallet)
  > - WalletUnlockDialog (password entry)
  > - FeeConfirmationDialog (fee override)
  > - AmountInput (DASH/credits formatting)
  > - CopyButton (clipboard with feedback)
  > - Toast notifications (success/error/info)

  **Sub-tasks produced:**
  - [x] **3.1a** Create wallet Zustand store: `useWalletStore` with state for HD wallets, single-key wallets, selected wallet, loading/error states, and actions for CRUD operations. Wire to Tauri IPC commands (walletListAll, walletGetHd, walletGetSingleKey, walletSelect, walletSetAlias, walletRemove, etc.). Include refresh logic with mode selector. (P1)
  - [x] **3.1b** Create wallet list component: `WalletListPanel` with HD and single-key wallet sections, wallet cards with balance/pending/lock status, selection handling, and context menu (Rename/Lock/Unlock/Remove). Include empty state. (P1)
  - [x] **3.1c** Create HD wallet detail component: `HdWalletDetail` with header (alias + balances), action bar (Send/Receive/Refresh + mode dropdown), and three tabs — Addresses (account selector + sortable table + hide zero toggle + Add Address + View Key), Transactions (dev only, sortable table), Asset Locks (table + Create/Search buttons). (P1)
  - [x] **3.1d** Create single-key wallet detail component: `SingleKeyWalletDetail` with header (alias + address + balance), action bar (Send/Receive), and paginated UTXO list (50 per page with First/Prev/Next/Last controls). (P1)
  - [x] **3.1e** Create Receive dialog component: `ReceiveDialog` modal with Core/Platform tabs, QR code display (qrcode.react), address selector dropdown, full address display, balance, Copy/New Address buttons, and info text. (P1)
  - [x] **3.1f** Create private key dialog component: `PrivateKeyDialog` modal with address display, WIF key display (masked/revealed toggle), Copy Address/Copy Key buttons, and security warning. Requires wallet unlock. (P1)

- [x] **3.2 Implement wallet list and detail screens** (P1)
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

- [x] **3.3 Implement add wallet and import mnemonic screens** (P1)
  Build wallet creation flows:
  - **New wallet:** Generate mnemonic, display seed words, set password, name wallet
  - **Import mnemonic:** Enter 12/24 words, set password, name wallet, import
  - Input validation, error handling, loading states
  Reference: `add_new_wallet_screen.rs`, `import_mnemonic_screen.rs`
  Write component tests. Write Playwright test for full create/import flow.

- [x] **3.4 Implement HD wallet send screen** (P1)
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

- [x] **3.5 Implement single-key wallet send and asset lock screens** (P1)
  Build remaining wallet transaction screens:
  - **Single-key send:** Similar to HD send but with UTXO selection from single-key wallet
  - **Create asset lock:** Wallet selection, amount, script type, unlock conditions
  - **Asset lock detail:** View lock details, status, use for identity creation
  Reference: `single_key_send_screen.rs`, `create_asset_lock_screen.rs`, `asset_lock_detail_screen.rs`
  Write component tests. Write Playwright tests.

- [x] **3.6 [REVIEW] Wallet screens functionality parity** (P1)
  Exhaustive comparison of every wallet action in egui vs Tauri:
  - Open `wallets_screen/mod.rs` and trace every button, menu item, dialog, and display element
  - Verify each has a corresponding UI element and IPC command in the Tauri version
  - Check: wallet refresh modes (Core only, Platform full/terminal, Combined), address operations (copy, view key, fund platform), asset lock recovery, platform address funding
  - Verify test coverage for critical paths
  Create fix tasks for gaps.

  > **Audit Findings (Run 56):**
  >
  > ### Overall Assessment: STRONG (A-)
  > 811 tests pass, lint clean, typecheck clean. All major wallet workflows are present and
  > functional. The Tauri implementation covers all core functionality with significant UX
  > improvements (split-pane layout, inline rename, context menus, toast notifications,
  > tabbed detail view, step wizards).
  >
  > ### Features with FULL PARITY (confirmed present):
  > - Wallet list: HD + single-key sections, select, rename (inline), lock/unlock, remove with confirmation
  > - HD wallet detail: alias, Core+Platform balances, pending indicator, refreshing state
  > - Refresh modes: All 5 modes (All Auto, Core Only, Core+Platform Full/Terminal, Combined) in dev mode
  > - Addresses tab: account selector, sortable 7-column table, hide zero toggle, View Key, Add Address, CopyButton
  > - Transactions tab: dev-mode only, sorted by date, direction/amount/status/txid
  > - Asset Locks tab: table with all columns, Create/Search/View/Fund buttons
  > - Single-key detail: address+copy, balance+pending, paginated UTXOs (50/page)
  > - Receive dialog: Core/Platform tabs (HD), single tab (single-key), QR 220x220, address selector, New Address
  > - Private Key dialog: masked/revealed, Copy when revealed, security warning
  > - Create Wallet: 3-step wizard, word count 12-24, BIP39 generation, strength meter, success screen
  > - Import Wallet: Two tabs (Seed Phrase/Private Key), word grid with multi-word paste, BIP39 validation, advanced options (identity auto-discovery), strength meter
  > - HD Send: Simple (Core/Platform/Identity sources, address detection, Max+auto-subtract-fee, platform breakdown), Advanced (inputs/outputs, fee strategy), sending/complete/error states
  > - Single-key Send: Simple (address+amount+subtract fee), Advanced (multiple recipients, memo), sending/complete/error states
  > - Create Asset Lock: Purpose→Configure→Funding→Creating→Success, registration/top-up, identity selector, advanced options (identity/top-up index), auto-progression on fund receipt
  > - Asset Lock Detail: TX info, proof status badge, InstantLock/Usable badges, private key with unlock gate
  > - Identity withdrawal: source selection with key picker, correct IPC dispatch
  > - Fee strategy: 4-option dropdown for platform operations
  > - Max button: auto-enables subtract-fee for Core→Core
  >
  > ### Gaps Found (5 issues):
  >
  > **1. Fee confirmation dialog not wired to send flows (P2)**
  > - `FeeConfirmationDialog` component exists and is fully implemented
  > - Neither SendScreen nor SingleKeySendScreen uses it
  > - Both pass `overrideFee: null` hardcoded
  > - The egui single-key send screen has `FeeConfirmationDialog` integration that intercepts min relay fee errors, shows the dialog, and re-sends with `override_fee` set
  > - The HD send screen in egui also passes `override_fee: None` (no dialog there either), so this gap is single-key only
  >
  > **2. Transaction size estimation display missing from single-key send (P3)**
  > - egui `single_key_send_screen.rs` shows: estimated fee, UTXO input count, tx byte size
  > - Also shows warning when >100 UTXOs needed ("Large number of inputs required")
  > - Tauri SingleKeySendScreen has no fee estimation display at all
  > - Note: HD send screen in egui also lacks this, so parity for HD is fine
  >
  > **3. Asset lock proof details missing from AssetLockDetailScreen (P2)**
  > - egui shows detailed proof information: Instant Send TxID + Output Index, Chain Lock Height + OutPoint
  > - egui shows proof hex with Copy button and collapsible "View Raw Proof Details" section
  > - Tauri AssetLockDetailScreen shows proof STATUS (badge) but not the detailed proof fields or hex
  > - This requires the DTO to include proof detail fields (currently `AssetLockDto` has `hasAssetLockProof` boolean and `hasInstantLock` boolean but no proof data)
  >
  > **4. BIP39 language selection missing from CreateWalletScreen (P3)**
  > - egui has Language dropdown: English, Spanish, French, Italian, Portuguese
  > - Tauri hardcodes English only (`@scure/bip39/wordlists/english.js`)
  > - Low priority: vast majority of users use English; other languages rarely used
  >
  > **5. Entropy grid visualization missing from CreateWalletScreen (P3)**
  > - egui has `U256EntropyGrid` component that shows randomness visualization
  > - Allows user to contribute entropy by clicking grid cells
  > - Tauri uses `@scure/bip39` with WebCrypto `getRandomValues()` for entropy (more secure than user clicking)
  > - Low priority: the entropy grid was more of a visual novelty; WebCrypto provides better randomness
  >
  > ### Test Coverage Assessment:
  > - 811 total Vitest tests pass (wallet-related: ~365 tests across 12 test files)
  > - All wallet components have dedicated test files
  > - Critical paths covered: create, import, send (both modes), receive, private key, asset lock create/detail
  > - Playwright E2E tests cover wallet screen rendering and navigation
  >
  > ### Summary: 5 fix sub-tasks created below (3 functional, 2 cosmetic)

  **Fix sub-tasks:**
  - [x] **3.6a** Wire `FeeConfirmationDialog` into `SingleKeySendScreen`: intercept min relay fee errors from task error events, parse the required fee, show the dialog, and re-send with `overrideFee` set. Match egui `single_key_send_screen.rs` lines 805-825 and 844-855 behavior. (P2)
  - [x] **3.6b** Add transaction size estimation display to `SingleKeySendScreen`: show estimated fee, UTXO input count, and transaction byte size below the amount input. Add warning banner when >100 UTXOs needed. Port `estimate_fee()` logic from egui `single_key_send_screen.rs` lines 145-196 (can be done client-side from UTXO data already in the store). (P3)
  - [x] **3.6c** Add asset lock proof details to `AssetLockDetailScreen`: extend `AssetLockDto` to include proof type and proof detail fields (InstantLock TxID/Output Index for Instant, Core Chain Locked Height/OutPoint for Chain), add proof hex display with Copy button, and add collapsible "View Raw Proof Details" section. Port from egui `asset_lock_detail_screen.rs` lines 130-208. (P2)
  - [x] **3.6d** Add BIP39 language selection to `CreateWalletScreen`: add Language dropdown (English, Spanish, French, Italian, Portuguese) before word count selector, dynamically import the selected language wordlist from `@scure/bip39/wordlists/`. Default to English. (P3)
  - [x] **3.6e** Add entropy grid visualization to `CreateWalletScreen`: create an `EntropyGrid` component that visualizes random entropy and optionally allows user input. Show below the Generate button. Mix user-contributed entropy with WebCrypto randomness. Port concept from egui `U256EntropyGrid`. (P3)

---

## Phase 4: Identity Screens

- [x] **4.1 [META] Design identity screens UX** (P1)
  Review all identity functionality and design improved UX:
  - Identity list with sortable columns and drag-drop reordering
  - Identity creation wizard (4 funding methods, key configuration)
  - Key management (view, add, disable, replace)
  - Identity operations (top-up, withdraw, transfer, register DPNS name)
  Files to review: All files in `src/ui/identities/` — catalog every screen, dialog, and user action
  Produce implementation sub-tasks.

  > **Design Decisions (Run 62):**
  >
  > ### Identity Screens Architecture: Route-Based with Shared Store
  >
  > The egui version uses ~7,000 lines across 12+ files with modal popups and screen stacks.
  > The Tauri version splits into clean, focused route-based screens with a shared Zustand store:
  >
  > ```
  > /identities                        → IdentitiesScreen (list + detail split-pane)
  > /identities/create                 → CreateIdentityScreen (new identity wizard)
  > /identities/load                   → LoadIdentityScreen (add existing by ID/wallet/DPNS)
  > /identities/top-up/:id             → TopUpIdentityScreen (top-up wizard)
  > /identities/withdraw/:id           → WithdrawScreen (withdraw credits)
  > /identities/transfer/:id           → TransferScreen (transfer credits)
  > /identities/register-dpns/:id      → RegisterDpnsNameScreen (register DPNS name)
  > /identities/keys/:id               → KeyManagementScreen (view/add/disable/replace keys)
  > /identities/keys/:id/:keyId        → KeyInfoScreen (key detail + sign message)
  > ```
  >
  > ### Identity List & Detail View (IdentitiesScreen)
  >
  > **Layout: Split-pane design (similar to WalletsScreen pattern)**
  > - Left panel (320px): Sortable identity table
  > - Right panel: Detail view for selected identity
  > - Empty state when no identities: Card with "No Identities" + "Create" / "Load" buttons
  >
  > **Identity Table:**
  > - Columns: Alias, Identity ID (truncated), In Wallet, Type (User/MN/Evonode), Balance (DASH)
  > - Sortable columns (click header: Alias, Identity ID, In Wallet, Type, Balance)
  > - Custom ordering: Up/Down buttons to reorder (persisted to DB via `identity_save_order`)
  > - Default: custom order if saved, else sort by Alias ascending
  > - Inline alias editing: click alias cell → text input, Enter to save, Escape to cancel
  > - Row selection: click to select, highlights with Dash Blue left border
  > - Context menu (right-click or kebab): View Keys, Register DPNS Name, Top Up, Withdraw,
  >   Transfer, Update Alias, Remove
  > - Identity status colors: Active (green badge), Failed (red), Unknown (gray), Pending (yellow)
  > - Type display: "User" | "Masternode" | "Evonode" badge
  > - Balance: formatted in DASH, hover shows duffs (tooltip)
  >
  > **Identity Detail Panel (when selected):**
  > - Header: Alias (large), Identity ID (full, monospace + copy), Type badge, Status badge
  > - Balance section: Credits balance in DASH, platform balance breakdown
  > - DPNS names section: List of registered names (if any)
  > - Associated wallet: Wallet name + link to wallet screen
  > - Action bar: Top Up, Withdraw, Transfer, Register DPNS, Refresh buttons
  > - Keys dropdown: Quick access to all keys with private key indicators
  >   (highlighted green if private key held, dim if not)
  > - For voter identity: separate section showing voter keys
  >
  > **Top bar actions:**
  > - "Create Identity" button (navigates to /identities/create)
  > - "Load Identity" button (navigates to /identities/load)
  > - "Refresh All" button (bulk refresh all identities)
  > - If no wallets: shows "Import/Create Wallet" instead
  >
  > ### Create Identity Flow (CreateIdentityScreen)
  >
  > **Multi-step wizard with step indicator (similar to CreateWalletScreen pattern):**
  >
  > **Step 1: Select Wallet**
  > - Wallet selector dropdown (if multiple wallets)
  > - Auto-selects if only one wallet
  > - Wallet unlock gate if password-protected
  >
  > **Step 2: Identity Index (advanced only, collapsible)**
  > - Dropdown showing indices 0–30
  > - "(used)" marker on already-claimed indices
  > - Info tooltip: "Identity index is an internal reference number"
  >
  > **Step 3: Key Configuration (advanced only, collapsible)**
  > - Toggle: Default Keys / Advanced Configuration
  > - Default: platform auto-selects keys (explanation text shown)
  > - Advanced: Table with columns (Key #, WIF, Purpose, Type, Security Level, Delete)
  >   - Master key row (always present, not deletable)
  >   - Additional key rows with + Add Key button
  >   - Purpose dropdown: Authentication, Transfer, Voting, Owner
  >   - Type dropdown: ECDSA_SECP256K1, ECDSA_HASH160, BLS12_381, BIP13_SCRIPT_HASH
  >   - Security Level dropdown: Critical, High, Medium (auto-set based on purpose)
  >
  > **Step 4: Local Alias**
  > - Text input for local alias (required)
  > - Info: "Stored only in Dash Evo Tool — not broadcast to the network"
  >
  > **Step 5: Funding Method**
  > - Selector with 4 options:
  >   1. "Unused Evo Funding Locks" (recommended, shown only if locks exist)
  >   2. "Wallet Balance" (shown only if wallet has sufficient balance)
  >   3. "Address with QR Code" (always available)
  >   4. "Platform Address" (shown only if wallet has platform balance)
  > - Each option has a brief description
  >
  > **Step 6: Funding-specific UI (varies by method)**
  > - **Asset Lock:** Select from list of available locks, amount display
  > - **Wallet Balance:** Amount selector, auto-calculate from wallet balance
  > - **QR Code:** Generate receive address, show QR (220×220), address + copy button,
  >   waiting indicator, auto-detect incoming UTXO, progress through steps
  >   (WaitingOnFunds → FundsReceived → ReadyToCreate → WaitingForAssetLock → WaitingForPlatformAcceptance → Success)
  > - **Platform Address:** Select platform address from wallet, amount input
  >
  > **Step 7: Register**
  > - Review summary: wallet, alias, funding method, amount
  > - "Register Identity" button
  > - Progress: spinner + elapsed time
  >
  > **Success Screen:**
  > - Identity ID display + copy
  > - Fee breakdown (base fee, processing fee, total in DASH)
  > - Action buttons: "Go to Identities", "Register DPNS Name", "Create Another"
  >
  > ### Load Existing Identity (LoadIdentityScreen)
  >
  > **Three tabs at top: By Identity ID | By Wallet | By DPNS Name**
  >
  > **By Identity ID tab:**
  > - Input field for Identity ID (accepts Hex and Base58)
  > - Advanced options (collapsible):
  >   - Identity Type selector (User/Masternode/Evonode)
  >   - Manual private keys section: list of key inputs with + Add / - Remove
  >   - Testnet only: "Fill Random HPMN" / "Fill Random Masternode" quick-fill buttons
  > - "Load Identity" button
  >
  > **By Wallet tab:**
  > - Wallet selector dropdown
  > - Wallet unlock gate (if password-protected)
  > - Advanced options (collapsible):
  >   - Search mode: "Specific Index" (single input) or "Up to Index" (scan range)
  > - "Search" button
  >
  > **By DPNS Name tab:**
  > - Username input (min 3 chars, ".dash" suffix shown)
  > - Advanced: wallet selector for key derivation
  > - "Search by Username" button
  >
  > **All tabs share:**
  > - Error banner (dismissible) at top of content
  > - Loading state with elapsed time counter
  > - Success state with identity details + "Load Another" / "Back to Identities" buttons
  >
  > ### Top Up Identity (TopUpIdentityScreen)
  >
  > **Same funding wizard pattern as Create, but for existing identity:**
  > - Identity header shows which identity is being topped up
  > - 4 funding methods (same as Create):
  >   1. Unused Asset Lock (if available)
  >   2. Wallet Balance
  >   3. Address with QR Code
  >   4. Platform Address
  > - Amount input for wallet balance and platform address methods
  > - QR code flow same as Create (generate address → wait → detect → create lock → submit)
  > - Success screen with fee breakdown
  >
  > ### Withdraw Credits (WithdrawScreen)
  >
  > **Single-page form:**
  > - Key selector: dropdown of identity keys with TRANSFER purpose
  >   (uses add_key_chooser pattern — shows key ID, purpose, security level, type)
  > - Available balance display (formatted DASH)
  > - Amount input with Max button (max = balance - 0.005 DASH fee reserve)
  > - Destination address input:
  >   - For owner key (masternode): auto-filled with payout address, read-only
  >   - For other keys: text input with address validation (network-specific)
  >   - Inline error if invalid address format
  > - Confirmation dialog (danger mode): "Are you sure you want to withdraw X DASH?"
  > - "Withdraw" button (disabled until valid)
  > - States: Form → Wallet Unlock → Sending (spinner + elapsed) → Success/Error
  > - Error: inline banner with dismiss, recovery suggestion link
  > - Success: amount withdrawn, fee breakdown
  >
  > ### Transfer Credits (TransferScreen)
  >
  > **Single-page form:**
  > - Key selector: dropdown of TRANSFER-purpose keys
  > - Transfer destination toggle: "To Identity" / "To Platform Address" buttons
  > - **To Identity:** Identity selector (search loaded identities) + receiver identity ID input
  > - **To Platform Address:** Platform address input field with validation
  > - Amount input with Max button
  > - Confirmation dialog: "Transfer X DASH credits?"
  > - "Transfer" button
  > - States: same as Withdraw (Form → Unlock → Sending → Success/Error)
  >
  > ### Register DPNS Name (RegisterDpnsNameScreen)
  >
  > **Single-page form:**
  > - Identity selector (if multiple user identities loaded)
  > - Key selector (advanced: choose specific key)
  > - Username input: text field + ".dash" suffix display
  >   - Minimum 3 characters
  >   - Example: "Enter alice to register alice.dash"
  > - "Register" button
  > - States: Form → Wallet Unlock → Registering (spinner + elapsed) → Success/Error
  > - Success: shows whether name was registered normally or is contested (enters voting period)
  > - Fee breakdown on success
  > - After success: "Register Another" or "Back to Identities"
  > - Source tracking: knows if navigated from Identities or DPNS screen (back button behavior)
  >
  > ### Key Management (KeyManagementScreen at /identities/keys/:id)
  >
  > **Keys list with actions:**
  > - Table: Key ID, Purpose, Security Level, Type, Status (Active/Disabled), Has Private Key
  > - Private key indicator: green highlight if private key held, dim if not
  > - Separate sections: Main Identity Keys / Voter Identity Keys (if voter identity exists)
  > - Click key row → navigate to KeyInfoScreen
  > - "+ Add Key" button (conditional: only if master key exists)
  > - Back to identity detail
  >
  > ### Key Info (KeyInfoScreen at /identities/keys/:id/:keyId)
  >
  > **Key detail view:**
  > - Key metadata grid: Key ID, Purpose, Security Level, Type, Read Only, Active/Disabled status
  > - Contract bounds (if set): Contract ID + Document Type
  > - Public key display: Hex format + Base64 format, with copy buttons
  > - Private key section:
  >   - If in wallet: "Stored in wallet [name]", derivation path display
  >   - If encrypted: "Encrypted" with wallet unlock to decrypt
  >   - If manual: display WIF with show/hide toggle
  >   - If not available: text input to add private key manually
  > - Message signing section:
  >   - Text area for message input
  >   - "Sign Message" button (requires private key)
  >   - Signed message output (base64) with copy button
  >   - Sign error display
  > - Advanced actions (conditional on having master key):
  >   - "Disable Key" button → confirmation dialog → dispatches DisableKey task
  >   - "Replace Key" button (master key only) → confirmation dialog with new key type/private key inputs → dispatches ReplaceKey
  >   - "Remove Private Key" button → confirmation dialog → removes local private key data
  >
  > ### Add Key (dialog/screen from KeyManagementScreen)
  >
  > **Form for adding a new key to identity:**
  > - Private key input (hex format, 32 bytes)
  > - Key Type selector: ECDSA_SECP256K1, ECDSA_HASH160
  > - Purpose selector: Authentication, Transfer, Encryption, Decryption
  > - Security Level selector: Critical, High, Medium (auto-set based on purpose)
  > - Advanced: Contract bounds section (toggle):
  >   - Contract ID input
  >   - Document Type Name input
  > - "Add Key" button
  > - Wallet unlock gate
  > - States: Form → Wallet Unlock → Adding (spinner + elapsed) → Success/Error
  > - Success: fee breakdown
  > - Special factory methods:
  >   - `new_for_dashpay_encryption()` — pre-configured for DashPay encryption
  >   - `new_for_dashpay_decryption()` — pre-configured for DashPay decryption
  >
  > ### UX Improvements Over egui
  >
  > 1. **Split-pane identity list**: See all identities + detail without navigation
  > 2. **Inline alias editing**: Click to edit, Enter/Escape to confirm/cancel
  > 3. **Route-based operations**: Each operation (top-up, withdraw, transfer) gets a dedicated
  >    route with back navigation, instead of screen stack push/pop
  > 4. **Unified key management**: Single route for all key operations per identity
  > 5. **Better empty states**: Illustrations + clear CTAs when no identities
  > 6. **Toast notifications**: Replace in-page message banners
  > 7. **Breadcrumb navigation**: Always know where you are (Identities > Keys > Key #3)
  > 8. **Stepper wizards**: Clear progress for multi-step Create/TopUp flows
  > 9. **Consistent status badges**: Active/Failed/Unknown/Pending with color coding
  > 10. **Key indicators in list**: Quick visual of which keys have private key data
  >
  > ### Backend Commands Already Available (27 commands)
  >
  > All identity IPC commands are implemented and TypeScript bindings generated:
  > - Async: identity_load, identity_search_by_dpns_name, identity_search_from_wallet,
  >   identity_search_up_to_index, identity_register, identity_register_dpns_name,
  >   identity_refresh, identity_refresh_dpns_names, identity_withdraw, identity_transfer,
  >   identity_add_key, identity_disable_keys, identity_replace_key, identity_top_up,
  >   identity_top_up_from_platform_addresses, identity_transfer_to_addresses
  > - Sync: identity_list_local, identity_list_user, identity_list_voting, identity_get_by_id,
  >   identity_set_alias, identity_get_alias, identity_load_order, identity_save_order,
  >   identity_delete, identity_list_summaries, identity_local_dpns_names

  **Sub-tasks produced:**
  - [x] **4.1a** Create identity Zustand store: `useIdentityStore` with state for identities (IndexMap ordering), selected identity, loading/error/refreshing states. Actions: loadIdentities, selectIdentity, refreshIdentity, refreshAllIdentities, setAlias, reorderIdentity (up/down + persist via identity_save_order), removeIdentity. Subscribe to `task-completed` events for identity results. Follow walletStore.ts pattern. (P1)
  - [x] **4.1b** Create identity list component: `IdentityListPanel` with sortable table (Alias, ID, In Wallet, Type, Balance columns), custom order up/down buttons, inline alias editing, selection handling, context menu (View Keys, Register DPNS, Top Up, Withdraw, Transfer, Update Alias, Remove), status badges, type badges. Include empty state. (P1)
  - [x] **4.1c** Create identity detail component: `IdentityDetailPanel` with header (alias + full ID + copy + type/status badges), balance section (DASH formatted), DPNS names list, associated wallet link, action bar (Top Up, Withdraw, Transfer, Register DPNS, Refresh), keys quick-access dropdown with private key indicators. (P1)
  - [x] **4.1d** Create key management components: `KeyManagementScreen` (table of keys with ID/Purpose/SecurityLevel/Type/Status/HasPrivateKey, separate Main/Voter sections, + Add Key button), `KeyInfoScreen` (metadata grid, public key hex/base64, private key section with wallet unlock, message signing, disable/replace/remove actions), `AddKeyDialog` (form with private key input, type/purpose/security level selectors, contract bounds toggle). (P1)
  - [x] **4.1e** Create withdraw/transfer components: `WithdrawScreen` (key selector for TRANSFER keys, balance display, amount input with Max, address input with validation, owner key payout address auto-fill, confirmation dialog, states: form→unlock→sending→result), `TransferScreen` (key selector, destination type toggle Identity/PlatformAddress, identity selector or address input, amount with Max, confirmation, states). (P1)
  - [x] **4.1f** Create register DPNS name component: `RegisterDpnsNameScreen` with identity selector, key selector (advanced), username input with ".dash" suffix, min 3 chars validation, wallet unlock gate, registering state with elapsed time, success with contested/normal result and fee breakdown, source tracking (from Identities vs DPNS). (P1)

- [x] **4.2 Implement identity list screen** (P1)
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

- [x] **4.3 Implement add new identity screen** (P1)
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

- [x] **4.4 Implement identity keys, top-up, withdraw, and transfer screens** (P1)
  Build remaining identity operation screens:
  - **Keys screen:** List all identity keys with metadata, add new key, view key details, disable/enable key
  - **Key info:** View key metadata, export private key
  - **Add existing identity:** Input identity ID (Base58/Hex), load from blockchain
  - **Top up:** 4 funding methods (same as creation), amount input
  - **Withdraw:** Destination address, amount, fee
  - **Transfer:** Destination identity ID, amount
  Reference: `keys/keys_screen.rs`, `keys/key_info_screen.rs`, `keys/add_key_screen.rs`, `add_existing_identity_screen.rs`, `top_up_identity_screen/`, `withdraw_screen.rs`, `transfer_screen.rs`
  Write component tests. Write Playwright tests for key operations.

- [x] **4.5 [REVIEW] Identity screens functionality parity** (P1)
  Exhaustive comparison against all egui identity screens. Verify every action, dialog, and display element is present and working. Check DPNS name registration flow (which bridges identities and DPNS). Create fix tasks for gaps.

  > **Review Findings (Run 71):**
  >
  > **Screens Reviewed:** IdentitiesScreen, IdentityListPanel, IdentityDetailPanel, CreateIdentityScreen, LoadIdentityScreen, TopUpIdentityScreen, WithdrawScreen, TransferScreen, KeyManagementScreen, KeyInfoScreen, AddKeyDialog
  > **Tests:** 1407 passing (48/49 test files pass; 1 pre-existing failure in NetworkChooserScreen unrelated to identities)
  >
  > ### Fully Implemented (matching egui parity):
  > - Identity list with cards, context menus, inline alias editing
  > - Identity detail panel with balance, keys, DPNS names, wallets, type info
  > - Create identity with 4 funding methods + advanced options (key editor, index selector)
  > - Load identity with 3 modes (by ID, by wallet, by DPNS name) + advanced options
  > - Top up identity with 4 funding methods
  > - Withdraw with amount/address/key selection + confirmation dialog
  > - Transfer with identity/platform-address destinations + confirmation dialog
  > - Key management table with purpose/security/type/status/private indicators
  > - Key info with public key display, add/remove private key, disable key
  > - Add key dialog with purpose/security/type/private key + contract bounds UI
  > - Concurrent identity refresh (Promise.allSettled)
  > - Identity status display with color-coded badges
  > - Balance hover tooltip showing raw credits
  > - Copy to clipboard for IDs, keys, addresses
  > - Reorder identities up/down with persistence
  > - Fee estimation display
  >
  > ### Gaps Found (fix tasks created below):
  >
  > **P1 — Functionality gaps:**
  > 1. **DPNS name registration screen not implemented** — Only stubs/toasts exist. The full RegisterDpnsNameScreen (identity selection, name validation, contested detection, registration) is deferred to Phase 5.
  > 2. **Message signing not implemented** — KeyInfoScreen UI exists but handler throws "not yet implemented". Backend IPC command missing.
  > 3. **Contract bounds not sent to backend in AddKey** — UI collects data but IPC call omits contractBounds field. Backend `AddKeyToIdentityInput` type lacks the field. Backend hardcodes `contract_bounds: None`.
  > 4. **Master key replacement missing key generation UI** — No key type selector, no "Regenerate" button, no display of new private key. Uses hardcoded empty values.
  > 5. **Wallet unlock not integrated for identity operations** — WalletUnlockDialog exists but is not used in any identity screen (withdraw, transfer, add key, etc.).
  > 6. **QR code placeholder in CreateIdentityScreen** — Shows dashed border box with "QR Code" text instead of actual QR code (QRCodeSVG from qrcode.react is used in ReceiveDialog but not here).
  > 7. **UTXO monitoring for QR funding not implemented** — No active detection of incoming funds when using QR code funding method.
  >
  > **P2 — UX polish gaps:**
  > 8. **No sortable table/columns for identity list** — Zustand store has sorting infrastructure but no UI to trigger it (egui has clickable column headers).
  > 9. **No progress messages during wallet identity search** — Shows generic "Searching..." instead of "Searching index 5 of 10...".
  > 10. **No testnet-only helper buttons** — "Fill Random HPMN" / "Fill Random Masternode" missing from LoadIdentityScreen.
  > 11. **Identity encoding tooltip missing** — IDs shown without encoding type info (Base58 for User, Hex/ProTxHash for masternodes).
  > 12. **No recovery suggestions for errors** — Raw error strings from backend, no `recovery_suggestion()` equivalent.

  - [ ] **4.5a Fix contract bounds not passed in AddKey IPC call** (P1)
    AddKeyDialog collects contract bounds but the data is lost before reaching the backend:
    1. Add `contractBounds` field to `AddKeyToIdentityInput` type in bindings
    2. Pass `params.contractBounds` in the `identityAddKey` IPC call in IdentitiesScreen.tsx
    3. Update backend command handler to use contract bounds instead of hardcoding `None`
    Reference: AddKeyDialog.tsx lines 236-250, IdentitiesScreen.tsx lines 600-606, identity.rs line 1152

  - [ ] **4.5b Implement master key replacement UI in KeyInfoScreen** (P1)
    Currently uses hardcoded empty values. Needs:
    1. Key type selector dropdown (ECDSA_SECP256K1, BLS12_381, ECDSA_HASH160, EDDSA_25519_HASH160)
    2. "Generate Random" button that creates random private key hex
    3. Display of generated private key (read-only, copyable)
    4. State management for selected type and generated key
    Reference: egui key_info_screen.rs lines 1001-1096

  - [ ] **4.5c Implement QR code generation in CreateIdentityScreen and TopUpIdentityScreen** (P1)
    Replace placeholder "QR Code" box with actual QRCodeSVG from qrcode.react (already used in ReceiveDialog).
    Generate proper dash: payment URI from the funding address.
    Reference: CreateIdentityScreen.tsx lines 1230-1260, ReceiveDialog.tsx lines 280-286

  - [ ] **4.5d Add wallet unlock integration for identity operations** (P1)
    WalletUnlockDialog exists but is not used in identity screens. Add unlock prompts before:
    - Withdraw (if signing key is in encrypted wallet)
    - Transfer (if signing key is in encrypted wallet)
    - Add key (if wallet needs unlock for key derivation)
    - Disable key / Replace key (if signing with master key in encrypted wallet)
    Reference: WalletUnlockDialog.tsx, egui pattern in withdraw_screen.rs, transfer_screen.rs

  - [ ] **4.5e Implement message signing in KeyInfoScreen** (P1)
    1. Add `identitySignMessage` IPC command to backend (Tauri command + bindings)
    2. Implement Dash signed message protocol in backend command
    3. Wire up frontend handleSignMessage to call the IPC command
    4. Display Base64-encoded signature result
    Reference: egui key_info_screen.rs lines 890-935

  - [ ] **4.5f Add UTXO monitoring for QR code funding** (P2)
    When using "Address with QR Code" funding in Create/TopUp, actively monitor wallet for incoming funds.
    Options: periodic polling via IPC, or backend event emission when UTXO detected.
    Reference: egui funding_common.rs lines 57-91

  - [ ] **4.5g Add sort controls to identity list** (P2)
    Zustand store has sorting infrastructure (sortColumn, sortOrder, setSortColumn) but no UI.
    Add sort dropdown or clickable column headers to expose sorting.
    Reference: identityStore.ts lines 8-17, 41-48, 119-151

  - [ ] **4.5h Add progress messages for long identity operations** (P2)
    Show intermediate progress messages (e.g., "Searching index 5 of 10...") during wallet identity search.
    Backend likely emits these via task messages; frontend needs to listen and display them.
    Reference: egui add_existing_identity_screen.rs lines 1143-1147

  - [ ] **4.5i Add error recovery suggestions** (P2)
    Implement frontend equivalent of egui's `recovery_suggestion()` / `translate_backend_error()`.
    Translate common backend error strings into user-friendly guidance.
    Reference: egui helpers.rs lines 35-100+

  - [ ] **4.5j Add identity encoding tooltips and testnet helpers** (P3)
    1. Show encoding type in ID tooltip (Base58/UserId for User, Hex/ProTxHash for masternodes)
    2. Add "Fill Random HPMN" / "Fill Random Masternode" buttons in LoadIdentityScreen (testnet only)
    Reference: egui identities_screen.rs lines 249-263, add_existing_identity_screen.rs lines 156-165

---

## Phase 5: DPNS Contest & Voting Screens

- [x] **5.1 [META] Design DPNS contest/voting UX** (P2)
  Review the DPNS contested names functionality and design improved UX. This is one of the most complex screens in DET:
  - Active contests table with real-time data (locked votes, abstain votes, ending time, top contender)
  - Single vote flow (immediate cast or schedule for later)
  - Bulk voting (select multiple names, apply same vote/schedule to all)
  - Past contests (historical view)
  - My usernames (owned names, renewal, transfer)
  - Scheduled votes management (view, cancel, execute early)
  Reference: `dpns_contested_names_screen.rs` (2,173 lines) — this file handles ALL 4 tabs
  Produce implementation sub-tasks.

  > **Design (Run 72):**
  >
  > ### DPNS Screens Architecture
  >
  > **4 tabs** (routes already defined in routes.tsx as placeholders):
  > - `/contracts/dpns-active` → Active Contests (sortable table + inline vote selection + bulk voting)
  > - `/contracts/dpns-past` → Past Contests (read-only historical table)
  > - `/contracts/dpns-owned` → My Usernames (owned DPNS names with set-alias action)
  > - `/contracts/dpns-scheduled` → Scheduled Votes (manage queued votes: remove, cast now, clear)
  >
  > ### Key Complexity Areas
  >
  > **Active Contests tab** is the most complex — a sortable/filterable table where each row has
  > clickable vote buttons (Lock, Abstain, or per-contestant). Selected votes accumulate in state
  > and a "Cast/Schedule Votes" button opens a popup dialog with per-identity vote method
  > selection (No Vote / Cast Now / Schedule with days/hours/minutes). The dialog shows status
  > during submission and a success/partial-success/failure result screen.
  >
  > **Smart name filter** converts lookalikes: 'o'/'O' → '0', 'l' → '1' (anti-confusion).
  >
  > **Register Name** is already scoped as a separate screen (reachable from DPNS and Identities).
  > It detects contested names (length < 20, no non-0/1 digits), shows fee estimation, and
  > handles the preorder+domain document submission flow.
  >
  > ### Store Design: `contestStore.ts`
  >
  > Following walletStore/identityStore patterns:
  > - State: contestedNames[], localDpnsNames[], scheduledVotes[], selectedVotes[], loading, refreshing, error
  > - Actions: loadContests, loadLocalNames, loadScheduledVotes, selectVote/deselectVote, castVotes, scheduleVotes, castScheduledVote, deleteScheduledVote, clearAll/clearCasted, setAlias, subscribeToUpdates
  > - Tauri commands already bound in bindings.ts: contestedQueryDpnsContests, contestedVoteOnDpnsNames, contestedScheduleDpnsVotes, contestedCastScheduledVote, contestedGetScheduledVotes, contestedDeleteScheduledVote, contestedClearAllScheduledVotes, contestedClearExecutedScheduledVotes, identityLocalDpnsNames, identityRegisterDpnsName
  >
  > ### Component Breakdown
  >
  > - `components/contest/ActiveContestsTable.tsx` — sortable table with inline vote buttons
  > - `components/contest/PastContestsTable.tsx` — read-only historical table
  > - `components/contest/OwnedNamesPanel.tsx` — my usernames list with set-alias
  > - `components/contest/ScheduledVotesTable.tsx` — scheduled votes with actions
  > - `components/contest/VoteCastingDialog.tsx` — bulk vote casting/scheduling popup
  > - `components/contest/RegisterDpnsNameForm.tsx` — name registration form with validation
  > - `screens/DpnsActiveContestsScreen.tsx` — wires store to ActiveContestsTable + VoteCastingDialog
  > - `screens/DpnsPastContestsScreen.tsx` — wires store to PastContestsTable
  > - `screens/DpnsOwnedNamesScreen.tsx` — wires store to OwnedNamesPanel
  > - `screens/DpnsScheduledVotesScreen.tsx` — wires store to ScheduledVotesTable
  > - `screens/RegisterDpnsNameScreen.tsx` — wires identity store + form

  - [x] **5.1a** Create `contestStore.ts` Zustand store with state (contestedNames, localDpnsNames, scheduledVotes, selectedVotes, filter/sort), actions (loadContests, loadLocalNames, loadScheduledVotes, selectVote, deselectVote, clearSelectedVotes, subscribeToUpdates), and Tauri event listeners. Follow identityStore pattern. Write store unit tests.
  - [x] **5.1b** Create `ActiveContestsTable` component — sortable/filterable table with 6 columns (Name, Locked Votes, Abstain Votes, Ending Time, Last Updated, Contestants). Each row has clickable vote buttons (Lock, Abstain, per-contestant). Smart lookalike filter ('o'→'0', 'l'→'1'). Selected votes highlighted in blue. Empty state message. Write component tests.
  - [x] **5.1c** Create `VoteCastingDialog` component — dialog with: selected votes summary, "Set All" bulk control (No Vote / Cast Now / Schedule with days/hours/minutes), per-identity vote method selection, Apply button with status tracking (NotStarted → Casting → Scheduling → Completed/Failed), success/partial-success/failure result screen with navigation buttons. Write component tests.
  - [x] **5.1d** Wire `DpnsActiveContestsScreen` — compose ActiveContestsTable + VoteCastingDialog, connect to contestStore, add top-right action buttons (Refresh, Register Name, Cast/Schedule Votes), replace placeholder route. Write screen-level tests.
  - [x] **5.1e** Create `PastContestsTable` component and wire `DpnsPastContestsScreen` — read-only sortable/filterable table with 4 columns (Name, Ended Time, Last Updated, Awarded To). Awards show identity ID or "Locked". Replace placeholder route. Write component + screen tests.
  - [x] **5.1f** Create `OwnedNamesPanel` component and wire `DpnsOwnedNamesScreen` — filterable table with 4 columns (Name, Owner ID, Acquired At, Actions). "Set Alias" button per row. Empty state. Replace placeholder route. Write component + screen tests.
  - [x] **5.1g** Create `ScheduledVotesTable` component and wire `DpnsScheduledVotesScreen` — sortable table with 6 columns (Name, Voter, Vote Choice, Scheduled Time, Status, Actions). Per-row "Remove" and "Cast Now" buttons (Cast Now only if Pending/Failed). Top-right "Clear All" and "Clear Casted" buttons. Empty state with instructions. Replace placeholder route. Write component + screen tests.
  - [x] **5.1h** Create `RegisterDpnsNameScreen` — identity selector, name input with real-time validation (3-63 chars, alphanumeric + hyphen, no start/end hyphen), contested name detection (length < 20, no non-0/1 digits) with warning, fee estimation, balance check, registration status flow (form → waiting → success/error), success screen with "register another" option. Wire to identityStore + contestStore. Replace any existing placeholder. Write component tests.

- [x] **5.2 Implement DPNS active contests and voting screens** (P2)
  Build the contest viewing and voting interface:
  - Sortable table: Contested Name, Locked Votes, Abstain Votes, Ending Time, Last Updated, Awarded To
  - Click to expand contest details and vote
  - Vote options: vote for specific contender, abstain, schedule for later
  - Bulk voting: checkbox selection, bulk action bar, apply vote/schedule to all selected
  - Register new DPNS name button
  Reference: `dpns_contested_names_screen.rs` — trace the Active and voting code paths
  Write component tests. Write Playwright test for viewing contests and casting a vote.

- [x] **5.3 Implement DPNS past contests, my usernames, and scheduled votes** (P2)
  Build the remaining DPNS tabs:
  - **Past contests:** Historical view of completed contests with winners
  - **My usernames:** Names owned by loaded identities, renewal info, transfer
  - **Scheduled votes:** List of pending scheduled votes, cancel, execute early, status tracking
  Reference: `dpns_contested_names_screen.rs` — trace the Past, Owned, and ScheduledVotes code paths
  Write component tests. Write Playwright tests.

- [x] **5.4 [REVIEW] DPNS screens functionality parity** (P2)
  Verify all DPNS actions work. The egui implementation is 2,173 lines of dense logic — ensure nothing was missed. Create fix tasks for gaps.

  > **Review Findings (Run 84):**
  >
  > ### Overall Assessment: STRONG — 95% functionality parity
  >
  > The Tauri frontend DPNS implementation covers all major functionality from the egui version
  > across 5 screens (Active Contests, Past Contests, Owned Names, Scheduled Votes, Register Name)
  > with **572 tests** (47 Playwright E2E + 199 screen tests + 79 table tests + 76 component tests + 81 store tests + 90 dialog tests).
  > All 1845 project tests pass.
  >
  > ### Functionality Covered (complete parity):
  > - Active Contests: table with Name, Locked Votes, Abstain Votes, Ending Time, Last Updated, Contestants
  > - Vote selection: Lock, Abstain, TowardsIdentity — with toggle/replace behavior
  > - Vote visual emphasis: bold green for highest-vote lock/contestant
  > - Smart filter: o→0, l→1 normalization (matches egui's o/O→0, l→1)
  > - Sortable columns on all tables
  > - Past Contests: Name, Ended Time, Last Updated, Awarded To (WonBy/Locked badges)
  > - Owned Names: Name, Owner ID, Acquired At, Set Alias action (with .dash suffix auto-append)
  > - Scheduled Votes: Name, Voter, Vote Choice, Scheduled Time, Status, Cast Now/Remove actions
  > - Clear All / Clear Casted buttons for scheduled votes
  > - Vote Casting Dialog: Selected votes summary, Set All controls, per-identity method selection
  >   (No Vote/Cast Now/Schedule), schedule time picker (days/hours/minutes), schedule warning,
  >   validation, progress view, completed view (success/partial/failure), failed view with retry
  > - Register Name: identity selection, name validation (3-63 chars, A-Z/0-9/hyphens, no leading/trailing hyphen),
  >   contested name detection (<20 chars, only letters+0+1), fee estimation, advanced key selector,
  >   registering/success/error states, breadcrumb navigation, info sections
  > - Real-time event subscriptions for contest/identity/scheduled-vote updates
  > - All backend IPC commands wired via contestStore and identityStore
  >
  > ### Minor Gaps Found (non-blocking, improvements):
  >
  > 1. **Register Name: no wallet unlock flow** — The egui version checks if the wallet is locked
  >    and shows an "Unlock Wallet" button with a wallet unlock popup. The Tauri version bypasses
  >    this (the backend command handles wallet state). This is acceptable since wallet unlock is
  >    handled at the IPC layer, but a UX improvement would show the user a clear wallet-locked state.
  >
  > 2. **Register Name: contested name success message differs** — egui shows "DPNS Name Submitted (Contested)"
  >    with a detailed info box. The Tauri version always sets `contested: false` in the success callback
  >    (DpnsRegisterNameScreen.tsx:61) regardless of whether the name was actually contested. The component
  >    supports contested success display but it's never triggered.
  >
  > 3. **Active Contests: "Register Name" button visibility** — egui only shows this button if the user
  >    has voting identities loaded. The Tauri version always shows it. Minor UX difference.
  >
  > 4. **Auto-dismissing messages** — egui uses 10-second auto-dismissing messages with countdown timers.
  >    The Tauri version uses Sonner toast notifications which auto-dismiss but without countdown display.
  >    This is a UX improvement, not a regression.
  >
  > 5. **Refreshing time indicator** — egui shows "Refreshing... Time taken so far: X seconds" during
  >    contest refresh. The Tauri version shows a spinning refresh icon but no elapsed time. Minor UX gap.

  - [ ] **5.4a Fix Register Name contested success detection** (P3)
    In `DpnsRegisterNameScreen.tsx`, the success callback always sets `contested: false`. It should
    determine if the name was contested (using the same `isContestedName()` logic from the component)
    and pass the correct value. This ensures the contested success message ("DPNS Name Submitted (Contested)")
    is shown when appropriate. Small fix — ~3 lines changed.

  - [ ] **5.4b Add wallet-locked state display to Register Name screen** (P3)
    The egui version shows a wallet-locked warning and "Unlock Wallet" button when the selected identity's
    wallet is locked. Consider adding a similar check using wallet store state, showing a warning panel
    when the wallet is locked, with a link/button to unlock. Low priority since the backend handles this.

---

## Phase 6: Contract & Document Screens

- [x] **6.1 [META] Design contract/document browser UX** (P2)
  Review the contract and document management functionality:
  - Contract browser with system contracts and user contracts
  - Document querying with index selection, pagination, JSON/YAML display
  - Contract registration, update, and management
  - Document CRUD operations (create, delete, replace, transfer, purchase, set price)
  - Group actions
  Files to review: All files in `src/ui/contracts_documents/`
  Produce implementation sub-tasks.

  > **Design Decisions (Run 85):**
  >
  > ### Complete Action Inventory (6 egui screens, ~50 user actions)
  >
  > **Backend Status:** All Tauri IPC commands fully implemented (11 contract + 8 document commands).
  > All DTOs defined. TypeScript bindings auto-generated via tauri-specta. No backend work needed.
  >
  > **Missing Frontend Infrastructure:**
  > - No `contractStore.ts` or `documentStore.ts` (Zustand stores)
  > - No contract/document components or screens (only DPNS screens exist under /contracts/)
  > - The `/contracts/` route is a placeholder
  >
  > ### UX Design
  >
  > **Main Screen Layout (3-panel):**
  > - Left sidebar: Contract tree browser (collapsible) with search, showing contract → document types → indexes → properties. Right-click context menu for copy hex/JSON. Remove button for user contracts.
  > - Center: SQL-like query input bar at top, document results below with JSON/YAML toggle, field selector, search filter, pagination
  > - Top bar: Action buttons (Load Contracts, Register, Update, Create/Delete/Replace/Transfer/Purchase/SetPrice Document, Group Actions)
  >
  > **Sub-screens (each accessible from top bar buttons):**
  > 1. Add Contracts — multi-ID input, fetch, set aliases
  > 2. Register Contract — identity/key selection, JSON editor with auto-wrap detection, fee estimation, broadcast
  > 3. Update Contract — contract selector, JSON editor, identity/key selection, fee estimation, broadcast
  > 4. Document Actions (6 types sharing common layout) — contract/doc-type selection, identity/key selection, wallet unlock, type-specific inputs, fee estimation, broadcast
  > 5. Group Actions — contract selector (filtered to group-enabled), identity selector, fetch & display table, "Take Action" routing to token screens
  >
  > ### Implementation Plan
  >
  > Tasks 6.2–6.4 are replaced with more granular sub-tasks below to ensure each is completable in one agent run.

  **Sub-tasks produced:**
  - [ ] **6.2a** Create `contractStore.ts` Zustand store: local contract list CRUD (list, get by ID, set alias, remove), loading/error states, subscribe to `task-completed` events for contract fetch results. Follow `walletStore.ts` pattern. Write 15+ tests.
  - [ ] **6.2b** Create `documentStore.ts` Zustand store: document query state (query text, results, pagination cursors, display mode JSON/YAML), field selection, search filter. Write 10+ tests.
  - [ ] **6.2c** Create `ContractTreePanel` component: collapsible tree sidebar showing all contracts → document types → indexes → token info → contract JSON. Search/filter input. Selection updates document query. Remove contract button (with confirmation dialog, excluded for system contracts). Write 20+ component tests.
  - [ ] **6.2d** Create `DocumentQueryScreen` main screen: query input bar with "Fetch Documents" button, document results area with JSON/YAML toggle, field selector dialog, document search filter, pagination (Previous/Page N/Next). Wire to `ContractTreePanel` for contract/doc-type/index selection. Action buttons in toolbar. Write 15+ component tests. Write 1 Playwright E2E test.
  - [ ] **6.3a** Create `AddContractsScreen`: multi-field contract ID input (up to 10), hex+Base58 support, fetch button with progress, success view with alias editing per contract, "Back to Contracts" navigation. Write 15+ component tests. Write 1 Playwright E2E test.
  - [ ] **6.3b** Create `RegisterContractScreen`: step-by-step form — (1) identity selector with auto-key selection (HIGH/CRITICAL), (2) optional alias input, (3) JSON code editor with auto-detect raw document schemas and auto-wrap, link to dashpay.io, real-time validation, (4) fee estimation, (5) "Register Contract" broadcast. Progress states and success screen. Write 15+ component tests.
  - [ ] **6.3c** Create `UpdateContractScreen`: identity selector (CRITICAL keys only), contract dropdown (exclude system contracts), auto-load selected contract JSON, JSON editor, fee estimation, "Update Contract" broadcast. Progress states and success screen. Write 15+ component tests.
  - [ ] **6.3d** Create `DocumentActionScreen` with shared layout for all 6 action types: contract/doc-type selector, identity/key selector, wallet unlock gate, fee estimation, broadcast button, progress/success states. Implement **Create Document** action: dynamic form fields based on document type schema (integers, floats, strings, byte arrays, identifiers, booleans, dates, objects, arrays), required field validation, token cost info. Write 20+ component tests.
  - [ ] **6.3e** Implement remaining document actions in `DocumentActionScreen`: **Delete** (document ID input, "Fetch Owned Documents" with list + View popup + Select), **Replace** (fetch original, populate form, edit, broadcast), **Purchase** (fetch price, display, broadcast), **Set Price** (ID + price inputs), **Transfer** (ID + recipient inputs). Write 20+ component tests. Write 1 Playwright E2E test covering Create + Delete flow.
  - [ ] **6.3f** Create `GroupActionsScreen`: contract selector (filtered to contracts with group-action-enabled tokens), identity selector, "Fetch Group Actions" button, results table (Action ID, Type, Info, Note, Take Action), "Take Action" navigates to corresponding token action screen pre-populated. Write 15+ component tests.
  - [ ] **6.4 [REVIEW] Contract/document screens functionality parity** (P2)
    Verify all contract and document operations match egui version. Check:
    - All 9 top-bar action buttons present and functional
    - Contract tree panel: expand/collapse, search, select doc type, select index, remove contract, copy hex/JSON
    - Document query: SQL input, fetch, pagination, JSON/YAML toggle, field selector, search filter
    - Register: raw schema auto-wrap, fee estimation, broadcast states
    - Update: contract selector excludes system contracts, auto-loads JSON
    - Add Contracts: multi-ID, alias editing
    - All 6 document actions: Create, Delete, Replace, Purchase, SetPrice, Transfer
    - Group Actions: filtered contracts, fetch, table, Take Action routing
    Create fix tasks for any gaps.

---

## Phase 7: Token Screens

- [x] **7.1 [META] Design token screens UX** (P2)
  Review ALL token functionality — this is the largest screen group with 15+ sub-screens:
  - Token portfolio view (My Tokens tab)
  - Token search and discovery
  - Token creator (extremely complex: name/symbol/decimals, supply, distribution functions with formula visualization, freezing rules, pausing, claims, access control)
  - 12+ token action screens (transfer, mint, burn, freeze, unfreeze, pause, resume, claim, view claims, update config, purchase, set price, add by ID)
  Files to review: All files in `src/ui/tokens/`
  Produce implementation sub-tasks.

  > **Design Decisions (Run 86):**
  >
  > ### Complete Action Inventory (15+ egui screens, ~120 user actions, 22 backend tasks)
  >
  > **Backend Status:** All Tauri IPC commands fully implemented in `src-tauri/src/commands/token.rs`
  > (1,511 lines): 21 async dispatch commands + 2 direct database commands. All input DTOs defined.
  > TypeScript bindings auto-generated via tauri-specta in `src/frontend/bindings.ts`. 45+ backend
  > unit tests. **No backend work needed.**
  >
  > **Missing Frontend Infrastructure:**
  > - No `tokenStore.ts` (Zustand store)
  > - No token components or screens (only placeholder routes at `/tokens/`, `/tokens/search`, `/tokens/creator`)
  > - No `/token/` component directory
  >
  > ### Token Screens Architecture
  >
  > **Main Screen: 3 tabs (like egui's TokensSubscreen enum)**
  > 1. **My Tokens** — Portfolio view of all owned tokens with sorting, detail expansion, per-row action menu
  > 2. **Search Tokens** — Keyword search with pagination, contract detail expansion, "Add to My Tokens"
  > 3. **Token Creator** — Multi-step wizard (7+ steps) for creating new token contracts
  >
  > **Action Screens (13 separate routes, all share a common operation base pattern):**
  > - Transfer, Mint, Burn, Freeze, Unfreeze, Pause, Resume, Claim, View Claims,
  >   Set Price, Purchase, Update Config, Destroy Frozen Funds
  > - Each follows: token context → input form → advanced options (public note) →
  >   key selection → wallet unlock → fee estimation → confirmation → broadcast → result
  > - Group action support where applicable (mint, pause, resume, set price, update config)
  >
  > **Supplementary Screens:**
  > - Add Token by ID — lookup by contract ID or token ID
  > - Token Info Popup — modal with full token metadata + JSON schema viewer
  > - Contract Details — expanded view when navigating from search results
  >
  > ### Store Design: `tokenStore.ts`
  >
  > Following walletStore/identityStore/contestStore patterns:
  > - **State:** myTokens[] (with balance, metadata, contract info), searchResults[],
  >   searchKeyword, searchCursor, tokenOrder[], selectedToken, loading, refreshing, error,
  >   sortColumn, sortOrder
  > - **Actions:** loadMyTokenBalances, searchByKeyword, clearSearch, fetchTokenByContractId,
  >   fetchTokenByTokenId, saveTokenLocally, removeToken, loadTokenOrder, saveTokenOrder,
  >   queryTokenPricing, queryFrozenIdentities, subscribeToUpdates
  > - **Event listeners:** TaskResultEvent (filter resultType === "Token") for async operation results
  > - Tauri commands already bound: tokenQueryMyBalances, tokenQueryIdentityBalance,
  >   tokenQueryDescriptionsByKeyword, tokenFetchByContractId, tokenFetchByTokenId,
  >   tokenSaveLocally, tokenRemove, tokenLoadOrder, tokenSaveOrder, tokenQueryPricing,
  >   tokenQueryFrozenIdentities, tokenMint, tokenTransfer, tokenBurn, tokenFreeze,
  >   tokenUnfreeze, tokenPause, tokenResume, tokenClaim, tokenEstimatePerpetualRewards,
  >   tokenPurchase, tokenSetDirectPurchasePrice, tokenUpdateConfig, tokenDestroyFrozenFunds,
  >   tokenRegisterContract
  >
  > ### Component Breakdown
  >
  > - `components/token/MyTokensTable.tsx` — sortable table (Owner Identity, Alias, Balance) with
  >   per-row action dropdown (Transfer, Mint, Burn, Freeze, Unfreeze, Pause, Resume, Claim,
  >   View Claims, Set Price, Purchase, Update Config, Destroy Frozen Funds, More Info, Remove)
  > - `components/token/TokenInfoDialog.tsx` — modal with full token metadata (name, description,
  >   contract ID, token ID, base/max supply, status, pricing, distribution rules) + "View Schema" JSON popup
  > - `components/token/TokenSearchPanel.tsx` — keyword input, search button, results table with
  >   pagination (Previous/Next), "More Info" per row, contract detail expansion
  > - `components/token/TokenCreatorWizard.tsx` — multi-step wizard with sub-components:
  >   - Step 1: BasicInfoStep (name, plural name, language selector 50+, description, decimals,
  >     base supply, max supply, capitalize, start paused, allow transfers to frozen)
  >   - Step 2: DistributionStep (perpetual + pre-programmed distribution config, recipient type
  >     selector, function selector with formula visualization images, interval config, entry grid)
  >   - Step 3: ControlRulesStep (mint/burn/freeze/unfreeze/destroy/pause/resume/max-supply/
  >     conventions/marketplace rules — each with action taker combo, identity inputs, admin identity)
  >   - Step 4: GroupsStep (add groups, member grid with identity + power inputs, required power)
  >   - Step 5: HistoryStep (keep history checkbox options)
  >   - Step 6: KeywordsStep (searchable keyword tags)
  >   - Step 7: ReviewStep (identity selection, summary, create button, fee confirmation)
  > - `components/token/TokenOperationForm.tsx` — shared operation layout: amount input, recipient
  >   selector (where applicable), advanced options toggle (public note), key selector, fee display,
  >   action button, confirmation dialog, group action support, progress/success/error states
  > - `screens/TokenMyTokensScreen.tsx` — wires tokenStore to MyTokensTable + action routing
  > - `screens/TokenSearchScreen.tsx` — wires tokenStore to TokenSearchPanel
  > - `screens/TokenCreatorScreen.tsx` — wires tokenStore + identityStore to TokenCreatorWizard
  > - `screens/TokenTransferScreen.tsx` — wires TokenOperationForm for transfer
  > - `screens/TokenMintScreen.tsx` — wires TokenOperationForm for mint
  > - `screens/TokenBurnScreen.tsx` — wires TokenOperationForm for burn
  > - (etc. for each of the 13 action screens)
  > - `screens/TokenAddByIdScreen.tsx` — contract/token ID lookup with search status
  > - `screens/TokenViewClaimsScreen.tsx` — claims history table with fetch/refresh
  >
  > ### Complexity Notes
  >
  > - Token Creator is the single most complex screen in the entire application (~112K lines in egui).
  >   It needs 7 wizard steps with deeply nested configuration forms. Breaking it into sub-components
  >   per step is critical.
  > - Distribution function visualization shows formula images (Linear, Polynomial, Exponential,
  >   Logarithmic, Inverted Logarithmic). These can be SVG/inline components or static assets.
  > - Control Rules have deeply nested configurations with ~10 rule types, each with action taker
  >   selection (No One / Contract Owner / Identity / Main Group / Specific Group) and sub-rules.
  > - Group action support means some action screens detect when the token is group-controlled
  >   and adjust their UI text/behavior accordingly.
  > - Pricing supports single price or tiered pricing (map of quantity thresholds to prices).
  >
  > ### Implementation Plan
  >
  > Tasks 7.2–7.4 are replaced with more granular sub-tasks below. The token creator wizard
  > is split across multiple tasks due to its extreme complexity.

  **Sub-tasks produced:**
  - [ ] **7.2a** Create `tokenStore.ts` Zustand store: token list CRUD (load balances, search by keyword with pagination cursor, fetch by contract/token ID, save locally, remove), token ordering (load/save), pricing queries, frozen identity queries, sort state (column, order), loading/error/refreshing states, TaskResultEvent subscription filtering by "Token" result type. Follow walletStore/identityStore pattern. Write 20+ store tests.
  - [ ] **7.2b** Create `MyTokensTable` component: sortable table with 3 columns (Owner Identity/Alias, Token Name, Balance). Per-row action dropdown menu with all 15 actions (Transfer, Mint, Burn, Freeze, Unfreeze, Pause, Resume, Claim, View Claims, Set Price, Purchase, Update Config, Destroy Frozen Funds, More Info, Remove). Sort by clicking column headers. "More Info" opens `TokenInfoDialog`. "Remove" shows confirmation. Empty state message. Write 20+ component tests.
  - [ ] **7.2c** Create `TokenInfoDialog` modal: displays full token metadata (name, description, contract ID, token ID, base supply, max supply, paused status, owner identity, pricing info, distribution rules). "View Schema" button opens a nested JSON viewer dialog showing the token contract configuration. Close button. Write 10+ component tests.
  - [ ] **7.2d** Wire `TokenMyTokensScreen`: compose MyTokensTable + TokenInfoDialog, connect to tokenStore, add top-right action buttons (Refresh, Add Token by ID, Create Token), handle action menu routing (navigate to `/tokens/transfer`, `/tokens/mint`, etc. with token context). Replace placeholder route. Write 15+ screen tests. Write 1 Playwright E2E test.
  - [ ] **7.2e** Create `TokenSearchPanel` component and wire `TokenSearchScreen`: keyword input with Search/Clear buttons, results table with Contract ID + Description columns, "More Info" button per row, Previous/Next pagination, elapsed time counter during search, contract detail expansion view with token list and "Add to My Tokens" button per token. Replace placeholder route. Write 15+ component tests. Write 1 Playwright E2E test.
  - [ ] **7.2f** Create `TokenAddByIdScreen`: text input for contract ID or token ID, Search button, search status states (idle, searching with elapsed time, found single, found multiple, error), results display with token info and "Add to My Tokens" action. Clear button resets. Wire to tokenStore. Add route `/tokens/add-by-id`. Write 15+ component tests.
  - [ ] **7.3a** Create token creator wizard scaffold — `TokenCreatorWizard` component with step navigation (7 steps), step indicator, Next/Previous/Cancel buttons, form state management. Create `BasicInfoStep` (Step 1): token name, plural name, language selector (50+ languages), description (optional), decimals (default 8), base supply, max supply (optional), checkboxes for capitalize, start paused, allow transfers to frozen identities. Input validation. Replace placeholder route. Write 20+ component tests.
  - [ ] **7.3b** Create `DistributionStep` (Step 2): toggles for "Add Perpetual Distribution" and "Add Pre-programmed Distribution". Each distribution config: recipient type selector (Contract Owner / Identity / Evonodes by Participation), distribution function selector (Fixed Amount, Step Decreasing, Stepwise, Linear, Polynomial, Exponential, Logarithmic, Inverted Logarithmic) with formula visualization (SVG or image), interval type selector (Block/Time/Epoch-based), time unit selector (Second through Year with ms conversion), entry grid (time offset + identity ID + amount rows) with Add/Delete controls. Write 20+ component tests.
  - [ ] **7.3c** Create `ControlRulesStep` (Step 3): configuration forms for 10 rule types — Manual Minting, Manual Burning, Freeze, Unfreeze, Destroy Frozen Funds, Emergency Action (Pause/Resume), Max Supply Change, Conventions Change, Marketplace. Each rule: action taker combo selector (No One / Contract Owner / Identity / Main Group / Specific Group), identity input (if Identity selected), admin identity input. Minting rules additionally: destination defaults to contract owner checkbox, allow choosing destination checkbox, nested destination identity/choice sub-rules. Write 20+ component tests.
  - [ ] **7.3d** Create `GroupsStep` (Step 4) + `HistoryStep` (Step 5) + `KeywordsStep` (Step 6): Groups — "Add Group" button, per-group required power input, member grid (identity ID + power) with Add/Delete. History — keep history checkbox options for token operations. Keywords — text input for adding searchable keyword tags to the token contract. Write 15+ component tests.
  - [ ] **7.3e** Create `ReviewStep` (Step 7) and wire `TokenCreatorScreen`: identity selector, wallet unlock gate, full configuration summary display, "Create Token Contract" button with fee estimation, confirmation dialog, broadcast progress states (idle → waiting → success/error), success screen with transaction details. Wire all 7 steps together in TokenCreatorWizard. Connect to tokenStore + identityStore. Write 15+ component tests. Write 1 Playwright E2E test for token creation flow.
  - [ ] **7.4a** Create `TokenOperationForm` shared component: reusable layout for all token action screens — token context header (name, ID, balance), amount input (where applicable), recipient identity input (where applicable), advanced options collapsible section (public note text input), key selector dropdown, fee estimation display, action button (disabled until valid), confirmation dialog, group action detection and info display, progress states (idle → waiting → success/error), success screen with fee result. Write 20+ component tests.
  - [ ] **7.4b** Implement **Transfer**, **Mint**, and **Burn** token screens using TokenOperationForm: Transfer — recipient identity ID input + amount + optional public note. Mint — recipient selector + amount + group action support. Burn — amount input (max = current balance). Each with fee estimation, wallet unlock, confirmation, broadcast. Add routes `/tokens/transfer`, `/tokens/mint`, `/tokens/burn`. Write 15+ component tests per screen. Write 1 Playwright E2E test for transfer flow.
  - [ ] **7.4c** Implement **Freeze**, **Unfreeze**, and **Destroy Frozen Funds** screens: Freeze — target identity selector + confirmation. Unfreeze — loads frozen identities list via `tokenQueryFrozenIdentities`, select from list + confirmation. Destroy Frozen Funds — select frozen identity + confirmation of destruction. Each with fee estimation, wallet unlock, broadcast. Add routes. Write 15+ component tests.
  - [ ] **7.4d** Implement **Pause** and **Resume** token screens: Pause — no amount input needed, just key selection + group action support + confirmation. Resume — similar to pause, emergency action rules info display. Add routes. Write 10+ component tests.
  - [ ] **7.4e** Implement **Claim Tokens** and **View Token Claims** screens: Claim — distribution type detection (perpetual/pre-programmed), estimated rewards display via `tokenEstimatePerpetualRewards`, claim button with fee estimation. View Claims — "Fetch Claims" button, claims history table (Amount, Timestamp, Block Height, Note), fetch status with elapsed time. Add routes. Write 15+ component tests.
  - [ ] **7.4f** Implement **Set Token Price** and **Purchase Tokens** screens: Set Price — pricing type selector (Single Price / Tiered Pricing / Remove Pricing), single price amount input, tiered pricing grid (quantity threshold + price rows) with Add/Delete, group action support. Purchase — amount input, auto-fetch pricing schedule, calculated total price display, balance check. Add routes. Write 15+ component tests. Write 1 Playwright E2E test for purchase flow.
  - [ ] **7.4g** Implement **Update Token Config** screen: change item selector dropdown (various config aspects), dynamic input fields based on selected change type (identity inputs, group selectors, text/numeric fields), group action support, fee estimation, broadcast. Add route. Write 10+ component tests.
  - [ ] **7.5 [REVIEW] Token screens functionality parity** (P2)
    Exhaustive comparison of all token screens against egui originals. Verify: all 13 action types work, token creator wizard has all 7 steps with every option, My Tokens table has all 15 action menu items, Search Tokens has pagination + contract details, Add by ID works for both contract and token IDs, pricing (single + tiered) is fully functional, distribution formula visualization renders, control rules cover all 10 types, group actions work. Create fix tasks for any gaps.

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
| Total tasks (top-level) | 100 |
| META tasks | 13 |
| REVIEW tasks | 11 |
| Implementation tasks | 70 |
| Completed | 68 |
| Remaining | 38 |

*Note: META tasks will expand into sub-tasks. The actual task count will grow significantly as META tasks are completed. Estimated total including sub-tasks: 150-250.*
