# Manual Test Scenarios: Duplicate Key Error Handling (Issue #714)

Covers the fix for showing user-friendly error messages when adding a duplicate
key to an identity, instead of raw base64-encoded CBOR.

---

## Prerequisites (all scenarios)

- Dash Evo Tool running and connected to **Testnet** (or Devnet).
- A funded **wallet** loaded in the application.
- An **identity** registered on the network with at least one existing key
  (ECDSA_SECP256K1 AUTHENTICATION MASTER is the default).
- The identity's master private key is available (wallet unlocked or key known).

---

## Scenario 1: Adding a key with the same public key data (DuplicatedIdentityPublicKeyStateError)

### Goal

Verify the app displays a clear, actionable message when the user tries to add
a key whose public key data already exists on the identity.

### Steps

1. Open **Identities** screen and select the target identity.
2. Click **Keys** to view the identity's existing keys.
3. Note the public key data of an existing key (e.g., copy the hex value of key
   ID 0).
4. Click **Add Key**.
5. Set **Purpose** to `AUTHENTICATION`, **Security Level** to `HIGH`,
   **Key Type** to `ECDSA_SECP256K1` (or match the existing key's type).
6. In the **Private key (hex)** field, enter the exact same private key that
   corresponds to the existing public key on the identity.
7. Click the **Add Key** button to submit.

### Expected Result

- An **error banner** appears at the top of the screen with the message:
  > This public key is already registered on the platform. Try a different key.
- The message is plain text -- no base64, no CBOR encoding, no raw error dump.
- The banner type is **Error** (red/error styling).
- The app does **not** crash or freeze.
- The **Add Key** form remains accessible so the user can correct the input and
  retry.

---

## Scenario 2: Adding a key with a conflicting key ID (DuplicatedIdentityPublicKeyIdStateError)

### Goal

Verify the app displays a clear message when a key ID collision occurs.

### Context

This error is less likely via the GUI because the app auto-assigns the next
available key ID. It can occur if the identity state is out of sync (e.g.,
another client added a key concurrently). To trigger it manually, a modified
client or direct Platform interaction may be needed. This scenario validates the
error display path regardless of trigger.

### Steps

1. Open **Identities** screen and select the target identity.
2. Click **Keys**, then **Add Key**.
3. Fill in a valid new key (different key data from existing keys).
4. Arrange for the key ID to collide with an existing key ID (this may require
   a race condition where another client adds a key between nonce fetch and
   broadcast, or direct Platform API manipulation).
5. Submit the **Add Key** request.

### Expected Result

- An **error banner** appears with the message:
  > This key hash is already registered on the platform. Try a different key.
- The message is plain text -- no encoded data.
- The banner type is **Error**.
- The app does **not** crash.
- The user can dismiss the banner and retry.

---

## Scenario 3: Adding a key that conflicts with unique contract bounds (IdentityPublicKeyAlreadyExistsForUniqueContractBoundsError)

### Goal

Verify the app displays a clear message including the conflicting contract ID
when a key violates unique contract bounds.

### Steps

1. Open **Identities** screen and select the target identity.
2. Click **Keys**, then **Add Key**.
3. Check the **Contract bounds** checkbox.
4. Enter a valid **Contract ID** for a contract that enforces unique key bounds.
5. Set the **Purpose** and **Security Level** to match an existing key that is
   already bound to the same contract.
6. Generate or enter a new private key.
7. Click the **Add Key** button to submit.

### Expected Result

- An **error banner** appears with the message:
  > This key conflicts with an existing key bound to contract \<contract-id\>. Use a different key or purpose.

  where `<contract-id>` is the actual identifier of the conflicting contract
  (not a placeholder, not encoded).
- The message is plain text -- no base64, no CBOR.
- The banner type is **Error**.
- The app does **not** crash.
- The user can change the **Purpose**, **Contract ID**, or key data and retry.

---

## Scenario 4: Fallback for unrecognized broadcast errors

### Goal

Verify that broadcast errors not matching the three handled patterns still
display a readable message (prefixed with "Broadcasting error:") rather than
crashing.

### Steps

1. Open **Identities** screen and select the target identity.
2. Click **Keys**, then **Add Key**.
3. Trigger a broadcast failure that is NOT a duplicate key error (e.g.,
   disconnect from the network mid-broadcast, or submit with insufficient
   identity balance for the fee).
4. Observe the error display.

### Expected Result

- An **error banner** appears with a message starting with:
  > Broadcasting error: ...
- The remaining text may be technical but must not contain raw base64 CBOR
  blobs.
- The app does **not** crash.
- The user can retry.

---

## Scenario 5: Successful key addition after a duplicate key error

### Goal

Verify the app recovers gracefully and allows a successful operation after a
prior duplicate key error.

### Steps

1. Reproduce **Scenario 1** to trigger the duplicate key error banner.
2. Observe that the error banner is displayed.
3. Change the **Private key (hex)** field to a new, unique private key.
4. Click the **Add Key** button again.

### Expected Result

- The duplicate key error banner is dismissed/replaced.
- A progress/info banner may appear while the transaction broadcasts.
- On success, a **success banner** appears confirming the key was added,
  including fee information.
- The new key appears in the identity's key list.
- No residual error state from the previous failed attempt.

---

## Edge Cases

| Case | Action | Expected |
|------|--------|----------|
| Double-click Add Key | Click Add Key rapidly twice | Only one broadcast attempt; no duplicate submission |
| Wallet locks during broadcast | Lock wallet (if possible) after clicking Add Key | Graceful error, no crash |
| Network disconnect during broadcast | Disable network after clicking Add Key | Timeout or connection error displayed as banner, not raw panic |
| Very long contract ID in error | Trigger Scenario 3 with a valid 256-bit identifier | Full contract ID shown in the message, no truncation |
