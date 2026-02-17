# Dash Evo Tool

**Dash Evo Tool** is a graphical user interface for easily interacting with Dash Evolution. The current version enables the following actions:

- Registering a DPNS username
- Viewing active DPNS username contests
- Voting on active DPNS username contests
- Decoding and viewing state transitions

The tool supports both Mainnet and Testnet networks. Check out the [documentation](https://docs.dash.org/en/stable/docs/user/network/dash-evo-tool/index.html) for additional information.

## Getting prebuilt binaries

Download the latest release from the [Releases](https://github.com/dashpay/dash-evo-tool/releases) page.

### Windows runtime dependencies

If you use the prebuilt Windows binary, make sure the target machine has:

- Microsoft Visual C++ Redistributable (vc_redist x64): https://aka.ms/vc14/vc_redist.x64.exe
- OpenGL 2.0 support. If OpenGL 2.0 is not available (or the app fails to start with OpenGL-related errors), install the OpenCL, OpenGL, and Vulkan Compatibility Pack:
  https://apps.microsoft.com/detail/9nqpsl29bfff?ocid=webpdpshare

## Building from source

See the [Contributing Guide](CONTRIBUTING.md) for prerequisites, build instructions, and development workflow.

## Application directory

When the application runs for the first time, it creates an application directory and stores an `.env` file in it (based on [`.env.example`](.env.example)). It also stores application data in the directory. If you need to update the `.env` file, locate it in the application directory for your Operating System:

| Operating System | Application Directory Path |
| - | - |
| macOS | `~/Library/Application Support/Dash-Evo-Tool/` |
| Windows | `C:\Users\<User>\AppData\Roaming\Dash-Evo-Tool\config` |
| Linux | `/home/<user>/.config/dash-evo-tool/` |

## Connect to a Network

1. **Open Network Chooser**: In the app, navigate to the **Network Chooser** screen.

2. **Select Network**: Choose **Mainnet** or **Testnet**.

3. **Start Connection**: Click **Start** next to the selected network.

   - If Dash Core Wallet is running and synced, the status will show **Online**.
   - If not, the app attempts to start Dash Core Wallet automatically.

To switch networks later, return to the Network Chooser and select a different network. Ensure Dash Core Wallet is fully synchronized before proceeding.

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

## Contributing

Contributions are welcome! See the [Contributing Guide](CONTRIBUTING.md) for details.

## License

This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.

## Support

For assistance:

- **Issues**: Open an issue on [GitHub Issues](https://github.com/dashpay/dash-evo-tool/issues).
- **Community**: Join the Dash community forums or Discord server for discussions.

## Security Note

Keep your private keys and identity information secure. Do not share them with untrusted parties or applications.
