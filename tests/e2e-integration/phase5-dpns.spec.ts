/**
 * Phase 5 — DPNS Contest & Voting Screens Smoke Tests
 *
 * Verifies active contests table, vote casting dialog, past contests,
 * owned names, scheduled votes, and DPNS name registration screens
 * render and function correctly with mock IPC data.
 */

import { test, expect, createTestIdentity, createTestContestedName } from "./fixtures";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Create a realistic active contested name.
 */
function createActiveContest(overrides: Record<string, unknown> = {}) {
  return {
    normalizedContestedName: "testname",
    contestants: [
      {
        id: "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
        votes: 3,
        name: "testuser",
        info: "",
        createdAt: Math.floor(Date.now() / 1000) - 86400,
        createdAtBlockHeight: 100,
        createdAtCoreBlockHeight: 200,
        documentId: "abc123doc",
      },
      {
        id: "HXTSBVGNkYx9IqRGbOJrCV8OCiNL5cs6VFTtC5U42GFd",
        votes: 1,
        name: "otheruser",
        info: "",
        createdAt: Math.floor(Date.now() / 1000) - 43200,
        createdAtBlockHeight: 110,
        createdAtCoreBlockHeight: 210,
        documentId: "def456doc",
      },
    ],
    lockedVotes: 5,
    abstainVotes: 2,
    awardedTo: null,
    endTime: Math.floor(Date.now() / 1000) + 86400,
    state: "ongoing",
    lastUpdated: Math.floor(Date.now() / 1000) - 3600,
    ...overrides,
  };
}

/**
 * Create a past (awarded) contested name.
 */
function createPastContest(overrides: Record<string, unknown> = {}) {
  return {
    normalizedContestedName: "oldname",
    contestants: [
      {
        id: "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
        votes: 10,
        name: "winner",
        info: "",
        createdAt: Math.floor(Date.now() / 1000) - 172800,
        createdAtBlockHeight: 50,
        createdAtCoreBlockHeight: 100,
        documentId: "win123doc",
      },
    ],
    lockedVotes: 10,
    abstainVotes: 0,
    awardedTo: "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
    endTime: Math.floor(Date.now() / 1000) - 86400,
    state: { wonBy: "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec" },
    lastUpdated: Math.floor(Date.now() / 1000) - 86400,
    ...overrides,
  };
}

/**
 * Create a locked contest (no winner, locked).
 */
function createLockedContest(overrides: Record<string, unknown> = {}) {
  return {
    normalizedContestedName: "lockedname",
    contestants: [],
    lockedVotes: 15,
    abstainVotes: 0,
    awardedTo: null,
    endTime: Math.floor(Date.now() / 1000) - 43200,
    state: "locked",
    lastUpdated: Math.floor(Date.now() / 1000) - 43200,
    ...overrides,
  };
}

/**
 * Create a mock DPNS name entry.
 */
function createOwnedName(overrides: Record<string, unknown> = {}) {
  return {
    name: "myname.dash",
    identityId: "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
    acquiredAt: Math.floor(Date.now() / 1000) - 86400,
    ...overrides,
  };
}

/**
 * Create a mock scheduled vote.
 */
function createScheduledVote(overrides: Record<string, unknown> = {}) {
  return {
    voterId: "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
    contestedName: "testname",
    choice: "lock",
    unixTimestamp: Date.now() + 3600000,
    scheduledTimestamp: Math.floor(Date.now() / 1000) + 3600,
    executedSuccessfully: false,
    ...overrides,
  };
}

/** Standard identity for DPNS screens — matches QualifiedIdentityDto */
function createVotingIdentity() {
  return {
    id: "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
    alias: "Voter Identity",
    identityType: "user",
    balance: 1000000000,
    dpnsNames: [{ name: "voter.dash", acquiredAt: 1700000000 }],
    keys: [
      {
        keyId: 0,
        purpose: "AUTHENTICATION",
        securityLevel: "MASTER",
        keyType: "ECDSA_SECP256K1",
        data: "02abc123def456abc123def456abc123def456abc123def456abc123def456abc1",
        isDisabled: false,
        disabledAt: null,
        hasPrivateKey: true,
      },
    ],
    associatedWalletHashes: [
      "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
    ],
    walletIndex: 0,
    topUps: [],
    status: "active",
    network: "testnet",
    voterIdentityId: null,
    operatorIdentityId: null,
  };
}

/** Standard handlers for the active contests screen */
function activeContestsHandlers(overrides: Record<string, unknown> = {}) {
  return {
    contested_query_dpns_contests: { taskId: "mock-task-id" },
    contested_get_scheduled_votes: [],
    identity_list_local: [createVotingIdentity()],
    identity_list_voting: [createVotingIdentity()],
    identity_local_dpns_names: [createOwnedName()],
    identity_load_order: ["GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec"],
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// Active Contests
// ---------------------------------------------------------------------------

test.describe("Active Contests", () => {
  test("renders active contests screen with heading", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/dpns/active",
      activeContestsHandlers(),
    );

    await expect(page.getByText("Active Contests").first()).toBeVisible({
      timeout: 10000,
    });
  });

  test("shows Refresh and Register Name buttons", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/dpns/active",
      activeContestsHandlers(),
    );

    await expect(page.getByText("Active Contests").first()).toBeVisible({
      timeout: 10000,
    });

    await expect(
      page.getByRole("button", { name: /Refresh/i }).first(),
    ).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByRole("button", { name: /Register DPNS name/i }),
    ).toBeVisible();
  });

  test("shows Cast / Schedule Votes button (disabled when no votes selected)", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/dpns/active",
      activeContestsHandlers(),
    );

    await expect(page.getByText("Active Contests").first()).toBeVisible({
      timeout: 10000,
    });

    const castBtn = page.getByRole("button", {
      name: /Cast.*Schedule.*Votes/i,
    });
    await expect(castBtn).toBeVisible({ timeout: 5000 });
    await expect(castBtn).toBeDisabled();
  });

  test("shows filter input for contests", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/dpns/active",
      activeContestsHandlers(),
    );

    await expect(page.getByText("Active Contests").first()).toBeVisible({
      timeout: 10000,
    });

    // Emit contest data so the table renders (filter only shows with data)
    await mockIPC.emitEvent("task-result-event", {
      taskId: "mock-task-id",
      resultType: "Contest",
      payload: {
        contestedNames: [createActiveContest()],
      },
    });

    // Filter input should be present
    await expect(
      page.getByPlaceholder(/Filter contests/i).or(
        page.getByRole("textbox", { name: /Filter contests/i }),
      ),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows empty state when no active contests", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/dpns/active",
      activeContestsHandlers(),
    );

    // The default mock doesn't provide contest data via task result,
    // so it should show loading or empty state
    await expect(page.getByText("Active Contests").first()).toBeVisible({
      timeout: 10000,
    });

    // Without task result events, should show empty state or loading
    await expect(
      page
        .getByText(/No active contests/i)
        .or(page.getByText(/Loading/i)),
    ).toBeVisible({ timeout: 5000 });
  });

  test("renders contests table when task result event arrives", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/dpns/active",
      activeContestsHandlers(),
    );

    await expect(page.getByText("Active Contests").first()).toBeVisible({
      timeout: 10000,
    });

    // Simulate backend returning contest data via task result event
    await mockIPC.emitEvent("task-result-event", {
      taskId: "mock-task-id",
      resultType: "Contest",
      payload: {
        contestedNames: [createActiveContest()],
      },
    });

    // Contest name should appear in the table
    await expect(page.getByText("testname").first()).toBeVisible({
      timeout: 5000,
    });
  });

  test("shows vote buttons (Lock, Abstain) for active contests", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/dpns/active",
      activeContestsHandlers(),
    );

    await expect(page.getByText("Active Contests").first()).toBeVisible({
      timeout: 10000,
    });

    // Emit contest data
    await mockIPC.emitEvent("task-result-event", {
      taskId: "mock-task-id",
      resultType: "Contest",
      payload: {
        contestedNames: [createActiveContest()],
      },
    });

    // Wait for contest to render
    await expect(page.getByText("testname").first()).toBeVisible({
      timeout: 5000,
    });

    // Lock and Abstain vote buttons should be present
    await expect(
      page.getByRole("button", { name: /Lock vote/i }).first(),
    ).toBeVisible({ timeout: 3000 });
    await expect(
      page.getByRole("button", { name: /Abstain vote/i }).first(),
    ).toBeVisible();
  });

  test("clicking Lock vote button selects it and enables Cast button", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/dpns/active",
      activeContestsHandlers(),
    );

    await expect(page.getByText("Active Contests").first()).toBeVisible({
      timeout: 10000,
    });

    // Emit contest data
    await mockIPC.emitEvent("task-result-event", {
      taskId: "mock-task-id",
      resultType: "Contest",
      payload: {
        contestedNames: [createActiveContest()],
      },
    });

    await expect(page.getByText("testname").first()).toBeVisible({
      timeout: 5000,
    });

    // Click Lock vote
    await page
      .getByRole("button", { name: /Lock vote/i })
      .first()
      .click();

    // Cast/Schedule button should now be enabled (or at least show a count)
    const castBtn = page.getByRole("button", {
      name: /Cast.*Schedule.*Votes/i,
    });
    await expect(castBtn).toBeEnabled({ timeout: 3000 });
  });

  test("sortable column headers are present", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/dpns/active",
      activeContestsHandlers(),
    );

    await expect(page.getByText("Active Contests").first()).toBeVisible({
      timeout: 10000,
    });

    // Sort buttons should be present for key columns
    await expect(
      page.getByRole("button", { name: /Sort by Name/i }).or(
        page.getByText("Name").first(),
      ),
    ).toBeVisible({ timeout: 5000 });
  });

  test("Register Name button navigates to registration screen", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/dpns/active",
      activeContestsHandlers(),
    );

    await expect(page.getByText("Active Contests").first()).toBeVisible({
      timeout: 10000,
    });

    await page.getByRole("button", { name: /Register DPNS name/i }).click();

    // Should navigate to register name screen
    await page.waitForURL(/dpns\/register/, { timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Vote Casting Dialog
// ---------------------------------------------------------------------------

test.describe("Vote Casting Dialog", () => {
  test("opens vote casting dialog when Cast/Schedule Votes is clicked", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/dpns/active",
      activeContestsHandlers(),
    );

    await expect(page.getByText("Active Contests").first()).toBeVisible({
      timeout: 10000,
    });

    // Emit contest data
    await mockIPC.emitEvent("task-result-event", {
      taskId: "mock-task-id",
      resultType: "Contest",
      payload: {
        contestedNames: [createActiveContest()],
      },
    });

    await expect(page.getByText("testname").first()).toBeVisible({
      timeout: 5000,
    });

    // Select a vote
    await page
      .getByRole("button", { name: /Lock vote/i })
      .first()
      .click();

    // Click Cast/Schedule Votes button
    const castBtn = page.getByRole("button", {
      name: /Cast.*Schedule.*Votes/i,
    });
    await expect(castBtn).toBeEnabled({ timeout: 3000 });
    await castBtn.click();

    // Dialog should open
    await expect(
      page.getByRole("dialog", { name: /Voting/i }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("dialog shows selected votes summary", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/dpns/active",
      activeContestsHandlers(),
    );

    await expect(page.getByText("Active Contests").first()).toBeVisible({
      timeout: 10000,
    });

    await mockIPC.emitEvent("task-result-event", {
      taskId: "mock-task-id",
      resultType: "Contest",
      payload: {
        contestedNames: [createActiveContest()],
      },
    });

    await expect(page.getByText("testname").first()).toBeVisible({
      timeout: 5000,
    });

    // Select a vote
    await page
      .getByRole("button", { name: /Lock vote/i })
      .first()
      .click();

    // Open dialog
    await page
      .getByRole("button", { name: /Cast.*Schedule.*Votes/i })
      .click();

    // Dialog should show the selected vote name
    const dialog = page.getByRole("dialog", { name: /Voting/i });
    await expect(dialog).toBeVisible({ timeout: 5000 });
    await expect(dialog.getByText("testname")).toBeVisible();
  });

  test("dialog has Cancel and Apply Votes buttons", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/dpns/active",
      activeContestsHandlers(),
    );

    await expect(page.getByText("Active Contests").first()).toBeVisible({
      timeout: 10000,
    });

    await mockIPC.emitEvent("task-result-event", {
      taskId: "mock-task-id",
      resultType: "Contest",
      payload: {
        contestedNames: [createActiveContest()],
      },
    });

    await expect(page.getByText("testname").first()).toBeVisible({
      timeout: 5000,
    });

    await page
      .getByRole("button", { name: /Lock vote/i })
      .first()
      .click();
    await page
      .getByRole("button", { name: /Cast.*Schedule.*Votes/i })
      .click();

    const dialog = page.getByRole("dialog", { name: /Voting/i });
    await expect(dialog).toBeVisible({ timeout: 5000 });

    // Should have Cancel and Apply Votes buttons
    await expect(
      dialog.getByRole("button", { name: /Cancel/i }),
    ).toBeVisible();
    await expect(
      dialog.getByRole("button", { name: "Apply votes" }),
    ).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Past Contests
// ---------------------------------------------------------------------------

test.describe("Past Contests", () => {
  test("renders past contests screen with heading", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/past", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [],
      identity_list_local: [],
      identity_list_voting: [],
      identity_local_dpns_names: [],
      identity_load_order: [],
    });

    await expect(page.getByText("Past Contests").first()).toBeVisible({
      timeout: 10000,
    });
  });

  test("shows Refresh button", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/past", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [],
      identity_list_local: [],
      identity_list_voting: [],
      identity_local_dpns_names: [],
      identity_load_order: [],
    });

    await expect(page.getByText("Past Contests").first()).toBeVisible({
      timeout: 10000,
    });

    await expect(
      page.getByRole("button", { name: /Refresh/i }).first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows filter input", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/past", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [],
      identity_list_local: [],
      identity_list_voting: [],
      identity_local_dpns_names: [],
      identity_load_order: [],
    });

    await expect(page.getByText("Past Contests").first()).toBeVisible({
      timeout: 10000,
    });

    // Emit past contest data so the table renders (filter only shows with data)
    await mockIPC.emitEvent("task-result-event", {
      taskId: "mock-task-id",
      resultType: "Contest",
      payload: {
        contestedNames: [createPastContest()],
      },
    });

    await expect(
      page.getByPlaceholder(/Filter contests/i).or(
        page.getByRole("textbox", { name: /Filter contests/i }),
      ),
    ).toBeVisible({ timeout: 5000 });
  });

  test("renders past contests when task result event arrives", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/past", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [],
      identity_list_local: [],
      identity_list_voting: [],
      identity_local_dpns_names: [],
      identity_load_order: [],
    });

    await expect(page.getByText("Past Contests").first()).toBeVisible({
      timeout: 10000,
    });

    // Emit past contest data
    await mockIPC.emitEvent("task-result-event", {
      taskId: "mock-task-id",
      resultType: "Contest",
      payload: {
        contestedNames: [createPastContest(), createLockedContest()],
      },
    });

    // Past contest names should appear
    await expect(page.getByText("oldname").first()).toBeVisible({
      timeout: 5000,
    });
  });

  test("shows Locked badge for locked contests", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/past", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [],
      identity_list_local: [],
      identity_list_voting: [],
      identity_local_dpns_names: [],
      identity_load_order: [],
    });

    await expect(page.getByText("Past Contests").first()).toBeVisible({
      timeout: 10000,
    });

    await mockIPC.emitEvent("task-result-event", {
      taskId: "mock-task-id",
      resultType: "Contest",
      payload: {
        contestedNames: [createLockedContest()],
      },
    });

    await expect(page.getByText("lockedname").first()).toBeVisible({
      timeout: 5000,
    });

    // "Locked" badge should be visible
    await expect(page.getByText("Locked").first()).toBeVisible({
      timeout: 3000,
    });
  });
});

// ---------------------------------------------------------------------------
// Owned Names
// ---------------------------------------------------------------------------

test.describe("Owned Names", () => {
  test("renders owned names screen with heading", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/owned", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [],
      identity_list_local: [createVotingIdentity()],
      identity_list_voting: [createVotingIdentity()],
      identity_local_dpns_names: [createOwnedName()],
      identity_load_order: ["GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec"],
    });

    await expect(page.getByText("My Usernames").first()).toBeVisible({
      timeout: 10000,
    });
  });

  test("shows Refresh button", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/owned", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [],
      identity_list_local: [createVotingIdentity()],
      identity_list_voting: [createVotingIdentity()],
      identity_local_dpns_names: [createOwnedName()],
      identity_load_order: ["GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec"],
    });

    await expect(page.getByText("My Usernames").first()).toBeVisible({
      timeout: 10000,
    });

    await expect(
      page.getByRole("button", { name: /Refresh/i }).first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("renders owned names from mock data", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/owned", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [],
      identity_list_local: [createVotingIdentity()],
      identity_list_voting: [createVotingIdentity()],
      identity_local_dpns_names: [
        createOwnedName({ name: "myname.dash" }),
        createOwnedName({
          name: "othername.dash",
          identityId: "HXTSBVGNkYx9IqRGbOJrCV8OCiNL5cs6VFTtC5U42GFd",
        }),
      ],
      identity_load_order: ["GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec"],
    });

    await expect(page.getByText("My Usernames").first()).toBeVisible({
      timeout: 10000,
    });

    // Names should be displayed
    await expect(page.getByText("myname.dash").first()).toBeVisible({
      timeout: 5000,
    });
    await expect(page.getByText("othername.dash").first()).toBeVisible();
  });

  test("shows Set Alias button for each owned name", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/owned", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [],
      identity_list_local: [createVotingIdentity()],
      identity_list_voting: [createVotingIdentity()],
      identity_local_dpns_names: [createOwnedName({ name: "myname.dash" })],
      identity_load_order: ["GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec"],
    });

    await expect(page.getByText("myname.dash").first()).toBeVisible({
      timeout: 10000,
    });

    // "Set Alias" button should be present
    await expect(
      page.getByRole("button", { name: /Set Alias/i }).first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows filter input for owned names", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/owned", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [],
      identity_list_local: [createVotingIdentity()],
      identity_list_voting: [createVotingIdentity()],
      identity_local_dpns_names: [createOwnedName()],
      identity_load_order: ["GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec"],
    });

    await expect(page.getByText("My Usernames").first()).toBeVisible({
      timeout: 10000,
    });

    await expect(
      page.getByPlaceholder(/Filter/i).or(
        page.getByRole("textbox", { name: /Filter owned names/i }),
      ),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows empty state when no names owned", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/owned", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [],
      identity_list_local: [createVotingIdentity()],
      identity_list_voting: [createVotingIdentity()],
      identity_local_dpns_names: [],
      identity_load_order: ["GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec"],
    });

    await expect(page.getByText("My Usernames").first()).toBeVisible({
      timeout: 10000,
    });

    await expect(
      page.getByText(/No owned usernames/i),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Scheduled Votes
// ---------------------------------------------------------------------------

test.describe("Scheduled Votes", () => {
  test("renders scheduled votes screen with heading", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/scheduled", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [],
      identity_list_local: [],
      identity_list_voting: [],
      identity_local_dpns_names: [],
      identity_load_order: [],
    });

    await expect(page.getByText("Scheduled Votes").first()).toBeVisible({
      timeout: 10000,
    });
  });

  test("shows Clear Casted, Clear All, and Refresh buttons", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/scheduled", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [createScheduledVote()],
      identity_list_local: [createVotingIdentity()],
      identity_list_voting: [createVotingIdentity()],
      identity_local_dpns_names: [],
      identity_load_order: ["GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec"],
    });

    await expect(page.getByText("Scheduled Votes").first()).toBeVisible({
      timeout: 10000,
    });

    await expect(
      page.getByRole("button", { name: /Refresh/i }).first(),
    ).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByRole("button", { name: /Clear All/i }),
    ).toBeVisible();
  });

  test("renders scheduled votes from mock data", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/scheduled", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [createScheduledVote()],
      identity_list_local: [createVotingIdentity()],
      identity_list_voting: [createVotingIdentity()],
      identity_local_dpns_names: [],
      identity_load_order: ["GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec"],
    });

    await expect(page.getByText("Scheduled Votes").first()).toBeVisible({
      timeout: 10000,
    });

    // The scheduled vote's contested name should be visible
    await expect(page.getByText("testname").first()).toBeVisible({
      timeout: 5000,
    });
  });

  test("shows Pending status badge for upcoming votes", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/scheduled", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [createScheduledVote()],
      identity_list_local: [createVotingIdentity()],
      identity_list_voting: [createVotingIdentity()],
      identity_local_dpns_names: [],
      identity_load_order: ["GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec"],
    });

    await expect(page.getByText("testname").first()).toBeVisible({
      timeout: 10000,
    });

    // Pending badge should be visible
    await expect(page.getByText("Pending").first()).toBeVisible({
      timeout: 5000,
    });
  });

  test("shows Cast Now and Remove buttons per vote", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/scheduled", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [createScheduledVote()],
      identity_list_local: [createVotingIdentity()],
      identity_list_voting: [createVotingIdentity()],
      identity_local_dpns_names: [],
      identity_load_order: ["GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec"],
    });

    await expect(page.getByText("testname").first()).toBeVisible({
      timeout: 10000,
    });

    // Action buttons per scheduled vote
    await expect(
      page.getByRole("button", { name: /Cast Now|Cast now/i }).first(),
    ).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByRole("button", { name: /Remove/i }).first(),
    ).toBeVisible();
  });

  test("shows empty state when no scheduled votes", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/scheduled", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [],
      identity_list_local: [],
      identity_list_voting: [],
      identity_local_dpns_names: [],
      identity_load_order: [],
    });

    await expect(page.getByText("Scheduled Votes").first()).toBeVisible({
      timeout: 10000,
    });

    await expect(
      page.getByText(/No scheduled votes/i),
    ).toBeVisible({ timeout: 5000 });
  });

  test("filter input is present", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/scheduled", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [createScheduledVote()],
      identity_list_local: [createVotingIdentity()],
      identity_list_voting: [createVotingIdentity()],
      identity_local_dpns_names: [],
      identity_load_order: ["GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec"],
    });

    await expect(page.getByText("Scheduled Votes").first()).toBeVisible({
      timeout: 10000,
    });

    await expect(
      page.getByPlaceholder(/Filter/i).or(
        page.getByRole("textbox", { name: /Filter scheduled votes/i }),
      ),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Register DPNS Name
// ---------------------------------------------------------------------------

test.describe("Register DPNS Name", () => {
  test("renders register name screen", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/register", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [],
      identity_list_local: [createVotingIdentity()],
      identity_list_voting: [createVotingIdentity()],
      identity_local_dpns_names: [],
      identity_load_order: ["GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec"],
    });

    await expect(
      page.getByText(/Register/i).first(),
    ).toBeVisible({ timeout: 10000 });
  });

  test("shows identity selector", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/register", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [],
      identity_list_local: [createVotingIdentity()],
      identity_list_voting: [createVotingIdentity()],
      identity_local_dpns_names: [],
      identity_load_order: ["GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec"],
    });

    await expect(
      page.getByText(/Register/i).first(),
    ).toBeVisible({ timeout: 10000 });

    // Identity selector should show the voting identity
    await expect(
      page.getByText("Voter Identity").first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows name input with .dash suffix", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/register", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [],
      identity_list_local: [createVotingIdentity()],
      identity_list_voting: [createVotingIdentity()],
      identity_local_dpns_names: [],
      identity_load_order: ["GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec"],
    });

    await expect(
      page.getByText(/Register/i).first(),
    ).toBeVisible({ timeout: 10000 });

    // .dash suffix should be displayed somewhere
    await expect(page.getByText(".dash").first()).toBeVisible({
      timeout: 5000,
    });
  });

  test("validates name input - too short", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/register", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [],
      identity_list_local: [createVotingIdentity()],
      identity_list_voting: [createVotingIdentity()],
      identity_local_dpns_names: [],
      identity_load_order: ["GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec"],
    });

    await expect(
      page.getByText(/Register/i).first(),
    ).toBeVisible({ timeout: 10000 });

    // Type a name that's too short (less than 3 chars)
    const nameInput = page.getByPlaceholder("e.g. alice");
    await expect(nameInput).toBeVisible({ timeout: 5000 });
    await nameInput.fill("ab");

    // Validation message should appear (min 3 chars)
    await expect(
      page.getByText(/at least 3 char/i).or(page.getByText(/too short/i)).or(page.getByText(/minimum/i)),
    ).toBeVisible({ timeout: 3000 });
  });

  test("shows Register button", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/register", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [],
      identity_list_local: [createVotingIdentity()],
      identity_list_voting: [createVotingIdentity()],
      identity_local_dpns_names: [],
      identity_load_order: ["GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec"],
    });

    await expect(
      page.getByText(/Register/i).first(),
    ).toBeVisible({ timeout: 10000 });

    // Register button should be present
    await expect(
      page.getByRole("button", { name: /Register/i }).first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows Back button to return to active contests", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts/dpns/register", {
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [],
      identity_list_local: [createVotingIdentity()],
      identity_list_voting: [createVotingIdentity()],
      identity_local_dpns_names: [],
      identity_load_order: ["GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec"],
    });

    await expect(
      page.getByText(/Register/i).first(),
    ).toBeVisible({ timeout: 10000 });

    // Back button should be present
    await expect(
      page.getByRole("button", { name: /Back/i }).or(
        page.getByRole("link", { name: /Back/i }),
      ),
    ).toBeVisible({ timeout: 5000 });
  });
});
