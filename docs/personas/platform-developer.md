# Persona: Platform Developer

## Identity

| Field | Value |
|---|---|
| **Name** | Jordan Kim |
| **Age** | 28 |
| **Occupation** | Software developer building a dApp on Dash Platform |
| **Technical Proficiency** | Very high; writes code in Rust and JavaScript; understands cryptographic primitives, state transitions, and protocol-level concepts |
| **Crypto Experience** | 3+ years; deep familiarity with Dash Platform internals from SDK documentation and source code |
| **Network** | Primarily Testnet and Devnet; rarely Mainnet (only for final deployment verification) |

## Description

Jordan is building a decentralized application on Dash Platform. Jordan uses Dash Evo Tool as a companion tool during development -- not as a primary wallet for holding significant funds, but as a graphical interface for operations that would otherwise require scripting: registering test identities, funding Platform addresses, creating asset locks with specific amounts, inspecting state transitions, and verifying contract deployments.

Jordan's wallet typically holds small amounts of test Dash (tDash). The wallet is a means to an end: Jordan needs funded identities and Platform addresses to test dApp functionality. Jordan values speed and directness -- being able to create an identity, fund it, and verify the result without leaving the tool.

Jordan is the user who genuinely benefits from seeing raw protocol details: credit amounts, nonce values, derivation paths, and state transition results. Jordan also needs to rapidly iterate -- create a wallet, fund it from a faucet, register an identity, deploy a contract, and tear it all down to start over.

## Primary Goals

1. **Rapid identity creation and funding** -- Create test identities quickly, fund them with specific credit amounts, and top them up as needed during development.
2. **Platform address operations** -- Fund Platform addresses from asset locks or wallet UTXOs; transfer credits between addresses; withdraw back to Core.
3. **Network flexibility** -- Switch between Testnet and Devnet easily; possibly run against a local regtest network.
4. **Contract and state transition inspection** -- Use the Tools screens (transition visualizer, proof log, document query) alongside the wallet to verify dApp behavior.
5. **Disposable wallet workflows** -- Create temporary wallets, use them for a testing session, and remove them without ceremony.

## Secondary Goals

- Export and import private keys for use in scripts or other tools.
- Inspect raw UTXO details for debugging transaction construction.
- Verify that asset lock proofs are valid and usable.
- Monitor Platform credit consumption during state transition testing.
- Use the app as a reference implementation for how wallet operations should work.

## Pain Points (Current App)

1. **No quick-fund workflow.** Jordan wants a "fund this identity with X credits" button that handles the entire chain: select UTXOs, create asset lock, wait for proof, fund identity. Currently, each step is a separate screen.
2. **Wallet creation is too ceremonial.** For throwaway test wallets, the mnemonic backup flow and password setup are unnecessary friction. Jordan wants a "create and skip" option.
3. **No faucet integration.** On Testnet, Jordan has to manually visit a faucet website, copy the address, paste it, wait, and then refresh. An in-app faucet request would save time.
4. **Devnet configuration is manual.** Setting up a Devnet connection requires editing the .env file. Jordan wants to enter Devnet parameters in the UI.
5. **No bulk operations.** Jordan sometimes needs to create 5 identities for testing. This requires repeating the same workflow 5 times.
6. **Credit/Duff conversion confusion.** The UI sometimes shows Platform balances in "credits" and sometimes in "DASH equivalent." Jordan needs to see both, or at least have a consistent unit with easy conversion.
7. **Error messages from backend tasks are opaque.** When a state transition fails, the error message is often a raw Rust error string. Jordan needs actionable feedback.

## Success Metrics

| Metric | Target |
|---|---|
| Time from empty wallet to funded identity (Testnet) | Under 3 minutes |
| Number of clicks to create an asset lock and fund a Platform address | Under 8 |
| Ability to see exact credit balance (not just DASH equivalent) | Always available |
| Network switch time | Under 5 seconds, no app restart |
| Error message actionability | Every error suggests a next step or links to documentation |

## Frequency of Interaction

- Uses the app intensively during development sprints (multiple hours per day).
- May not open the app for weeks between sprints.
- Creates and destroys wallets frequently.
- Performs many small transactions (100-10,000 credits) rather than large ones.
- Switches between Testnet and Devnet multiple times per session.

## What Jordan Needs That Neither Alex nor Priya Do

- Devnet configuration through the UI.
- Streamlined identity creation that combines asset lock and funding in one flow.
- Raw credit amounts displayed alongside DASH equivalents.
- Faucet integration for Testnet.
- Minimal-friction wallet creation for throwaway wallets.
- Bulk operations (create N identities, fund N addresses).
- State transition error details with protocol-level context.
- Quick access to Tools screens (transition visualizer, proof log) from wallet context.

## Quotes (Illustrative)

> "I need to test 5 different identity scenarios. Creating each one takes 12 clicks across 3 screens. Can this be a single operation?"

> "The error says 'insufficient credits.' How many credits do I have? How many do I need? What is the fee? Show me the numbers."

> "I just want a funded wallet on Testnet. I do not care about the mnemonic backup -- this wallet will be deleted in an hour."
