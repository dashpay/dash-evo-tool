# Dash Evo Tool

**Dash Evo Tool** is a cross-platform GUI application for interacting with [Dash Evolution](https://www.dash.org/). It supports Mainnet, Testnet, Devnet, and local regtest networks.

See the [user documentation](https://docs.dash.org/en/stable/docs/user/network/dash-evo-tool/) for setup and usage instructions.

## Features

**Identity**
- Register, load, and manage Platform identities
- Top up identity credits from wallet UTXOs, asset locks, or Platform addresses
- Add authentication, voting, owner, and transfer keys
- Withdraw or transfer credits between identities

**DPNS**
- Register usernames
- View and vote on contested name auctions
- Schedule votes for future execution

**Wallet**
- Import wallets from mnemonic seed phrases or single private keys
- Send payments to Core addresses (single or batch)
- Create and recover asset locks for identity funding

**Tokens**
- Register, mint, burn, and transfer fungible tokens
- Freeze/unfreeze tokens, pause/resume transfers, destroy frozen funds
- Configure direct purchase pricing and claim distribution rewards

**DashPay**
- Manage your profile (display name, bio, avatar)
- Send and accept contact requests
- Send payments to contacts

**Documents & Contracts**
- Fetch, register, and update data contracts
- Create, query, replace, delete, transfer, and purchase documents

**Tools**
- Decode and inspect state transitions
- Visualize Grovedb proofs and documents
- View platform information and masternode quorum details
- Check Core address balances

## Programmatic Access (MCP)

- [MCP Server](docs/MCP.md) — expose wallet and core operations via Model Context Protocol (HTTP or stdio)
- [CLI Client](docs/CLI.md) — command-line tool to call MCP operations from scripts and terminals

## Getting prebuilt binaries

Download the latest release from the [Releases](https://github.com/dashpay/dash-evo-tool/releases) page.

### Install via Flatpak (Linux)

The easiest way to run Dash Evo Tool on Linux is via Flatpak. Download the `.flatpak` bundle for your architecture from the [latest release](https://github.com/dashpay/dash-evo-tool/releases) and install it:

``` shell
# x86_64
flatpak install dash-evo-tool-linux-x86_64.flatpak

# aarch64 (ARM)
flatpak install dash-evo-tool-linux-aarch64.flatpak
```

To run:

``` shell
flatpak run org.dash.DashEvoTool
```

To uninstall:

``` shell
flatpak uninstall org.dash.DashEvoTool
```

The Flatpak version runs in SPV (light client) mode — no full Dash Core node is required. Application data is stored in `~/.var/app/org.dash.DashEvoTool/config/dash-evo-tool/`.

> **Note:** The Flatpak data path differs from native Linux builds, which use `~/.config/dash-evo-tool/`.

### Windows runtime dependencies

If you use the prebuilt Windows binary, make sure the target machine has:

- Microsoft Visual C++ Redistributable (vc_redist x64): https://aka.ms/vc14/vc_redist.x64.exe
- OpenGL 2.0 support. If OpenGL 2.0 is not available (or the app fails to start with OpenGL-related errors), install the OpenCL, OpenGL, and Vulkan Compatibility Pack:
  https://apps.microsoft.com/detail/9nqpsl29bfff?ocid=webpdpshare

## Building from source

See the [Contributing Guide](CONTRIBUTING.md) for prerequisites, build instructions, and development workflow.

## Application directory

When the application runs for the first time, it creates an application directory and stores an `.env` file in it (based on [`.env.example`](.env.example)). It also stores application data in the directory. If you need to update the `.env` file, locate it in the application directory for your operating system:

| Operating System | Application Directory Path |
| - | - |
| macOS | `~/Library/Application Support/Dash-Evo-Tool/` |
| Windows | `C:\Users\<User>\AppData\Roaming\Dash-Evo-Tool\config` |
| Linux | `/home/<user>/.config/dash-evo-tool/` |

## Environment Variables

| Variable | Values | Default | Description |
| - | - | - | - |
| `DASH_EVO_TOOL_ACCESSIBILITY` | `1` / unset | unset | Enable accessibility support. Activates AccessKit eagerly so the UI element tree is populated every frame and (on macOS) forces the platform accessibility adapter to initialize without VoiceOver. Required for AXUIElement-based automation tools (e.g. Peekaboo) and useful for screen readers on all platforms. |

## Contributing

Contributions are welcome! See the [Contributing Guide](CONTRIBUTING.md) for details.

## License

This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.

## Support

- **Issues**: Open an issue on [GitHub Issues](https://github.com/dashpay/dash-evo-tool/issues).
- **Community**: Join the Dash community forums or Discord server for discussions.

## Security Note

Keep your private keys and identity information secure. Do not share them with untrusted parties or applications.
