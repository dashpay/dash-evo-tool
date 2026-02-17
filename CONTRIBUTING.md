# Contributing to Dash Evo Tool

Contributions are welcome! This guide covers how to set up a development environment, build the project, run tests, and submit changes.

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
sudo apt install -y build-essential libssl-dev pkg-config unzip
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
