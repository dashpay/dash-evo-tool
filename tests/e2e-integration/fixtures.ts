/**
 * Playwright fixtures for E2E integration tests with mock IPC.
 *
 * Provides a `mockIPC` fixture that allows tests to:
 * - Configure mock responses for specific Tauri IPC commands
 * - Assert that commands were called with correct arguments
 * - Emit mock events to simulate backend-to-frontend communication
 *
 * Usage:
 *
 *   import { test, expect, createTestHdWallet } from "./fixtures";
 *
 *   test("wallet list renders", async ({ page, mockIPC }) => {
 *     const wallet = createTestHdWallet({ alias: "My Wallet" });
 *     // preconfigure sets handlers BEFORE page load (survives reload)
 *     await mockIPC.navigateWithHandlers("/wallets", {
 *       wallet_list_all: {
 *         hdWallets: [wallet],
 *         singleKeyWallets: [],
 *         selected: { type: "hd", seedHash: wallet.seedHash },
 *       },
 *     });
 *     await expect(page.getByText("My Wallet")).toBeVisible();
 *   });
 */

import { test as base, expect, type Page } from "@playwright/test";

// ---------------------------------------------------------------------------
// MockIPC helper class
// ---------------------------------------------------------------------------

class MockIPCHelper {
  private preConfiguredHandlers: Record<string, unknown> = {};

  constructor(private page: Page) {}

  /**
   * Wait for the mock IPC to be initialized in the browser.
   * Call this after page.goto() if you need to configure handlers
   * before the app makes IPC calls.
   */
  async waitForInit() {
    await this.page.waitForFunction(
      () => window.__E2E_MOCK_IPC__?.isInitialized === true,
      undefined,
      { timeout: 10_000 },
    );
  }

  /**
   * Pre-configure mock handlers BEFORE page load.
   * These handlers will survive page.goto() and page.reload() because
   * they're injected via addInitScript into window.__E2E_MOCK_OVERRIDES__
   * before the app initializes.
   *
   * Call this before page.goto() for handlers that need to be active
   * from the very first IPC call.
   *
   * Response values are RAW invoke return values (not wrapped in Result).
   */
  async preconfigure(handlers: Record<string, unknown>) {
    Object.assign(this.preConfiguredHandlers, handlers);
    const allHandlers = { ...this.preConfiguredHandlers };
    // Serialize to JSON string to avoid issues with complex object serialization
    // in addInitScript. The init script parses it back.
    const json = JSON.stringify(allHandlers);
    await this.page.addInitScript((jsonStr) => {
      (window as unknown as Record<string, unknown>).__E2E_MOCK_OVERRIDES__ = JSON.parse(jsonStr);
    }, json);
  }

  /**
   * Navigate to a route with pre-configured handlers active.
   * Convenience method that combines preconfigure + goto + waitForInit.
   */
  async navigateWithHandlers(path: string, handlers: Record<string, unknown>) {
    await this.preconfigure(handlers);
    await this.page.goto(path);
    await this.waitForInit();
  }

  /**
   * Set a mock response for a specific command on an already-loaded page.
   * Command names use snake_case (Tauri's internal format).
   *
   * NOTE: These handlers will NOT survive page.reload(). For handlers
   * that need to survive reloads, use preconfigure() instead.
   *
   * Response values are RAW invoke return values (not wrapped in Result).
   *
   * Common command names:
   * - wallet_list_all, wallet_get_hd, wallet_create
   * - identity_list_local, identity_list_summaries
   * - contract_list_local, contract_get_by_id
   * - settings_get, context_get_network
   * - token_load_order, token_query_my_balances
   * - contested_get_scheduled_votes
   */
  async setHandler(cmd: string, response: unknown) {
    await this.page.evaluate(
      ({ cmd, response }) => {
        window.__E2E_MOCK_IPC__!.setHandler(cmd, response);
      },
      { cmd, response },
    );
  }

  /**
   * Set multiple mock handlers at once on an already-loaded page.
   */
  async setHandlers(handlers: Record<string, unknown>) {
    await this.page.evaluate((handlers) => {
      window.__E2E_MOCK_IPC__!.setHandlers(handlers);
    }, handlers);
  }

  /**
   * Get the call history for a specific command.
   */
  async getCallHistory(cmd: string): Promise<unknown[]> {
    return this.page.evaluate((cmd) => {
      return window.__E2E_MOCK_IPC__!.getCallHistory(cmd);
    }, cmd);
  }

  /**
   * Get all call history across all commands.
   */
  async getAllCallHistory(): Promise<Record<string, unknown[]>> {
    return this.page.evaluate(() => {
      return window.__E2E_MOCK_IPC__!.getAllCallHistory();
    });
  }

  /**
   * Clear call history for a specific command or all commands.
   */
  async clearCallHistory(cmd?: string) {
    await this.page.evaluate((cmd) => {
      window.__E2E_MOCK_IPC__!.clearCallHistory(cmd ?? undefined);
    }, cmd ?? null);
  }

  /**
   * Reset all custom handlers and call history.
   */
  async reset() {
    await this.page.evaluate(() => {
      window.__E2E_MOCK_IPC__!.reset();
    });
  }

  /**
   * Emit a mock event to the frontend.
   * Event names use kebab-case (Tauri's event naming convention).
   *
   * Common event names:
   * - "task-result-event" — backend task results
   * - "task-error-event" — backend task errors
   * - "wallet-updated-event" — wallet state changes
   * - "spv-status-event" — SPV sync status
   * - "zmq-connection-status-event" — ZMQ connection state
   * - "zmq-chain-locked-block-event" — new chain-locked block
   * - "zmq-is-locked-transaction-event" — instant-locked transaction
   * - "scheduled-vote-executed-event" — scheduled vote result
   */
  async emitEvent(eventName: string, payload: unknown) {
    await this.page.evaluate(
      ({ eventName, payload }) => {
        window.__E2E_MOCK_IPC__!.emitEvent(eventName, payload);
      },
      { eventName, payload },
    );
  }

  /**
   * Navigate to a route and wait for the mock IPC to be ready.
   * This is a convenience method that combines page.goto() and waitForInit().
   */
  async navigateTo(path: string) {
    await this.page.goto(path);
    await this.waitForInit();
  }
}

// ---------------------------------------------------------------------------
// Custom test fixture
// ---------------------------------------------------------------------------

interface MockIPCFixtures {
  mockIPC: MockIPCHelper;
}

export const test = base.extend<MockIPCFixtures>({
  mockIPC: async ({ page }, use) => {
    const helper = new MockIPCHelper(page);
    await use(helper);
  },
});

export { expect };

// ---------------------------------------------------------------------------
// Test data helpers
// ---------------------------------------------------------------------------

/**
 * Create a mock HD wallet DTO for use in tests.
 * Uses realistic defaults that can be overridden.
 */
export function createTestHdWallet(overrides: Record<string, unknown> = {}) {
  return {
    seedHash: "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
    alias: "Test HD Wallet",
    identityRegistrations: [],
    accounts: [
      {
        accountIndex: 0,
        coreAddresses: [
          {
            address: "yXJqLJt8MWAMN2VSJQEuYAxzjXCzJEHpjF",
            path: "m/44'/1'/0'/0/0",
            balanceSatoshis: 500000000,
            isUsed: true,
          },
        ],
        platformAddresses: [],
      },
    ],
    utxos: [
      {
        address: "yXJqLJt8MWAMN2VSJQEuYAxzjXCzJEHpjF",
        txid: "abc123def456abc123def456abc123def456abc123def456abc123def456abc1",
        outputIndex: 0,
        satoshis: 500000000,
        scriptPubKey: "76a914abc12345678901234567890123456789012345678988ac",
      },
    ],
    assetLocks: [],
    ...overrides,
  };
}

/**
 * Create a mock single-key wallet DTO for use in tests.
 */
export function createTestSingleKeyWallet(overrides: Record<string, unknown> = {}) {
  return {
    keyHash: "b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3",
    alias: "Test Single Key Wallet",
    address: "yMockSingleKeyAddress123456789",
    balanceSatoshis: 100000000,
    utxos: [
      {
        address: "yMockSingleKeyAddress123456789",
        txid: "def456abc123def456abc123def456abc123def456abc123def456abc123def4",
        outputIndex: 0,
        satoshis: 100000000,
        scriptPubKey: "76a914def45678901234567890123456789012345678988ac",
      },
    ],
    ...overrides,
  };
}

/**
 * Create a mock identity DTO for use in tests.
 */
export function createTestIdentity(overrides: Record<string, unknown> = {}) {
  return {
    identityId: "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
    alias: "Test Identity",
    identityType: "User",
    balance: 1000000000,
    dpnsNames: ["testuser.dash"],
    keys: [
      {
        keyId: 0,
        purpose: "AUTHENTICATION",
        securityLevel: "MASTER",
        keyType: "ECDSA_SECP256K1",
        readOnly: false,
        disabled: false,
        publicKeyHex: "abc123",
        hasPrivateKey: true,
        contractBounds: null,
      },
    ],
    walletRef: { type: "hd", seedHash: "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2" },
    ...overrides,
  };
}

/**
 * Create a mock contested name DTO for use in tests.
 */
export function createTestContestedName(overrides: Record<string, unknown> = {}) {
  return {
    name: "testname",
    lockedVotes: 5,
    abstainVotes: 1,
    endTime: Date.now() + 86400000,
    lastUpdated: Date.now(),
    contestants: [
      {
        identityId: "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
        votes: 3,
      },
    ],
    awardedTo: null,
    ...overrides,
  };
}

/**
 * Create mock settings matching the SettingsDto type.
 */
export function createTestSettings(overrides: Record<string, unknown> = {}) {
  return {
    theme: "Dark",
    developerMode: false,
    disableZmq: false,
    onboardingCompleted: true,
    showEvonodeTools: false,
    userMode: "Basic",
    closeDashQtOnExit: false,
    autoStartSpv: false,
    ...overrides,
  };
}
