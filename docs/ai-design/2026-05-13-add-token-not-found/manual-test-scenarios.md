# Add Token Not-Found Fallback Manual Test Scenarios

## Scope

Validate the add-token-by-ID lookup state machine when a contract ID is missing, when a token ID is used as fallback, and when stale state could otherwise be reused.

## Scenario 1: Missing contract falls back to token ID lookup

1. Open `Tokens` -> `Add Token`.
2. Enter a valid base58 identifier that is not a contract ID but is a real token ID.
3. Click `Search`.
4. Verify the screen does not show an error after the first `ContractNotFound` result.
5. Verify the screen automatically performs the token-ID lookup and resolves to the token or contract result.

## Scenario 2: Token not found shows terminal error

1. Open `Tokens` -> `Add Token`.
2. Enter a valid base58 identifier that is neither a contract ID nor a token ID.
3. Click `Search`.
4. Verify the lookup ends with an error banner saying no contract or token was found.
5. Verify the screen does not loop or retry indefinitely.

## Scenario 3: Token found but backing contract missing

1. Open `Tokens` -> `Add Token`.
2. Enter a valid base58 token ID whose token record resolves but whose data contract is unavailable.
3. Click `Search`.
4. Verify the final banner says the token was found but its data contract could not be fetched.
5. Verify the screen stays in an error state and does not retry again.

## Scenario 4: New search clears stale add-token state

1. Resolve a token so the `Add Token` button becomes available.
2. Without adding it, enter a different identifier and click `Search`.
3. Verify the previous token selection and fetched contract are cleared immediately.
4. Verify the old `Add Token` action is no longer available unless the new lookup succeeds.

## Scenario 5: Search button is disabled during active lookup

1. Enter any valid identifier and click `Search`.
2. While the status line shows `Searching...`, verify the `Search` button is disabled.
3. Verify only one lookup result sequence is processed for that search attempt.
