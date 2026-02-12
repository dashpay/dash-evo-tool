# Wallet Redesign: User Personas

This directory contains persona documents for the Dash Evo Tool wallet screen redesign. These personas represent the three primary user archetypes identified through analysis of the current codebase, the existing "user" vs. "developer" mode distinction, and the range of features the app supports.

## Personas

| Persona | File | Summary |
|---|---|---|
| **Everyday User** | [everyday-user.md](everyday-user.md) | Holds and transacts Dash; uses DPNS and DashPay; wants simplicity; does not understand derivation paths or asset locks |
| **Power User** | [power-user.md](power-user.md) | Manages multiple wallets; operates masternodes; needs full address visibility, asset lock management, and granular refresh control; currently mislabeled as "developer" |
| **Platform Developer** | [platform-developer.md](platform-developer.md) | Builds dApps on Dash Platform; uses Testnet/Devnet; needs rapid identity creation, faucet integration, and minimal-friction workflows for throwaway wallets |

## Key Insight: "Developer Mode" Is Mislabeled

The current app uses a "user mode" / "developer mode" toggle. This is misleading because:

1. **"Developer mode" is really "power user mode."** The features it unlocks (transaction history, refresh controls, detailed address tables) are not development tools -- they are standard wallet management features that any technical user expects.

2. **Actual developers (Platform dApp builders)** need an entirely different set of features: Testnet faucet integration, streamlined identity creation, bulk operations, raw credit displays, and Devnet configuration. These are not addressed by the current mode toggle.

3. **Everyday users** are the ones who need feature reduction -- hiding complexity, not adding it. The default ("user") mode currently shows too much already: 12+ account categories, full address lists, derivation paths.

## Recommended Mode Structure

Rather than a binary "user/developer" toggle, the personas suggest a **progressive disclosure** model:

| Level | Who | What They See |
|---|---|---|
| **Default view** | Everyday User (Alex) | Total balance, Send/Receive buttons, transaction history, DPNS name. Account categories and address details are hidden behind expandable sections. |
| **Detailed view** | Power User (Priya) | Full account breakdown, address tables with derivation paths, asset lock management, refresh controls, private key export. Activated per-section (expand/collapse) rather than a global toggle. |
| **Developer tools** | Platform Developer (Jordan) | Everything in detailed view, plus: raw credit amounts, state transition context, Devnet configuration, faucet integration, bulk operations. Activated by a setting, not just a toggle. |

This approach means:
- Alex sees a clean, simple wallet by default.
- Priya can expand any section to see full detail without switching a global mode.
- Jordan gets developer-specific tools (faucet, bulk ops, Devnet config) behind a real developer setting.

## Persona Relationships

```
Alex (Everyday User)
  |
  | Needs: simplicity, balance, send/receive
  | Does NOT need: derivation paths, asset locks, refresh modes
  |
  v
Priya (Power User)
  |
  | Needs: everything Alex needs, PLUS full address detail,
  |        asset locks, account categories, tx history, key export
  | Does NOT need: Devnet config, faucet, bulk ops
  |
  v
Jordan (Platform Developer)
  |
  | Needs: everything Priya needs, PLUS faucet integration,
  |        Devnet config, bulk identity creation, raw credit values
```

Each persona is a strict superset of the one above. The UI should support this by layering detail rather than switching between modes.

## Impact on Wallet Screen Redesign

1. **Balance display**: Show a single prominent total balance for Alex. Offer a Core/Platform breakdown as an expandable detail for Priya. Show raw credit values for Jordan.

2. **Address management**: Alex sees one receive address and a QR code. Priya sees the full address table grouped by account category. Jordan sees the same as Priya plus raw derivation paths for scripting.

3. **Asset locks**: Invisible to Alex (automated behind identity creation if needed). Visible as a management section to Priya. Bulk-creation capable for Jordan.

4. **Send/Receive**: Identical primary flow for all personas. Priya and Jordan get additional options (Platform address send, fee control). Alex gets "send to address" and "show my QR code."

5. **Transaction history**: Visible to all personas by default (currently hidden behind developer mode -- this is a bug, not a feature).
