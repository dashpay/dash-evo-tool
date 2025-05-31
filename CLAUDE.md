# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Dash Evo Tool (formerly PMT - Platform Mass Transactions) is a cross-platform GUI application for interacting with Dash Evolution/Platform. Built with Rust and egui, it provides identity management, DPNS (username) registration, wallet integration, and blockchain visualization tools.

## Common Development Commands

### Build and Run
```bash
# Run in development mode
cargo run

# Build release version
cargo build --release

# Run tests (minimal test coverage exists)
cargo test
```

### Code Quality
```bash
# Format code
cargo fmt

# Run clippy linter
cargo clippy -- -D warnings

# Check for compilation errors
cargo check
```

## Architecture Overview

### Core Components

1. **Task System** (`src/backend_task/`)
   - Async blockchain operations using Tokio channels
   - Tasks can run sequentially or concurrently
   - Communication between UI and backend via `AppContext.task_result_sender`

2. **Screen System** (`src/ui/`)
   - Hierarchical screen management with `ScreenLike` trait
   - Root screens contain sub-screens
   - Stack-based navigation (push/pop)

3. **State Management** (`src/context.rs`, `src/app.rs`)
   - `AppContext`: Thread-safe shared state using `Arc<Mutex<T>>`
   - Contains SDK, database, wallets, identities, network config
   - Real-time updates via ZeroMQ listener

4. **Database Layer** (`src/database/`)
   - SQLite for persistent storage
   - Tables: identities, wallets, contested_names, scheduled_votes, etc.
   - Migrations handled in `initialization.rs`

### Key Patterns

1. **SDK Integration** (`src/sdk_wrapper.rs`)
   - Custom provider for Dash SDK
   - Handles Core RPC authentication
   - Network-specific configurations

2. **Identity Encryption** (`src/model/qualified_identity/`)
   - Private keys encrypted with Argon2 + AES-GCM
   - Password-based encryption for security

3. **UI Components** (`src/ui/components/`)
   - Reusable panels: left panel, wallet panel, top panel
   - Consistent styling and layout

## Development Workflow

### Adding New Features

1. **Backend Task**: Create in `src/backend_task/`
2. **UI Screen**: Add to appropriate `src/ui/` subdirectory
3. **Database Schema**: Update in `src/database/`
4. **Wire Together**: Add to screen navigation and task handling

### Network Configuration

- Mainnet/Testnet configs in `dash_core_configs/`
- Environment variables loaded from app directory's `.env`
- ZMQ connection for real-time updates

### Platform-Specific Notes

- Windows uses `zeromq` crate instead of `zmq`
- macOS app icons in `mac_os/AppIcons/`
- Cross-compilation configured in `Cross.toml`
- CPU feature detection for AVX compatibility

## Important Conventions

- Error handling: Use `Result<T, String>` for UI-facing errors
- Thread safety: Always use `Arc<Mutex<T>>` for shared state
- Task execution: Check `app_context.executables` before running tasks
- Screen updates: Call `mark_for_update()` when state changes