/**
 * Phase 2 — App Shell & Design System Smoke Tests
 *
 * Verifies the app shell (sidebar, top bar, navigation), theme toggle,
 * welcome/onboarding screen, and network chooser/settings screen render
 * and function correctly with mock IPC data.
 */

import {
  test,
  expect,
  createTestSettings,
} from "./fixtures";
import { navigateToSection } from "./helpers";

// ---------------------------------------------------------------------------
// App Shell
// ---------------------------------------------------------------------------

test.describe("App Shell", () => {
  test("renders sidebar with all 7 navigation items", async ({
    page,
    mockIPC,
  }) => {
    await page.goto("/wallets");
    await mockIPC.waitForInit();

    const sidebar = page.getByTestId("sidebar");
    await expect(sidebar).toBeVisible();

    // Each nav item has data-testid="nav-{id}"
    await expect(page.getByTestId("nav-dashpay")).toBeVisible();
    await expect(page.getByTestId("nav-identities")).toBeVisible();
    await expect(page.getByTestId("nav-contracts")).toBeVisible();
    await expect(page.getByTestId("nav-tokens")).toBeVisible();
    await expect(page.getByTestId("nav-wallets")).toBeVisible();
    await expect(page.getByTestId("nav-tools")).toBeVisible();
    await expect(page.getByTestId("nav-settings")).toBeVisible();
  });

  test("renders top bar with connection indicator and breadcrumbs", async ({
    page,
    mockIPC,
  }) => {
    await page.goto("/wallets");
    await mockIPC.waitForInit();

    const topBar = page.getByTestId("top-bar");
    await expect(topBar).toBeVisible();

    // Connection indicator
    await expect(page.getByTestId("connection-indicator")).toBeVisible();

    // Breadcrumb should show "Wallets"
    await expect(topBar.getByText("Wallets")).toBeVisible();
  });

  test("shows Testnet network badge from mock", async ({ page, mockIPC }) => {
    await mockIPC.preconfigure({
      get_network_info: {
        activeNetwork: "testnet",
        availableNetworks: ["testnet"],
        coreVersion: "21.0.0",
        platformVersion: "1.0.0",
        connected: false,
      },
    });
    await page.goto("/wallets");
    await mockIPC.waitForInit();

    // The sidebar or top bar should display "Testnet" badge
    await expect(page.getByText("Testnet").first()).toBeVisible({
      timeout: 5000,
    });
  });

  test("content area renders outlet for the current route", async ({
    page,
    mockIPC,
  }) => {
    await page.goto("/wallets");
    await mockIPC.waitForInit();

    // Default wallet_list_all returns empty lists, so the wallet list panel
    // should show its empty state. The "No Wallets Loaded" text may be in
    // the DOM inside a scrollable region — check that the region exists.
    const walletRegion = page.getByRole("region", { name: "Wallet list" });
    await expect(walletRegion).toBeVisible({ timeout: 10000 });
    // Within the region, check the empty state text
    await expect(walletRegion.getByText("No Wallets Loaded")).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Navigation
// ---------------------------------------------------------------------------

test.describe("Navigation", () => {
  test("navigates to all 7 sections via sidebar", async ({
    page,
    mockIPC,
  }) => {
    await page.goto("/wallets");
    await mockIPC.waitForInit();

    const sections = [
      { label: "DashPay" as const, url: /\/dashpay/ },
      { label: "Identities" as const, url: /\/identities/ },
      { label: "Contracts" as const, url: /\/contracts/ },
      { label: "Tokens" as const, url: /\/tokens/ },
      { label: "Wallets" as const, url: /\/wallets/ },
      { label: "Tools" as const, url: /\/tools/ },
      { label: "Settings" as const, url: /\/settings/ },
    ];

    for (const { label, url } of sections) {
      await navigateToSection(page, label);
      await page.waitForURL(url, { timeout: 5000 });
    }
  });

  test("breadcrumbs update when navigating between sections", async ({
    page,
    mockIPC,
  }) => {
    await page.goto("/identities");
    await mockIPC.waitForInit();

    const topBar = page.getByTestId("top-bar");
    await expect(topBar.getByText("Identities")).toBeVisible();

    await navigateToSection(page, "Wallets");
    await expect(topBar.getByText("Wallets")).toBeVisible();

    await navigateToSection(page, "Tokens");
    await expect(topBar.getByText("Tokens")).toBeVisible();
  });

  test("active sidebar item has aria-current=page", async ({
    page,
    mockIPC,
  }) => {
    await page.goto("/wallets");
    await mockIPC.waitForInit();

    // The Wallets nav button should have aria-current="page"
    const walletsNav = page.getByTestId("nav-wallets");
    await expect(walletsNav).toHaveAttribute("aria-current", "page");

    // Navigate to Identities
    await navigateToSection(page, "Identities");
    await page.waitForURL(/\/identities/, { timeout: 5000 });

    // Now Identities should be current
    const identitiesNav = page.getByTestId("nav-identities");
    await expect(identitiesNav).toHaveAttribute("aria-current", "page");
  });

  test("sub-routes show nested breadcrumbs", async ({ page, mockIPC }) => {
    await page.goto("/wallets/create");
    await mockIPC.waitForInit();

    const topBar = page.getByTestId("top-bar");
    // Should show "Wallets" breadcrumb (clickable) and "Create" sub-breadcrumb
    await expect(topBar.getByText("Wallets")).toBeVisible();
    await expect(topBar.getByText("Create")).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Theme Toggle
// ---------------------------------------------------------------------------

test.describe("Theme", () => {
  test("top bar has interactive buttons", async ({
    page,
    mockIPC,
  }) => {
    await page.goto("/wallets");
    await mockIPC.waitForInit();

    // Top bar should have at least one button (theme toggle or connection indicator)
    const topBar = page.getByTestId("top-bar");
    const buttons = topBar.getByRole("button");
    const count = await buttons.count();
    expect(count).toBeGreaterThanOrEqual(1);
  });

  test("theme toggle dropdown shows Light, Dark, and System options", async ({
    page,
    mockIPC,
  }) => {
    await page.goto("/wallets");
    await mockIPC.waitForInit();

    // Find and click the theme toggle button (aria-label="Toggle theme")
    const themeToggle = page.getByRole("button", { name: "Toggle theme" });
    await expect(themeToggle).toBeVisible({ timeout: 5000 });
    await themeToggle.click();

    // Dropdown should show three options
    await expect(page.getByRole("menuitem", { name: "Light" })).toBeVisible({
      timeout: 3000,
    });
    await expect(page.getByRole("menuitem", { name: "Dark" })).toBeVisible();
    await expect(page.getByRole("menuitem", { name: "System" })).toBeVisible();
  });

  test("selecting Light theme applies light class to root element", async ({
    page,
    mockIPC,
  }) => {
    await page.goto("/wallets");
    await mockIPC.waitForInit();

    // Open theme dropdown and select Light
    const themeToggle = page.getByRole("button", { name: "Toggle theme" });
    await themeToggle.click();
    await page.getByRole("menuitem", { name: "Light" }).click();

    // Root element should have class "light"
    await expect(page.locator("html")).toHaveClass(/light/, { timeout: 3000 });
  });

  test("selecting Dark theme applies dark class to root element", async ({
    page,
    mockIPC,
  }) => {
    await page.goto("/wallets");
    await mockIPC.waitForInit();

    // Open theme dropdown and select Dark
    const themeToggle = page.getByRole("button", { name: "Toggle theme" });
    await themeToggle.click();
    await page.getByRole("menuitem", { name: "Dark" }).click();

    // Root element should have class "dark"
    await expect(page.locator("html")).toHaveClass(/dark/, { timeout: 3000 });
  });

  test("theme toggle persists via settings IPC call", async ({
    page,
    mockIPC,
  }) => {
    await page.goto("/wallets");
    await mockIPC.waitForInit();
    await mockIPC.clearCallHistory("system_update_theme");

    // Open theme dropdown and select Light
    const themeToggle = page.getByRole("button", { name: "Toggle theme" });
    await themeToggle.click();
    await page.getByRole("menuitem", { name: "Light" }).click();
    await page.waitForTimeout(500);

    // The theme update IPC should have been called
    const calls = await mockIPC.getCallHistory("system_update_theme");
    expect(calls.length).toBeGreaterThanOrEqual(1);
  });

  test("settings screen has theme selector in advanced settings", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.preconfigure({
      get_network_info: {
        activeNetwork: "testnet",
        availableNetworks: ["testnet"],
        coreVersion: "21.0.0",
        platformVersion: "1.0.0",
        connected: false,
      },
    });
    await page.goto("/settings");
    await mockIPC.waitForInit();

    // Open advanced settings
    await page.getByTestId("advanced-settings-toggle").click();

    // Theme selector should be visible
    await expect(page.getByText("Theme:")).toBeVisible({ timeout: 3000 });
    await expect(page.getByTestId("theme-select-trigger")).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Welcome / Onboarding
// ---------------------------------------------------------------------------

test.describe("Welcome Screen", () => {
  test("shows welcome screen when onboarding not completed", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.preconfigure({
      settings_get: createTestSettings({ onboardingCompleted: false }),
    });
    await page.goto("/");
    await mockIPC.waitForInit();

    await page.waitForURL(/\/welcome/, { timeout: 10000 });

    await expect(
      page.getByText("Welcome to Dash Evo Tool"),
    ).toBeVisible();
  });

  test("renders all 3 action cards", async ({ page, mockIPC }) => {
    await mockIPC.preconfigure({
      settings_get: createTestSettings({ onboardingCompleted: false }),
    });
    await page.goto("/welcome");
    await mockIPC.waitForInit();

    await expect(page.getByText("Create Wallet")).toBeVisible();
    await expect(page.getByText("Import Wallet")).toBeVisible();
    await expect(page.getByText("Just Explore")).toBeVisible();
  });

  test("clicking Create Wallet navigates to wallets and marks onboarding done", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.preconfigure({
      settings_get: createTestSettings({ onboardingCompleted: false }),
    });
    await page.goto("/welcome");
    await mockIPC.waitForInit();

    await page.getByText("Create Wallet").click();

    await page.waitForURL(/\/wallets/, { timeout: 10000 });

    // Verify the onboarding completed IPC was called
    const calls = await mockIPC.getCallHistory(
      "settings_update_onboarding_completed",
    );
    expect(calls.length).toBeGreaterThanOrEqual(1);
  });

  test("clicking Just Explore navigates to identities", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.preconfigure({
      settings_get: createTestSettings({ onboardingCompleted: false }),
    });
    await page.goto("/welcome");
    await mockIPC.waitForInit();

    await page.getByText("Just Explore").click();

    await page.waitForURL(/\/identities/, { timeout: 10000 });
  });

  test("skips welcome screen when onboarding already completed", async ({
    page,
    mockIPC,
  }) => {
    // Default settings have onboardingCompleted: true
    await page.goto("/");
    await mockIPC.waitForInit();

    // Should redirect past welcome
    await page.waitForURL(
      /\/(identities|wallets|dashpay|contracts|tokens|tools|settings)/,
      { timeout: 10000 },
    );
  });
});

// ---------------------------------------------------------------------------
// Network Chooser / Settings Screen
// ---------------------------------------------------------------------------

test.describe("Network Chooser Screen", () => {
  test("renders connection settings section", async ({ page, mockIPC }) => {
    await mockIPC.preconfigure({
      get_network_info: {
        activeNetwork: "testnet",
        availableNetworks: ["testnet", "dash"],
        coreVersion: "21.0.0",
        platformVersion: "1.0.0",
        connected: false,
      },
    });
    await page.goto("/settings");
    await mockIPC.waitForInit();

    await expect(page.getByTestId("network-chooser-screen")).toBeVisible();
    await expect(page.getByText("Connection Settings")).toBeVisible();
  });

  test("renders connection status section", async ({ page, mockIPC }) => {
    await mockIPC.preconfigure({
      get_network_info: {
        activeNetwork: "testnet",
        availableNetworks: ["testnet"],
        coreVersion: "21.0.0",
        platformVersion: "1.0.0",
        connected: false,
      },
    });
    await page.goto("/settings");
    await mockIPC.waitForInit();

    await expect(page.getByText("Connection Status")).toBeVisible();
  });

  test("advanced settings toggle expands and collapses", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.preconfigure({
      get_network_info: {
        activeNetwork: "testnet",
        availableNetworks: ["testnet"],
        coreVersion: "21.0.0",
        platformVersion: "1.0.0",
        connected: false,
      },
    });
    await page.goto("/settings");
    await mockIPC.waitForInit();

    const toggle = page.getByTestId("advanced-settings-toggle");
    await expect(toggle).toBeVisible();

    // Initially collapsed — Theme selector should not be visible
    await expect(page.getByText("Theme:")).not.toBeVisible();

    // Click to expand
    await toggle.click();
    await expect(page.getByText("Theme:")).toBeVisible({ timeout: 3000 });

    // Click again to collapse
    await toggle.click();
    await expect(page.getByText("Theme:")).not.toBeVisible();
  });

  test("displays developer mode toggle in advanced settings", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.preconfigure({
      get_network_info: {
        activeNetwork: "testnet",
        availableNetworks: ["testnet"],
        coreVersion: "21.0.0",
        platformVersion: "1.0.0",
        connected: false,
      },
    });
    await page.goto("/settings");
    await mockIPC.waitForInit();

    // Open advanced settings
    await page.getByTestId("advanced-settings-toggle").click();

    // Developer mode toggle should be visible
    await expect(page.getByText("Developer Mode")).toBeVisible({
      timeout: 3000,
    });
  });

  test("displays connection type selection in developer mode", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.preconfigure({
      get_network_info: {
        activeNetwork: "testnet",
        availableNetworks: ["testnet"],
        coreVersion: "21.0.0",
        platformVersion: "1.0.0",
        connected: false,
      },
      context_is_developer_mode: true,
    });
    await page.goto("/settings");
    await mockIPC.waitForInit();

    // Connection Type selector is visible in developer mode
    await expect(
      page.getByText("Connection Type:"),
    ).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByTestId("connection-type-trigger"),
    ).toBeVisible();
  });

  test("settings_get IPC is called on app load", async ({
    page,
    mockIPC,
  }) => {
    await page.goto("/wallets");
    await mockIPC.waitForInit();
    await page.waitForTimeout(1000);

    const calls = await mockIPC.getCallHistory("settings_get");
    expect(calls.length).toBeGreaterThanOrEqual(1);
  });
});
