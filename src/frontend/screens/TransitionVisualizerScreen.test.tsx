import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { TransitionVisualizerScreen } from "./TransitionVisualizerScreen";

// ─── Centralized mock bindings ──────────────────────────────────

const { mocks, mockNavigate } = await vi.hoisted(async () => {
  const { createMockBindings } = await import("../test/mock-ipc");
  const initial = createMockBindings();
  const mockNavigate = vi.fn();
  return { mocks: initial, mockNavigate };
});

vi.mock("@/bindings", () => ({
  commands: mocks.commands,
  events: mocks.events,
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

vi.mock("sonner", () => ({
  toast: { error: vi.fn(), success: vi.fn() },
}));

vi.mock("@/components/theme/ThemeProvider", () => ({
  useTheme: () => ({ resolvedTheme: "light", theme: "light", setTheme: () => {} }),
}));

// ─── Tests ──────────────────────────────────────────────────────────

describe("TransitionVisualizerScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders the page title and input", () => {
    render(<TransitionVisualizerScreen />);

    expect(screen.getByText("Transition Visualizer")).toBeInTheDocument();
    expect(screen.getByText(/Deserialize, inspect, and broadcast/)).toBeInTheDocument();
    expect(
      screen.getByLabelText(/Enter hex, base64, or comma-separated/),
    ).toBeInTheDocument();
  });

  it("shows awaiting input state initially", () => {
    render(<TransitionVisualizerScreen />);

    expect(screen.getByText("Awaiting input…")).toBeInTheDocument();
  });

  it("shows back button that navigates to /tools", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    render(<TransitionVisualizerScreen />);

    const backButton = screen.getByRole("button", { name: "Back to Tools" });
    expect(backButton).toBeInTheDocument();

    await user.click(backButton);
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/tools" });
  });

  it("calls parseStateTransition when hex input is provided", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseStateTransition.mockResolvedValue({
      status: "ok",
      data: { json: '{"$type": "identityCreditTransfer"}', detectedContractIds: [] },
    });

    render(<TransitionVisualizerScreen />);

    const input = screen.getByLabelText(/Enter hex, base64, or comma-separated/);
    await user.type(input, "deadbeef");

    // Advance past the 300ms debounce
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(mocks.commands.parseStateTransition).toHaveBeenCalledWith({
        hexData: "deadbeef",
      });
    });
  });

  it("displays parsed JSON on success", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseStateTransition.mockResolvedValue({
      status: "ok",
      data: {
        json: '{\n  "$type": "identityCreditTransfer",\n  "amount": 100\n}',
        detectedContractIds: [],
      },
    });

    render(<TransitionVisualizerScreen />);

    const input = screen.getByLabelText(/Enter hex, base64, or comma-separated/);
    await user.type(input, "aabbccdd");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.queryByText("Awaiting input…")).not.toBeInTheDocument();
    });
  });

  it("displays error message on parse failure", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseStateTransition.mockResolvedValue({
      status: "error",
      error: "Failed to parse state transition: invalid bytes",
    });

    render(<TransitionVisualizerScreen />);

    const input = screen.getByLabelText(/Enter hex, base64, or comma-separated/);
    await user.type(input, "aabbccdd");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(
        screen.getByText("Failed to parse state transition: invalid bytes"),
      ).toBeInTheDocument();
    });

    expect(screen.getByRole("alert")).toBeInTheDocument();
  });

  it("dismisses error when dismiss button is clicked", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseStateTransition.mockResolvedValue({
      status: "error",
      error: "Parse error",
    });

    render(<TransitionVisualizerScreen />);

    const input = screen.getByLabelText(/Enter hex, base64, or comma-separated/);
    await user.type(input, "aabbccdd");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.getByRole("alert")).toBeInTheDocument();
    });

    const dismissButton = screen.getByRole("button", { name: "Dismiss error" });
    await user.click(dismissButton);

    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(screen.getByText("Awaiting input…")).toBeInTheDocument();
  });

  it("shows decode error for unknown input format", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    render(<TransitionVisualizerScreen />);

    const input = screen.getByLabelText(/Enter hex, base64, or comma-separated/);
    await user.type(input, "xyz!@#$%");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.getByRole("alert")).toBeInTheDocument();
      expect(screen.getByText(/Unable to decode input/)).toBeInTheDocument();
    });

    expect(mocks.commands.parseStateTransition).not.toHaveBeenCalled();
  });

  it("debounces parsing (only calls once for rapid typing)", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseStateTransition.mockResolvedValue({
      status: "ok",
      data: { json: "{}", detectedContractIds: [] },
    });

    render(<TransitionVisualizerScreen />);

    const input = screen.getByLabelText(/Enter hex, base64, or comma-separated/);

    await user.type(input, "aa");
    vi.advanceTimersByTime(100);
    await user.type(input, "bb");
    vi.advanceTimersByTime(100);
    await user.type(input, "cc");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      const calls = mocks.commands.parseStateTransition.mock.calls;
      expect(calls.length).toBeGreaterThanOrEqual(1);
      const lastCall = calls[calls.length - 1];
      expect(lastCall[0].hexData).toBe("aabbcc");
    });
  });

  it("returns to idle state when input is cleared", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseStateTransition.mockResolvedValue({
      status: "ok",
      data: { json: '{"test": true}', detectedContractIds: [] },
    });

    render(<TransitionVisualizerScreen />);

    const input = screen.getByLabelText(/Enter hex, base64, or comma-separated/);
    await user.type(input, "aabb");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.queryByText("Awaiting input…")).not.toBeInTheDocument();
    });

    await user.clear(input);
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.getByText("Awaiting input…")).toBeInTheDocument();
    });
  });

  it("handles command throwing an exception", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseStateTransition.mockRejectedValue(
      new Error("Backend unavailable"),
    );

    render(<TransitionVisualizerScreen />);

    const input = screen.getByLabelText(/Enter hex, base64, or comma-separated/);
    await user.type(input, "aabbccdd");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.getByRole("alert")).toBeInTheDocument();
      expect(screen.getByText("Backend unavailable")).toBeInTheDocument();
    });
  });

  it("supports base64 input format", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseStateTransition.mockResolvedValue({
      status: "ok",
      data: { json: '{"from": "base64"}', detectedContractIds: [] },
    });

    render(<TransitionVisualizerScreen />);

    const input = screen.getByLabelText(/Enter hex, base64, or comma-separated/);
    await user.type(input, "SGVsbG8gV29ybGQ=");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(mocks.commands.parseStateTransition).toHaveBeenCalledWith({
        hexData: "48656c6c6f20576f726c64",
      });
    });
  });

  it("supports comma-separated integer input format", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseStateTransition.mockResolvedValue({
      status: "ok",
      data: { json: '{"from": "csv"}', detectedContractIds: [] },
    });

    render(<TransitionVisualizerScreen />);

    const input = screen.getByLabelText(/Enter hex, base64, or comma-separated/);
    await user.type(input, "0,255,128");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(mocks.commands.parseStateTransition).toHaveBeenCalledWith({
        hexData: "00ff80",
      });
    });
  });

  // ─── Contract ID detection ─────────────────────────────────────────

  it("displays detected contract IDs", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseStateTransition.mockResolvedValue({
      status: "ok",
      data: {
        json: '{"type": "dataContractUpdate"}',
        detectedContractIds: ["GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec"],
      },
    });

    render(<TransitionVisualizerScreen />);

    const input = screen.getByLabelText(/Enter hex, base64, or comma-separated/);
    await user.type(input, "aabbccdd");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.getByText("Detected Contract IDs")).toBeInTheDocument();
      expect(
        screen.getByText("GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec"),
      ).toBeInTheDocument();
    });
  });

  it("does not show contract IDs section when none detected", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseStateTransition.mockResolvedValue({
      status: "ok",
      data: { json: '{"type": "identityCreate"}', detectedContractIds: [] },
    });

    render(<TransitionVisualizerScreen />);

    const input = screen.getByLabelText(/Enter hex, base64, or comma-separated/);
    await user.type(input, "aabbccdd");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.queryByText("Awaiting input…")).not.toBeInTheDocument();
    });

    expect(screen.queryByText("Detected Contract IDs")).not.toBeInTheDocument();
  });

  it("opens contract fetch dialog when contract ID is clicked", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseStateTransition.mockResolvedValue({
      status: "ok",
      data: {
        json: "{}",
        detectedContractIds: ["ABC123"],
      },
    });

    render(<TransitionVisualizerScreen />);

    const input = screen.getByLabelText(/Enter hex, base64, or comma-separated/);
    await user.type(input, "aabbccdd");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.getByText("ABC123")).toBeInTheDocument();
    });

    // Click the contract ID button (not the copy button)
    const contractButton = screen.getByText("ABC123").closest("button")!;
    await user.click(contractButton);

    await waitFor(() => {
      expect(screen.getByText("Fetch Contract")).toBeInTheDocument();
      expect(screen.getByText("Would you like to fetch this contract from Platform?")).toBeInTheDocument();
    });
  });

  it("fetches contract when confirmed in dialog", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseStateTransition.mockResolvedValue({
      status: "ok",
      data: {
        json: "{}",
        detectedContractIds: ["DEF456"],
      },
    });

    mocks.commands.contractFetch.mockResolvedValue({
      status: "ok",
      data: { taskId: "task-123" },
    });

    render(<TransitionVisualizerScreen />);

    const input = screen.getByLabelText(/Enter hex, base64, or comma-separated/);
    await user.type(input, "aabbccdd");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.getByText("DEF456")).toBeInTheDocument();
    });

    const contractButton = screen.getByText("DEF456").closest("button")!;
    await user.click(contractButton);

    await waitFor(() => {
      expect(screen.getByText("Fetch Contract")).toBeInTheDocument();
    });

    const fetchButton = screen.getByRole("button", { name: "Yes, Fetch" });
    await user.click(fetchButton);

    await waitFor(() => {
      expect(mocks.commands.contractFetch).toHaveBeenCalledWith({
        contractIds: ["DEF456"],
      });
    });

    await waitFor(() => {
      expect(screen.getByText(/fetched successfully/)).toBeInTheDocument();
    });
  });

  it("shows View in Contracts button after contract fetch", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseStateTransition.mockResolvedValue({
      status: "ok",
      data: { json: "{}", detectedContractIds: ["XYZ789"] },
    });

    mocks.commands.contractFetch.mockResolvedValue({
      status: "ok",
      data: { taskId: "task-456" },
    });

    render(<TransitionVisualizerScreen />);

    const input = screen.getByLabelText(/Enter hex, base64, or comma-separated/);
    await user.type(input, "aabbccdd");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.getByText("XYZ789")).toBeInTheDocument();
    });

    const contractButton = screen.getByText("XYZ789").closest("button")!;
    await user.click(contractButton);

    await waitFor(() => {
      expect(screen.getByText("Fetch Contract")).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: "Yes, Fetch" }));

    await waitFor(() => {
      expect(screen.getByText(/View in Contracts/)).toBeInTheDocument();
    });

    await user.click(screen.getByText(/View in Contracts/));
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/contracts" });
  });

  it("closes contract dialog on cancel", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseStateTransition.mockResolvedValue({
      status: "ok",
      data: { json: "{}", detectedContractIds: ["CANCEL_TEST"] },
    });

    render(<TransitionVisualizerScreen />);

    const input = screen.getByLabelText(/Enter hex, base64, or comma-separated/);
    await user.type(input, "aabbccdd");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.getByText("CANCEL_TEST")).toBeInTheDocument();
    });

    const contractButton = screen.getByText("CANCEL_TEST").closest("button")!;
    await user.click(contractButton);

    await waitFor(() => {
      expect(screen.getByText("Fetch Contract")).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: "Cancel" }));

    await waitFor(() => {
      expect(screen.queryByText("Fetch Contract")).not.toBeInTheDocument();
    });

    expect(mocks.commands.contractFetch).not.toHaveBeenCalled();
  });

  // ─── Broadcast ────────────────────────────────────────────────────

  it("shows broadcast button when JSON is parsed successfully", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseStateTransition.mockResolvedValue({
      status: "ok",
      data: { json: '{"type": "test"}', detectedContractIds: [] },
    });

    render(<TransitionVisualizerScreen />);

    const input = screen.getByLabelText(/Enter hex, base64, or comma-separated/);
    await user.type(input, "aabbccdd");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /Broadcast Transition to Platform/ }),
      ).toBeInTheDocument();
    });
  });

  it("does not show broadcast button in error state", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseStateTransition.mockResolvedValue({
      status: "error",
      error: "bad data",
    });

    render(<TransitionVisualizerScreen />);

    const input = screen.getByLabelText(/Enter hex, base64, or comma-separated/);
    await user.type(input, "aabbccdd");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.getByRole("alert")).toBeInTheDocument();
    });

    expect(
      screen.queryByRole("button", { name: /Broadcast Transition/ }),
    ).not.toBeInTheDocument();
  });

  it("calls broadcastStateTransition when broadcast button is clicked", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseStateTransition.mockResolvedValue({
      status: "ok",
      data: { json: '{"type": "test"}', detectedContractIds: [] },
    });

    mocks.commands.broadcastStateTransition.mockResolvedValue({
      status: "ok",
      data: { taskId: "broadcast-task-1" },
    });

    render(<TransitionVisualizerScreen />);

    const input = screen.getByLabelText(/Enter hex, base64, or comma-separated/);
    await user.type(input, "aabbccdd");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.getByRole("button", { name: /Broadcast Transition/ })).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: /Broadcast Transition/ }));

    await waitFor(() => {
      expect(mocks.commands.broadcastStateTransition).toHaveBeenCalledWith({
        stateTransitionHex: "aabbccdd",
      });
    });
  });

  it("shows broadcasting status with elapsed time", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseStateTransition.mockResolvedValue({
      status: "ok",
      data: { json: '{"type": "test"}', detectedContractIds: [] },
    });

    mocks.commands.broadcastStateTransition.mockResolvedValue({
      status: "ok",
      data: { taskId: "broadcast-task-2" },
    });

    render(<TransitionVisualizerScreen />);

    const input = screen.getByLabelText(/Enter hex, base64, or comma-separated/);
    await user.type(input, "aabbccdd");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.getByRole("button", { name: /Broadcast Transition/ })).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: /Broadcast Transition/ }));

    await waitFor(() => {
      expect(screen.getByText(/Broadcasting…/)).toBeInTheDocument();
    });

    // Broadcast button should be hidden during submitting
    expect(
      screen.queryByRole("button", { name: /Broadcast Transition/ }),
    ).not.toBeInTheDocument();
  });

  it("handles broadcast dispatch error", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseStateTransition.mockResolvedValue({
      status: "ok",
      data: { json: '{"type": "test"}', detectedContractIds: [] },
    });

    mocks.commands.broadcastStateTransition.mockResolvedValue({
      status: "error",
      error: "Network error: connection refused",
    });

    render(<TransitionVisualizerScreen />);

    const input = screen.getByLabelText(/Enter hex, base64, or comma-separated/);
    await user.type(input, "aabbccdd");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.getByRole("button", { name: /Broadcast Transition/ })).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: /Broadcast Transition/ }));

    await waitFor(() => {
      expect(screen.getByText(/Network error: connection refused/)).toBeInTheDocument();
    });
  });
});
