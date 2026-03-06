# Manual Test Scenarios: Fix Nonce Reset on Refresh (#652)

## Prerequisites
- Dash Evo Tool running and connected to a network (Testnet or Devnet)
- An HD wallet with at least one Platform Payment address that has been used (nonce > 0)
- If no address has nonce > 0, perform a Platform transaction first (e.g., transfer credits)

## Test Scenario 1: Refresh All preserves nonces (default mode)

**Steps:**
1. Open Wallets screen
2. Select an HD wallet
3. Navigate to Platform Payment addresses (click the Platform account)
4. Note the nonce values for addresses with nonce > 0
5. Click the Refresh button (default "Core + Platform" mode)
6. Wait for refresh to complete

**Expected:** All nonce values remain unchanged after refresh. Addresses that had nonce > 0 still show the same nonce.

## Test Scenario 2: Platform Only refresh preserves nonces

**Steps:**
1. Open Wallets screen and select an HD wallet
2. Navigate to Platform Payment addresses
3. Note the nonce values
4. Switch refresh mode to "Platform Only" (dev mode dropdown)
5. Click Refresh
6. Wait for refresh to complete

**Expected:** Nonce values remain unchanged. Balances may update if changed on-chain.

## Test Scenario 3: Nonce updates correctly after new transaction

**Steps:**
1. Open Wallets screen and note current nonce for a Platform address
2. Perform a Platform transaction using that address (e.g., transfer credits)
3. After transaction completes, note the updated nonce
4. Click Refresh
5. Wait for refresh to complete

**Expected:** Nonce reflects the post-transaction value both before and after refresh.

## Test Scenario 4: Zero-balance addresses retain nonces

**Steps:**
1. Have a Platform address that was previously funded (nonce > 0) but now has 0 balance (credits were withdrawn or transferred out)
2. Navigate to Platform Payment addresses
3. Enable "Show zero-balance addresses" if the address is hidden
4. Note the nonce value
5. Click Refresh

**Expected:** The address retains its nonce value even though balance is 0.

## Test Scenario 5: Locked wallet shows error on refresh

**Steps:**
1. Open a password-protected wallet
2. Lock the wallet
3. Attempt to click Refresh for Platform sync

**Expected:** Error message indicating wallet must be unlocked. Nonces from before locking are unchanged.
