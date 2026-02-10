/**
 * Setup Verification Tests — E2E Integration with Mock IPC
 *
 * These tests verify that the mock IPC infrastructure works correctly:
 * 1. Mock IPC initializes in the browser
 * 2. Default handlers return expected data
 * 3. Custom handlers can be pre-configured before navigation
 * 4. Call history is tracked
 * 5. Navigation works with mock data
 * 6. Events can be emitted
 *
 * IMPORTANT: Mock responses are RAW invoke return values.
 * bindings.ts adds the Result { status: "ok", data } wrapper itself.
 */

import {
  test,
  expect,
  createTestHdWallet,
  createTestSingleKeyWallet,
  createTestSettings,
} from "./fixtures";

test.describe("Mock IPC Infrastructure", () => {
  test("mock IPC initializes successfully", async ({ page, mockIPC }) => {
    await page.goto("/wallets");
    await mockIPC.waitForInit();

    const isInit = await page.evaluate(
      () => window.__E2E_MOCK_IPC__?.isInitialized,
    );
    expect(isInit).toBe(true);
  });

  test("default handlers respond to IPC calls", async ({ page, mockIPC }) => {
    await page.goto("/wallets");
    await mockIPC.waitForInit();
    await page.waitForTimeout(1000);

    const history = await mockIPC.getAllCallHistory();
    expect(Object.keys(history).length).toBeGreaterThan(0);
  });

  test("pre-configured handlers override defaults", async ({
    page,
    mockIPC,
  }) => {
    const wallet = createTestHdWallet({ alias: "Pre-Configured Wallet" });
    await mockIPC.navigateWithHandlers("/wallets", {
      wallet_list_all: {
        hdWallets: [wallet],
        singleKeyWallets: [],
        selected: null,
      },
    });

    await expect(page.getByText("Pre-Configured Wallet").first()).toBeVisible({
      timeout: 10000,
    });
  });

  test("call history tracks IPC invocations", async ({ page, mockIPC }) => {
    await page.goto("/wallets");
    await mockIPC.waitForInit();
    await page.waitForTimeout(1000);

    const walletCalls = await mockIPC.getCallHistory("wallet_list_all");
    expect(walletCalls.length).toBeGreaterThanOrEqual(1);
  });

  test("call history can be cleared", async ({ page, mockIPC }) => {
    await page.goto("/wallets");
    await mockIPC.waitForInit();
    await page.waitForTimeout(1000);

    const beforeClear = await mockIPC.getAllCallHistory();
    expect(Object.keys(beforeClear).length).toBeGreaterThan(0);

    await mockIPC.clearCallHistory();
    const afterClear = await mockIPC.getAllCallHistory();
    expect(Object.keys(afterClear).length).toBe(0);
  });

  test("reset clears handlers and call history", async ({
    page,
    mockIPC,
  }) => {
    await page.goto("/wallets");
    await mockIPC.waitForInit();
    await page.waitForTimeout(500);

    await mockIPC.reset();
    const history = await mockIPC.getAllCallHistory();
    expect(Object.keys(history).length).toBe(0);
  });
});

test.describe("Wallet Screen with Mock IPC", () => {
  test("wallet list renders HD wallets from mock data", async ({
    page,
    mockIPC,
  }) => {
    const hdWallet = createTestHdWallet({ alias: "Main HD Wallet" });
    await mockIPC.navigateWithHandlers("/wallets", {
      wallet_list_all: {
        hdWallets: [hdWallet],
        singleKeyWallets: [],
        selected: null,
      },
    });

    await expect(page.getByText("Main HD Wallet").first()).toBeVisible({
      timeout: 10000,
    });
  });

  test("wallet list renders single-key wallets from mock data", async ({
    page,
    mockIPC,
  }) => {
    const skWallet = createTestSingleKeyWallet({
      alias: "My Single Key",
    });
    await mockIPC.navigateWithHandlers("/wallets", {
      wallet_list_all: {
        hdWallets: [],
        singleKeyWallets: [skWallet],
        selected: { type: "singleKey", keyHash: skWallet.keyHash },
      },
    });

    await expect(page.getByText("My Single Key").first()).toBeVisible({
      timeout: 10000,
    });
  });

  test("empty wallet list shows empty state", async ({ page, mockIPC }) => {
    await page.goto("/wallets");
    await mockIPC.waitForInit();

    await expect(page.getByText("No Wallets Loaded")).toBeVisible({
      timeout: 5000,
    });
  });
});

test.describe("Navigation with Mock IPC", () => {
  test("app redirects past welcome when onboarding completed", async ({
    page,
    mockIPC,
  }) => {
    // Default settings_get returns onboardingCompleted: true
    await page.goto("/");
    await mockIPC.waitForInit();

    await page.waitForURL(/\/(identities|wallets|dashpay|contracts|tokens|tools|settings)/, {
      timeout: 10000,
    });
  });

  test("app shows welcome when onboarding not completed", async ({
    page,
    mockIPC,
  }) => {
    const settings = createTestSettings({ onboardingCompleted: false });
    await mockIPC.preconfigure({
      settings_get: settings,
    });

    await page.goto("/");
    await mockIPC.waitForInit();

    await page.waitForURL(/\/welcome/, { timeout: 10000 });
  });

  test("sidebar navigation works between sections", async ({
    page,
    mockIPC,
  }) => {
    await page.goto("/wallets");
    await mockIPC.waitForInit();

    const nav = page.getByRole("navigation");
    await nav.getByText("Identities", { exact: true }).click();
    await page.waitForURL(/\/identities/, { timeout: 5000 });

    await nav.getByText("Tokens", { exact: true }).click();
    await page.waitForURL(/\/tokens/, { timeout: 5000 });
  });
});
