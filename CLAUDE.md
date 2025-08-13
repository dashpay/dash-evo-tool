# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Dash Evo Tool is a cross-platform GUI application built with Rust and egui for interacting with Dash Evolution. It supports identity management, DPNS username registration and voting, token operations, and state transition visualization across multiple networks (Mainnet, Testnet, Devnet, Regtest).

## Build and Development Commands

```bash
# Development build and run
cargo run

# Production build
cargo build --release

# Run linting (used in CI)
cargo clippy --all-features --all-targets -- -D warnings

# Build for specific target (cross-compilation)
cross build --target x86_64-pc-windows-gnu --release
```

## Architecture Overview

### Core Application Structure
- **Entry Point**: `src/main.rs` - Sets up Tokio runtime (40 worker threads), loads fonts, and launches egui app
- **App State Manager**: `src/app.rs` - Central state with screen management, network switching, and backend task coordination  
- **Context System**: `src/context.rs` and `src/context_provider.rs` - Network-specific app contexts with SDK integration
- **Configuration**: `src/config.rs` - Environment and network configuration management

### Module Organization
- `backend_task/` - Async task handlers organized by domain (identity, contracts, tokens, core, contested_names)
- `ui/` - Screen components organized by feature (identities, tokens, tools, wallets, contracts_documents, dpns)
- `database/` - SQLite persistence layer with tables for each domain
- `model/` - Data structures, including qualified identities with encrypted key storage
- `components/` - Shared components including ZMQ core listeners
- `utils/` - Parsers and helper functions

### Key Design Patterns
- **Screen-based Navigation**: Stack-based screen management with `ScreenType` enum
- **Async Backend Tasks**: Communication via crossbeam channels with result handling
- **Network Isolation**: Separate app contexts per network with independent databases
- **Real-time Updates**: ZMQ listeners for core blockchain events on network-specific ports
- **Custom UI components**: we build a library of reusable widgets in `ui/components` whenever we need similar
  widget displayed in more than 2 places

### Critical Dependencies
- **dash-sdk**: Core Dash Platform SDK (git dependency, specific revision)
- **egui/eframe**: GUI framework with persistence features
- **tokio**: Full-featured async runtime
- **rusqlite**: SQLite with bundled libsqlite3
- **zmq/zeromq**: Platform-specific ZMQ implementations (Unix vs Windows)

## Development Environment Setup

### Prerequisites
1. **Rust**: Version 1.88+ (enforced by rust-toolchain.toml)
2. **System Dependencies** (Ubuntu): `build-essential libssl-dev pkg-config unzip`
3. **Protocol Buffers**: protoc v25.2+ required for dash-sdk
4. **Dash Core Wallet**: Must be synced for full functionality

### Application Data Locations
- **macOS**: `~/Library/Application Support/Dash-Evo-Tool/`
- **Windows**: `C:\Users\<User>\AppData\Roaming\Dash-Evo-Tool\config`
- **Linux**: `/home/<user>/.config/dash-evo-tool/`

Configuration loaded from `.env` file in application directory (created from `.env.example` on first run).

## Key Implementation Details

### Multi-Network Support
- Each network maintains separate SQLite databases
- ZMQ listeners on different ports per network (Core integration)
- Network switching preserves state and loaded identities
- Core wallet auto-startup with network-specific configurations

### Security Architecture
- Identity private keys encrypted with Argon2 + AES-256-GCM
- Password-protected storage with zxcvbn strength validation
- Secure memory handling with zeroize for sensitive data
- CPU compatibility checking on x86 platforms

### Performance Considerations
- 40-thread Tokio runtime for heavy blockchain operations
- Font loading optimized for international scripts (CJK, Arabic, Hebrew, etc.)
- SQLite connection pooling and prepared statements
- Efficient state updates via targeted screen refreshes

### Cross-Platform Specifics
- Different ZMQ implementations (zmq vs zeromq for Windows)
- Platform-specific file dialogs and CPU detection
- Cross-compilation support via Cross.toml configuration
- Font rendering optimized per platform

## Testing and CI

- **Clippy**: Runs on push to main/v*-dev branches and PRs with strict warning enforcement
- **Release**: Multi-platform builds (Linux, macOS, Windows) with attestation
- No dedicated test suite currently - integration testing via manual workflows

## Common Development Patterns

When working with this codebase:
- Follow the modular organization: backend tasks in `backend_task/`, UI in `ui/`
- Use the context system for SDK operations rather than direct SDK calls
- Implement async operations as backend tasks with channel communication
- Screen transitions should update the screen stack in `app.rs`
- Database operations should follow the established schema patterns in `database/`
- Error handling uses `thiserror` for structured error types