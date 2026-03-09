# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Branching

- `master` is a **release-only** branch, updated every few months. Do not use it as a base for diffs or PRs during active development.
- `v1.0-dev` is the current active development branch. Use it as the base for general diffs, comparisons, and new feature branches.
- PR and commits should follow conventional commit naming rules.

## Build Commands

```bash
cargo build                    # Debug build
cargo build --release          # Release build
cargo run                      # Run application
cargo fmt --all                # Format code
cargo clippy --all-features --all-targets -- -D warnings  # Lint (warnings as errors)
```

## Testing

```bash
cargo test --all-features --workspace              # All tests
cargo test --doc --all-features --workspace        # Doc tests only
cargo test <test_name> --all-features              # Single test
cargo test --test kittest --all-features           # UI integration tests (egui_kittest)
cargo test --test e2e --all-features               # End-to-end tests
```


Test locations:
- Unit tests: inline in source files (`#[test]`)
- UI integration: `tests/kittest/`
- E2E: `tests/e2e/`

Always run `cargo clippy` and `cargo +nightly fmt` when finalizing your work.


### Manual test scenarios

End-to-end manual test scenarios live in `docs/ai-design/manual-tests.md` (max 10 scenarios covering multiple features each). When a PR changes user-visible behavior, verify it's covered by an existing scenario or update the file. Reference applicable scenario numbers in the PR description under "Test plan".
Skip manual test updates only for non-functional changes (CI, docs, formatting, pure refactoring) — state why in the PR description.

## CI: Safe Cargo Wrapper

In GitHub Actions (Claude Code workflow), use `scripts/safe-cargo.sh` instead of `cargo` directly. This wrapper strips CI secrets from the environment before running cargo, preventing build scripts from accessing credentials.

```bash
scripts/safe-cargo.sh build --all-features
scripts/safe-cargo.sh test --all-features --workspace
scripts/safe-cargo.sh clippy --all-features --all-targets -- -D warnings
scripts/safe-cargo.sh +nightly fmt --all
```


## Coding Conventions

### General rules

* When a method takes `&AppContext` (or `Option<&AppContext>`), place it as the first parameter after `self`.
* Screen constructors handle errors internally via `MessageBanner` and return `Self` with degraded state. Keep `create_screen()` clean — no error handling at callsites.

## Architecture Overview

**Dash Evo Tool** is a cross-platform GUI application (Rust + egui) for interacting with Dash Evolution. It enables DPNS username registration, contest voting, state transition viewing, wallet management, and identity operations across Mainnet/Testnet/Devnet.

## Documentation

- **docs/ai-design** should contain architecture, technical design and manual testing scenarios files, grouped in subdirectories prefixed with ISO-formatted date
- **docs/personas** contains user personas (Everyday User, Power User, Platform Developer) that define the three target user archetypes and the progressive disclosure model for UI complexity. Consult these when making UX decisions about what to show/hide or how to structure wallet features.
- **docs/user-stories.md** catalogs 98 user stories across 10 feature areas, tagged by persona and marked `[Implemented]` or `[Gap]`. Reference when planning new features or verifying coverage.
- **docs/ux-design-patterns.md** is the UI/UX reference card — design tokens, button styles, dialog conventions, form patterns, accessibility rules, and progressive disclosure model. Consult when building or reviewing UI.
- end-user documentation is in a separate repo: https://github.com/dashpay/docs/tree/HEAD/docs/user/network/dash-evo-tool , published at https://docs.dash.org/en/stable/docs/user/network/dash-evo-tool/

### Core Module Structure

- **app.rs** - `AppState`: owns all screens, polls task results each frame, dispatches to visible screen
- **ui/** - Screens and reusable components (`ui/components/`)
- **backend_task/** - Async business logic, one submodule per domain (identity, wallet, contract, etc.)
- **model/** - Data types (amounts, fees, settings, wallet/identity models)
- **database/** - SQLite persistence (rusqlite), one module per domain
- **context/** - `AppContext`: network config, SDK client, database, wallets, settings cache (split into submodules: `identity_db.rs`, `wallet_lifecycle.rs`, `settings_db.rs`, etc.)
- **spv/** - Simplified Payment Verification for light wallet support
- **components/core_zmq_listener** - Real-time Dash Core event listening via ZMQ

### Key Dependencies

- `dash-sdk` - Dash blockchain SDK (git dep from dashpay/platform)
- `egui/eframe 0.33` - Immediate mode GUI framework
- `tokio` - Async runtime (12 worker threads)
- `rusqlite` - SQLite with bundled library
- Rust edition 2024, minimum rust-version 1.92

### Configuration

Environment config via `.env` in app directory:
- macOS: `~/Library/Application Support/Dash-Evo-Tool/.env`
- Linux: `~/.config/dash-evo-tool/.env`
- Windows: `C:\Users\<User>\AppData\Roaming\Dash-Evo-Tool\config\.env`

See `.env.example` for network configuration options.

## App Task System (Critical Pattern)

The UI and async backend communicate through an action/channel pattern:

1. **Screens return `AppAction`** from their `ui()` method (e.g., `AppAction::BackendTask(task)`)
2. **`AppState` spawns a tokio task** that calls `app_context.run_backend_task(task, sender)`
3. **`AppContext::run_backend_task()`** matches on the `BackendTask` enum and dispatches to domain-specific async methods
4. **Results come back** via tokio MPSC channel as `TaskResult` (Success/Error/Refresh)
5. **Main `update()` loop** polls `task_result_receiver.try_recv()` each frame and routes results to the visible screen's `display_task_result()`

```
Screen::ui() → AppAction::BackendTask(task)
    → tokio::spawn → AppContext::run_backend_task()
    → sender.send(TaskResult::Success(result))
    → AppState::update() polls receiver → Screen::display_task_result()
```

**Backend task enums**: `BackendTask` has variants like `IdentityTask(IdentityTask)`, `WalletTask(WalletTask)`, `TokenTask(Box<TokenTask>)`, etc. Each sub-enum has its own variants and corresponding `run_*_task()` method. Results are `BackendTaskSuccessResult` with 50+ typed variants.

**Error handling**: Backend tasks return `Result<T, TaskError>` (`src/backend_task/error.rs`). `TaskError` is a typed error envelope — `Display` produces user-friendly text for `MessageBanner`, `Debug` provides technical details for logs. `From<String>` ensures backwards compatibility: existing `Result<T, String>` code works unchanged. Domain errors (`DashPayError`, `SpvError`, etc.) are wired as `#[from]` variants for automatic conversion via `?`. When adding new backend error types, add a `#[from]` variant to `TaskError` rather than converting to `String`.

## Screen Pattern

All screens implement the `ScreenLike` trait:
- `ui(&mut self, ctx: &Context) -> AppAction` - Render UI, return actions
- `display_task_result(&mut self, result: BackendTaskSuccessResult)` - Handle async results
- `display_message(&mut self, msg: &str, type: MessageType)` - Show user feedback
- `refresh(&mut self)` / `refresh_on_arrival(&mut self)` - Re-fetch data
- `change_context(&mut self, app_context: &Arc<AppContext>)` - Handle network switch

**Screen types**:
- **Root screens**: Stored in `AppState.main_screens` (BTreeMap by `RootScreenType`), persist across navigation
- **Modal/detail screens**: Pushed onto `AppState.screen_stack`, popped when dismissed

Screens hold `Arc<AppContext>` and manage their own UI state.

## AppContext

`AppContext` (~50 fields) is `Arc`-wrapped and shared across all screens and async tasks. Key contents:
- `sdk: RwLock<Sdk>` - Dash SDK (clone for async use to avoid holding lock across await)
- `db: Arc<Database>` - SQLite persistence
- `wallets: RwLock<BTreeMap<...>>` - Loaded wallets
- Cached system contracts (DPNS, DashPay, withdrawals, tokens, keyword search)
- `connection_status`, `developer_mode`, `fee_multiplier_permille`
- Per-network instances (mainnet always present, others created on demand)

## UI Component Pattern

Components follow a lazy initialization pattern (see `docs/COMPONENT_DESIGN_PATTERN.md`):

```rust
struct MyScreen {
    amount: Option<Amount>,              // Domain data
    amount_widget: Option<AmountInput>,  // UI component (lazy)
}

// In show():
let widget = self.amount_widget.get_or_insert_with(|| AmountInput::new(type));
let response = widget.show(ui);
response.inner.update(&mut self.amount);
```

**Requirements:**
- Private fields only
- Builder methods for configuration (`with_label()`, etc.)
- Response struct with `ComponentResponse` trait (`has_changed()`, `is_valid()`, `changed_value()`)
- Self-contained validation and error handling
- Support both light and dark mode via `ComponentStyles`

**Anti-patterns:** public mutable fields, eager initialization, not clearing invalid data

## Message Display

User-facing messages (errors, warnings, success, infos) use `MessageBanner` (`src/ui/components/message_banner.rs`). Global banners are rendered centrally by `island_central_panel()` — `AppState::update()` sets them automatically for backend task results. When using `MessageBanner::set_global()`, no guard is needed — it is idempotent and automatically logs at the appropriate level (error/warn/debug). Screens only override `display_message()` for side-effects. See the component's doc comments and `docs/ai-design/2026-02-17-unified-messages/` for details.

**BannerHandle lifecycle**: Screens that run backend tasks typically store a `refresh_banner: Option<BannerHandle>` field. On task dispatch, set it via `MessageBanner::set_global()` with an info/progress message. In `display_message()` (called as a side-effect by AppState), dismiss the progress banner via `self.refresh_banner.take_and_clear()` (from `OptionBannerExt`). Simply setting the field to `None` would leak the banner — `take_and_clear()` removes it from the egui context. AppState handles displaying the actual result banner.

**Logging**: MessageBanner logs all displayed messages (with details) automatically. Additional logging is unnecessary.

**Error banners**: Never expose raw backend/database errors to users. Use a generic user-friendly message in the banner and attach technical details via `BannerHandle::with_details()`. When the error implements `Display` and its text is user-appropriate, pass it directly to `set_global`; otherwise use a descriptive generic message:
```rust
MessageBanner::set_global(ctx, "Failed to load token balances", MessageType::Error)
    .with_details(e);
```

## Database

Single SQLite connection wrapped in `Mutex<Connection>`. Schema initialized in `database/initialization.rs`. Domain modules provide typed CRUD methods. Backend task errors use `TaskError` (`src/backend_task/error.rs`) — see App Task System section above.

## Platform Targets

Linux (x86_64/aarch64), Windows (x86_64), macOS (x86_64/aarch64 with code signing)

Requires protoc v25.2+ for protocol buffer compilation. Different ZMQ libraries for Windows (`zeromq`) vs Unix (`zmq`).
