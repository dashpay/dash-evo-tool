# Dash Evo Tool

**Dash Evo Tool** is a graphical user interface for easily interacting with Dash Evolution. The current version enables the following actions:

- Registering a DPNS username
- Viewing active DPNS username contests
- Voting on active DPNS username contests
- Decoding and viewing state transitions

The tool supports both Mainnet and Testnet networks. Check out the [documentation](https://docs.dash.org/en/stable/docs/user/network/dash-evo-tool/index.html) for additional information.

## Table of Contents

- [Prerequisites](#prerequisites)
  - [Rust Installation](#rust-installation)
  - [Dependencies](#dependencies)
  - [Dash Core Wallet Setup](#dash-core-wallet-setup)
- [Installation](#installation)
- [Getting Started](#getting-started)
  - [Start the App](#start-the-app)
  - [Application directory](#application-directory)
  - [Connect to a Network](#connect-to-a-network)
- [Usage](#usage)
  - [Register a DPNS Username](#register-a-dpns-username)
  - [Vote on an Active DPNS Contest](#vote-on-an-active-dpns-contest)
  - [View Decoded State Transition](#view-decoded-state-transition)
- [Switching Networks](#switching-networks)
- [Contributing](#contributing)
- [License](#license)
- [Support](#support)
- [Security Note](#security-note)

## Prerequisites

Before you begin, ensure you have met the following requirements:

### Rust Installation

- **Rust**: Install Rust using [rustup](https://rustup.rs/):

  ``` shell
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```

- Update Rust to the latest version:

  ``` shell
  rustup update
  ```

### Dependencies

- Install build-essential tools, SSL development libraries, and other required dependencies. On
Ubuntu, use:

   ``` shell
   sudo apt install -y build-essential libssl-dev pkg-config unzip
   ```

   On other Unix-like systems, use the equivalent package management commands.

- Install Protocol Buffers Compiler (protoc). Download the appropriate protoc binary for your
system, unzip, and install:

   ``` shell
   wget https://github.com/protocolbuffers/protobuf/releases/download/v26.1/protoc-26.1-linux-x86_64.zip
   sudo unzip protoc-*-linux-x86_64.zip -d /usr/local
   ```

### Dash Core Wallet Setup

- **Dash Core Wallet**: Download and install from [dash.org/wallets](https://www.dash.org/wallets/).

- **Synchronize Wallet**: Ensure the wallet is fully synced with the network you intend to use (Mainnet or Testnet).

## Installation

To install Dash Evo Tool:

1. **Clone the repository**:

   ``` shell
   git clone https://github.com/dashpay/dash-evo-tool.git
   ```

2. **Navigate to the project directory**:

   ``` shell
   cd dash-evo-tool
   ```

3. **Build the project**:

   ``` shell
   cargo build --release
   ```

## Getting Started

### Start the App

Run the application using:

``` shell
cargo run
```

### Application directory

When the application runs for the first time, it creates an application directory and stores an `.env` file in it (based on [`.env.example`](.env.example)). It also stores application data in the directory. If you need to update the `.env` file, locate it in the application directory for your Operating System:

| Operating System | Application Directory Path |
| - | - |
| macOS | `~/Library/Application Support/Dash-Evo-Tool/` |
| Windows | `C:\Users\<User>\AppData\Roaming\Dash-Evo-Tool\config` |
| Linux | `/home/<user>/.config/dash-evo-tool/` |

### Connect to a Network

1. **Open Network Chooser**: In the app, navigate to the **Network Chooser** screen.

2. **Select Network**: Choose **Mainnet** or **Testnet**.

3. **Start Connection**: Click **Start** next to the selected network.

   - If Dash Core Wallet is running and synced, the status will show **Online**.
   - If not, the app attempts to start Dash Core Wallet automatically.

## Usage

### Register a DPNS Username

1. **Load User Identity**:

   - Go to the **Identity** screen.
   - Click **Load Identity** at the top right.
   - Fill in your user identity details:
     - **Identity ID** (Hex or Base58)
     - **Identity Type** should be "User"
     - **Alias** (optional alias for use within Dash Evo Tool)
     - **Private Keys** (only the authentication key that will be used to register the name is required for registering a username. Other keys can be added later.)
   - Click **Submit**.

2. **Register Username**:

   - Navigate to the **DPNS** screen.
   - Click **Register Username** at the top right.
   - Select the Identity you'd like to register the username for.
   - Enter your desired username.
   - Click **Register Name**

### Vote on an Active DPNS Contest

1. **Load HPMN Identity**:

   - Go to the **Identity** screen.
   - Fill in your Masternode or HPMN (High Performance Masternode) identity details:
     - For **Testnet**, you can click "Fill Random HPMN" or "Fill Random Masternode".
     - For **Mainnet**, ensure you have valid Masternode or HPMN credentials.
   - Click **Submit**.

2. **Vote on Contest**:

   - Navigate to the **DPNS** screen.
   - If no contests appear, click **Refresh**. If still no contests appear, there are probably no active contests.
   - Locate the active contest you wish to vote on.
   - Click the button for the option you'd like to vote for within the contest's row (Lock, Abstain, or an Identity ID).
   - Choose the Masternode or HPMN identity to vote with or select **All** to vote with all loaded Masternodes and HPMNs.
   - Confirm your vote.

### View Decoded State Transition

1. **Open State Transition Viewer**:

   - Navigate to the **State Transition Viewer** screen.

2. **Decode State Transition**:

   - Paste a hex or base58 encoded state transition into the input box at the top.
   - View the decoded details displayed below.

## Switching Networks

1. **Open Network Chooser**:

   - Go to the **Network Chooser** screen.

2. **Select Network**:

   - Choose the network you'd like to interact with (**Mainnet** or **Testnet**).

3. **Check Wallet Status**:

   - If Dash Core Wallet is already running on that network, the status column will show **Online**.
   - If not, click **Start** to launch Dash Core Wallet on the selected network.

4. **Wait for Sync**:

   - Ensure Dash Core Wallet is fully synchronized before proceeding.

## Contributing

Contributions are welcome!

- **Fork the Repository**: Click the **Fork** button on the GitHub repository page.

- **Create a Branch**:

  ``` shell
  git checkout -b feature/YourFeatureName
  ```

- **Commit Changes**: Make your changes and commit them with descriptive messages.

  ``` shell
  git commit -m "Add feature: YourFeatureName"
  ```

- **Push to Branch**:

  ``` shell
  git push origin feature/YourFeatureName
  ```

- **Submit Pull Request**: Open a pull request on GitHub and describe your changes.

- **Follow Guidelines**: Please ensure your code adheres to the project's coding standards and passes all tests.

## License

This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.

## Support

For assistance:

- **Issues**: Open an issue on [GitHub Issues](https://github.com/dashpay/dash-evo-tool/issues).
- **Community**: Join the Dash community forums or Discord server for discussions.

## Security Note

Keep your private keys and identity information secure. Do not share them with untrusted parties or applications.
