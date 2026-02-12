# Use Cases: Network and Settings

## UC-NS-01: Switch Networks

**Personas**: Priya, Jordan

### User Story
As a user who operates on multiple networks, I want to switch between Mainnet, Testnet, and Devnet so that I can manage wallets and identities on each network.

### Acceptance Criteria

```
Given I am on Mainnet,
When I navigate to Settings and select Testnet,
Then the app switches to Testnet context: wallets, identities, and Platform data are loaded for Testnet.

Given I switch networks,
When the wallet screen loads for the new network,
Then the wallet selector shows only wallets applicable to that network, and the previously selected wallet for this network (if any) is restored.
```

### Persona-Specific Notes

| Persona | Expectation |
|---|---|
| Alex | Almost never switches networks. Mainnet is the default and only relevant network. The setting exists but is not prominent. |
| Priya | Switches between Mainnet and Testnet. Expects wallet selections to be remembered per network. |
| Jordan | Switches frequently between Testnet and Devnet. Needs to configure custom Devnet parameters (gRPC endpoint, Core RPC URL). Currently requires editing .env file -- should be configurable in-app. |

---

## UC-NS-02: Toggle Developer Mode

**Personas**: Priya, Jordan

### User Story
As a technical user, I want to enable an advanced mode that shows additional wallet details and controls so that I can access power features.

### Acceptance Criteria

```
Given I am in the Settings screen,
When I enable Developer Mode (or the redesigned equivalent),
Then additional UI elements appear across the app: refresh mode selector, raw credit values, state transition details, etc.

Given Developer Mode is enabled,
When I view the wallet screen,
Then I see: granular refresh controls, account category details, full address tables, and transaction history.
```

### Redesign Note
As documented in the persona analysis, the current binary "developer mode" toggle should be reconsidered. The recommended approach is:
1. **Transaction history should always be visible** (not gated behind developer mode).
2. **Address detail and account categories should use progressive disclosure** (expand/collapse per section).
3. **Developer-specific features** (Devnet config, raw credit values, bulk operations) should be behind a "Developer Tools" setting.
4. **Power user features** (refresh control, UTXO details, key export) should be accessible through contextual UI (expand arrows, right-click menus, or per-section toggles) rather than a global mode.

---

## UC-NS-03: Configure Devnet Connection

**Personas**: Jordan

### User Story
As a Platform developer, I want to configure a custom Devnet connection (gRPC endpoint, Core RPC URL, network name) through the UI so that I do not need to edit configuration files manually.

### Acceptance Criteria

```
Given I am in Settings,
When I select "Add Devnet" and enter the gRPC endpoint, Core RPC URL, and network name,
Then the app saves the configuration and makes the Devnet available in the network selector.

Given I have a configured Devnet,
When I select it from the network selector,
Then the app connects to the specified endpoints and loads Devnet-specific data.
```

### Current Status
Devnet configuration currently requires editing the `.env` file (located in the platform-specific app config directory). This is a friction point for developers.

---

## UC-NS-04: Request Testnet Funds (Faucet)

**Personas**: Jordan

### User Story
As a developer on Testnet, I want to request test Dash from within the app so that I do not need to visit an external faucet website.

### Acceptance Criteria

```
Given I am on Testnet and have a wallet selected,
When I click "Get Test Dash" (or similar),
Then the app sends a faucet request using my wallet's receive address and displays the result.

Given the faucet request succeeds,
When the funds arrive,
Then my wallet balance updates automatically.
```

### Current Status
Not implemented. This is a new feature request driven by the Platform Developer persona. It reduces the friction of the test-develop-iterate cycle significantly.
