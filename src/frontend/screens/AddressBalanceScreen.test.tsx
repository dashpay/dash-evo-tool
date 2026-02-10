import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { AddressBalanceScreen } from "./AddressBalanceScreen";

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

// ─── Helpers ─────────────────────────────────────────────────────

function fireTaskResult(payload: unknown) {
  act(() => {
    mocks.emitMockEvent("taskResultEvent", payload);
  });
}

function fireTaskError(payload: unknown) {
  act(() => {
    mocks.emitMockEvent("taskErrorEvent", payload);
  });
}

// ─── Tests ───────────────────────────────────────────────────────

describe("AddressBalanceScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Clear event listener arrays so each test starts fresh
    for (const arr of mocks.eventListeners.values()) {
      arr.length = 0;
    }
  });

  it("renders the page title and subtitle", () => {
    render(<AddressBalanceScreen />);
    expect(
      screen.getByRole("heading", {
        name: /platform address balance lookup/i,
      }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/look up the balance and nonce/i),
    ).toBeInTheDocument();
  });

  it("renders address input field and fetch button", () => {
    render(<AddressBalanceScreen />);
    expect(screen.getByPlaceholderText("evo1... or tevo1...")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /fetch balance/i }),
    ).toBeInTheDocument();
  });

  it("shows empty state initially", () => {
    render(<AddressBalanceScreen />);
    expect(
      screen.getByText(/enter a platform address above/i),
    ).toBeInTheDocument();
  });

  it("disables fetch button when input is empty", () => {
    render(<AddressBalanceScreen />);
    const button = screen.getByRole("button", { name: /fetch balance/i });
    expect(button).toBeDisabled();
  });

  it("shows validation error for invalid address prefix", async () => {
    const user = userEvent.setup();
    render(<AddressBalanceScreen />);

    const input = screen.getByPlaceholderText("evo1... or tevo1...");
    await user.type(input, "invalid_address");

    expect(
      screen.getByText(/address must start with "evo1"/i),
    ).toBeInTheDocument();
  });

  it("enables fetch button for valid address", async () => {
    const user = userEvent.setup();
    render(<AddressBalanceScreen />);

    const input = screen.getByPlaceholderText("evo1... or tevo1...");
    await user.type(input, "evo1abc123");

    const button = screen.getByRole("button", { name: /fetch balance/i });
    expect(button).not.toBeDisabled();
  });

  it("enables fetch button for testnet address", async () => {
    const user = userEvent.setup();
    render(<AddressBalanceScreen />);

    const input = screen.getByPlaceholderText("evo1... or tevo1...");
    await user.type(input, "tevo1abc123");

    const button = screen.getByRole("button", { name: /fetch balance/i });
    expect(button).not.toBeDisabled();
  });

  it("dispatches fetch command when button clicked", async () => {
    const user = userEvent.setup();
    render(<AddressBalanceScreen />);

    const input = screen.getByPlaceholderText("evo1... or tevo1...");
    await user.type(input, "evo1abc123");

    const button = screen.getByRole("button", { name: /fetch balance/i });
    await user.click(button);

    expect(mocks.commands.platformFetchAddressBalance).toHaveBeenCalledWith({
      address: "evo1abc123",
    });
  });

  it("dispatches fetch command on Enter key", async () => {
    const user = userEvent.setup();
    render(<AddressBalanceScreen />);

    const input = screen.getByPlaceholderText("evo1... or tevo1...");
    await user.type(input, "evo1abc123");
    await user.keyboard("{Enter}");

    expect(mocks.commands.platformFetchAddressBalance).toHaveBeenCalledWith({
      address: "evo1abc123",
    });
  });

  it("shows loading state while fetching", async () => {
    const user = userEvent.setup();
    render(<AddressBalanceScreen />);

    const input = screen.getByPlaceholderText("evo1... or tevo1...");
    await user.type(input, "evo1abc123");

    const button = screen.getByRole("button", { name: /fetch balance/i });
    await user.click(button);

    expect(screen.getByText("Loading...")).toBeInTheDocument();
    expect(screen.getByText("Fetching balance...")).toBeInTheDocument();
  });

  it("disables fetch button while loading", async () => {
    const user = userEvent.setup();
    render(<AddressBalanceScreen />);

    const input = screen.getByPlaceholderText("evo1... or tevo1...");
    await user.type(input, "evo1abc123");

    const button = screen.getByRole("button", { name: /fetch balance/i });
    await user.click(button);

    // The button text changes to "Loading...", need to get it by role
    const loadingButton = screen
      .getByText("Loading...")
      .closest("button")!;
    expect(loadingButton).toBeDisabled();
  });

  it("shows result after task completes", async () => {
    const user = userEvent.setup();
    render(<AddressBalanceScreen />);

    await waitFor(() => {
      expect(mocks.events.taskResultEvent.listen).toHaveBeenCalled();
    });

    const input = screen.getByPlaceholderText("evo1... or tevo1...");
    await user.type(input, "evo1abc123");

    const button = screen.getByRole("button", { name: /fetch balance/i });
    await user.click(button);

    fireTaskResult({
      taskId: "mock-task-id",
      resultType: "Platform",
      payload: {
        type: "addressBalance",
        address: "evo1abc123",
        balance: 1000000000000,
        nonce: 42,
      },
    });

    await waitFor(() => {
      expect(screen.getByText("evo1abc123")).toBeInTheDocument();
      expect(
        screen.getByText(/1,000,000,000,000 credits \(10\.00000000 Dash\)/),
      ).toBeInTheDocument();
      expect(screen.getByText("42")).toBeInTheDocument();
    });
  });

  it("shows result heading", async () => {
    const user = userEvent.setup();
    render(<AddressBalanceScreen />);

    await waitFor(() => {
      expect(mocks.events.taskResultEvent.listen).toHaveBeenCalled();
    });

    const input = screen.getByPlaceholderText("evo1... or tevo1...");
    await user.type(input, "evo1abc123");

    const button = screen.getByRole("button", { name: /fetch balance/i });
    await user.click(button);

    fireTaskResult({
      taskId: "mock-task-id",
      resultType: "Platform",
      payload: {
        type: "addressBalance",
        address: "evo1abc123",
        balance: 500,
        nonce: 0,
      },
    });

    await waitFor(() => {
      expect(
        screen.getByRole("heading", { name: /result/i }),
      ).toBeInTheDocument();
    });
  });

  it("shows error banner on task error and allows dismissal", async () => {
    const user = userEvent.setup();
    render(<AddressBalanceScreen />);

    await waitFor(() => {
      expect(mocks.events.taskErrorEvent.listen).toHaveBeenCalled();
    });

    const input = screen.getByPlaceholderText("evo1... or tevo1...");
    await user.type(input, "evo1abc123");

    const button = screen.getByRole("button", { name: /fetch balance/i });
    await user.click(button);

    fireTaskError({
      taskId: "mock-task-id",
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

  it("re-enables button after result arrives", async () => {
    const user = userEvent.setup();
    render(<AddressBalanceScreen />);

    await waitFor(() => {
      expect(mocks.events.taskResultEvent.listen).toHaveBeenCalled();
    });

    const input = screen.getByPlaceholderText("evo1... or tevo1...");
    await user.type(input, "evo1abc123");

    const fetchBtn = screen.getByRole("button", { name: /fetch balance/i });
    await user.click(fetchBtn);

    fireTaskResult({
      taskId: "mock-task-id",
      resultType: "Platform",
      payload: {
        type: "addressBalance",
        address: "evo1abc123",
        balance: 100,
        nonce: 1,
      },
    });

    await waitFor(() => {
      const btn = screen.getByRole("button", { name: /fetch balance/i });
      expect(btn).not.toBeDisabled();
    });
  });

  it("re-enables button after error arrives", async () => {
    const user = userEvent.setup();
    render(<AddressBalanceScreen />);

    await waitFor(() => {
      expect(mocks.events.taskErrorEvent.listen).toHaveBeenCalled();
    });

    const input = screen.getByPlaceholderText("evo1... or tevo1...");
    await user.type(input, "evo1abc123");

    const fetchBtn = screen.getByRole("button", { name: /fetch balance/i });
    await user.click(fetchBtn);

    fireTaskError({
      taskId: "mock-task-id",
      message: "Some error",
      details: "",
      recoverable: false,
    });

    await waitFor(() => {
      const btn = screen.getByRole("button", { name: /fetch balance/i });
      expect(btn).not.toBeDisabled();
    });
  });

  it("ignores results from other task types", async () => {
    const user = userEvent.setup();
    render(<AddressBalanceScreen />);

    await waitFor(() => {
      expect(mocks.events.taskResultEvent.listen).toHaveBeenCalled();
    });

    const input = screen.getByPlaceholderText("evo1... or tevo1...");
    await user.type(input, "evo1abc123");

    const button = screen.getByRole("button", { name: /fetch balance/i });
    await user.click(button);

    // Fire an Identity result — should be ignored
    fireTaskResult({
      taskId: "mock-task-id",
      resultType: "Identity",
      payload: null,
    });

    // Should still be loading
    expect(screen.getByText("Loading...")).toBeInTheDocument();
  });

  it("handles dispatch errors gracefully", async () => {
    const user = userEvent.setup();
    mocks.commands.platformFetchAddressBalance.mockRejectedValueOnce(
      new Error("IPC unavailable"),
    );

    render(<AddressBalanceScreen />);

    const input = screen.getByPlaceholderText("evo1... or tevo1...");
    await user.type(input, "evo1abc123");

    const button = screen.getByRole("button", { name: /fetch balance/i });
    await user.click(button);

    await waitFor(() => {
      expect(screen.getByText("IPC unavailable")).toBeInTheDocument();
    });

    // Button should be re-enabled
    const fetchBtn = screen.getByRole("button", { name: /fetch balance/i });
    expect(fetchBtn).not.toBeDisabled();
  });

  it("subscribes to task events on mount", async () => {
    render(<AddressBalanceScreen />);

    await waitFor(() => {
      expect(mocks.events.taskResultEvent.listen).toHaveBeenCalledTimes(1);
      expect(mocks.events.taskErrorEvent.listen).toHaveBeenCalledTimes(1);
    });
  });

  it("renders back button to tools index", () => {
    render(<AddressBalanceScreen />);
    expect(
      screen.getByRole("button", { name: /back to tools/i }),
    ).toBeInTheDocument();
  });

  it("trims whitespace from address before dispatching", async () => {
    const user = userEvent.setup();
    render(<AddressBalanceScreen />);

    const input = screen.getByPlaceholderText("evo1... or tevo1...");
    await user.type(input, "  evo1abc123  ");

    const button = screen.getByRole("button", { name: /fetch balance/i });
    await user.click(button);

    expect(mocks.commands.platformFetchAddressBalance).toHaveBeenCalledWith({
      address: "evo1abc123",
    });
  });

  it("clears validation error when address becomes valid", async () => {
    const user = userEvent.setup();
    render(<AddressBalanceScreen />);

    const input = screen.getByPlaceholderText("evo1... or tevo1...");

    // Type invalid address
    await user.type(input, "invalid");
    expect(
      screen.getByText(/address must start with "evo1"/i),
    ).toBeInTheDocument();

    // Clear and type valid address
    await user.clear(input);
    await user.type(input, "evo1valid");

    expect(
      screen.queryByText(/address must start with "evo1"/i),
    ).not.toBeInTheDocument();
  });

  it("sets aria-invalid on input when validation fails", async () => {
    const user = userEvent.setup();
    render(<AddressBalanceScreen />);

    const input = screen.getByPlaceholderText("evo1... or tevo1...");
    await user.type(input, "bad");

    expect(input).toHaveAttribute("aria-invalid", "true");
  });
});
