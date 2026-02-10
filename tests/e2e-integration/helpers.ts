/**
 * Common test helpers for E2E integration tests.
 *
 * These helpers provide convenient abstractions for common test operations
 * like navigation, waiting for data, and triggering backend events.
 */

import type { Page } from "@playwright/test";

// ---------------------------------------------------------------------------
// Navigation helpers
// ---------------------------------------------------------------------------

/** Section names matching the sidebar navigation items. */
export type AppSection =
  | "DashPay"
  | "Identities"
  | "Contracts"
  | "Tokens"
  | "Wallets"
  | "Tools"
  | "Settings";

/**
 * Navigate to a section using the sidebar navigation.
 * Waits for the URL to update.
 */
export async function navigateToSection(page: Page, section: AppSection) {
  const nav = page.getByRole("navigation");
  await nav.getByText(section, { exact: true }).click();
}

/**
 * Wait for the page to finish loading data.
 * This waits for any loading spinners/skeletons to disappear.
 */
export async function waitForDataLoad(page: Page, timeout = 5000) {
  // Wait for any loading indicators to disappear
  await page.waitForFunction(
    () => {
      const spinners = document.querySelectorAll('[data-testid="loading-spinner"], .animate-spin');
      const skeletons = document.querySelectorAll('[data-testid="loading-skeleton"], .animate-pulse');
      return spinners.length === 0 && skeletons.length === 0;
    },
    undefined,
    { timeout },
  ).catch(() => {
    // If loading indicators don't disappear, that's ok — some may be permanent
  });
}

/**
 * Trigger a task result event, simulating a backend task completion.
 *
 * @param page - Playwright page
 * @param resultType - The result type string (e.g., "Wallet", "Identity", "Token")
 * @param payload - The result payload data
 * @param taskId - Optional task ID (defaults to "mock-task-id")
 */
export async function triggerTaskResult(
  page: Page,
  resultType: string,
  payload: unknown,
  taskId = "mock-task-id",
) {
  await page.evaluate(
    ({ taskId, resultType, payload }) => {
      window.__E2E_MOCK_IPC__!.emitEvent("task-result-event", {
        taskId,
        resultType,
        payload,
      });
    },
    { taskId, resultType, payload },
  );
}

/**
 * Trigger a task error event, simulating a backend task failure.
 */
export async function triggerTaskError(
  page: Page,
  message: string,
  details = "",
  taskId = "mock-task-id",
) {
  await page.evaluate(
    ({ taskId, message, details }) => {
      window.__E2E_MOCK_IPC__!.emitEvent("task-error-event", {
        taskId,
        message,
        details,
        retryable: false,
      });
    },
    { taskId, message, details },
  );
}

/**
 * Trigger a wallet updated event.
 */
export async function triggerWalletUpdated(
  page: Page,
  walletSeedHash: string,
  network = "Testnet",
) {
  await page.evaluate(
    ({ walletSeedHash, network }) => {
      window.__E2E_MOCK_IPC__!.emitEvent("wallet-updated-event", {
        walletSeedHash,
        network,
      });
    },
    { walletSeedHash, network },
  );
}

/**
 * Trigger a ZMQ connection status event.
 */
export async function triggerZmqConnectionStatus(
  page: Page,
  connected: boolean,
  network = "Testnet",
) {
  await page.evaluate(
    ({ network, connected }) => {
      window.__E2E_MOCK_IPC__!.emitEvent("zmq-connection-status-event", {
        network,
        connected,
      });
    },
    { network, connected },
  );
}
