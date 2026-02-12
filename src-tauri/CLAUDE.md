# CLAUDE.md

## What This Crate Is

`dash-evo-tool-tauri` is the Tauri v2 shell that bridges the existing `dash-evo-tool` Rust backend to a React + TypeScript frontend. It contains no business logic itself — it exposes IPC commands and events that translate between frontend DTOs and backend `BackendTask`/`AppContext` types.

## Build & Test

```bash
# From src-tauri/
cargo build                                          # Debug build (also regenerates TS bindings)
cargo build --release                                # Release build
cargo test --all-features                            # Run unit tests
cargo clippy --all-features --all-targets -- -D warnings  # Lint
cargo fmt --all                                      # Format

# Full app dev (from repo root, not src-tauri/)
npm run tauri dev                                    # Launches Vite + Tauri together
```

Debug builds auto-generate `src/frontend/bindings.ts` via tauri-specta. The `export_typescript_bindings` test in `main.rs` can also regenerate bindings without a full build.

## Module Layout

```
src/
├── main.rs              # Tauri setup, specta builder, command/event registration
├── state.rs             # AppState: multi-network AppContext management
├── task_dispatcher.rs   # Async task dispatch, result classification, polling loops
├── events.rs            # Tauri event structs (TaskResult, ZMQ, SPV, Wallet, Vote)
├── commands/            # IPC command handlers, one file per domain (14 modules)
│   ├── identity.rs      # ~28 commands
│   ├── wallet.rs        # ~22 commands
│   ├── token.rs         # ~28 commands
│   ├── dashpay.rs       # ~24 commands
│   ├── core.rs          # ~10 commands
│   ├── contract.rs      # ~11 commands
│   ├── document.rs      # ~8 commands
│   ├── contested.rs     # ~10 commands
│   ├── system.rs        # ~10 commands + MnList + QRInfo + GroveSTARK
│   ├── settings.rs      # ~15 commands
│   ├── platform_info.rs # ~8 commands
│   ├── visualizer.rs    # ~5 commands
│   └── proof_log.rs     # ~2 commands
└── dto/                 # Serializable DTOs for the IPC boundary
    ├── common.rs        # NetworkDto, type aliases (IdentifierDto, CreditsDto, etc.)
    ├── wallet.rs        # WalletDto, WalletRefDto (discriminated union), UtxoDto
    ├── identity.rs      # IdentitySummaryDto, QualifiedIdentityDto, input types
    ├── contract.rs      # DataContractDto, ContractWithTokensDto
    ├── document.rs      # DocumentDto, DocumentPropertyValueDto
    ├── token.rs         # TokenDto, TokenBalanceDto, TokenConfigDto
    ├── fee.rs           # FeeEstimationDto
    ├── task_result.rs   # TaskResultPayloadDto (20+ variants), TaskDomain enum
    └── tests.rs         # DTO serialization tests
```

## Core Architecture: Two Command Patterns

### 1. Direct Returns (synchronous)
For fast operations (DB reads, config queries, local state):
```rust
#[tauri::command]
#[specta::specta]
fn wallet_list_all(state: tauri::State<'_, Arc<AppState>>) -> Result<Vec<WalletDto>, String> {
    // Read from DB/memory, return immediately
}
```

### 2. Async Dispatch (event-based)
For long-running operations (blockchain interactions, wallet sync):
```rust
#[tauri::command]
#[specta::specta]
async fn identity_register(
    input: RegisterIdentityInput,
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<DispatchTaskResponse, String> {
    let task = BackendTask::IdentityTask(IdentityTask::RegisterIdentity { ... });
    let task_id = task_dispatcher::dispatch_task(&app_handle, state.inner(), task);
    Ok(DispatchTaskResponse { task_id })
}
```

The frontend receives a `task_id` immediately, then listens for `TaskResultEvent` or `TaskErrorEvent` with that ID. Result payloads are typed via `TaskResultPayloadDto` (internally-tagged discriminated union with `type` field).

## Task Dispatch Flow

```
Command fn → dispatch_task(BackendTask) → returns task_id
    ↓ spawns tokio task
    AppContext::run_backend_task(task, sender)
    ↓ on completion
    classify_success_result() → TaskResultPayloadDto
    ↓
    TaskResultEvent { task_id, result }.emit()  // or TaskErrorEvent on failure
```

Intermediate progress comes through a `SenderAsync<TaskResult>` channel — `Refresh` variants are forwarded as events; `Success`/`Error` are ignored (handled by the outer completion handler).

## Adding a New Command

1. Define input/output DTOs in the appropriate `dto/` module (derive `Serialize, Deserialize, Type`, use `#[serde(rename_all = "camelCase")]`)
2. Write the `#[tauri::command] #[specta::specta]` function in the appropriate `commands/` module
3. Add the command to `collect_commands![]` in `main.rs`
4. Run `cargo build` (debug) to regenerate `bindings.ts`

For async commands, construct a `BackendTask` variant and call `task_dispatcher::dispatch_task()`. Add result classification in `classify_success_result()` if the backend returns a new `BackendTaskSuccessResult` variant.

## State Management

`AppState` holds one `Arc<AppContext>` per network (mainnet required, testnet/devnet/regtest optional). Commands access it via `tauri::State<'_, Arc<AppState>>`.

- `state.current_context()` — context for active network (falls back to mainnet)
- `state.context_for_network(network)` — context for specific network
- `state.db()` — shared SQLite database handle
- `state.switch_network(network)` — change active network

## Background Polling Loops

Started in `main.rs` setup, defined in `task_dispatcher.rs`:
- **Scheduled vote polling** (60s interval) — checks for due DPNS votes, dispatches them
- **SPV status polling** (2s interval) — emits `SpvStatusEvent` for all networks
- **ZMQ forwarding** — receives Dash Core ZMQ messages, emits typed events

## Key Conventions

- All DTOs use `#[serde(rename_all = "camelCase")]` for JSON field names
- Identifiers are hex-encoded strings at the IPC boundary (not raw bytes)
- Amounts are in duffs (1 DASH = 100,000,000 duffs) or credits as appropriate
- `WalletRefDto` is a discriminated union: `{ type: "hd"; seedHash } | { type: "singleKey"; keyHash }`
- Commands return `Result<T, String>` — error strings are displayed to users
- The parent `dash-evo-tool` crate (at `..`) contains all business logic; this crate only does IPC translation
