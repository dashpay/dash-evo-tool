# Contributing to Dash Evo Tool

Contributions are welcome! This guide covers how to set up a development environment, build the project, run tests, and submit changes.

> **Note:** The instructions below are written for Ubuntu x86_64. If you are building on another platform (e.g. Linux aarch64 or macOS) the same steps apply, but you may need to adjust package manager commands and download the appropriate `protoc` binary for your architecture. Windows needs one additional step — see [Building natively on Windows (MSVC)](#building-natively-on-windows-msvc).

## Prerequisites

### Rust

Install Rust using [rustup](https://rustup.rs/):

```shell
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Update Rust to the latest version:

```shell
rustup update
```

### System dependencies

On Ubuntu, install build tools, SSL libraries, and related packages:

```shell
sudo apt install -y build-essential libssl-dev pkg-config clang cmake libsqlite3-dev unzip
```

On other Unix-like systems, use the equivalent package management commands.

### Protocol Buffers Compiler

Install protoc (v25.2+). Download the appropriate binary for your system, unzip, and install:

```shell
wget https://github.com/protocolbuffers/protobuf/releases/download/v26.1/protoc-26.1-linux-x86_64.zip
sudo unzip protoc-*-linux-x86_64.zip -d /usr/local
```

### Dash Core Wallet

- Download and install from [dash.org/wallets](https://www.dash.org/wallets/).
- Ensure the wallet is fully synced with the network you intend to use (Mainnet or Testnet).

## Building

Clone the repository and build:

```shell
git clone https://github.com/dashpay/dash-evo-tool.git
cd dash-evo-tool
cargo build --release
```

Run the application:

```shell
cargo run
```

### Building natively on Windows (MSVC)

Release binaries for Windows are cross-compiled from Linux with the mingw toolchain (target `x86_64-pc-windows-gnu`), so a native MSVC build needs one extra step.

The `rs-x11-hash` dependency compiles its C sources with clang and includes the POSIX header `<unistd.h>`, which the MSVC toolchain does not ship. This repository provides an empty stand-in at `build-support/windows-msvc/`. Put that directory on the compiler's include path before building:

```shell
$env:CFLAGS_x86_64_pc_windows_msvc = "-I$PWD/build-support/windows-msvc"
```

In VS Code, set the same variable in `.vscode/settings.json` so that both rust-analyzer and the integrated terminal pick it up:

```json
{
  "rust-analyzer.cargo.extraEnv": {
    "CFLAGS_x86_64_pc_windows_msvc": "-I${workspaceFolder}/build-support/windows-msvc"
  },
  "terminal.integrated.env.windows": {
    "CFLAGS_x86_64_pc_windows_msvc": "-I${workspaceFolder}/build-support/windows-msvc"
  }
}
```

Note that the checkout path must not contain spaces, because the compiler splits this variable on whitespace.

No mingw toolchain is required for a native MSVC build: the Windows icon resource is embedded with the Windows SDK's `rc.exe`, which `winres` locates on its own. Cross-compiling to `x86_64-pc-windows-gnu` still needs `windres` and `ar`, or the `WINDRES` and `AR_x86_64_pc_windows_gnu` variables pointing at them.

## Feature flags

The default `cargo build` produces only the `dash-evo-tool` GUI binary. Optional features enable additional capabilities:

| Feature | Binary | What it adds |
|---|---|---|
| _(none)_ | `dash-evo-tool` | GUI application (default) |
| `mcp` | `dash-evo-tool` | Embeds an MCP HTTP server in the GUI app. Activated at runtime by setting `MCP_API_KEY`. See [docs/MCP.md](docs/MCP.md). |
| `cli` | `det-cli` | Standalone CLI binary. Includes an in-process MCP service (no server needed), HTTP client mode, `det-cli serve` stdio server, tool caching, and shell completion. See [docs/CLI.md](docs/CLI.md). |
| `headless` | `det-cli` | Combines `cli` + `mcp` for headless HTTP server mode via `det-cli headless`. No GUI required; `MCP_API_KEY` must be set. See [docs/MCP.md](docs/MCP.md). |
| `testing` | — | Test-only utilities (not for production builds) |

`mcp` and `cli` are independent of each other. `headless` depends on both `cli` and `mcp` (it enables both automatically).

### Adding MCP tools

To expose a `BackendTask` as a new MCP/CLI tool, follow the step-by-step checklist in [docs/MCP_TOOL_DEVELOPMENT.md](docs/MCP_TOOL_DEVELOPMENT.md). It covers architecture rules, the standard invocation pattern, registration, and common pitfalls.

## Code quality

Before submitting changes, run the formatter and linter:

```shell
cargo +nightly fmt --all
cargo clippy --all-features --all-targets -- -D warnings
```

## Testing

```shell
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

## Local network (development only)

For development and testing you can connect Dash Evo Tool to a dashmate-managed local network running in regtest mode. See the [Local Network Guide](docs/local-network.md) for full setup instructions.

## Submitting changes

1. **Fork** the repository on GitHub.

2. **Create a branch** from `v1.0-dev` (the active development branch):

   ```shell
   git checkout -b feature/YourFeatureName v1.0-dev
   ```

3. **Commit** your changes with descriptive messages following [Conventional Commits](https://www.conventionalcommits.org/):

   ```shell
   git commit -m "feat: add YourFeatureName"
   ```

4. **Push** to your fork:

   ```shell
   git push origin feature/YourFeatureName
   ```

5. **Open a pull request** against `v1.0-dev` and describe your changes.

Please ensure your code passes `cargo clippy` and `cargo +nightly fmt` before submitting.
