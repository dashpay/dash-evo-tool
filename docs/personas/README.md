# User Personas

This directory contains persona documents for the Dash Evo Tool wallet screen redesign, representing three primary user archetypes.

## Personas

| Persona | File | Summary |
|---|---|---|
| **Everyday User** | [everyday-user.md](everyday-user.md) | Holds and transacts Dash; uses DPNS and DashPay; wants simplicity; does not understand derivation paths or asset locks |
| **Power User** | [power-user.md](power-user.md) | Manages multiple wallets; operates masternodes; needs full address visibility, asset lock management, and granular refresh control; currently mislabeled as "developer" |
| **Platform Developer** | [platform-developer.md](platform-developer.md) | Builds dApps on Dash Platform; uses Testnet/Devnet; needs rapid identity creation, faucet integration, and minimal-friction workflows for throwaway wallets |

## Key Insight: "Developer Mode" Is Mislabeled

The current "user mode" / "developer mode" toggle is misleading. What it calls "developer mode" is really power-user mode -- the features it unlocks (transaction history, refresh controls, address tables) are standard wallet management features, not development tools. Actual Platform developers need an entirely different feature set (Testnet faucet, bulk identity creation, Devnet configuration) that the current toggle does not address.

## Recommended Mode Structure

Rather than a binary toggle, the personas suggest a **progressive disclosure** model:

| Level | Who | What They See |
|---|---|---|
| **Default view** | Everyday User (Alex) | Total balance, Send/Receive buttons, transaction history, DPNS name. Account categories and address details are hidden behind expandable sections. |
| **Detailed view** | Power User (Priya) | Full account breakdown, address tables with derivation paths, asset lock management, refresh controls, private key export. Activated per-section (expand/collapse) rather than a global toggle. |
| **Developer tools** | Platform Developer (Jordan) | Everything in detailed view, plus: raw credit amounts, state transition context, Devnet configuration, faucet integration, bulk operations. Activated by a setting, not just a toggle. |
