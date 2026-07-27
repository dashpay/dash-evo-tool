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

### Backend E2E tests (network-dependent)

Tests that exercise backend tasks against a live Dash testnet via SPV (no GUI). Marked `#[ignore]` — require network access, a funded wallet, and serial execution. See `tests/backend-e2e/README.md` for run commands, architecture, and writing guide.

Test locations:
- Unit tests: inline in source files (`#[test]`)
- UI integration: `tests/kittest/`
- E2E: `tests/e2e/`
- Backend E2E: `tests/backend-e2e/` (network-dependent, `#[ignore]`)

### GUI testing (live app, real display)

Driving the actual compiled binary through a real display — for flows that need real navigation, real async/network timing, or visual verification beyond what `kittest` (no display) or `backend-e2e` (no UI) can cover. Read `docs/gui-testing/README.md` before running this kind of test — it has the safety rules (isolated data dir, credential handling, fund-movement caps) and the reusable scenario library under `docs/gui-testing/scenarios/`.

Always run `cargo fmt --all` when finalizing your work — this honors the `rust-toolchain.toml` pin (1.92), matching what `clippy.yml`'s `cargo fmt --all -- --check` actually validates. For `cargo clippy`, see the scope guidance below.

### Local vs CI — avoid duplicate test runs

CI is the full-suite backstop. Do not reproduce it locally. (This section covers your own machine — an agent running *inside* the Claude Code review GitHub Action should follow "CI: Safe Cargo Wrapper" below instead.)

On every **non-draft** PR that touches Rust code, and on pushes to `v*-dev`, GitHub Actions runs the full non-ignored-test gate:

| Workflow | Runs |
|---|---|
| `tests.yml` | `cargo test --all-features --workspace` and `cargo test --doc --all-features --workspace` |
| `clippy.yml` | `cargo fmt --all -- --check` and `cargo clippy --all-features --all-targets -- -D warnings` |

The workflows are path-filtered independently, each on `**/*.rs` (which includes `build.rs`), `**/Cargo.toml`, `Cargo.lock`, `.cargo/config.toml`, and its own workflow file. `tests.yml` additionally watches `tests/backend-e2e/**`, so a documentation-only change under that directory (e.g. `tests/backend-e2e/README.md`) still triggers the test workflow; other documentation-only changes run neither workflow.

Because CI always runs the full sweep, locally you should:

- Run only the **narrowest scope covering your change** — `cargo test <test_name> --all-features`, or `cargo test --test kittest --all-features` for a UI change. Running the whole workspace suite locally only duplicates the run CI is about to do anyway.
- Always run `cargo fmt --all` before committing (honors the `rust-toolchain.toml` pin). It needs no compile, and `clippy.yml` fails the build on unformatted code.
- Run `cargo clippy` locally only for the scope you touched, or when you expect lint fallout. This repo has no `[workspace]`, so there's no `-p <crate>` to narrow with — scope by target instead (e.g. `--bin dash-evo-tool`). CI owns the `--all-features --all-targets` sweep.
- After pushing, watch the PR checks instead of re-running the suite locally.

Two gaps where CI will **not** cover you:

- **Draft PRs run no automatic CI.** Both workflows are gated on `github.event.pull_request.draft != true`, so a draft PR's `pull_request` runs are suppressed. Neither workflow declares a `workflow_dispatch` trigger, so there is no way to run them by hand against a draft branch — mark the PR ready for review (`ready_for_review` triggers the full run) to get CI coverage.
- **Backend E2E tests are not in CI.** The step is commented out in `tests.yml`, and the tests are `#[ignore]`d. If a change touches backend behaviour that only `tests/backend-e2e/` covers, run those locally; CI will not.

A green CI run is only meaningful if it actually executed your tests. `cargo test <filter>` exits 0 and prints `test result: ok` even when the filter matches nothing — when checking a run, confirm your new test names appear in the log **with a pass status**, not merely present. A `#[ignore]`d test (e.g. `test_drop_zeroes_full_capacity` in `src/model/secret.rs`) can appear in the log as `ignored` without having actually run — the "full non-ignored-test gate" above intentionally excludes these; they stay a manual check.

### User stories catalog

When a PR adds or significantly changes user-facing features, check `docs/user-stories.md`:
- If a new feature matches no existing story, add one following the existing format (ID, persona, description, acceptance criteria, `[Implemented]` tag).
- If a `[Gap]` story is now implemented, flip its tag to `[Implemented]`.
- Skip user-story updates for non-functional changes (CI, docs, formatting, refactoring).

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
* **i18n-ready strings**: All user-facing strings (labels, messages, tooltips, errors) must be simple, complete sentences. Avoid concatenating fragments, positional assumptions, or grammar that breaks in other languages. Each string should be extractable as a single translation unit with named placeholders for dynamic values and no logic in the text itself. Current code uses standard Rust format specifiers (`{name}`, `{max}`). When i18n extraction happens later, these will become Fluent-style placeholders (`{ $name }`, `{ $max }`).
* **Never parse error strings** to extract information. Always use the typed error chain (downcast, match on variants, access structured fields). If no typed variant exists for the information you need, define a new `TaskError` variant or extend the existing error type. String parsing is fragile, breaks on message changes, and bypasses the type system.
* **Validation placement**: Pure input validation (format, length, character sets) lives in `model/` as stateless functions — single source of truth, unit-testable, no dependencies on `AppContext` or `Sdk`. Backend tasks are the authoritative enforcement layer: they call model validators for format checks AND perform stateful validation that requires network or database (existence checks, uniqueness, business rules). UI screens may call model validators for instant user feedback, but must never implement their own validation logic — always delegate to the model function.
* **Never commit secrets.** Never put plaintext recovery phrases (BIP39 mnemonics), private keys, passwords, seeds, or API tokens anywhere in the repository — not in source, tests, fixtures, or documentation (including QA notes, design docs, and `docs/ai-design/**`). Refer to wallets and keys by name or public identifier only; import real secrets from the operator's secure store at runtime, never by pasting them into a file. A secret committed even once persists in git history after removal, so any exposed secret must be treated as compromised and rotated.

### DET Module Placement Policy

Code lives by responsibility, not convenience:

- **`model/`** — stateless data types and pure validation (format/length/charset). The single source of truth for validation. No `AppContext`, `Sdk`, DB, or `BackendTask`. All fee estimation goes in `model/fee_estimation.rs` — never inlined elsewhere.
- **`backend_task/`** — async business logic, one submodule per domain; the authoritative enforcement layer. `TaskError` and its typed variants live in `backend_task/error.rs`.
- **`database/`** — **Frozen legacy `data.db`, read-only in production.** Production opens an existing `data.db` with `SQLITE_OPEN_READ_ONLY` (`Database::open_legacy_read_only`, `src/app.rs`); the schema ladder in `database/initialization.rs` runs only on a fresh install that has no `data.db` yet, never on an existing one. Never add a table, column, or write path here — it becomes permanently unwritable after the install's first boot. All current durable state is a `DetKv` key (see `docs/kv-keys.md`) or a `SecretStore` entry (`wallet_backend/secret_seam.rs`); `database/` exists solely as a v0.9.3→v1.0 migration-read source and recovery artifact.
- **`context/`** — `AppContext` submodules (`*_db.rs`, lifecycle, settings, status).
- **`wallet_backend/`** — the wallet orchestration seam: adapters, views, backend-side live caches, signers, the secret chokepoint, the event bridge. All wallet secret bytes (HD seed, imported single key, identity private key) enter/leave the vault through ONE chokepoint, `wallet_backend/secret_seam.rs` (raw `SecretBytes`, no DET-side serialization). Per-secret at-rest encryption is implemented via `put_secret_protected`/`get_secret_protected` (Argon2id + XChaCha20-Poly1305, per-secret object-password envelope, AAD bound to `wallet_id ‖ label`); unprotected secrets use `put_secret`/`get_secret` (raw, keyless vault). Identity keys (imported/loaded, including masternode voting/owner/payout) enter unprotected (Tier-1 keyless) at load/creation time — the load flow has no password field — but can be sealed to Tier-2 per-identity afterward via `IdentityTask::ProtectIdentityKeys` (Key Info screen → "Add password protection…"; gated by vault-key scheme, not identity type). The keyless-vault residual is only no-password secrets and keys the user has not opted to protect. Design + migration: `docs/ai-design/2026-06-19-secret-storage-seam/`.
- **`ui/<domain>/`** — screens (`ScreenLike`). UI may *call* `model/` validators for instant feedback but never implements its own validation.
- **`ui/components/`** — reusable **Component-pattern widgets ONLY**: a `show()` plus a `ComponentResponse`, a display-only render widget, or component infrastructure. If it does not render egui, it is not a component.
- **`ui/state/`** — non-widget UI state: per-screen view-models and async fetch-state caches (e.g. `TrackedAssetLockCache`). Owned by screens, may return `BackendTask`, render nothing.
- **`src/mcp/tools/`** — MCP tool logic, one file per domain (e.g. `wallet.rs`, `shielded.rs`, `identity.rs`) with multiple tool structs per file; never in `src/bin/det_cli/`.
- **`src/localization.rs`** — localization logic. **`src/ui/theme.rs`** — theme/alignment helpers.

Discriminator for `ui/components/` vs `ui/state/`: *does it render egui (`show`/`ui`/a render fn)?* Yes → component. No → state.

### Error messages

User-facing error messages (shown in `MessageBanner` via `Display`) must follow these rules:

1. **Audience**: Write for the Everyday User persona (`docs/personas/everyday-user.md`). No jargon — no "consensus error", "nonce", "state transition", "SDK", "RPC", or error codes.
2. **Structure**: *What happened* + *what to do*. Every message must include a concrete action the user can take themselves: retry, wait, try a different approach. Never redirect to "contact support" — users must be able to self-resolve.
3. **Tone**: Calm, direct, brief. Not apologetic ("Sorry!"), not alarming ("Something went wrong!"), not vague ("An error occurred").
4. **Technical details**: Never in the message itself — no raw error strings, stack traces, SDK internals, or error codes. Attach via `BannerHandle::with_details(e)` — the `Debug` repr goes to the collapsible details panel and logs. Never refer users to "details" or "details panel" — these are not visible in basic mode. Exception: Base58 identifiers (see rule 6) are not technical details — they are user-meaningful handles.
5. **Reference implementation**: `sdk_error_user_message()` in `src/backend_task/error.rs` demonstrates the pattern for SDK errors. New `TaskError` variants should follow the same style.
6. **Base58 IDs are allowed in messages**: Contract IDs, identity IDs, document IDs, and similar Base58-encoded identifiers may appear in user-facing messages when they help the user identify which object is involved (e.g., *"This key conflicts with an existing key bound to contract `Abc123…`."*). They are not jargon — they are opaque-but-copyable handles the user can look up.
7. **Use dedicated `TaskError` variants**: Every error should get a dedicated `TaskError` variant with a `#[source]` field that preserves the error chain, enables structural matching, and keeps `Display` / `Debug` separation explicit. For `#[source]` fields in SDK-originated error variants, use `Box<SdkError>` — convert upstream types (e.g. `ProtocolError`) via `SdkError::Protocol(e)`. Use the concrete domain type directly for non-SDK errors (e.g. `rusqlite::Error`). Omit `#[source]` entirely when the upstream error carries no useful diagnostic information (e.g. a channel `SendError`). **Never store user-facing strings in error variants** — error variants must not contain `String` fields that hold messages for the user. The `#[error("...")]` attribute on the variant provides the user-facing message; `String` fields (regardless of name) break this separation. Instead, create a dedicated variant with typed `#[source]` fields, or a fieldless variant if no upstream error exists. When encountering existing variants that store stringified errors, replace them with properly typed variants as part of the change.

## Architecture Overview

**Dash Evo Tool** is a cross-platform GUI application (Rust + egui) for interacting with Dash Evolution. It enables DPNS username registration, contest voting, state transition viewing, wallet management, and identity operations across Mainnet/Testnet/Devnet.

## Documentation

- **docs/ai-design** should contain architecture, technical design and manual testing scenarios files, grouped in subdirectories prefixed with ISO-formatted date. Exception: `docs/user-stories.md` is a living document maintained at the top level — not date-grouped.
- **docs/personas** contains user personas (Everyday User, Power User, Platform Developer) that define the three target user archetypes and the progressive disclosure model for UI complexity. Consult these when making UX decisions about what to show/hide or how to structure wallet features.
- **docs/user-stories.md** catalogs user stories across feature areas, tagged by persona and marked `[Implemented]` or `[Gap]`. Reference when planning new features or verifying coverage.
- **docs/ux-design-patterns.md** is the UI/UX reference card — explains **when and how** to use design tokens, buttons, dialogs, forms, accessibility rules, and progressive disclosure. For exact values (sizes, colors, padding), refer to source files (`src/ui/theme.rs`, `src/ui/components/`). Consult when building or reviewing UI.
- **docs/gui-testing/** contains standing guidelines and a reusable scenario library for testing the real compiled app through a real display (not date-grouped — this is reusable practice, not a point-in-time design record). Read `docs/gui-testing/README.md` before driving the GUI directly for verification.
- end-user documentation is in a separate repo: https://github.com/dashpay/docs/tree/HEAD/docs/user/network/dash-evo-tool , published at https://docs.dash.org/en/stable/docs/user/network/dash-evo-tool/

### System Layers (top → bottom)

- **UI (`ui/`)** — Screens (`ui/<domain>/`), reusable components (`ui/components/`), and non-widget view state (`ui/state/`). No business logic. Returns `AppAction`s.
- **App (`app.rs`)** — `AppState`: owns all screens, polls task results each frame, dispatches to visible screen. Bridges UI and backend.
- **Backend Tasks (`backend_task/`)** — Async business logic and the authoritative validation/enforcement layer, one submodule per domain (identity, wallet, contract, etc.). Operates through `AppContext`, returns typed `Result<T, TaskError>` over a channel.
- **Wallet Backend (`wallet_backend/`)** — Wallet orchestration seam: adapters, views, backend-side live caches, signers, the secret chokepoint (`secret_seam.rs`), and the event bridge. A thin adapter over the upstream `platform-wallet` crate.
- **Context (`context/`)** — `AppContext`: shared state — network config, SDK client, database, wallets, settings cache, connection health (`ConnectionStatus` / `SpvManager`), split into submodules (`identity_db.rs`, `wallet_lifecycle.rs`, `settings_db.rs`, etc.). Glue between layers.
- **Model (`model/`)** — Pure data types and stateless validation (amounts, fees, settings, wallet/identity models). No side effects, no IO. All fee estimation lives in `model/fee_estimation.rs` — never inline fee math elsewhere.
- **Database (`database/`)** — Frozen legacy `data.db`. Read-only in production (migration-source reads and recovery only); no new tables, columns, or writes. Current persistence is `DetKv` over `det-app.sqlite` / `platform-wallet.sqlite` (`docs/kv-keys.md`) and `SecretStore`.
- **Platform Integration** — Chain sync, address derivation, asset-lock/identity handling, and the shielded coordinator come from the upstream **`platform-wallet`** crate (git dep, dashpay/platform); DET is a thin adapter over it via `wallet_backend/`. SPV health is surfaced through `SpvManager` → `ConnectionStatus`. (DET's bespoke `src/spv/` stack and the `core_zmq_listener` module were removed in the platform-wallet migration.)

### Layer Rules

**Model rules** (ideal target for new code):
- UI never calls SDK or database directly — always through `BackendTask`
- Backend tasks receive `AppContext`, do async work, return typed results
- Models are shared across all layers — pure data types, no IO
- Database modules are pure data access — no business logic or domain decisions
- Context is the glue: UI reads from it, backend tasks operate through it
- Data types shared between layers belong in `model/`, not in `ui/` or `database/`
- Wallet secret bytes enter/leave only through the `wallet_backend/secret_seam.rs` chokepoint

**In practice**, the codebase has established patterns that differ from the model:
- UI may **read** from DB through `AppContext` wrapper methods (e.g., `app_context.load_local_qualified_identities()`)
- UI may **write** to DB in `display_task_result()` for caching backend results
- `Wallet` (`model/wallet/`) is a large module that mixes data, address derivation, and SDK/RPC concerns — this is intentional
- Some data types live in `ui/` and are imported by `backend_task/`
- Database methods occasionally contain domain logic (e.g., contest state derivation)

These are accepted. Do not refactor existing code to match the model rules.

### Standard Flows

**User action → async work → result displayed:**
```
Screen::ui() → AppAction::BackendTask(task)
  → tokio::spawn → AppContext::run_backend_task()
  → sender.send(TaskResult::Success(result))
  → AppState::update() polls → Screen::display_task_result()
```

**UI needs fresh data on construction/refresh:**
```
Screen::new() or refresh() → app_context.read_wrapper_method()
  → returns cached or DB-read data (read-only, no writes)
```

**Backend task fetches + persists data:**
```
BackendTask variant → AppContext::run_*_task()
  → SDK/RPC call → persist results to DB → return typed result
  → Screen::display_task_result() updates in-memory state only
```

**Anti-patterns (do not add new instances):**
- `app_context.db.save_*()` / `db.delete_*()` from UI code
- `tokio::spawn` in UI bypassing the `BackendTask` system
- Business logic (signing, filtering, state derivation) in UI or database layers
- Accessing wallet secret bytes outside the `wallet_backend/secret_seam.rs` chokepoint

### MCP Server & CLI (`src/mcp/`, `src/bin/det_cli/`)

- **Dual transport**: HTTP (`mcp` feature, embedded in GUI via ArcSwap) and stdio (`cli` feature, lazy-init standalone). `headless` combines both.
- **CLI ≠ MCP**: `src/bin/det_cli/` is a separate client that talks to the MCP server — it must work over HTTP too, not just in-process. Never put tool logic in the CLI binary; tools live in `src/mcp/tools/` and the CLI discovers them dynamically via `tools/list`.
- **Tool architecture**: each tool is a struct implementing `ToolBase` (metadata) + `AsyncTool<DashMcpService>` (invocation). Adding a tool requires only the struct + registering in `tool_router()` — zero CLI changes.
- **Tool naming**: `{domain}_{object}_{action}` — e.g. `core_address_create`, `platform_withdrawals_get`, `tool_describe`. CLI converts underscores to hyphens.
- **Context provider**: `ContextHolder::Shared(ArcSwap)` for HTTP mode (follows GUI network switches), `ContextHolder::Standalone(ArcSwapOption)` for stdio (init on first tool call).
- **Network safety**: tools accept optional `network` param — request fails if it doesn't match the active network. Exempt: `network_info`, `tool_describe`.
- **SPV sync**: wallet tools call `resolve::ensure_spv_synced()` before operating — polls SPV status with 1s interval, 10min timeout.
- **Backend dispatch**: tools reuse the app's `BackendTask` system via `dispatch::dispatch_task()` — creates a throwaway channel, calls `app_context.run_backend_task()`.
- **Schema quirk**: `schemars` v1 derives bare `true` for `serde_json::Value` fields — some MCP clients reject this. Use `#[schemars(transform)]` to override.
- **Error type**: `McpToolError` enum (InvalidParam, WalletNotFound, SpvSyncFailed, TaskFailed, Internal) converts to `rmcp::ErrorData` via `From`.
- **Docs**: `docs/MCP.md` (server config, tool reference), `docs/CLI.md` (usage, examples), `docs/MCP_TOOL_DEVELOPMENT.md` (checklist for adding new MCP tools).

### Smoke-testing changes with det-cli

`det-cli` in standalone (stdio, lazy-init) mode is a fast, no-funds, no-GUI smoke test for the **MCP-tool layer + context wiring**. Run these after changes that touch MCP tools (`src/mcp/`), `AppContext` construction, or the wallet-backend boot path — they catch compile/API drift and context-init regressions before any live-network testing.

Build:

```bash
cargo build --bin det-cli --features cli
```

Then, with `MCP_API_KEY` unset (or empty — the default `.env` ships it empty, which means standalone), run the read-only checks. Point `DASH_EVO_DATA_DIR` at a throwaway dir to avoid touching real user data or contending with a running GUI / `det-cli serve` instance:

```bash
DET=$(mktemp -d) && cp .env.example "$DET/.env"
BIN=target/debug/det-cli   # or "$CARGO_TARGET_DIR/debug/det-cli" if that env var is set
run() { env -u MCP_API_KEY DASH_EVO_DATA_DIR="$DET" RUST_LOG=off "$BIN" "$@"; }

run network-info                       # active network as JSON — no SPV sync (network-exempt)
run tools                              # discovers all tools via tools/list
run tool-describe name=network_info    # full schema for one tool (meta tool, network-exempt)
run core-wallets-list                  # exercises in-process MCP -> tool -> AppContext -> DB; returns {"wallets":[]}
```

What each verifies:

- **`network-info`** — binary starts, lazy-inits `AppContext` (creates `.env`/DB/secret store), reports the active network. No SPV gate, so it's a pure context-wiring check.
- **`tools`** — the in-process MCP server is up and the dynamic `tools/list` discovery path works (catches a tool that fails to register in `tool_router()`).
- **`tool-describe name=...`** — the meta tool returns a tool's JSON schema; confirms tool metadata serializes cleanly.
- **`core-wallets-list`** — drives the full dispatch chain (MCP service → tool invoke → `AppContext` → SQLite) without funds; skips the SPV gate.

`--help`, `<cmd> --help`, and `completion <shell>` work from the on-disk tool cache without any context init.

**Not smoke tests** (need a synced chain / live DAPI — they wait on the SPV gate, up to a 10-min timeout): all fund-moving and balance/withdrawal tools — `core-balances-get`, `core-funds-send`, `platform-addresses-list`, `platform-withdrawals-get`, every `identity-*` and `shielded-*` tool. Don't force these in a no-network smoke run.

### Key Dependencies

- `dash-sdk` - Dash blockchain SDK (git dep from dashpay/platform)
- `platform-wallet` / `platform-wallet-storage` - Upstream wallet backend (git dep from dashpay/platform): SPV chain sync, address derivation, asset-lock/identity handling, shielded coordinator
- `egui/eframe 0.35` - Immediate mode GUI framework
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

The UI and async backend communicate through the action/channel pattern described in Standard Flows above.

**Backend task enums**: `BackendTask` has variants like `IdentityTask(IdentityTask)`, `WalletTask(WalletTask)`, `TokenTask(Box<TokenTask>)`, etc. Each sub-enum has its own variants and corresponding `run_*_task()` method. Results are `BackendTaskSuccessResult` with 50+ typed variants.

**Error handling**: Backend tasks return `Result<T, TaskError>` (`src/backend_task/error.rs`). `TaskError` is a typed error envelope — `Display` produces user-friendly text for `MessageBanner`, `Debug` provides technical details for logs. Domain errors (`DashPayError`, `SpvError`, etc.) are wired as `#[from]` variants for automatic conversion via `?`. When adding new backend error types, add a dedicated `TaskError` variant rather than converting to `String`.

## Screen Pattern

All screens implement the `ScreenLike` trait:
- `ui(&mut self, ui: &mut egui::Ui) -> AppAction` - Render UI, return actions
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

### ConnectionStatus (single source of truth for connection health)

`ConnectionStatus` (`src/context/connection_status.rs`) is the **single source of truth** for all high-level connection health state — SPV and DAPI. For connection health (status, peer counts, errors, overall state), always read from `ConnectionStatus`, not directly from `SpvManager` or other subsystems.

SPV status is **push-based**: `SpvManager` event handlers write directly to `ConnectionStatus` atomics (status, peer count, errors) as events arrive. The UI frame loop calls `refresh_state()` to recompute `overall_state` from these atomics, but does not poll SPV for health. This means `ConnectionStatus` is up-to-date in both GUI and headless/test contexts. Detailed SPV sync progress (heights, phase summaries used by tooltips) may still be read directly from `SpvManager.status()` until that progress reporting is migrated into `ConnectionStatus`.

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

### UI Components Catalog

See `src/ui/components/README.md` for a complete reference of available components, their APIs, and usage patterns. **Always consult this file before creating new UI elements** to avoid duplicating existing components.

## Message Display

User-facing messages (errors, warnings, success, infos) use `MessageBanner` (`src/ui/components/message_banner.rs`). Global banners are rendered centrally by `island_central_panel()` — `AppState::update()` sets them automatically for backend task results. When using `MessageBanner::set_global()`, no guard is needed — it is idempotent and automatically logs at the appropriate level (error/warn/debug). Screens only override `display_message()` for side-effects. See the component's doc comments and `docs/ai-design/2026-02-17-unified-messages/` for details.

**BannerHandle lifecycle**: Screens that run backend tasks typically store a `refresh_banner: Option<BannerHandle>` field. On task dispatch, set it via `MessageBanner::set_global()` with an info/progress message. In `display_message()` (called as a side-effect by AppState), dismiss the progress banner via `self.refresh_banner.take_and_clear()` (from `OptionBannerExt`). Simply setting the field to `None` would leak the banner — `take_and_clear()` removes it from the egui context. AppState handles displaying the actual result banner.

**Logging**: MessageBanner logs all displayed messages (with details) automatically. Additional logging is unnecessary.

**Error banners**: Never expose raw backend/database errors to users. Use a user-friendly message in the banner and attach technical details via `BannerHandle::with_details()`. When the error implements `Display` and its text is user-appropriate, pass it directly to `set_global`; otherwise write a descriptive, actionable message:
```rust
MessageBanner::set_global(ctx, "Failed to load token balances", MessageType::Error)
    .with_details(e);
```
Consider whether a repeated or reused message belongs in a dedicated `TaskError` variant instead of being written as a string literal at the callsite. A variant centralises the wording, keeps `Display` / `Debug` separation clean, and makes the error testable. This is a soft guideline — a one-off screen-level message that wraps no upstream error is fine as a literal; errors that originate in backend tasks should generally live in `TaskError`.

## Database

`AppContext.db` is the **legacy** `data.db` — a frozen migration-read source and recovery artifact, not a general persistence layer. Production opens it with `SQLITE_OPEN_READ_ONLY` whenever the file already exists, and only initializes (runs `database/initialization.rs`'s schema ladder) on a fresh install that has none yet; consequently that ladder never executes against an existing production install, and any write attempted on one fails at the SQLite layer. **Never add a table, column, or write path to `database/`.**

All current durable state lives in `DetKv` (wraps the upstream `platform_wallet_storage::KvStore`; two backing SQLite files, `det-app.sqlite` and `spv/<network>/platform-wallet.sqlite` — see `docs/kv-keys.md` for the full key registry) or `SecretStore` (`wallet_backend/secret_seam.rs`). New persistent state is a new `DetKv` key registered in `docs/kv-keys.md`, never a new SQL table. Backend task errors use `TaskError` (`src/backend_task/error.rs`) — see App Task System section above.

## Platform Targets

Linux (x86_64/aarch64), Windows (x86_64), macOS (x86_64/aarch64 with code signing)

Requires protoc v25.2+ for protocol buffer compilation.
