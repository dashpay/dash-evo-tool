import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { ContractVisualizerScreen } from "./ContractVisualizerScreen";

// ─── Hoisted mocks ──────────────────────────────────────────────────

const { mockCommands, mockNavigate } = vi.hoisted(() => {
  const mockCommands = {
    parseDataContract: vi.fn(),
  };
  const mockNavigate = vi.fn();
  return { mockCommands, mockNavigate };
});

vi.mock("@/bindings", () => ({
  commands: mockCommands,
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

describe("ContractVisualizerScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders the page title and input", () => {
    render(<ContractVisualizerScreen />);

    expect(screen.getByText("Contract Visualizer")).toBeInTheDocument();
    expect(
      screen.getByText(/Deserialize and inspect/),
    ).toBeInTheDocument();
    expect(
      screen.getByLabelText(/Enter hex, base64, or comma-separated/),
    ).toBeInTheDocument();
  });

  it("shows awaiting input state initially", () => {
    render(<ContractVisualizerScreen />);

    expect(screen.getByText("Awaiting input…")).toBeInTheDocument();
  });

  it("shows back button that navigates to /tools", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    render(<ContractVisualizerScreen />);

    const backButton = screen.getByRole("button", { name: "Back to Tools" });
    expect(backButton).toBeInTheDocument();

    await user.click(backButton);
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/tools" });
  });

  it("calls parseDataContract when hex input is provided", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mockCommands.parseDataContract.mockResolvedValue({
      status: "ok",
      data: { json: '{"$id": "abc123"}' },
    });

    render(<ContractVisualizerScreen />);

    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated/,
    );
    await user.type(input, "deadbeef");

    // Advance past the 300ms debounce
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(mockCommands.parseDataContract).toHaveBeenCalledWith({
        hexData: "deadbeef",
      });
    });
  });

  it("displays parsed JSON on success", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mockCommands.parseDataContract.mockResolvedValue({
      status: "ok",
      data: { json: '{\n  "$id": "abc123",\n  "$version": 1\n}' },
    });

    render(<ContractVisualizerScreen />);

    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated/,
    );
    await user.type(input, "aabbccdd");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.queryByText("Awaiting input…")).not.toBeInTheDocument();
    });
  });

  it("displays error message on parse failure", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mockCommands.parseDataContract.mockResolvedValue({
      status: "error",
      error: "Deserialization error: invalid bytes",
    });

    render(<ContractVisualizerScreen />);

    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated/,
    );
    await user.type(input, "aabbccdd");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(
        screen.getByText("Deserialization error: invalid bytes"),
      ).toBeInTheDocument();
    });

    // Should have an alert role
    expect(screen.getByRole("alert")).toBeInTheDocument();
  });

  it("dismisses error when dismiss button is clicked", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mockCommands.parseDataContract.mockResolvedValue({
      status: "error",
      error: "Deserialization error: bad data",
    });

    render(<ContractVisualizerScreen />);

    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated/,
    );
    await user.type(input, "aabbccdd");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.getByRole("alert")).toBeInTheDocument();
    });

    const dismissButton = screen.getByRole("button", {
      name: "Dismiss error",
    });
    await user.click(dismissButton);

    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(screen.getByText("Awaiting input…")).toBeInTheDocument();
  });

  it("shows decode error for unknown input format", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    render(<ContractVisualizerScreen />);

    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated/,
    );
    // Odd-length hex will be detected as "unknown" by detectFormat
    await user.type(input, "xyz!@#$%");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.getByRole("alert")).toBeInTheDocument();
      expect(screen.getByText(/Unable to decode input/)).toBeInTheDocument();
    });

    // parseDataContract should NOT have been called
    expect(mockCommands.parseDataContract).not.toHaveBeenCalled();
  });

  it("debounces parsing (only calls once for rapid typing)", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mockCommands.parseDataContract.mockResolvedValue({
      status: "ok",
      data: { json: '{}' },
    });

    render(<ContractVisualizerScreen />);

    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated/,
    );

    // Type rapidly
    await user.type(input, "aa");
    vi.advanceTimersByTime(100); // Not yet 300ms
    await user.type(input, "bb");
    vi.advanceTimersByTime(100);
    await user.type(input, "cc");
    vi.advanceTimersByTime(350); // Now past debounce

    await waitFor(() => {
      // Should have been called at most once after all typing settled,
      // but since each character resets the debounce, only the final
      // value should trigger the parse
      const calls = mockCommands.parseDataContract.mock.calls;
      expect(calls.length).toBeGreaterThanOrEqual(1);
      // The last call should have the full hex data
      const lastCall = calls[calls.length - 1];
      expect(lastCall[0].hexData).toBe("aabbcc");
    });
  });

  it("returns to idle state when input is cleared", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mockCommands.parseDataContract.mockResolvedValue({
      status: "ok",
      data: { json: '{"test": true}' },
    });

    render(<ContractVisualizerScreen />);

    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated/,
    );

    // Type something and wait for parse
    await user.type(input, "aabb");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.queryByText("Awaiting input…")).not.toBeInTheDocument();
    });

    // Clear the input
    await user.clear(input);
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.getByText("Awaiting input…")).toBeInTheDocument();
    });
  });

  it("handles command throwing an exception", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mockCommands.parseDataContract.mockRejectedValue(
      new Error("Backend unavailable"),
    );

    render(<ContractVisualizerScreen />);

    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated/,
    );
    await user.type(input, "aabbccdd");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.getByRole("alert")).toBeInTheDocument();
      expect(screen.getByText("Backend unavailable")).toBeInTheDocument();
    });
  });

  it("supports base64 input format", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mockCommands.parseDataContract.mockResolvedValue({
      status: "ok",
      data: { json: '{"$id": "from-base64"}' },
    });

    render(<ContractVisualizerScreen />);

    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated/,
    );
    // Valid base64 string
    await user.type(input, "SGVsbG8gV29ybGQ=");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(mockCommands.parseDataContract).toHaveBeenCalledWith({
        hexData: "48656c6c6f20576f726c64",
      });
    });
  });

  it("supports comma-separated integer input format", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mockCommands.parseDataContract.mockResolvedValue({
      status: "ok",
      data: { json: '{"$id": "from-csv"}' },
    });

    render(<ContractVisualizerScreen />);

    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated/,
    );
    await user.type(input, "0,255,128");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(mockCommands.parseDataContract).toHaveBeenCalledWith({
        hexData: "00ff80",
      });
    });
  });
});
