/**
 * Shared test state that persists across spec files via a JSON file on disk.
 *
 * Spec files run sequentially (00-setup → 01-faucet → 02-wallet → …).
 * Each writes its outputs here so later specs can read them.
 */

import fs from "fs";

const CONTEXT_PATH =
  process.env.E2E_CONTEXT_PATH || "/tmp/e2e-test-context.json";

export interface TestContext {
  walletSeedHash: string;
  receiveAddress: string;
  balanceDuffs: number;
  spvSynced: boolean;
  faucetUsed: boolean;
  network: string;
}

const EMPTY: TestContext = {
  walletSeedHash: "",
  receiveAddress: "",
  balanceDuffs: 0,
  spvSynced: false,
  faucetUsed: false,
  network: "testnet",
};

/** Write a full context (overwrites). */
export function write(ctx: TestContext): void {
  fs.writeFileSync(CONTEXT_PATH, JSON.stringify(ctx, null, 2));
}

/** Read the current context. Returns defaults if no file exists. */
export function read(): TestContext {
  if (!fs.existsSync(CONTEXT_PATH)) return { ...EMPTY };
  try {
    return JSON.parse(fs.readFileSync(CONTEXT_PATH, "utf-8")) as TestContext;
  } catch {
    return { ...EMPTY };
  }
}

/** Merge partial updates into the existing context. */
export function update(partial: Partial<TestContext>): TestContext {
  const ctx = { ...read(), ...partial };
  write(ctx);
  return ctx;
}

/** Remove the context file (called during teardown). */
export function clear(): void {
  if (fs.existsSync(CONTEXT_PATH)) fs.unlinkSync(CONTEXT_PATH);
}
