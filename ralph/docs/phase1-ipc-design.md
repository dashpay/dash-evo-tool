# Phase 1: Backend Bridge — IPC Design & Audit

## 1.1 IPC Command API Design (Run 10)

### Complete Backend Inventory

**13 BackendTask domains, ~120 task variants, ~100 result variants:**
- IdentityTask: 16 variants -> IdentityResult: 10 variants
- DocumentTask: 8 variants -> DocumentResult: 9 variants
- ContractTask: 7 variants -> ContractResult: 11 variants
- ContestedResourceTask: 7 variants -> ContestResult: 6 variants
- CoreTask: 10 variants -> CoreResult: 1 variant (wraps CoreItem: 5 variants)
- WalletTask: 6 variants -> WalletResult: 8 variants
- DashPayTask: 14 variants -> DashPayResult: 14 variants
- TokenTask: 23 variants -> TokenResult: 20 variants
- SystemTask: 2 variants -> SystemResult: 1 variant
- MnListTask: 5 variants -> MnListResult: 4 variants
- PlatformInfoTask: 8 variants -> PlatformResult: 1 variant (wraps 3 sub-variants)
- GroveSTARKTask: 2 variants -> GroveSTARKResult: 2 variants
- BroadcastStateTransition: 1 (no separate enum)

**18 direct database methods called by UI** (bypassing BackendTask):
- Identity ordering, alias management, deletion
- Token ordering
- DashPay contacts, profiles, payments (save/load/update)
- Contract listing
- Onboarding completion, SPV auto-start, wallet address management

**~76 AppContext public methods** across 6 modules:
- Core: initialization, animation, developer mode, fee management, platform info
- Settings: get/update settings, password, Dash Core config, ZMQ config
- Identity DB: CRUD for identities, aliases, DPNS names, scheduled votes
- Contract/Token DB: CRUD for contracts, tokens, balances
- Wallet lifecycle: SPV management, wallet bootstrapping, address registration
- Transaction processing: finality, asset locks

### TypeScript Type Generation Strategy: tauri-specta

**Chosen: `tauri-specta` v2 (Specta + Tauri 2.0 native integration)**
- Automatic dependency resolution — when a parent type is registered, all dependent types are included automatically (critical for 100+ types)
- Native Tauri 2.0 integration — Tauri added a `specta` feature flag for AppHandle, State, Window types
- Generates both TypeScript types AND type-safe command wrapper functions
- Event type generation supported in v2
- `#[specta::specta]` decorator on each `#[tauri::command]` function
- `tauri_specta::collect_commands!` macro gathers all commands
- Types exported to `src/frontend/bindings.ts` automatically during dev builds

### IPC Command Design Decisions

**1. Command Grouping:** One Rust module per domain (13 modules in `src-tauri/src/commands/`)

**2. Naming Convention:** `snake_case` Rust functions, auto-converted to `camelCase` by tauri-specta for TypeScript. Prefix with domain: `identity_load`, `wallet_send_payment`, `token_mint`, etc.

**3. Serialization:** serde JSON (Tauri default). All command args and return types derive `Serialize + Deserialize + specta::Type`. Complex Rust types (Arc, RwLock) replaced with serializable DTOs at the IPC boundary.

**4. Error Handling:** All commands return `Result<T, String>` at the Tauri boundary. Domain errors mapped to descriptive strings. Frontend receives structured error objects.

**5. Async Pattern:** Commands that dispatch BackendTasks are `async`. Long-running operations use Tauri events for progress/results rather than blocking the IPC call. Short reads (database queries, config) return directly.

**6. State Access:** `tauri::State<AppState>` injected into every command. AppState wraps `Arc<AppContext>` per network + active network selection.

**7. Direct DB Access:** The 18 UI-direct database calls will be exposed as dedicated Tauri commands (not routed through BackendTask). Grouped into `commands/db.rs` or folded into the relevant domain module.

**8. Wallet References:** The egui code passes `Arc<RwLock<Wallet>>` to tasks. For IPC, commands accept `WalletSeedHash` (a serializable identifier) and look up the wallet from AppState.

**9. Events (Backend -> Frontend):**
- `task-completed` — BackendTaskSuccessResult payloads
- `task-error` — BackendTaskError payloads
- `zmq-instant-lock` — InstantLock transaction data
- `zmq-chain-lock` — ChainLock data
- `zmq-chain-locked-block` — Block + ChainLock data
- `zmq-connection-status` — ZMQ connection state changes
- `spv-status` — SPV sync progress
- `wallet-updated` — Wallet balance/state changes
- `scheduled-vote-executed` — Vote casting results

---

## 1.9 Backend Bridge Completeness Audit (Run 20)

### TypeScript Bindings: COMPLETE (A+)
- 163/163 registered Rust commands exported to TypeScript
- 8/8 events properly mapped with correct naming
- 153 TypeScript types exported (all DTOs, enums, input types)
- Consistent `Result<T, string>` error pattern across 134 commands
- Proper camelCase conversion, nullable handling (`| null`), enum -> string union
- No `any`/`unknown` in public API types

### BackendTask Coverage: 7 GAPS FOUND

**IdentityTask gaps (4 variants missing):**
- `RegisterIdentity` — No `identity_register` Tauri command (critical: identity creation)
- `TopUpIdentity` — No `identity_top_up` Tauri command (critical: identity funding)
- `TopUpIdentityFromPlatformAddresses` — No Tauri command
- `TransferToAddresses` — No `identity_transfer_to_addresses` command (only `identity_transfer` for single-identifier transfers)

**WalletTask gaps (1 variant missing):**
- `FundPlatformAddressFromAssetLock` — Not exposed (deferred: needs AssetLockProof serialization)

**Other domains: COMPLETE** — All variants for Document (8/8), Contract (7/7), Core (10/10), DashPay (14/14), Token (22/22), ContestedResource (7/7), PlatformInfo (8/8), System (2/2), MnList (5/5), GroveSTARK (2/2), BroadcastStateTransition (1/1) are covered.

### AppContext Method Gaps (4 methods missing):
- `set_core_backend_mode()` — Used in network_chooser_screen.rs (6 call sites), no Tauri command
- `get_contract_by_token_id()` — Used in token detail screens, no Tauri command
- `bootstrap_wallet_addresses()` — Used in wallet import/creation, no Tauri command
- Wallet creation flow (save_wallet in add_new_wallet_screen.rs and import_mnemonic_screen.rs) — No Tauri command covers creating/importing wallets

### Event System: COMPLETE
- All 8 events properly typed and exported
- Error handling consistent (`Result<T, String>` mapped to `Result<T, string>`)
