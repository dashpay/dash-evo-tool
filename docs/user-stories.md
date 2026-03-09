# User Stories

This document catalogs user stories for Dash Evo Tool, organized by feature area and mapped to the three user personas: **Alex** (Everyday User), **Priya** (Power User), and **Jordan** (Platform Developer). Each story follows a progressive disclosure model where simpler needs come first. Stories marked `[Implemented]` reflect features present in the current codebase; stories marked `[Gap]` represent identified needs not yet addressed.

See [docs/personas/](personas/) for full persona descriptions.

## Table of Contents

- [Wallet Management (WAL)](#wallet-management-wal)
- [Send and Receive (SND)](#send-and-receive-snd)
- [Asset Locks (ALK)](#asset-locks-alk)
- [Identity Operations (IDN)](#identity-operations-idn)
- [DPNS (DPN)](#dpns-dpn)
- [DashPay (DPY)](#dashpay-dpy)
- [Token Operations (TOK)](#token-operations-tok)
- [Contracts and Documents (DOC)](#contracts-and-documents-doc)
- [Developer and Power Tools (DEV)](#developer-and-power-tools-dev)
- [Network and Settings (NET)](#network-and-settings-net)

---

## Wallet Management (WAL)

### WAL-001: Create a new wallet [Implemented]
**Persona:** Alex, Priya, Jordan

As a user, I want to create a new wallet with a generated mnemonic so that I can start holding and transacting Dash.

- Mnemonic is generated using mouse entropy for randomness.
- User can select mnemonic language and wallet name.
- Optional password protection is offered.
- Recovery phrase is displayed for backup.

### WAL-002: Import wallet via mnemonic [Implemented]
**Persona:** Alex, Priya, Jordan

As a user, I want to import an existing wallet by entering its seed phrase so that I can access funds from another wallet.

- Accepts standard BIP39 mnemonic phrases.
- User can assign a name and optional password.
- Wallet syncs balances after import.

### WAL-003: Import single private key [Implemented]
**Persona:** Priya, Jordan

As a power user, I want to import a single private key so that I can manage funds from a standalone address.

- Creates a single-key wallet from WIF-format key.
- Wallet appears in the wallet selector.

### WAL-004: Switch between wallets [Implemented]
**Persona:** Priya, Jordan

As a user managing multiple wallets, I want to switch between them quickly so that I can manage different funds without restarting.

- Wallet selector dropdown shows all loaded wallets.
- Switching is instant with no app restart.

### WAL-005: Rename a wallet [Implemented]
**Persona:** Priya

As a power user, I want to rename wallets so that I can identify them by purpose (e.g., "Masternode Collateral").

- Name change persists across sessions.

### WAL-006: Lock and unlock wallet [Implemented]
**Persona:** Alex, Priya

As a user, I want to lock my wallet with a password so that others cannot access my funds if I leave the app open.

- Locked wallet requires password to unlock.
- Sensitive operations are blocked while locked.

### WAL-007: Remove a wallet [Implemented]
**Persona:** Priya, Jordan

As a user, I want to remove a wallet I no longer need so that it does not clutter my wallet list.

- Confirmation prompt before removal.
- Wallet data is deleted from local storage.

### WAL-008: View wallet balances [Implemented]
**Persona:** Alex, Priya, Jordan

As a user, I want to see my wallet balance so that I know how much Dash I hold.

- Displays Core balance and Platform balance.
- Alex sees a simplified view; Priya sees per-account breakdown.

### WAL-009: View fiat equivalent of balances [Gap]
**Persona:** Alex

As an everyday user, I want to see my balance in my local currency so that I can understand the value of my holdings without mental conversion.

- Fiat amount displayed alongside DASH balance.
- Supports common fiat currencies (USD, EUR, etc.).
- Exchange rate updates periodically.

### WAL-010: Generate receive address [Implemented]
**Persona:** Alex, Priya

As a user, I want to generate a new receive address so that I can share it with someone paying me.

- Address displayed with QR code.
- Alex sees a single address by default.

### WAL-011: View address table [Implemented]
**Persona:** Priya, Jordan

As a power user, I want to see all addresses with derivation paths, UTXO counts, and balances so that I can audit my wallet structure.

- Shows BIP44 external/internal addresses and Platform Payment addresses.
- Displays derivation path, balance, and UTXO count per address.

### WAL-012: View and export private keys [Implemented]
**Persona:** Priya, Jordan

As a power user, I want to view and export private keys in WIF format so that I can use them in other tools or back them up individually.

- Key export requires wallet to be unlocked.
- Keys displayed in WIF format.

### WAL-013: View SPV sync status [Implemented]
**Persona:** Alex, Priya

As a user, I want to see the sync status of my wallet so that I know when my balance is up to date.

- Connection status indicator shows current sync stage.
- Color-coded status (red/orange/green/magenta).

### WAL-014: Label addresses [Gap]
**Persona:** Priya

As a power user, I want to annotate individual addresses with labels so that I can remember their purpose (e.g., "Masternode collateral", "Cold storage").

- Labels persist across sessions.
- Labels are visible in the address table.
- Labels are searchable.

### WAL-015: Create throwaway wallet without mnemonic backup [Gap]
**Persona:** Jordan

As a developer, I want to create a temporary wallet quickly without being required to back up the mnemonic so that I can start testing within seconds.

- Skip or defer mnemonic backup step.
- Wallet is clearly marked as "unbackup" or "temporary."

### WAL-016: View transaction history [Implemented]
**Persona:** Alex, Priya

As a user, I want to review my past transactions so that I can track payments sent and received.

- Lists transactions with amounts, dates, and direction.
- Priya sees TxID, block height, and confirmation count.

---

## Send and Receive (SND)

### SND-001: Send Dash to an address [Implemented]
**Persona:** Alex, Priya

As a user, I want to send Dash to a recipient address so that I can make payments.

- Enter destination address and amount.
- Confirmation dialog before broadcast.

### SND-002: Send Dash from single-key wallet [Implemented]
**Persona:** Priya, Jordan

As a user with an imported private key, I want to send Dash from that single-key wallet so that I can move funds to another address.

- Send flow works the same as for HD wallets.

### SND-003: Receive Dash with QR code [Implemented]
**Persona:** Alex, Priya

As a user, I want to see a QR code for my receive address so that the sender can scan it instead of copying a long string.

- QR code displayed alongside the text address.
- Copy-to-clipboard button available.

### SND-004: Send to a DPNS username [Gap]
**Persona:** Alex

As an everyday user, I want to send Dash to someone by entering their DPNS username instead of a raw address so that I do not have to deal with long cryptographic strings.

- Username is resolved to a Dash address before sending.
- Confirmation shows both the username and resolved address.
- Error displayed if username is not found.

### SND-005: See fee estimate before confirming send [Gap]
**Persona:** Alex, Priya

As a user, I want to see the estimated transaction fee and total amount to be deducted before confirming a send so that I know exactly what I am paying.

- Fee estimate shown in confirmation dialog.
- Total deduction (amount + fee) displayed clearly.

---

## Asset Locks (ALK)

### ALK-001: Create an asset lock [Implemented]
**Persona:** Priya, Jordan

As a user, I want to create an asset lock so that I can fund an identity or top up Platform credits.

- Select amount and create asset lock.
- Dynamic fee calculation based on input count.

### ALK-002: View asset lock details [Implemented]
**Persona:** Priya, Jordan

As a power user, I want to view the details of an existing asset lock so that I can verify its status and amounts.

- Shows transaction ID, amount, and status.

### ALK-003: Recover unused asset locks [Implemented]
**Persona:** Priya

As a power user, I want to recover funds from unused or stuck asset locks so that my Dash is not permanently locked.

- Search for unspent asset locks.
- Recovery flow returns funds to wallet.

### ALK-004: Quick-fund workflow [Gap]
**Persona:** Jordan

As a developer, I want a one-click "fund this identity with X credits" button so that I do not have to manually create an asset lock, wait for confirmation, and then fund the identity in separate steps.

- Single action handles UTXO selection, asset lock creation, proof wait, and identity funding.
- Progress indicator shows each stage.

---

## Identity Operations (IDN)

### IDN-001: Register a new identity [Implemented]
**Persona:** Alex, Priya, Jordan

As a user, I want to register a new identity on Dash Platform so that I can use Platform features like DPNS and DashPay.

- Multi-stage confirmation flow.
- Identity funded from an asset lock.

### IDN-002: Load existing identity by ID [Implemented]
**Persona:** Priya, Jordan

As a power user, I want to load an existing identity by its ID and owner private key so that I can manage an identity created elsewhere.

- Enter identity ID and private key.
- Identity details are fetched and displayed.

### IDN-003: Load evonode/masternode identity [Implemented]
**Persona:** Priya

As a masternode operator, I want to load my evonode identity via protx hash so that I can manage it through the GUI.

- Enter protx hash to load the associated identity.

### IDN-004: Top up identity credits [Implemented]
**Persona:** Priya, Jordan

As a user, I want to add credits to my identity so that I can continue performing Platform operations.

- Top up from wallet or Platform addresses.
- Amount selection with credit cost display.

### IDN-005: Withdraw credits to Core address [Implemented]
**Persona:** Priya, Jordan

As a user, I want to withdraw Platform credits back to a Core Dash address so that I can convert credits to spendable Dash.

- Enter destination address and withdrawal amount.
- Withdrawal appears in the queue.

### IDN-006: Transfer credits between identities [Implemented]
**Persona:** Priya, Jordan

As a user, I want to transfer credits from one identity to another so that I can redistribute Platform funds.

- Select source and destination identities.
- Enter transfer amount.

### IDN-007: Add key to identity [Implemented]
**Persona:** Priya, Jordan

As a power user, I want to add a new key to my identity so that I can authorize additional operations or devices.

- Select key type and purpose.
- Key is added via state transition.

### IDN-008: View identity keys and details [Implemented]
**Persona:** Priya, Jordan

As a user, I want to view all keys associated with my identity so that I can audit access and verify key configuration.

- Lists all keys with type, purpose, and status.
- View individual key details.

### IDN-009: Refresh identity state [Implemented]
**Persona:** Priya, Jordan

As a user, I want to refresh my identity data from the network so that I see the latest credit balance and key state.

- Manual refresh button.
- Updated data reflected immediately.

### IDN-010: Search identity by DPNS name [Implemented]
**Persona:** Alex, Priya

As a user, I want to find an identity by searching for its DPNS username so that I do not need to know the raw identity ID.

- Enter username and retrieve associated identity.

### IDN-011: Bulk identity creation [Gap]
**Persona:** Jordan

As a developer, I want to create multiple identities in one operation so that I can set up test scenarios without repeating the same workflow N times.

- Specify count of identities to create.
- Each is funded and registered automatically.
- Progress shown per identity.

---

## DPNS (DPN)

### DPN-001: Register a DPNS username [Implemented]
**Persona:** Alex, Priya

As a user, I want to register a human-readable username on DPNS so that others can send me Dash using a name instead of an address.

- Choose identity, enter desired name.
- Cost estimate displayed before confirmation.

### DPN-002: View owned usernames [Implemented]
**Persona:** Alex, Priya

As a user, I want to see all DPNS usernames I own so that I can manage my registered names.

- Lists all usernames tied to the current wallet's identities.

### DPN-003: View active name contests [Implemented]
**Persona:** Priya

As a power user, I want to view active DPNS name contests so that I can participate in voting on contested names.

- Lists all contests with status and vote counts.

### DPN-004: View past name contests [Implemented]
**Persona:** Priya

As a power user, I want to review past DPNS contests so that I can see outcomes and historical voting data.

- Lists completed contests with results.

### DPN-005: Vote on contested names [Implemented]
**Persona:** Priya

As a masternode operator, I want to vote on contested DPNS name registrations so that I can participate in network governance.

- Cast, change, or abstain votes (max 4 vote changes per contest).
- Evonode/masternode identity required.

### DPN-006: Schedule votes [Implemented]
**Persona:** Priya

As a masternode operator, I want to schedule votes for later execution so that I can plan my voting strategy in advance.

- Set vote to be cast at a future time.
- View and manage scheduled votes.

### DPN-007: Batch voting across contests [Implemented]
**Persona:** Priya

As a masternode operator, I want to apply voting choices across multiple contests in bulk so that I do not have to vote on each contest individually.

- "Set all" option for batch vote assignment.

---

## DashPay (DPY)

### DPY-001: View and edit DashPay profile [Implemented]
**Persona:** Alex, Priya

As a user, I want to create and edit my DashPay profile (name, bio, avatar) so that contacts can identify me.

- Set display name, bio, and profile image.
- Changes are published as a state transition.

### DPY-002: Search DashPay profiles [Implemented]
**Persona:** Alex, Priya

As a user, I want to search for other DashPay users so that I can find and add contacts.

- Search by username or display name.
- View profile details before sending a contact request.

### DPY-003: Send contact request [Implemented]
**Persona:** Alex, Priya

As a user, I want to send a contact request to another DashPay user so that we can transact more easily.

- Enter username or identity ID.
- Request is sent via state transition.

### DPY-004: Accept or reject contact requests [Implemented]
**Persona:** Alex, Priya

As a user, I want to accept or reject incoming contact requests so that I control who is in my contact list.

- Incoming requests listed with sender profile info.

### DPY-005: View contact list and details [Implemented]
**Persona:** Alex, Priya

As a user, I want to see my contacts and their profiles so that I can manage my social payment network.

- Lists all accepted contacts.
- View individual contact details and profile.

### DPY-006: Send payment to contact [Implemented]
**Persona:** Alex, Priya

As a user, I want to send Dash to a DashPay contact so that I can pay them without entering an address manually.

- Select contact and enter amount.
- Payment sent through the DashPay protocol.

### DPY-007: View payment history [Implemented]
**Persona:** Alex, Priya

As a user, I want to view past payments sent and received through DashPay so that I can track my social transactions.

- Lists payments with amounts, dates, and contact names.

### DPY-008: Generate DashPay QR code [Implemented]
**Persona:** Alex

As an everyday user, I want to generate a DashPay QR code so that someone nearby can scan it to add me as a contact or send a payment.

- QR code encodes DashPay profile or payment info.

---

## Token Operations (TOK)

### TOK-001: View token balances [Implemented]
**Persona:** Alex, Priya

As a user, I want to see all tokens I hold and their balances so that I can manage my token portfolio.

- "My Tokens" screen lists all held tokens with balances.

### TOK-002: Search and discover tokens [Implemented]
**Persona:** Alex, Priya, Jordan

As a user, I want to search for tokens by keyword so that I can find and add tokens to my wallet.

- Keyword search across token names and metadata.
- Add token from search results.

### TOK-003: Add token by contract or token ID [Implemented]
**Persona:** Priya, Jordan

As a power user, I want to add a token by entering its contract ID or token ID directly so that I can track a specific token I know about.

- Enter ID manually and add to token list.

### TOK-004: Transfer tokens [Implemented]
**Persona:** Alex, Priya

As a user, I want to transfer tokens to another identity so that I can send tokens I own.

- Select token, enter recipient and amount.
- Confirmation before broadcast.

### TOK-005: Create token contract [Implemented]
**Persona:** Jordan

As a developer, I want to create a new token contract with full configuration so that I can issue a custom token on Dash Platform.

- Configure naming, supply, decimals, action rules, distribution, and groups.
- Contract is registered via state transition.

### TOK-006: Mint tokens [Implemented]
**Persona:** Jordan

As a token issuer, I want to mint additional tokens so that I can increase the circulating supply according to my token's rules.

- Requires authorized identity.
- Specify amount to mint.

### TOK-007: Burn tokens [Implemented]
**Persona:** Jordan

As a token holder or issuer, I want to burn tokens so that I can reduce supply or destroy unwanted tokens.

- Specify amount to burn.
- Confirmation before execution.

### TOK-008: Freeze and unfreeze token recipients [Implemented]
**Persona:** Jordan

As a token issuer, I want to freeze or unfreeze a recipient so that I can enforce compliance rules on my token.

- Freeze blocks the recipient from receiving tokens.
- Unfreeze restores normal operation.

### TOK-009: Pause and resume token transfers [Implemented]
**Persona:** Jordan

As a token issuer, I want to pause all transfers of my token so that I can halt trading during an emergency or upgrade.

- Pause stops all transfers globally.
- Resume re-enables transfers.

### TOK-010: Destroy frozen funds [Implemented]
**Persona:** Jordan

As a token issuer, I want to destroy tokens held by a frozen recipient so that I can enforce penalties or recover funds.

- Only works on frozen recipients.
- Confirmation before destruction.

### TOK-011: Claim distributed tokens [Implemented]
**Persona:** Alex, Priya

As a user, I want to claim tokens distributed to me so that they appear in my balance.

- View available claims.
- Claim action transfers tokens to identity.

### TOK-012: Set token pricing and purchase tokens [Implemented]
**Persona:** Jordan

As a token issuer, I want to set a price for my token so that others can purchase it directly.

- Set price per token.
- Buyers can purchase at the set price.

### TOK-013: Update token configuration [Implemented]
**Persona:** Jordan

As a token issuer, I want to update my token's configuration so that I can adjust rules after deployment.

- Modify configurable parameters.
- Changes applied via state transition.

### TOK-014: Group actions for multi-party governance [Implemented]
**Persona:** Jordan

As a token issuer, I want to use group actions (query, sign, approve) so that token governance decisions require multi-party consensus.

- View pending group actions.
- Sign or approve actions as a group member.

---

## Contracts and Documents (DOC)

### DOC-001: Register a new data contract [Implemented]
**Persona:** Jordan

As a developer, I want to register a data contract on Dash Platform so that my dApp can store structured data.

- Define contract schema and register.
- Contract ID returned upon success.

### DOC-002: Update an existing data contract [Implemented]
**Persona:** Jordan

As a developer, I want to update a deployed contract so that I can evolve my dApp's data schema.

- Submit updated contract definition.
- Version incremented on Platform.

### DOC-003: Import and manage contracts [Implemented]
**Persona:** Priya, Jordan

As a user, I want to add contracts by ID so that I can browse documents from any deployed contract.

- Enter contract ID to import.
- Remove cached contracts when no longer needed.

### DOC-004: Query and browse documents [Implemented]
**Persona:** Priya, Jordan

As a user, I want to query documents from a data contract so that I can inspect the data stored on Platform.

- Select contract and document type.
- View query results as document list.

### DOC-005: Create a document [Implemented]
**Persona:** Jordan

As a developer, I want to create a document in a data contract so that I can test data submission from my dApp's perspective.

- Enter document properties according to contract schema.
- Document submitted via state transition.

### DOC-006: Replace or update a document [Implemented]
**Persona:** Jordan

As a developer, I want to update an existing document so that I can test document mutation workflows.

- Select document and modify fields.
- Updated document replaces the previous version.

### DOC-007: Delete a document [Implemented]
**Persona:** Jordan

As a developer, I want to delete a document so that I can test deletion logic and clean up test data.

- Select document and confirm deletion.
- Document is removed from Platform state.

### DOC-008: Transfer document ownership [Implemented]
**Persona:** Jordan

As a developer, I want to transfer a document to another identity so that I can test ownership transfer flows.

- Select document and destination identity.
- Ownership updated on Platform.

### DOC-009: Purchase a document and set document pricing [Implemented]
**Persona:** Jordan

As a developer, I want to set a price on a document and allow others to purchase it so that I can test marketplace mechanics.

- Set price on a document.
- Another identity can purchase at the set price.

---

## Developer and Power Tools (DEV)

### DEV-001: Decode state transitions [Implemented]
**Persona:** Jordan

As a developer, I want to paste and decode raw state transitions so that I can debug what my dApp is submitting to Platform.

- Transition visualizer parses and displays state transition contents.

### DEV-002: View proof request log [Implemented]
**Persona:** Jordan

As a developer, I want to review the history of proof requests made by the app so that I can debug query behavior and performance.

- Proof log lists all requests with timestamps and results.

### DEV-003: Inspect ZK proofs [Implemented]
**Persona:** Jordan

As a developer, I want to visualize and verify ZK proofs so that I can confirm Platform responses are valid.

- Proof visualizer displays proof structure.
- GroveSTARK proof generation and verification available.

### DEV-004: View document and contract JSON [Implemented]
**Persona:** Jordan

As a developer, I want to see raw JSON representations of documents and contracts so that I can verify data structure and content.

- Document visualizer shows full JSON.
- Contract visualizer shows contract schema JSON.

### DEV-005: View Platform info [Implemented]
**Persona:** Priya, Jordan

As a user, I want to see Platform status (epoch info, total credits, validators, version voting) so that I can understand the current state of the network.

- Displays epoch info, validator list, withdrawal queue, and version voting status.

### DEV-006: View masternode list diff [Implemented]
**Persona:** Priya

As a masternode operator, I want to view changes to the masternode list so that I can monitor network composition.

- Shows additions, removals, and changes between blocks.

### DEV-007: Check any address balance [Implemented]
**Persona:** Priya, Jordan

As a user, I want to check the balance of any Dash address so that I can verify payments or audit external addresses.

- Enter any address and see its balance.

### DEV-008: Mine blocks on Regtest [Implemented]
**Persona:** Jordan

As a developer, I want to mine blocks on a local Regtest network so that I can advance the chain during local testing.

- Available only in developer mode on Regtest/local network.
- Specify number of blocks to mine.

---

## Network and Settings (NET)

### NET-001: Switch networks [Implemented]
**Persona:** Priya, Jordan

As a user, I want to switch between Mainnet, Testnet, Devnet, and Local networks so that I can operate on the appropriate chain.

- Network chooser screen with all available networks.
- Per-network data isolation (wallets, identities, contracts).
- Switch without app restart.

### NET-002: Auto-update from dashmate config [Implemented]
**Persona:** Jordan

As a developer running a local network, I want the app to auto-detect dashmate configuration so that I do not have to manually enter connection details.

- Detects and imports local dashmate config.

### NET-003: Configure Dash-Qt path [Implemented]
**Persona:** Priya

As a power user, I want to configure the path to Dash-Qt so that the app can use it as a Core backend.

- Path set in settings.
- App validates the path exists.

### NET-004: Select theme [Implemented]
**Persona:** Alex, Priya, Jordan

As a user, I want to choose between light, dark, or auto theme so that the app matches my visual preference.

- Light, dark, and system-auto options.
- Theme change applied immediately.

### NET-005: Toggle developer mode [Implemented]
**Persona:** Priya, Jordan

As a user, I want to enable developer mode so that I can access advanced features like address tables, refresh controls, and debug tools.

- Toggles visibility of advanced UI elements.

### NET-006: Select user mode [Implemented]
**Persona:** Alex, Priya

As a user, I want to choose between Beginner and Advanced mode so that the interface matches my experience level.

- Beginner mode hides complexity; Advanced mode shows full detail.

### NET-007: Granular refresh controls [Implemented]
**Persona:** Priya

As a power user, I want to choose whether to refresh Core Only, Platform Only, or both so that I can save time when I only need part of the data updated.

- Refresh mode selector available in detailed/developer view.

### NET-008: Select Core backend mode [Implemented]
**Persona:** Priya, Jordan

As a user, I want to choose between SPV, RPC, or Auto mode for the Core backend so that I can control how the app connects to the Dash Core network.

- SPV for light sync, RPC for full node, Auto for app-selected.

### NET-009: Toggle ZMQ [Implemented]
**Persona:** Priya, Jordan

As a user, I want to enable or disable ZMQ notifications so that I can receive real-time block and transaction events when connected to a local node.

- ZMQ enable/disable toggle in settings.

### NET-010: Onboarding wizard [Implemented]
**Persona:** Alex

As a new user, I want a guided onboarding experience so that I can set up my first wallet and understand the app without reading documentation.

- Welcome screen with setup steps.
- Guides user through initial wallet creation.

### NET-011: Wipe Platform data [Implemented]
**Persona:** Jordan

As a developer, I want to wipe Platform data for Devnet or Testnet so that I can start fresh when the network has been reset.

- Available only for Devnet and Testnet.
- Clears cached Platform state.

### NET-012: Configure Devnet through the UI [Gap]
**Persona:** Jordan

As a developer, I want to enter Devnet connection parameters in the UI so that I do not have to manually edit the .env file.

- UI form for Devnet host, port, and other parameters.
- Settings saved and applied without restart.

### NET-013: Testnet faucet integration [Gap]
**Persona:** Jordan

As a developer, I want to request test Dash from a faucet directly within the app so that I do not have to leave the tool, copy addresses, and wait.

- In-app faucet request for the currently selected wallet.
- Balance updates after faucet delivery.

### NET-014: Bulk fund addresses [Gap]
**Persona:** Jordan

As a developer, I want to fund multiple addresses in one operation so that I can set up testing scenarios efficiently.

- Specify N addresses and amount per address.
- Single action distributes funds.
