# User Roles (Interface Mode)

Dash Evo Tool adapts its interface to how you use it. Instead of a single
"Expert mode" on/off switch, the app offers three interface modes, each
revealing more advanced controls than the one before it:

| Mode | What it shows |
| - | - |
| **Default view** | Your balance, send and receive, and usernames. |
| **Expert view** | Adds account details, address tables, and masternode tools. |
| **Developer view** | Adds raw protocol data, Devnet, and signing overrides. |

Each mode includes everything the one before it does — Developer view has
everything Expert view has, plus more. **Expert view** is the starting
point until you pick a mode yourself; switch down to Default view for a
simpler screen, or up to Developer view when you need raw protocol data.

## Where to set it

The three modes are chosen the same way, with the same descriptions, in two
places:

- **Network Settings** — open the network chooser screen. An **Interface
  mode** card sits above Advanced Settings, always visible (it is not tucked
  inside a collapsed section).
- **Welcome screen** — on first launch, the onboarding screen shows a
  "Choose your experience level" row with the same three options before you
  create or load a wallet.

Picking a mode in either place updates the other — there is only one active
mode, shared across the whole app.

**This choice is reversible.** You are not locking yourself into a mode by
picking one at onboarding: open **Network Settings** at any time and change
it from the **Interface mode** card. The app applies the change immediately,
with no restart required.

## Upgrading from an older install

Earlier versions had a **Developer mode** on/off switch instead of the three
interface modes. If you are upgrading, and you have not picked an interface
mode yet, the app starts you in **Expert view** — the mode that matches the
account details, address tables and masternode tools those builds exposed.
Nothing you had is hidden, and you can move to another mode at any time from
**Network Settings**.

The interface mode is chosen in the app, not in a file: it is stored with your
other settings, and it is the same on every network.

## Migration note: obsolete `.env` entries

Older configurations may still contain a global `DEVELOPER_MODE=true|false`
entry, or per-network entries such as `MAINNET_developer_mode=true`. Neither has
any effect on the interface mode — that is chosen in the app and stored with your
settings. New installs no longer get a `DEVELOPER_MODE` line at all.

Leave the leftover entries or delete them, as you prefer. The one place
`DEVELOPER_MODE` is still consulted is the one-time database upgrade that moved
existing installs onto the built-in light client (SPV): an install that had both
`DEVELOPER_MODE=true` and a Dash Core RPC password configured keeps talking to
its local Dash Core node. That upgrade runs once, and only for installs that
predate it.

The application directory holding `.env` is:

| Operating System | Path |
| - | - |
| macOS | `~/Library/Application Support/Dash-Evo-Tool/.env` |
| Windows | `C:\Users\<User>\AppData\Roaming\Dash-Evo-Tool\config\.env` |
| Linux | `~/.config/dash-evo-tool/.env` |
