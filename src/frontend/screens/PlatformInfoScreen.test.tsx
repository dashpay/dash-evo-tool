import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { PlatformInfoScreen } from "./PlatformInfoScreen";

// ─── Hoisted mocks (required for vi.mock factory) ─────────────────

type EventCallback = (event: { payload: unknown }) => void;

const { mockCommands, mockEvents, eventListeners, mockNavigate } = vi.hoisted(
  () => {
    const eventListeners: Record<string, EventCallback[]> = {
      taskResultEvent: [],
      taskErrorEvent: [],
    };

    const mockEvents = {
      taskResultEvent: {
        listen: vi.fn().mockImplementation((cb: EventCallback) => {
          eventListeners.taskResultEvent.push(cb);
          return Promise.resolve(() => {
            eventListeners.taskResultEvent =
              eventListeners.taskResultEvent.filter((l) => l !== cb);
          });
        }),
      },
      taskErrorEvent: {
        listen: vi.fn().mockImplementation((cb: EventCallback) => {
          eventListeners.taskErrorEvent.push(cb);
          return Promise.resolve(() => {
            eventListeners.taskErrorEvent =
              eventListeners.taskErrorEvent.filter((l) => l !== cb);
          });
        }),
      },
    };

    const mockCommands = {
      platformBasicInfo: vi.fn().mockResolvedValue({ taskId: "task-1" }),
      platformCurrentEpochInfo: vi
        .fn()
        .mockResolvedValue({ taskId: "task-2" }),
      platformTotalCredits: vi.fn().mockResolvedValue({ taskId: "task-3" }),
      platformVersionVotingState: vi
        .fn()
        .mockResolvedValue({ taskId: "task-4" }),
      platformValidatorSetInfo: vi
        .fn()
        .mockResolvedValue({ taskId: "task-5" }),
      platformWithdrawalsInQueue: vi
        .fn()
        .mockResolvedValue({ taskId: "task-6" }),
      platformRecentlyCompletedWithdrawals: vi
        .fn()
        .mockResolvedValue({ taskId: "task-7" }),
    };

    const mockNavigate = vi.fn();

    return { mockCommands, mockEvents, eventListeners, mockNavigate };
  },
);

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

vi.mock("@/bindings", () => ({
  commands: mockCommands,
  events: mockEvents,
}));

vi.mock("sonner", () => ({
  toast: {
    error: vi.fn(),
    success: vi.fn(),
  },
}));

// ─── Helpers ─────────────────────────────────────────────────────

function fireTaskResult(payload: unknown) {
  act(() => {
    for (const listener of eventListeners.taskResultEvent) {
      listener({ payload });
    }
  });
}

function fireTaskError(payload: unknown) {
  act(() => {
    for (const listener of eventListeners.taskErrorEvent) {
      listener({ payload });
    }
  });
}

// ─── Tests ───────────────────────────────────────────────────────

describe("PlatformInfoScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    eventListeners.taskResultEvent = [];
    eventListeners.taskErrorEvent = [];
  });

  it("renders the page title and subtitle", () => {
    render(<PlatformInfoScreen />);
    expect(
      screen.getByRole("heading", { name: /platform information/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/fetch and inspect platform data/i),
    ).toBeInTheDocument();
  });

  it("renders all 7 query cards", () => {
    render(<PlatformInfoScreen />);
    expect(screen.getByText("Basic Platform Info")).toBeInTheDocument();
    expect(screen.getByText("Current Epoch Info")).toBeInTheDocument();
    expect(screen.getByText("Total Credits on Platform")).toBeInTheDocument();
    expect(screen.getByText("Version Voting State")).toBeInTheDocument();
    expect(screen.getByText("Validator Set Info")).toBeInTheDocument();
    expect(screen.getByText("Withdrawals in Queue")).toBeInTheDocument();
    expect(
      screen.getByText("Recently Completed Withdrawals"),
    ).toBeInTheDocument();
  });

  it("shows empty state initially", () => {
    render(<PlatformInfoScreen />);
    expect(
      screen.getByText(/select a query from the left/i),
    ).toBeInTheDocument();
  });

  it("dispatches basic info command when clicked", async () => {
    const user = userEvent.setup();
    render(<PlatformInfoScreen />);

    const card = screen.getByText("Basic Platform Info").closest("button")!;
    await user.click(card);

    expect(mockCommands.platformBasicInfo).toHaveBeenCalledTimes(1);
  });

  it("dispatches epoch info command when clicked", async () => {
    const user = userEvent.setup();
    render(<PlatformInfoScreen />);

    const card = screen.getByText("Current Epoch Info").closest("button")!;
    await user.click(card);

    expect(mockCommands.platformCurrentEpochInfo).toHaveBeenCalledTimes(1);
  });

  it("dispatches total credits command when clicked", async () => {
    const user = userEvent.setup();
    render(<PlatformInfoScreen />);

    const card = screen
      .getByText("Total Credits on Platform")
      .closest("button")!;
    await user.click(card);

    expect(mockCommands.platformTotalCredits).toHaveBeenCalledTimes(1);
  });

  it("dispatches version voting command when clicked", async () => {
    const user = userEvent.setup();
    render(<PlatformInfoScreen />);

    const card = screen.getByText("Version Voting State").closest("button")!;
    await user.click(card);

    expect(mockCommands.platformVersionVotingState).toHaveBeenCalledTimes(1);
  });

  it("dispatches validator set command when clicked", async () => {
    const user = userEvent.setup();
    render(<PlatformInfoScreen />);

    const card = screen.getByText("Validator Set Info").closest("button")!;
    await user.click(card);

    expect(mockCommands.platformValidatorSetInfo).toHaveBeenCalledTimes(1);
  });

  it("dispatches withdrawals in queue command when clicked", async () => {
    const user = userEvent.setup();
    render(<PlatformInfoScreen />);

    const card = screen.getByText("Withdrawals in Queue").closest("button")!;
    await user.click(card);

    expect(mockCommands.platformWithdrawalsInQueue).toHaveBeenCalledTimes(1);
  });

  it("dispatches recently completed withdrawals command when clicked", async () => {
    const user = userEvent.setup();
    render(<PlatformInfoScreen />);

    const card = screen
      .getByText("Recently Completed Withdrawals")
      .closest("button")!;
    await user.click(card);

    expect(
      mockCommands.platformRecentlyCompletedWithdrawals,
    ).toHaveBeenCalledTimes(1);
  });

  it("disables all query cards while loading", async () => {
    const user = userEvent.setup();
    render(<PlatformInfoScreen />);

    const basicCard = screen
      .getByText("Basic Platform Info")
      .closest("button")!;
    await user.click(basicCard);

    // All 7 query cards should be disabled
    const allButtons = screen.getAllByRole("button");
    const queryCards = allButtons.filter(
      (b) => b.getAttribute("aria-label") !== "Back to Tools",
    );
    expect(queryCards).toHaveLength(7);
    for (const card of queryCards) {
      expect(card).toBeDisabled();
    }
  });

  it("shows result text after task completes", async () => {
    const user = userEvent.setup();
    render(<PlatformInfoScreen />);

    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
    });

    const card = screen.getByText("Basic Platform Info").closest("button")!;
    await user.click(card);

    fireTaskResult({
      taskId: "task-1",
      resultType: "Platform",
      payload: {
        type: "text",
        title: "Basic Platform Information",
        data: "Platform Version Information:\n\n• Protocol Version: 4",
      },
    });

    await waitFor(() => {
      expect(
        screen.getByText("Basic Platform Information"),
      ).toBeInTheDocument();
      expect(screen.getByText(/Protocol Version: 4/)).toBeInTheDocument();
    });
  });

  it("shows error banner on task error and allows dismissal", async () => {
    const user = userEvent.setup();
    render(<PlatformInfoScreen />);

    await waitFor(() => {
      expect(mockEvents.taskErrorEvent.listen).toHaveBeenCalled();
    });

    const card = screen.getByText("Basic Platform Info").closest("button")!;
    await user.click(card);

    fireTaskError({
      taskId: "task-1",
      message: "Network connection failed",
      details: "timeout",
      recoverable: true,
    });

    await waitFor(() => {
      expect(
        screen.getByText("Network connection failed"),
      ).toBeInTheDocument();
    });

    const dismissBtn = screen.getByRole("button", { name: /dismiss/i });
    await user.click(dismissBtn);

    expect(
      screen.queryByText("Network connection failed"),
    ).not.toBeInTheDocument();
  });

  it("re-enables cards after result arrives", async () => {
    const user = userEvent.setup();
    render(<PlatformInfoScreen />);

    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
    });

    const card = screen.getByText("Basic Platform Info").closest("button")!;
    await user.click(card);
    expect(card).toBeDisabled();

    fireTaskResult({
      taskId: "task-1",
      resultType: "Platform",
      payload: {
        type: "text",
        title: "Basic Platform Information",
        data: "Test result",
      },
    });

    await waitFor(() => {
      expect(card).not.toBeDisabled();
    });
  });

  it("re-enables cards after error arrives", async () => {
    const user = userEvent.setup();
    render(<PlatformInfoScreen />);

    await waitFor(() => {
      expect(mockEvents.taskErrorEvent.listen).toHaveBeenCalled();
    });

    const card = screen.getByText("Basic Platform Info").closest("button")!;
    await user.click(card);
    expect(card).toBeDisabled();

    fireTaskError({
      taskId: "task-1",
      message: "Some error",
      details: "",
      recoverable: false,
    });

    await waitFor(() => {
      expect(card).not.toBeDisabled();
    });
  });

  it("shows loading indicator on the active card", async () => {
    const user = userEvent.setup();
    render(<PlatformInfoScreen />);

    const card = screen.getByText("Basic Platform Info").closest("button")!;
    await user.click(card);

    expect(card).toHaveAttribute("aria-busy", "true");
  });

  it("subscribes to task events on mount", async () => {
    render(<PlatformInfoScreen />);

    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalledTimes(1);
      expect(mockEvents.taskErrorEvent.listen).toHaveBeenCalledTimes(1);
    });
  });

  it("ignores results from other task types", async () => {
    const user = userEvent.setup();
    render(<PlatformInfoScreen />);

    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
    });

    const card = screen.getByText("Basic Platform Info").closest("button")!;
    await user.click(card);

    // Fire an Identity result — should be ignored
    fireTaskResult({
      taskId: "task-1",
      resultType: "Identity",
      payload: null,
    });

    // Card should still be disabled (waiting for Platform result)
    expect(card).toBeDisabled();
  });

  it("handles dispatch errors gracefully", async () => {
    const user = userEvent.setup();
    mockCommands.platformBasicInfo.mockRejectedValueOnce(
      new Error("IPC unavailable"),
    );

    render(<PlatformInfoScreen />);

    const card = screen.getByText("Basic Platform Info").closest("button")!;
    await user.click(card);

    await waitFor(() => {
      expect(screen.getByText("IPC unavailable")).toBeInTheDocument();
    });

    expect(card).not.toBeDisabled();
  });

  it("renders back button to tools index", () => {
    render(<PlatformInfoScreen />);
    expect(
      screen.getByRole("button", { name: /back to tools/i }),
    ).toBeInTheDocument();
  });
});
