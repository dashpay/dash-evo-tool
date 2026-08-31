# Contributing to Dash Evo Tool

Contributions are welcome! This guide covers how to set up a development environment, build the project, run tests, and submit changes.

> **Note:** The instructions below are written for Ubuntu x86_64. If you are building on another platform (e.g. Linux aarch64 or macOS) the same steps apply, but you may need to adjust package manager commands and download the appropriate `protoc` binary for your architecture. Windows needs a few extra steps — see [Building natively on Windows](#building-natively-on-windows).

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

### Building natively on Windows

Windows binaries are built for the `x86_64-pc-windows-gnu` target — that is what the release workflow cross-compiles from Linux with the mingw toolchain, and a native Windows build uses the same target so that developers exercise the configuration that ships. The MSVC target is not supported: `rs-x11-hash` compiles its C sources with clang and includes the POSIX header `<unistd.h>`, which the MSVC toolchain does not provide.

#### 1. Install the toolchain

`pacman` is [MSYS2](https://www.msys2.org/)'s package manager and is not on `PATH` by default — run it from the MSYS2 shell (`C:\msys64\msys2_shell.cmd`) or by full path:

```shell
C:\msys64\usr\bin\pacman.exe -S --needed mingw-w64-x86_64-gcc mingw-w64-x86_64-cmake
rustup target add x86_64-pc-windows-gnu
```

Add `C:\msys64\mingw64\bin` to `PATH` so that `gcc`, `ar`, `windres` and `dlltool` are found. Building also needs `clang` (for `bindgen`) and `protoc`, which are covered by the prerequisites above.

#### 2. Create the cross-compiler aliases

`.cargo/config.toml` names the Debian cross-compilers the release workflow uses, because the shipped binaries are cross-compiled from Linux:

```toml
linker = "x86_64-w64-mingw32-gcc-posix"
ar = "x86_64-w64-mingw32-ar"
```

MSYS2 uses different names for the same tools. The `-posix` suffix is Debian's `update-alternatives` naming for choosing between the posix and win32 threading models; MSYS2 does not have that fork at all, because its `gcc` already uses posix threads. It also ships no prefixed `ar`. No package provides these names, so create them as symlinks:

```shell
$bin = "C:\msys64\mingw64\bin"; New-Item -ItemType SymbolicLink -Path "$bin\x86_64-w64-mingw32-gcc-posix.exe" -Target "$bin\gcc.exe"; New-Item -ItemType SymbolicLink -Path "$bin\x86_64-w64-mingw32-g++-posix.exe" -Target "$bin\g++.exe"; New-Item -ItemType SymbolicLink -Path "$bin\x86_64-w64-mingw32-ar.exe" -Target "$bin\ar.exe"
```

Creating symlinks requires an elevated PowerShell, or Developer Mode enabled under *Settings → System → For developers*.

Use symlinks rather than copies or hard links. `pacman` upgrades a package by extracting and renaming over the old file, so a copy or hard link would keep pointing at the previous binary and silently build with a stale compiler after the next `pacman -Syu`. A symlink follows the new file.

Without these aliases the build fails in `cc-rs` with:

```
error occurred in cc-rs: failed to find tool "x86_64-w64-mingw32-gcc-posix": program not found
```

The alternative is to override the toolchain variables per session instead of creating the symlinks: `CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER`, `CC_x86_64_pc_windows_gnu`, `CXX_x86_64_pc_windows_gnu` and `AR_x86_64_pc_windows_gnu`, pointed at `gcc`, `g++` and `ar`. Cargo's `[env]` entries do not force, so a real environment variable wins over the config file.

#### 3. Build

```shell
cargo build --target x86_64-pc-windows-gnu
```

Because `x86_64-pc-windows-gnu` is not the host's default target, pass `--target` to every cargo command, or set `CARGO_BUILD_TARGET` once for the session. In VS Code, `.vscode/settings.json` can set both that and rust-analyzer's target:

```json
{
  "rust-analyzer.cargo.target": "x86_64-pc-windows-gnu",
  "terminal.integrated.env.windows": {
    "CARGO_BUILD_TARGET": "x86_64-pc-windows-gnu"
  }
}
```

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
