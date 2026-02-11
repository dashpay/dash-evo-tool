import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { ProofLogScreen } from "./ProofLogScreen";
import type { ProofLogItemDto } from "@/bindings";

// ─── Centralized mock bindings ──────────────────────────────────

const { mocks, mockNavigate } = await vi.hoisted(async () => {
  const { createMockBindings } = await import("../test/mock-ipc");
  const initial = createMockBindings();
  const mockNavigate = vi.fn();
  return { mocks: initial, mockNavigate };
});

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

vi.mock("@/bindings", () => ({
  commands: mocks.commands,
  events: mocks.events,
}));

vi.mock("sonner", () => ({
  toast: {
    error: vi.fn(),
    success: vi.fn(),
  },
}));

// ─── Fixtures ───────────────────────────────────────────────────

function createItem(
  overrides: Partial<ProofLogItemDto> = {},
): ProofLogItemDto {
  return {
    requestType: "getIdentity",
    requestBytesHex: "deadbeef",
    verificationPathQueryHex: "cafe",
    height: 42,
    timeMs: 1700000000000,
    proofBytesHex: "010203",
    error: null,
    ...overrides,
  };
}

function okPage(items: ProofLogItemDto[], page = 0) {
  return {
    status: "ok" as const,
    data: { items, page, itemsPerPage: 100 },
  };
}

/** Helper: click the first data row (the cell containing the request type text) */
async function clickRow(
  user: ReturnType<typeof userEvent.setup>,
  requestTypeText: string,
) {
  await user.click(screen.getByText(requestTypeText));
}

// ─── Tests ──────────────────────────────────────────────────────

describe("ProofLogScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    for (const arr of mocks.eventListeners.values()) {
      arr.length = 0;
    }
  });

  // --- Rendering ---

  it("renders the page title and subtitle", async () => {
    mocks.commands.proofLogGetItems.mockResolvedValue(okPage([]));
    render(<ProofLogScreen />);

    expect(
      screen.getByRole("heading", { name: /proof log/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/browse and inspect historical proof log entries/i),
    ).toBeInTheDocument();
  });

  it("shows loading state initially", () => {
    mocks.commands.proofLogGetItems.mockReturnValue(new Promise(() => {}));
    render(<ProofLogScreen />);

    expect(screen.getByText(/loading proof log/i)).toBeInTheDocument();
  });

  it("shows empty state when no items are returned", async () => {
    mocks.commands.proofLogGetItems.mockResolvedValue(okPage([]));
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(
        screen.getByText(/no proof items to display/i),
      ).toBeInTheDocument();
    });
  });

  it("shows error state when IPC call fails", async () => {
    mocks.commands.proofLogGetItems.mockResolvedValue({
      status: "error",
      error: "Database read failed",
    });
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(screen.getByText(/database read failed/i)).toBeInTheDocument();
    });
  });

  // --- Table rendering ---

  it("renders table with correct column headers", async () => {
    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([createItem()]),
    );
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(screen.getByText("Request Type")).toBeInTheDocument();
      expect(screen.getByText("Height")).toBeInTheDocument();
      expect(screen.getByText("Time (ms)")).toBeInTheDocument();
      expect(screen.getByText("Error")).toBeInTheDocument();
    });
  });

  it("renders proof log items in the table", async () => {
    const items = [
      createItem({ requestType: "getIdentity", height: 42 }),
      createItem({
        requestType: "getDocuments",
        height: 100,
        timeMs: 1700000001000,
        error: "proof mismatch",
      }),
    ];
    mocks.commands.proofLogGetItems.mockResolvedValue(okPage(items));
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(screen.getByText("Get Identity")).toBeInTheDocument();
      expect(screen.getByText("Get Documents")).toBeInTheDocument();
      expect(screen.getByText("42")).toBeInTheDocument();
      expect(screen.getByText("100")).toBeInTheDocument();
    });
  });

  it("shows error text truncated to 40 chars in table", async () => {
    const longError = "a".repeat(50);
    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([createItem({ error: longError })]),
    );
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(screen.getByText(`${"a".repeat(40)}...`)).toBeInTheDocument();
    });
  });

  it("shows 'No Error' for items without errors", async () => {
    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([createItem({ error: null })]),
    );
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(screen.getByText("No Error")).toBeInTheDocument();
    });
  });

  // --- Row selection ---

  it("shows detail placeholder when no row is selected", async () => {
    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([createItem()]),
    );
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(
        screen.getByText(/select a proof log entry to view details/i),
      ).toBeInTheDocument();
    });
  });

  it("shows detail panel when a row is clicked", async () => {
    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([
        createItem({
          requestType: "getIdentityKeys",
          height: 99,
          timeMs: 1234567890000,
          proofBytesHex: "aabb",
        }),
      ]),
    );
    const user = userEvent.setup();
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(screen.getByText("Get Identity Keys")).toBeInTheDocument();
    });

    await clickRow(user, "Get Identity Keys");

    // Detail panel should show display mode radios
    await waitFor(() => {
      expect(screen.getByLabelText("Hex")).toBeInTheDocument();
      expect(screen.getByLabelText("JSON")).toBeInTheDocument();
      expect(screen.getByLabelText("Path Query")).toBeInTheDocument();
    });
  });

  it("deselects row on second click", async () => {
    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([createItem()]),
    );
    const user = userEvent.setup();
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(screen.getByText("Get Identity")).toBeInTheDocument();
    });

    // Select — click the table cell directly
    const rows = screen.getAllByRole("row");
    const dataRow = rows[1];
    await user.click(dataRow);
    await waitFor(() => {
      expect(
        screen.queryByText(/select a proof log entry/i),
      ).not.toBeInTheDocument();
    });

    // Deselect — click the same row again
    await user.click(dataRow);
    await waitFor(() => {
      expect(
        screen.getByText(/select a proof log entry/i),
      ).toBeInTheDocument();
    });
  });

  // --- Display modes ---

  it("shows hex display mode by default", async () => {
    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([createItem({ proofBytesHex: "deadbeef" })]),
    );
    const user = userEvent.setup();
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(screen.getByText("Get Identity")).toBeInTheDocument();
    });

    await clickRow(user, "Get Identity");

    await waitFor(() => {
      expect(screen.getByText("deadbeef")).toBeInTheDocument();
    });

    const hexRadio = screen.getByLabelText("Hex");
    expect(hexRadio).toBeChecked();
  });

  it("switches to JSON mode and calls parseGrovedbProof", async () => {
    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([createItem({ proofBytesHex: "aabbcc" })]),
    );
    mocks.commands.parseGrovedbProof.mockResolvedValue({
      status: "ok",
      data: { text: "GroveDBProof { root_hash: 0x1234 }" },
    });
    const user = userEvent.setup();
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(screen.getByText("Get Identity")).toBeInTheDocument();
    });

    await clickRow(user, "Get Identity");
    await user.click(screen.getByLabelText("JSON"));

    await waitFor(() => {
      expect(mocks.commands.parseGrovedbProof).toHaveBeenCalledWith({
        hexData: "aabbcc",
      });
      expect(
        screen.getByText("GroveDBProof { root_hash: 0x1234 }"),
      ).toBeInTheDocument();
    });
  });

  it("switches to Path Query mode and calls parsePathQuery", async () => {
    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([createItem({ verificationPathQueryHex: "ff00" })]),
    );
    mocks.commands.parsePathQuery.mockResolvedValue({
      status: "ok",
      data: { text: "PathQuery { path: [0x01, 0x02] }" },
    });
    const user = userEvent.setup();
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(screen.getByText("Get Identity")).toBeInTheDocument();
    });

    await clickRow(user, "Get Identity");
    await user.click(screen.getByLabelText("Path Query"));

    await waitFor(() => {
      expect(mocks.commands.parsePathQuery).toHaveBeenCalledWith({
        hexData: "ff00",
      });
      expect(
        screen.getByText("PathQuery { path: [0x01, 0x02] }"),
      ).toBeInTheDocument();
    });
  });

  it("shows error when JSON parse fails", async () => {
    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([createItem({ proofBytesHex: "baddata" })]),
    );
    mocks.commands.parseGrovedbProof.mockResolvedValue({
      status: "error",
      error: "Deserialization error: invalid bincode",
    });
    const user = userEvent.setup();
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(screen.getByText("Get Identity")).toBeInTheDocument();
    });

    await clickRow(user, "Get Identity");
    await user.click(screen.getByLabelText("JSON"));

    await waitFor(() => {
      expect(
        screen.getByText(/deserialization error/i),
      ).toBeInTheDocument();
    });
  });

  it("shows placeholder for empty proof bytes in JSON mode", async () => {
    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([createItem({ proofBytesHex: "" })]),
    );
    const user = userEvent.setup();
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(screen.getByText("Get Identity")).toBeInTheDocument();
    });

    await clickRow(user, "Get Identity");
    await user.click(screen.getByLabelText("JSON"));

    await waitFor(() => {
      expect(screen.getByText("(empty proof bytes)")).toBeInTheDocument();
    });
  });

  it("shows placeholder for empty path query in PathQuery mode", async () => {
    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([createItem({ verificationPathQueryHex: "" })]),
    );
    const user = userEvent.setup();
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(screen.getByText("Get Identity")).toBeInTheDocument();
    });

    await clickRow(user, "Get Identity");
    await user.click(screen.getByLabelText("Path Query"));

    await waitFor(() => {
      expect(
        screen.getByText("(empty path query bytes)"),
      ).toBeInTheDocument();
    });
  });

  // --- Sorting ---

  it("sorts by clicking column headers", async () => {
    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([
        createItem({ requestType: "getDocuments", height: 100 }),
        createItem({ requestType: "getIdentity", height: 42, timeMs: 1700000000001 }),
      ]),
    );
    const user = userEvent.setup();
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(screen.getByText("Get Documents")).toBeInTheDocument();
    });

    await user.click(screen.getByTitle("Sort by Height"));

    // Sort indicator should appear
    await waitFor(() => {
      const btn = screen.getByTitle("Sort by Height");
      expect(btn.textContent).toContain("▲");
    });
  });

  it("toggles sort direction on same column click", async () => {
    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([createItem()]),
    );
    const user = userEvent.setup();
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(screen.getByText("Get Identity")).toBeInTheDocument();
    });

    // Default sort is timeMs descending - indicator shows ▼
    const timeMsBtn = screen.getByTitle("Sort by Time (ms)");
    expect(timeMsBtn.textContent).toContain("▼");

    // Click to toggle to ascending
    await user.click(timeMsBtn);
    await waitFor(() => {
      expect(timeMsBtn.textContent).toContain("▲");
    });

    // Click again to toggle back to descending
    await user.click(timeMsBtn);
    await waitFor(() => {
      expect(timeMsBtn.textContent).toContain("▼");
    });
  });

  // --- Pagination ---

  it("shows pagination info", async () => {
    const items = Array.from({ length: 5 }, (_, i) =>
      createItem({ height: i + 1, timeMs: 1700000000000 + i }),
    );
    mocks.commands.proofLogGetItems.mockResolvedValue(okPage(items));
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(screen.getByText("Showing items 1 to 5")).toBeInTheDocument();
    });
  });

  it("disables Previous button on first page", async () => {
    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([createItem()]),
    );
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /previous page/i }),
      ).toBeDisabled();
    });
  });

  it("disables Next button when less items than page size", async () => {
    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([createItem()]),
    );
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /next page/i }),
      ).toBeDisabled();
    });
  });

  it("navigates to next page on Next click", async () => {
    const fullPage = Array.from({ length: 100 }, (_, i) =>
      createItem({ height: i + 1, timeMs: 1700000000000 + i }),
    );
    mocks.commands.proofLogGetItems.mockResolvedValue(okPage(fullPage));
    const user = userEvent.setup();
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /next page/i }),
      ).toBeEnabled();
    });

    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([createItem({ height: 999 })], 1),
    );

    await user.click(screen.getByRole("button", { name: /next page/i }));

    await waitFor(() => {
      expect(mocks.commands.proofLogGetItems).toHaveBeenCalledWith({
        onlyErrored: false,
        page: 1,
        itemsPerPage: 100,
      });
    });
  });

  it("navigates to previous page on Previous click", async () => {
    const fullPage = Array.from({ length: 100 }, (_, i) =>
      createItem({ height: i + 1, timeMs: 1700000000000 + i }),
    );
    mocks.commands.proofLogGetItems.mockResolvedValue(okPage(fullPage));
    const user = userEvent.setup();
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /next page/i }),
      ).toBeEnabled();
    });

    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([createItem({ height: 999 })], 1),
    );
    await user.click(screen.getByRole("button", { name: /next page/i }));

    await waitFor(() => {
      expect(screen.getByText("Showing items 101 to 101")).toBeInTheDocument();
    });

    mocks.commands.proofLogGetItems.mockResolvedValue(okPage(fullPage, 0));
    await user.click(
      screen.getByRole("button", { name: /previous page/i }),
    );

    await waitFor(() => {
      expect(mocks.commands.proofLogGetItems).toHaveBeenCalledWith({
        onlyErrored: false,
        page: 0,
        itemsPerPage: 100,
      });
    });
  });

  // --- Hash highlighting ---

  it("highlights 64-char hex hashes from error in proof text", async () => {
    const hash64 = "a".repeat(64);
    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([
        createItem({
          error: `Expected hash ${hash64}`,
          proofBytesHex: `prefix${hash64}suffix`,
        }),
      ]),
    );
    const user = userEvent.setup();
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(screen.getByText("Get Identity")).toBeInTheDocument();
    });

    await clickRow(user, "Get Identity");

    // In hex mode, the proof hex is displayed with hash highlighting
    await waitFor(() => {
      const highlighted = screen.getByText(hash64);
      expect(highlighted).toHaveClass("text-amber-500");
    });
  });

  it("does not highlight hashes in PathQuery mode", async () => {
    const hash64 = "b".repeat(64);
    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([
        createItem({
          error: `Hash ${hash64}`,
          verificationPathQueryHex: "ff00",
        }),
      ]),
    );
    mocks.commands.parsePathQuery.mockResolvedValue({
      status: "ok",
      data: { text: `PathQuery containing ${hash64} data` },
    });
    const user = userEvent.setup();
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(screen.getByText("Get Identity")).toBeInTheDocument();
    });

    await clickRow(user, "Get Identity");
    await user.click(screen.getByLabelText("Path Query"));

    await waitFor(() => {
      expect(
        screen.getByText(/pathquery containing/i),
      ).toBeInTheDocument();
    });

    // Hash should NOT be highlighted in path query mode
    const outputArea = screen.getByRole("log", {
      name: /proof pathquery content/i,
    });
    const goldSpans = outputArea.querySelectorAll(".text-amber-500");
    expect(goldSpans.length).toBe(0);
  });

  // --- Detail panel metadata ---

  it("shows correct metadata in detail panel", async () => {
    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([
        createItem({
          requestType: "broadcastStateTransition",
          height: 5678,
          timeMs: 1700000000123,
          error: "some verification error",
          proofBytesHex: "aa",
        }),
      ]),
    );
    const user = userEvent.setup();
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(
        screen.getByText("Broadcast State Transition"),
      ).toBeInTheDocument();
    });

    await clickRow(user, "Broadcast State Transition");

    // Height, timeMs, and error appear in both table and detail panel
    await waitFor(() => {
      expect(screen.getAllByText("5678")).toHaveLength(2);
      expect(screen.getAllByText("1700000000123")).toHaveLength(2);
      expect(screen.getAllByText("some verification error")).toHaveLength(2);
    });
  });

  it("shows 'None' for error in detail panel when no error", async () => {
    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([createItem({ error: null })]),
    );
    const user = userEvent.setup();
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(screen.getByText("Get Identity")).toBeInTheDocument();
    });

    await clickRow(user, "Get Identity");

    await waitFor(() => {
      expect(screen.getByText("None")).toBeInTheDocument();
    });
  });

  // --- IPC call args ---

  it("fetches proof log items on mount with correct args", async () => {
    mocks.commands.proofLogGetItems.mockResolvedValue(okPage([]));
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(mocks.commands.proofLogGetItems).toHaveBeenCalledWith({
        onlyErrored: false,
        page: 0,
        itemsPerPage: 100,
      });
    });
  });

  // --- Copy button ---

  it("shows copy button in detail panel", async () => {
    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([createItem({ proofBytesHex: "aabb" })]),
    );
    const user = userEvent.setup();
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(screen.getByText("Get Identity")).toBeInTheDocument();
    });

    await clickRow(user, "Get Identity");

    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /copy/i }),
      ).toBeInTheDocument();
    });
  });

  // --- Keyboard navigation ---

  it("allows row selection with keyboard Enter", async () => {
    mocks.commands.proofLogGetItems.mockResolvedValue(
      okPage([createItem({ proofBytesHex: "aabb" })]),
    );
    const user = userEvent.setup();
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(screen.getByText("Get Identity")).toBeInTheDocument();
    });

    // Focus the data row and press Enter
    const rows = screen.getAllByRole("row");
    const dataRow = rows[1]; // first data row
    dataRow.focus();
    await user.keyboard("{Enter}");

    await waitFor(() => {
      expect(
        screen.queryByText(/select a proof log entry/i),
      ).not.toBeInTheDocument();
    });
  });

  // --- Multiple request types format ---

  it("formats various request types correctly", async () => {
    const items = [
      createItem({ requestType: "broadcastStateTransition", timeMs: 1 }),
      createItem({ requestType: "getIdentityBalance", timeMs: 2 }),
      createItem({
        requestType: "getContestedResourceVotersForIdentity",
        timeMs: 3,
      }),
      createItem({
        requestType: "waitForStateTransitionResult",
        timeMs: 4,
      }),
    ];
    mocks.commands.proofLogGetItems.mockResolvedValue(okPage(items));
    render(<ProofLogScreen />);

    await waitFor(() => {
      expect(
        screen.getByText("Broadcast State Transition"),
      ).toBeInTheDocument();
      expect(
        screen.getByText("Get Identity Balance"),
      ).toBeInTheDocument();
      expect(
        screen.getByText("Get Contested Resource Voters For Identity"),
      ).toBeInTheDocument();
      expect(
        screen.getByText("Wait For State Transition Result"),
      ).toBeInTheDocument();
    });
  });
});
