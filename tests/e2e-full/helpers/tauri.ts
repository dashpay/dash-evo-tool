/**
 * Tauri-specific helpers for WebdriverIO E2E tests.
 *
 * Provides utilities for waiting for app ready state, navigating via sidebar,
 * and waiting for IPC responses in the real Tauri app.
 */

/** Sidebar navigation section names matching data-testid attributes */
export type SidebarSection =
  | "wallets"
  | "identities"
  | "dpns"
  | "contracts"
  | "tokens"
  | "dashpay"
  | "tools"
  | "settings";

/**
 * Wait for the Tauri app to finish loading and render the main UI.
 * Detects either the sidebar (normal state) or the welcome screen (first launch).
 */
export async function waitForAppReady(timeout = 60_000): Promise<void> {
  await browser.waitUntil(
    async () => {
      // Check if the sidebar or welcome screen is visible
      const sidebar = await browser.$('[data-testid="sidebar"]');
      const welcome = await browser.$('[data-testid="welcome-screen"]');
      const networkChooser = await browser.$(
        '[data-testid="network-chooser-screen"]'
      );
      return (
        (await sidebar.isExisting()) ||
        (await welcome.isExisting()) ||
        (await networkChooser.isExisting())
      );
    },
    {
      timeout,
      timeoutMsg: `App did not render main UI within ${timeout}ms`,
      interval: 500,
    }
  );
}

/**
 * Navigate to a sidebar section by clicking its nav item.
 * Waits for the content area to update.
 */
export async function navigateToSection(
  section: SidebarSection
): Promise<void> {
  const navItem = await browser.$(
    `[data-testid="nav-${section}"], [data-testid="sidebar-${section}"]`
  );
  if (!(await navItem.isExisting())) {
    throw new Error(
      `Sidebar nav item for "${section}" not found. ` +
        `Looked for [data-testid="nav-${section}"] and [data-testid="sidebar-${section}"]`
    );
  }
  await navItem.click();

  // Wait for navigation to complete — verify URL changed.
  // This catches silent click failures caused by WebDriver instability.
  await browser.waitUntil(
    async () => {
      const url = await browser.getUrl();
      return url.includes(`/${section}`);
    },
    {
      timeout: 10_000,
      timeoutMsg: `Navigation to /${section} did not complete — click may not have registered`,
    }
  );
}

/**
 * Wait for a loading indicator to disappear, indicating data has loaded.
 */
export async function waitForDataLoad(timeout = 15_000): Promise<void> {
  // Wait for any loading spinners or skeletons to disappear
  const loadingSelectors = [
    '[data-testid="loading-spinner"]',
    '[data-testid="loading-skeleton"]',
    ".animate-pulse",
    '[role="progressbar"]',
  ];

  for (const selector of loadingSelectors) {
    const el = await browser.$(selector);
    if (await el.isExisting()) {
      await el.waitForExist({
        timeout,
        reverse: true, // Wait for it to NOT exist
        timeoutMsg: `Loading indicator ${selector} did not disappear within ${timeout}ms`,
      });
    }
  }
}

/**
 * Wait for a specific element to appear by its data-testid.
 */
export async function waitForTestId(
  testId: string,
  timeout = 10_000
): Promise<WebdriverIO.Element> {
  const selector = `[data-testid="${testId}"]`;
  const el = await browser.$(selector);
  await el.waitForExist({
    timeout,
    timeoutMsg: `Element [data-testid="${testId}"] did not appear within ${timeout}ms`,
  });
  // Return the resolved element
  return el as unknown as WebdriverIO.Element;
}

/**
 * Get the current page title from the top bar / breadcrumb.
 */
export async function getPageTitle(): Promise<string> {
  const titleEl = await browser.$(
    '[data-testid="page-title"], [data-testid="breadcrumb-current"]'
  );
  if (await titleEl.isExisting()) {
    return titleEl.getText();
  }
  return "";
}

/**
 * Get the displayed network badge text.
 */
export async function getNetworkBadge(): Promise<string> {
  const badge = await browser.$(
    '[data-testid="network-badge"], [data-testid="network-indicator"]'
  );
  if (await badge.isExisting()) {
    return badge.getText();
  }
  return "";
}

/**
 * Check if the connection status indicator shows connected.
 */
export async function isConnected(): Promise<boolean> {
  const indicator = await browser.$(
    '[data-testid="connection-status"], [data-testid="zmq-status"]'
  );
  if (await indicator.isExisting()) {
    const classes = await indicator.getAttribute("class");
    // Green = connected, red = disconnected
    return (
      classes?.includes("bg-green") || classes?.includes("text-green") || false
    );
  }
  return false;
}

/**
 * Click a button by its text content.
 */
export async function clickButton(text: string): Promise<void> {
  const button = await browser.$(`button=${text}`);
  await button.waitForClickable({ timeout: 5000 });
  await button.click();
}

/**
 * Fill an input field identified by data-testid.
 */
export async function fillInput(
  testId: string,
  value: string
): Promise<void> {
  const input = await browser.$(`[data-testid="${testId}"]`);
  await input.waitForExist({ timeout: 5000 });
  await input.clearValue();
  await input.setValue(value);
}

/**
 * Get text content of an element by data-testid.
 */
export async function getTextByTestId(testId: string): Promise<string> {
  const el = await browser.$(`[data-testid="${testId}"]`);
  if (await el.isExisting()) {
    return el.getText();
  }
  return "";
}

/**
 * Check if an element with the given data-testid exists.
 */
export async function existsByTestId(testId: string): Promise<boolean> {
  const el = await browser.$(`[data-testid="${testId}"]`);
  return el.isExisting();
}

/**
 * Wait for a toast/notification to appear with specific text.
 */
export async function waitForToast(
  text: string,
  timeout = 10_000
): Promise<void> {
  await browser.waitUntil(
    async () => {
      const toasts = await browser.$$(
        '[data-sonner-toast], [role="status"], [data-testid="toast"]'
      );
      for (const toast of toasts) {
        const toastText = await toast.getText();
        if (toastText.includes(text)) {
          return true;
        }
      }
      return false;
    },
    {
      timeout,
      timeoutMsg: `Toast with text "${text}" did not appear within ${timeout}ms`,
      interval: 500,
    }
  );
}

/**
 * Wait for a dialog/modal to appear.
 */
export async function waitForDialog(timeout = 5000): Promise<void> {
  const dialog = await browser.$('[role="dialog"], [data-testid="dialog"]');
  await dialog.waitForExist({
    timeout,
    timeoutMsg: `Dialog did not appear within ${timeout}ms`,
  });
}

/**
 * Dismiss a dialog by clicking its close button or pressing Escape.
 */
export async function dismissDialog(): Promise<void> {
  const closeBtn = await browser.$(
    '[data-testid="dialog-close"], [role="dialog"] button[aria-label="Close"]'
  );
  if (await closeBtn.isExisting()) {
    await closeBtn.click();
  } else {
    await browser.keys("Escape");
  }
  // Wait for dialog to disappear
  await browser.pause(300);
}

/**
 * Take a named screenshot (for debugging, not just on failure).
 */
export async function takeScreenshot(name: string): Promise<string> {
  const screenshotPath = `./test-results/screenshot-${name}-${Date.now()}.png`;
  await browser.saveScreenshot(screenshotPath);
  return screenshotPath;
}

/**
 * Wait for the connection status indicator to reach a target state.
 */
export async function waitForConnectionStatus(
  connected: boolean,
  timeout = 60_000
): Promise<void> {
  await browser.waitUntil(
    async () => {
      const actual = await isConnected();
      return actual === connected;
    },
    {
      timeout,
      timeoutMsg: `Connection status did not become ${connected ? "connected" : "disconnected"} within ${timeout}ms`,
      interval: 1000,
    }
  );
}
