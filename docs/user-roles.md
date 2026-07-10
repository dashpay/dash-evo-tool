# User Roles (Interface Mode)

Dash Evo Tool adapts its interface to how you use it. Instead of a single
"Expert mode" on/off switch, the app offers three interface modes, each
revealing more advanced controls than the one before it:

| Mode | What it shows |
| - | - |
| **Default view** | Your balance, send and receive, and usernames. |
| **Detailed view** | Adds account details, address tables, and masternode tools. |
| **Developer tools** | Adds raw protocol data, Devnet, and signing overrides. |

Each mode includes everything the one before it does — Developer tools has
everything Detailed view has, plus more. **Default view** is the starting
point for a fresh install; pick a higher mode only once you need the
additional controls it unlocks.

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

## `.env` behavior: `DEVELOPER_MODE`

The `.env` file in the application directory still has a `DEVELOPER_MODE`
entry (`true` or `false`), but it now works differently than it used to:

- **It is a one-time starting point, not a live switch.** `DEVELOPER_MODE` is
  only read once — the very first time the app runs and no interface mode has
  ever been chosen yet (via the UI or a previous `.env` read).
- **Once a mode is chosen, `.env` is never consulted again.** This applies
  whether the mode was set by picking it in the UI or by the one-time
  `DEVELOPER_MODE` seed itself. Editing `DEVELOPER_MODE` afterward has no
  effect — use **Network Settings** to change modes instead.

### Migration from an older install

If you are upgrading from a version that had the old `DEVELOPER_MODE`
on/off toggle, your existing `.env` value is used exactly once, the first
time you launch this version:

- `DEVELOPER_MODE=true` seeds **Detailed view** — this app's earlier
  "developer mode" corresponded to what is now Detailed view (account
  details, address tables, masternode tools), not the new Developer tools
  mode.
- `DEVELOPER_MODE=false`, or no `DEVELOPER_MODE` entry at all, seeds
  **Default view** — no change from today's default.

After that one-time seed, the value in `.env` is ignored; change modes from
**Network Settings** going forward.

The application directory is:

| Operating System | Path |
| - | - |
| macOS | `~/Library/Application Support/Dash-Evo-Tool/.env` |
| Windows | `C:\Users\<User>\AppData\Roaming\Dash-Evo-Tool\config\.env` |
| Linux | `~/.config/dash-evo-tool/.env` |

## Migration note: obsolete per-network values

Earlier builds used per-network entries such as `MAINNET_developer_mode=true`
and `TESTNET_developer_mode=false` in `.env`. These are obsolete and ignored —
the interface mode is a single global setting, not configured per network.
Any leftover `*_developer_mode` entries from older configurations can be
removed safely.
