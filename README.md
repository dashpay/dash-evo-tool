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
extern crate serde;
extern crate serde_json;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct BitcoinBlock {
    hash: String,
    height: u64,
    chain: String,
    total: u64,
    fees: u64,
    size: u64,
    vsize: u64,
    ver: u64,
    time: String,
    received_time: String,
    relayed_by: String,
    bits: u64,
    nonce: u64,
    n_tx: u64,
    prev_block: String,
    mrkl_root: String,
    txids: Vec<String>,
    depth: u64,
    prev_block_url: String,
    tx_url: String,
    next_txids: String,
}

fn main() {
    let data = r#"
    {
        "hash": "00000000000000000000cfbeaeb0b5f18dffc118597310d3f87096a2a204b512",
        "height": 877599,
        "chain": "BTC.main",
        "total": 53802306540,
        "fees": 3790551,
        "size": 1443191,
        "vsize": 996671,
        "ver": 537223168,
        "time": "2025-01-03T06:06:44Z",
        "received_time": "2025-01-03T06:07:57.04Z",
        "relayed_by": "104.223.60.58:18336",
        "bits": 386043996,
        "nonce": 779452439,
        "n_tx": 1469,
        "prev_block": "00000000000000000001dc4780b5419dd828c2f2fecfc18a2a8c7387ca960206",
        "mrkl_root": "d0d926d9443a788c713c010a70002ef5ed5e5addde647797bf9056823aeba579",
        "txids": [
            "de6b6cfb392130af7d58f03fba7ff39c011b63b243582adc0c481d44ade94278",
            "384b3bb0a3d92b6e0af22f1fb8c498f323ce48b5971462c1dc2b70e905b6c5b3",
            "9e9771296f8a05fd44e2e1af9884e0b7eb43123ef1110c553e165dafe8f81a04",
            "a36372f70edfee188bd008666d5c149933509597c58c16fbc2fdbc3ecec6573e",
            "6df58358a9d09960747b1a288bda712e69cf273f3ad4c97fc9ae7ddfa07741be",
            "5d2b759752687fe9d852e28f5bd8ce75cd20ce101a784ac893a6c8635153006f",
            "3973d77b5d8d25124cf79ad393210bddbabd7b677f3f3843ac6fdf6055fd1007",
            "33100c82dc10aea34f40193edff4528d34fbe3fbe237d4b007875f2e94359c3b",
            "5bf620573f55da2e63f5f9a54c377156be58f62deaacc591b773bad43772ea45",
            "ef34802b1cf8c6f68974c6e627e9fe683eca8c510b539ffa816b08f510c3c948",
            "af4a2ebbbadc1421a10550086a9fc8a5241ead6156cee5005de58aff81c69c7f",
            "1c08cb2510c76b96d915e70144c5b053b0edd53f5c527157383beb99e9c6bad2",
            "cceae37a7eb5d1c3f462102e5393d85473266713217be55f8407c2824f56f8eb",
            "aa9ba02e13533608d8cb2bc827fccc78a96fab697e19ab04b3dd5853f26c08ad",
            "3819eef3eefce6da6ab7ba74dba30e98a213a9d6a83f1062993c41e00a57f095",
            "faabe0ac04060d05605d6184f0baecc950ae67eba565cd2f4921966e27957dc5",
            "8c5131c9d252b27e14f6c9e423a3b2eb88f3b4df6d16ba21c17db8b5975998d7",
            "5e57b5a2b2ecf3d9d4213cbf4f8a8a7410e729067852492c3cff3aacd641c9aa",
            "90e39e4eff3cf65f32d95cca9b8c63f7eb6039ac94c52d3ad492e9615e50ba37",
            "7fdcd66ff616ae238c73f8281786c3ec7a721264ed3fddc60b3eadd252d6d97c"
        ],
        "depth": 10862,
        "prev_block_url": "https://api.blockcypher.com/v1/btc/main/blocks/00000000000000000001dc4780b5419dd828c2f2fecfc18a2a8c7387ca960206",
        "tx_url": "https://api.blockcypher.com/v1/btc/main/txs/",
        "next_txids": "https://api.blockcypher.com/v1/btc/main/blocks/00000000000000000000cfbeaeb0b5f18dffc118597310d3f87096a2a204b512?txstart=20&limit=20"
    }
    "#;

    let block: BitcoinBlock = serde_json::from_str(data).unwrap();
    println!("{:#?}", block);
}
