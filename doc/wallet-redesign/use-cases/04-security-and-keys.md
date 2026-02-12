# Use Cases: Security and Key Management

## UC-SK-01: Set a Wallet Password During Creation

**Personas**: Alex, Priya

### User Story
As a user creating a new wallet, I want to set a password so that my private keys are encrypted at rest and require the password to use.

### Acceptance Criteria

```
Given I am creating a new wallet,
When I set a password,
Then the wallet seed is encrypted using AES-256-GCM with a key derived from the password.

Given the password is weak,
When I view the password strength indicator,
Then I see a warning with the estimated time to crack (via zxcvbn).

Given I choose not to set a password,
When the wallet is created,
Then the wallet seed is stored unencrypted and the wallet is always "open."
```

### Real-Life Scenario: Alex Secures a New Wallet
**Context**: Alex is creating a wallet to hold meaningful funds on Mainnet.
**Flow**: After backing up the seed phrase, Alex enters a password. The strength meter shows "Medium" and "Estimated crack time: 3 months." Alex changes to a stronger password. The meter shows "Strong" and "Estimated crack time: centuries." Alex confirms and the wallet is created.
**Expected Outcome**: Wallet is encrypted. Alex must enter the password to unlock it after closing and reopening the app.
**What Could Go Wrong**: Alex sets a weak password despite warnings. The app should allow this (user's choice) but display a persistent warning.

---

## UC-SK-02: Export a Private Key (WIF)

**Personas**: Priya, Jordan

### User Story
As a technical user, I want to export the private key for a specific address in WIF format so that I can use it in another tool or for backup.

### Acceptance Criteria

```
Given I have a wallet unlocked and an address selected,
When I click "View Key" for that address,
Then a modal shows the address and the private key in WIF format, hidden by default.

Given the private key is hidden,
When I click "Show Key,"
Then the WIF string is revealed.

Given I want to copy the key,
When I click "Copy Key,"
Then the WIF string is copied to the clipboard.

Given the wallet is locked,
When I click "View Key,"
Then the unlock popup appears before revealing the key.
```

### Security Warning
The dialog must always display: "Keep your private key secure. Never share it with anyone." in a prominent warning style.

---

## UC-SK-03: View Seed Phrase (Backup Verification)

**Personas**: Alex, Priya

### User Story
As a user, I want to view my wallet's seed phrase again so that I can verify my backup or create a new backup copy.

### Acceptance Criteria

```
Given I have an unlocked wallet,
When I navigate to wallet settings and request "Show Seed Phrase,"
Then the app requires password re-entry and then displays the seed phrase.
```

### Current Status
Not implemented in the current UI. This is a commonly expected feature in wallet applications and should be considered for the redesign.

---

## UC-SK-04: Auto-Lock Wallet on Inactivity

**Personas**: Alex, Priya

### User Story
As a security-conscious user, I want my wallet to lock automatically after a period of inactivity so that my keys are not exposed if I walk away from my computer.

### Acceptance Criteria

```
Given my wallet is unlocked and I have been inactive for the configured timeout (e.g., 5 minutes),
When the timeout expires,
Then the wallet automatically locks (seed is zeroized in memory).

Given auto-lock triggered,
When I return and attempt an operation requiring the private key,
Then the unlock popup appears.
```

### Current Status
Not implemented. Currently, wallets remain unlocked until manually locked or the app is closed. This is a security improvement to consider.

---

## UC-SK-05: Password-Protected Send Confirmation

**Personas**: Alex

### User Story
As an everyday user, I want to confirm sends with my password so that no one can send from my wallet without knowing the password.

### Acceptance Criteria

```
Given I have a password-protected wallet and I initiate a send,
When I click "Confirm Send,"
Then the app requires my password before broadcasting the transaction.
```

### Current Status
Currently, if the wallet is already unlocked, sends proceed without re-authentication. For everyday users handling meaningful amounts, password re-entry on send is a valuable safety net. This should be an opt-in setting.

---

## UC-SK-06: Secure Wallet Removal

**Personas**: Priya

### User Story
As a user removing a wallet, I want to be clearly warned about what will be lost and have the option to verify my backup before proceeding.

### Acceptance Criteria

```
Given I click "Remove" on a wallet,
When the confirmation dialog appears,
Then it explains: local data (addresses, balances, asset locks) will be deleted; on-chain identities remain but keys derived from this wallet will stop working unless re-imported.

Given the wallet has linked identities,
When the confirmation dialog appears,
Then it lists the affected identities by name/ID with an explicit warning.

Given I have not backed up the seed phrase,
When the confirmation dialog appears,
Then it offers to show the seed phrase one last time (requires unlock).
```

### Current Status
The current confirmation dialog explains the consequences well but does not offer a "show seed phrase" option before removal. This is a safety improvement.
