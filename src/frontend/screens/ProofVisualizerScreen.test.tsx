import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { ProofVisualizerScreen } from "./ProofVisualizerScreen";

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

describe("ProofVisualizerScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders the page title and input", () => {
    render(<ProofVisualizerScreen />);

    expect(screen.getByText("Proof Visualizer")).toBeInTheDocument();
    expect(
      screen.getByText(/Deserialize and inspect/),
    ).toBeInTheDocument();
    expect(
      screen.getByLabelText(/Enter hex, base64, or comma-separated/),
    ).toBeInTheDocument();
  });

  it("shows 'No proof parsed yet.' state initially", () => {
    render(<ProofVisualizerScreen />);

    expect(screen.getByText("No proof parsed yet.")).toBeInTheDocument();
  });

  it("shows back button that navigates to /tools", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    render(<ProofVisualizerScreen />);

    const backButton = screen.getByRole("button", { name: "Back to Tools" });
    expect(backButton).toBeInTheDocument();

    await user.click(backButton);
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/tools" });
  });

  it("calls parseGrovedbProof when hex input is provided", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseGrovedbProof.mockResolvedValue({
      status: "ok",
      data: { text: "GroveDBProof { root_layer: ... }" },
    });

    render(<ProofVisualizerScreen />);

    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated/,
    );
    await user.type(input, "deadbeef");

    // Advance past the 300ms debounce
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(mocks.commands.parseGrovedbProof).toHaveBeenCalledWith({
        hexData: "deadbeef",
      });
    });
  });

  it("displays parsed proof text on success", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseGrovedbProof.mockResolvedValue({
      status: "ok",
      data: { text: "GroveDBProof { root_layer: LayerProof { ... } }" },
    });

    render(<ProofVisualizerScreen />);

    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated/,
    );
    await user.type(input, "aabbccdd");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(
        screen.getByText("GroveDBProof { root_layer: LayerProof { ... } }"),
      ).toBeInTheDocument();
    });

    // Initial idle message should be gone
    expect(
      screen.queryByText("No proof parsed yet."),
    ).not.toBeInTheDocument();
  });

  it("displays error message on parse failure", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseGrovedbProof.mockResolvedValue({
      status: "error",
      error: "Deserialization error: unexpected end of input",
    });

    render(<ProofVisualizerScreen />);

    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated/,
    );
    await user.type(input, "aabbccdd");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(
        screen.getByText("Deserialization error: unexpected end of input"),
      ).toBeInTheDocument();
    });

    // Should have an alert role
    expect(screen.getByRole("alert")).toBeInTheDocument();
  });

  it("dismisses error when dismiss button is clicked", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseGrovedbProof.mockResolvedValue({
      status: "error",
      error: "Deserialization error: bad data",
    });

    render(<ProofVisualizerScreen />);

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
    expect(screen.getByText("No proof parsed yet.")).toBeInTheDocument();
  });

  it("shows decode error for unknown input format", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    render(<ProofVisualizerScreen />);

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

    // parseGrovedbProof should NOT have been called
    expect(mocks.commands.parseGrovedbProof).not.toHaveBeenCalled();
  });

  it("debounces parsing (only calls once for rapid typing)", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseGrovedbProof.mockResolvedValue({
      status: "ok",
      data: { text: "proof" },
    });

    render(<ProofVisualizerScreen />);

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
      const calls = mocks.commands.parseGrovedbProof.mock.calls;
      expect(calls.length).toBeGreaterThanOrEqual(1);
      // The last call should have the full hex data
      const lastCall = calls[calls.length - 1];
      expect(lastCall[0].hexData).toBe("aabbcc");
    });
  });

  it("returns to idle state when input is cleared", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseGrovedbProof.mockResolvedValue({
      status: "ok",
      data: { text: "some proof" },
    });

    render(<ProofVisualizerScreen />);

    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated/,
    );

    // Type something and wait for parse
    await user.type(input, "aabb");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(
        screen.queryByText("No proof parsed yet."),
      ).not.toBeInTheDocument();
    });

    // Clear the input
    await user.clear(input);
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.getByText("No proof parsed yet.")).toBeInTheDocument();
    });
  });

  it("handles command throwing an exception", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseGrovedbProof.mockRejectedValue(
      new Error("Backend unavailable"),
    );

    render(<ProofVisualizerScreen />);

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

    mocks.commands.parseGrovedbProof.mockResolvedValue({
      status: "ok",
      data: { text: "proof from base64" },
    });

    render(<ProofVisualizerScreen />);

    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated/,
    );
    // Valid base64 string
    await user.type(input, "SGVsbG8gV29ybGQ=");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(mocks.commands.parseGrovedbProof).toHaveBeenCalledWith({
        hexData: "48656c6c6f20576f726c64",
      });
    });
  });

  it("supports comma-separated integer input format", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseGrovedbProof.mockResolvedValue({
      status: "ok",
      data: { text: "proof from csv" },
    });

    render(<ProofVisualizerScreen />);

    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated/,
    );
    await user.type(input, "0,255,128");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(mocks.commands.parseGrovedbProof).toHaveBeenCalledWith({
        hexData: "00ff80",
      });
    });
  });

  it("shows MonospaceOutput with copy support on success", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mocks.commands.parseGrovedbProof.mockResolvedValue({
      status: "ok",
      data: { text: "GroveDBProof { root_layer: ... }" },
    });

    render(<ProofVisualizerScreen />);

    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated/,
    );
    await user.type(input, "aabbccdd");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      // MonospaceOutput renders a log role element
      expect(screen.getByRole("log")).toBeInTheDocument();
    });
  });
});
