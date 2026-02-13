/**
 * Reusable funding helper for E2E tests.
 *
 * Any spec that needs funds can call `ensureFunded(minBalanceDuffs)`.
 * It checks the current balance, requests from the faucet if needed,
 * and waits for the balance to update via SPV.
 *
 * Requires: 00-setup to have completed (wallet imported, SPV synced,
 * TestContext populated with walletSeedHash).
 */

import { requestFaucet } from "./faucet.js";
import { invoke, getWalletBalance } from "./ipc.js";
import { read as readContext, update as updateContext } from "./test-context.js";

/** Default minimum balance: 0.1 DASH = 10,000,000 duffs */
const DEFAULT_MIN_BALANCE = 10_000_000;

/** How long to wait for balance to update after faucet (90s) */
const BALANCE_POLL_TIMEOUT = 90_000;

/** Poll interval for balance checks (5s) */
const BALANCE_POLL_INTERVAL = 5_000;

export interface EnsureFundedResult {
  funded: boolean;
  balanceDuffs: number;
  faucetUsed: boolean;
}

/**
 * Ensure the test wallet has at least `minBalanceDuffs` available.
 *
 * 1. Checks current balance from TestContext (refreshed via IPC).
 * 2. If sufficient, returns immediately.
 * 3. Otherwise, generates a receive address, calls the faucet, and polls
 *    for balance update via SPV.
 *
 * @param minBalanceDuffs - Minimum required balance in duffs (default 0.1 DASH)
 * @returns Result with funding status and final balance
 */
export async function ensureFunded(
  minBalanceDuffs: number = DEFAULT_MIN_BALANCE
): Promise<EnsureFundedResult> {
  const ctx = readContext();
  const { walletSeedHash } = ctx;

  if (!walletSeedHash) {
    throw new Error(
      "ensureFunded: walletSeedHash not set in TestContext. " +
        "Did 00-setup complete successfully?"
    );
  }

  // 1. Refresh and check current balance
  let balance = await getWalletBalance(walletSeedHash);
  console.log(`  Current balance: ${balance} duffs (need ${minBalanceDuffs})`);

  if (balance >= minBalanceDuffs) {
    updateContext({ balanceDuffs: balance });
    return { funded: true, balanceDuffs: balance, faucetUsed: false };
  }

  // 2. Get receive address (from context or generate new one)
  let address = ctx.receiveAddress;
  if (!address) {
    console.log("  Generating receive address...");
    const result = await invoke<{ address: string }>(
      "wallet_generate_receive_address",
      { input: { walletSeedHash } }
    );
    address = result.address;
    updateContext({ receiveAddress: address });
    console.log(`  Receive address: ${address}`);
  } else {
    console.log(`  Using cached receive address: ${address}`);
  }

  // 3. Request funds from faucet
  console.log("  Requesting funds from faucet...");
  const faucetResult = await requestFaucet(address);

  if (!faucetResult.success) {
    console.error(`  Faucet failed: ${faucetResult.error}`);
    updateContext({ balanceDuffs: balance, faucetUsed: true });
    return { funded: false, balanceDuffs: balance, faucetUsed: true };
  }

  console.log(`  Faucet success! txid: ${faucetResult.txid}`);

  // 4. Trigger wallet refresh and poll for balance update
  console.log("  Waiting for balance to update via SPV...");
  try {
    await invoke("core_refresh_wallet_info", {
      input: { walletSeedHash, platformSyncMode: null },
    });
  } catch {
    // Fire-and-forget — the refresh is async
  }

  balance = await pollForBalance(walletSeedHash, minBalanceDuffs);

  // 5. Update context and return
  updateContext({ balanceDuffs: balance, faucetUsed: true });
  console.log(`  Final balance: ${balance} duffs`);

  return {
    funded: balance >= minBalanceDuffs,
    balanceDuffs: balance,
    faucetUsed: true,
  };
}

/**
 * Poll wallet balance until it reaches the target or times out.
 */
async function pollForBalance(
  seedHash: string,
  targetDuffs: number
): Promise<number> {
  const start = Date.now();
  let lastRefresh = 0;

  while (Date.now() - start < BALANCE_POLL_TIMEOUT) {
    const balance = await getWalletBalance(seedHash);
    if (balance >= targetDuffs) return balance;

    // Trigger a wallet refresh every 15s to speed up SPV detection
    const now = Date.now();
    if (now - lastRefresh >= 15_000) {
      try {
        await invoke("core_refresh_wallet_info", {
          input: { walletSeedHash: seedHash, platformSyncMode: null },
        });
        lastRefresh = now;
      } catch {
        // Ignore — refresh is best-effort
      }
    }

    await new Promise((r) => setTimeout(r, BALANCE_POLL_INTERVAL));
  }

  // Return whatever balance we have even if below target
  return getWalletBalance(seedHash);
}
