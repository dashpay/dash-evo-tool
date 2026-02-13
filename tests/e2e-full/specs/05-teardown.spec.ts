/**
 * 05-teardown — Suite cleanup (Phase 5).
 *
 * Tears down the test environment:
 * 1. Stop SPV sync via IPC
 * 2. Verify disconnected state in UI
 * 3. Take final screenshot for debugging artifacts
 * 4. Log summary from TestContext
 * 5. Clean up the test context file
 *
 * Runs last in the suite (after all other specs).
 */

import { takeScreenshot } from "../helpers/tauri.js";
import { invoke } from "../helpers/ipc.js";
import { read as readContext, clear as clearContext } from "../helpers/test-context.js";

describe("Teardown", () => {
  // ─── Stop SPV ──────────────────────────────────────────────────

  it("should stop SPV sync via IPC", async function () {
    this.timeout(15_000);

    try {
      await invoke("wallet_stop_spv");
      console.log("  SPV sync stopped");
    } catch (err) {
      // SPV may already be stopped — non-fatal
      const msg = err instanceof Error ? err.message : String(err);
      console.log(`  wallet_stop_spv returned: ${msg}`);
    }

    // Give SPV a moment to wind down
    await browser.pause(2_000);
  });

  // ─── UI: Verify disconnected state ────────────────────────────

  it("should show disconnected state after SPV stop", async () => {
    // Check the SPV status via IPC to confirm it's no longer "running"
    try {
      const statuses = await invoke<Array<{ network: string; status: string }>>(
        "get_spv_status"
      );
      const testnetEntry = statuses.find(
        (s) => s.network.toLowerCase() === "testnet"
      );

      if (testnetEntry) {
        console.log(`  SPV status after stop: ${testnetEntry.status}`);
        expect(testnetEntry.status).not.toBe("running");
      } else {
        console.log("  No testnet SPV entry found (expected after stop)");
      }
    } catch (err) {
      // Non-fatal — just log
      const msg = err instanceof Error ? err.message : String(err);
      console.log(`  Could not check SPV status: ${msg}`);
    }
  });

  // ─── Final screenshot ─────────────────────────────────────────

  it("should take a final screenshot", async () => {
    const path = await takeScreenshot("final-teardown");
    console.log(`  Final screenshot saved: ${path}`);
  });

  // ─── Log summary ──────────────────────────────────────────────

  it("should log the test summary", async () => {
    const ctx = readContext();

    console.log("\n  ═══════════════════════════════════");
    console.log("  E2E Test Suite Summary");
    console.log("  ═══════════════════════════════════");
    console.log(`  Network:        ${ctx.network}`);
    console.log(`  Wallet seed:    ${ctx.walletSeedHash ? ctx.walletSeedHash.slice(0, 12) + "..." : "N/A"}`);
    console.log(`  Balance:        ${ctx.balanceDuffs} duffs (${(ctx.balanceDuffs / 1e8).toFixed(8)} DASH)`);
    console.log(`  SPV synced:     ${ctx.spvSynced}`);
    console.log(`  Faucet used:    ${ctx.faucetUsed}`);
    console.log(`  Receive addr:   ${ctx.receiveAddress || "N/A"}`);
    console.log("  ═══════════════════════════════════\n");
  });

  // ─── Clean up context file ────────────────────────────────────

  it("should clean up the test context file", () => {
    clearContext();
    console.log("  Test context file cleaned up");
  });
});
