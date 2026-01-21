# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

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

## Architecture Overview

**Dash Evo Tool** is a cross-platform GUI application (Rust + egui) for interacting with Dash Evolution. It enables DPNS username registration, contest voting, state transition viewing, wallet management, and identity operations across Mainnet/Testnet/Devnet.

### Core Module Structure

- **app.rs** - Main application state, task result handling, screen management
- **ui/** - Screens (network_chooser, dpns, identities, wallets, contracts_documents, tokens, dashpay, tools) and reusable components
- **backend_task/** - Async business logic (contract, document, platform_info, identity, wallet operations)
- **model/** - Data types (amounts, fees, settings, wallet/identity models)
- **database/** - SQLite persistence (rusqlite) for wallets, identities, settings, proof logs
- **context.rs** - Application context (network config, SDK client, database connection)
- **spv/** - Simplified Payment Verification for light wallet support
- **components/core_zmq_listener** - Real-time Dash Core event listening

### Key Dependencies

- `dash-sdk` - Dash blockchain SDK (platform protocol, core interactions)
- `egui/eframe` - Immediate mode GUI framework
- `tokio` - Async runtime (12 worker threads)
- `rusqlite` - SQLite with bundled library

### Configuration

Environment config via `.env` in app directory:
- macOS: `~/Library/Application Support/Dash-Evo-Tool/.env`
- Linux: `~/.config/dash-evo-tool/.env`
- Windows: `C:\Users\<User>\AppData\Roaming\Dash-Evo-Tool\config\.env`

See `.env.example` for network configuration options.

## UI Component Pattern

Components follow a lazy initialization pattern (see `doc/COMPONENT_DESIGN_PATTERN.md`):

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
- Response struct with `ComponentResponse` trait
- Self-contained validation and error handling
- Support both light and dark mode via `ComponentStyles`

**Anti-patterns:** public mutable fields, eager initialization, not clearing invalid data

## Platform Targets

Linux (x86_64/aarch64), Windows (x86_64), macOS (x86_64/aarch64 with code signing)

Requires protoc v25.2+ for protocol buffer compilation.
