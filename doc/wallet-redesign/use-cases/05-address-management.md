# Use Cases: Address Management

## UC-AM-01: View Address Table for Selected Account

**Personas**: Priya, Jordan

### User Story
As a technical user, I want to see all addresses in a selected account category with their balances, UTXO counts, and derivation paths so that I can audit my wallet's address usage.

### Acceptance Criteria

```
Given I have selected an account category (e.g., "Main Account"),
When the address table renders,
Then I see columns: Address, Balance (DASH), UTXOs, Total Received (DASH), Type (Funds/Change/Platform/System), Index, Full Path, Private Key.

Given I click a column header,
When the table re-renders,
Then the rows are sorted by that column (toggle ascending/descending).
```

### Persona-Specific Notes

| Persona | Expectation |
|---|---|
| Alex | Does NOT see the address table. Addresses are managed automatically. At most, Alex sees a single receive address. |
| Priya | Full address table with all columns. Sortable. Can inspect any address. |
| Jordan | Same as Priya. Additionally, ability to copy the derivation path for use in scripts. |

---

## UC-AM-02: Add a New Receiving Address

**Personas**: Alex, Priya

### User Story
As a user, I want to generate a new receiving address so that I can receive payments at a fresh address for privacy.

### Acceptance Criteria

```
Given I am viewing the Main Account,
When I click "Add Receiving Address,"
Then a new BIP44 external address is derived at the next unused index, added to the wallet, and persisted to the database.

Given the new address is generated,
When I view the address table (Priya) or receive dialog (Alex),
Then the new address appears with a 0 balance.
```

---

## UC-AM-03: Generate a New Platform Address

**Personas**: Priya, Jordan

### User Story
As a user, I want to generate a new DIP-17 Platform payment address so that I can receive credits on a fresh Platform address.

### Acceptance Criteria

```
Given I am viewing the Platform Account or the Receive dialog (Platform tab),
When I click "New Address,"
Then a new DIP-17 Platform payment address is derived at the next unused index and displayed in Bech32m format (evo1.../tevo1...).

Given the address is generated,
When I check the address table for the Platform Account,
Then the new address appears with a 0 credit balance.
```

---

## UC-AM-04: Copy an Address to Clipboard

**Personas**: Alex, Priya, Jordan

### User Story
As a user, I want to copy any address to my clipboard so that I can share it or use it in another application.

### Acceptance Criteria

```
Given I see an address in the UI (address table, receive dialog, or elsewhere),
When I click a "Copy" button or click on the address,
Then the full address string is copied to the clipboard and a brief confirmation is shown.
```

---

## UC-AM-05: View Platform Address in Both Formats

**Personas**: Priya, Jordan

### User Story
As a technical user, I want to see Platform addresses in both Bech32m format (tevo1...) and the underlying Core address format so that I can use the appropriate format depending on context.

### Acceptance Criteria

```
Given I view a Platform Payment address in the address table,
When the address is displayed,
Then it shows the Bech32m (DIP-18) format by default.

Given I need the Core address format,
When I hover over or expand the address,
Then the Core address representation is also shown.
```

### Current Status
The current implementation shows Platform addresses in Bech32m format in the address table (`display_address` method) but does not offer a way to see the underlying Core address format. This could be added as a tooltip or expandable detail.

---

## UC-AM-06: Filter Addresses by Activity

**Personas**: Priya, Jordan

### User Story
As a user with many derived addresses, I want to filter the address list to show only addresses with balance or transaction history so that I can focus on relevant addresses.

### Acceptance Criteria

```
Given I have an account with 32+ addresses (many unused),
When I apply a "Has Activity" filter,
Then only addresses with non-zero balance or non-zero total received are shown.
```

### Current Status
Not implemented. The current address table shows all bootstrapped addresses regardless of activity. For accounts with 20+ addresses, this creates noise. A filter or "hide empty" toggle would improve usability.
