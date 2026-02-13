/**
 * Low-level faucet automation using Playwright.
 *
 * Launches a real Chromium browser to interact with the Dash testnet faucet
 * (faucet.thepasta.org). The faucet uses a custom JS proof-of-work captcha
 * ("Cap") that auto-solves in a real browser context.
 *
 * Runs in Node.js context (not the Tauri WebView).
 */

import { chromium } from "playwright";

const FAUCET_URL =
  process.env.E2E_FAUCET_URL || "https://faucet.thepasta.org";

/** Maximum number of retry attempts. */
const MAX_RETRIES = 3;

/** Backoff delays in ms for each retry (10s, 20s, 40s). */
const BACKOFF_MS = [10_000, 20_000, 40_000];

export interface FaucetResult {
  success: boolean;
  txid?: string;
  error?: string;
}

/**
 * Request funds from the testnet faucet for the given address.
 *
 * Opens a headless Chromium browser, fills in the address, clicks submit,
 * and waits for the result. Retries with exponential backoff on failure.
 *
 * @param address - Testnet address to fund (starts with `y`)
 * @returns Result with success status and optional txid or error
 */
export async function requestFaucet(address: string): Promise<FaucetResult> {
  let lastError = "";

  for (let attempt = 0; attempt < MAX_RETRIES; attempt++) {
    if (attempt > 0) {
      const delay = BACKOFF_MS[attempt - 1] ?? 40_000;
      console.log(
        `  Faucet retry ${attempt + 1}/${MAX_RETRIES} after ${delay / 1000}s backoff...`
      );
      await sleep(delay);
    }

    const result = await attemptFaucet(address);
    if (result.success) return result;

    lastError = result.error ?? "Unknown error";
    console.warn(`  Faucet attempt ${attempt + 1} failed: ${lastError}`);
  }

  return { success: false, error: `All ${MAX_RETRIES} attempts failed. Last error: ${lastError}` };
}

async function attemptFaucet(address: string): Promise<FaucetResult> {
  let browser;
  try {
    browser = await chromium.launch({
      headless: true,
      args: [
        "--no-sandbox",
        "--disable-setuid-sandbox",
        "--disable-dev-shm-usage",
      ],
    });

    const page = await browser.newPage();

    // Navigate to faucet
    await page.goto(FAUCET_URL, { waitUntil: "networkidle", timeout: 30_000 });

    // The faucet page has an initial view with a "Get tDash" button that must
    // be clicked first to reveal the config view containing the address input.
    // Click the "Get tDash" button in .initial-view to show the config view.
    await page.waitForSelector("#coreFaucetCard .initial-view .btn", {
      state: "visible",
      timeout: 15_000,
    });
    await page.click("#coreFaucetCard .initial-view .btn");

    // Wait for the address input to be visible in the config view
    await page.waitForSelector("#addressInput", { state: "visible", timeout: 15_000 });

    // Fill in the testnet address
    await page.fill("#addressInput", address);

    // Click the "Send" button (#coreFaucetBtn) in the config view
    await page.click("#coreFaucetBtn");

    // Wait for either success (txid appears) or error
    // The captcha solving + API call can take up to 60s
    const result = await Promise.race([
      waitForTxid(page),
      waitForError(page),
      timeout(90_000),
    ]);

    return result;
  } catch (err) {
    return {
      success: false,
      error: err instanceof Error ? err.message : String(err),
    };
  } finally {
    if (browser) {
      await browser.close().catch(() => {});
    }
  }
}

async function waitForTxid(page: import("playwright").Page): Promise<FaucetResult> {
  try {
    await page.waitForFunction(
      () => {
        const el = document.querySelector("#coreTxid");
        return el && el.textContent && el.textContent.trim().length > 0;
      },
      { timeout: 90_000 }
    );

    const txid = await page.$eval("#coreTxid", (el) => el.textContent?.trim() ?? "");
    return { success: true, txid };
  } catch {
    // Page closed or timed out — let the race winner decide
    return { success: false, error: "Txid wait cancelled" };
  }
}

async function waitForError(page: import("playwright").Page): Promise<FaucetResult> {
  try {
    await page.waitForFunction(
      () => {
        const el = document.querySelector("#coreErrorBox");
        if (!el) return false;
        const style = window.getComputedStyle(el);
        return (
          style.display !== "none" &&
          el.textContent &&
          el.textContent.trim().length > 0
        );
      },
      { timeout: 90_000 }
    );

    const errorText = await page.$eval(
      "#coreErrorBox",
      (el) => el.textContent?.trim() ?? ""
    );
    return { success: false, error: errorText };
  } catch {
    // Page closed or timed out — let the race winner decide
    return { success: false, error: "Error wait cancelled" };
  }
}

function timeout(ms: number): Promise<FaucetResult> {
  return new Promise((resolve) =>
    setTimeout(
      () => resolve({ success: false, error: `Timed out after ${ms / 1000}s` }),
      ms
    )
  );
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
