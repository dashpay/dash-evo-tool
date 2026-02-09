import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { DocumentVisualizerScreen } from "./DocumentVisualizerScreen";

// ─── jsdom polyfills for Radix Select ───────────────────────────────

if (!Element.prototype.hasPointerCapture) {
  Element.prototype.hasPointerCapture = () => false;
}
if (!Element.prototype.setPointerCapture) {
  Element.prototype.setPointerCapture = () => {};
}
if (!Element.prototype.releasePointerCapture) {
  Element.prototype.releasePointerCapture = () => {};
}
if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = () => {};
}

// ─── Hoisted mocks ──────────────────────────────────────────────────

const { mockCommands, mockNavigate } = vi.hoisted(() => {
  const mockCommands = {
    contractListLocal: vi.fn(),
    contractGetById: vi.fn(),
    parseDocument: vi.fn(),
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
  useTheme: () => ({
    resolvedTheme: "light",
    theme: "light",
    setTheme: () => {},
  }),
}));

// ─── Test data ──────────────────────────────────────────────────────

const mockContracts = [
  {
    id: "aabb0011aabb0011aabb0011aabb0011aabb0011aabb0011aabb0011aabb0011",
    alias: "DPNS",
    documentTypeCount: 2,
    tokenCount: 0,
  },
  {
    id: "ccdd2233ccdd2233ccdd2233ccdd2233ccdd2233ccdd2233ccdd2233ccdd2233",
    alias: null,
    documentTypeCount: 1,
    tokenCount: 0,
  },
];

const mockContractDetail = {
  id: "aabb0011aabb0011aabb0011aabb0011aabb0011aabb0011aabb0011aabb0011",
  ownerId: "1234",
  alias: "DPNS",
  version: 1,
  documentTypeNames: ["domain", "preorder"],
  tokenCount: 0,
  schemaJson: {},
};

// ─── Tests ──────────────────────────────────────────────────────────

describe("DocumentVisualizerScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers({ shouldAdvanceTime: true });

    // Default: return contracts list
    mockCommands.contractListLocal.mockResolvedValue({
      status: "ok",
      data: mockContracts,
    });

    // Default: return contract detail when requested
    mockCommands.contractGetById.mockResolvedValue({
      status: "ok",
      data: mockContractDetail,
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders the page title and subtitle", async () => {
    render(<DocumentVisualizerScreen />);

    expect(screen.getByText("Document Visualizer")).toBeInTheDocument();
    expect(
      screen.getByText(/Deserialize and inspect Dash Platform documents/),
    ).toBeInTheDocument();
  });

  it("loads contracts on mount", async () => {
    render(<DocumentVisualizerScreen />);

    await waitFor(() => {
      expect(mockCommands.contractListLocal).toHaveBeenCalledTimes(1);
    });
  });

  it("shows waiting-for-selection state initially", () => {
    render(<DocumentVisualizerScreen />);

    expect(
      screen.getByText("Select a contract and document type."),
    ).toBeInTheDocument();
  });

  it("shows back button that navigates to /tools", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    render(<DocumentVisualizerScreen />);

    const backButton = screen.getByRole("button", { name: "Back to Tools" });
    expect(backButton).toBeInTheDocument();

    await user.click(backButton);
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/tools" });
  });

  it("renders contract selector", async () => {
    render(<DocumentVisualizerScreen />);

    await waitFor(() => {
      expect(
        screen.getByRole("combobox", { name: /select contract/i }),
      ).toBeInTheDocument();
    });
  });

  it("renders document type selector (disabled initially)", async () => {
    render(<DocumentVisualizerScreen />);

    const docTypeSelect = screen.getByRole("combobox", {
      name: /select document type/i,
    });
    expect(docTypeSelect).toBeDisabled();
  });

  it("renders the hex input field", () => {
    render(<DocumentVisualizerScreen />);

    expect(
      screen.getByLabelText(
        /Enter hex, base64, or comma-separated integers for Document/,
      ),
    ).toBeInTheDocument();
  });

  it("enables document type selector after contract is selected", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    render(<DocumentVisualizerScreen />);

    // Wait for contracts to load
    await waitFor(() => {
      expect(mockCommands.contractListLocal).toHaveBeenCalled();
    });

    // Select a contract
    const contractSelect = screen.getByRole("combobox", {
      name: /select contract/i,
    });
    await user.click(contractSelect);
    const dpnsOption = screen.getByRole("option", { name: "DPNS" });
    await user.click(dpnsOption);

    // Document type selector should now be enabled
    await waitFor(() => {
      expect(mockCommands.contractGetById).toHaveBeenCalledWith(
        mockContracts[0].id,
      );
    });

    const docTypeSelect = screen.getByRole("combobox", {
      name: /select document type/i,
    });
    expect(docTypeSelect).not.toBeDisabled();
  });

  it("shows document type options after contract selection", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    render(<DocumentVisualizerScreen />);

    await waitFor(() => {
      expect(mockCommands.contractListLocal).toHaveBeenCalled();
    });

    // Select a contract
    const contractSelect = screen.getByRole("combobox", {
      name: /select contract/i,
    });
    await user.click(contractSelect);
    await user.click(screen.getByRole("option", { name: "DPNS" }));

    await waitFor(() => {
      expect(mockCommands.contractGetById).toHaveBeenCalled();
    });

    // Open doc type selector
    const docTypeSelect = screen.getByRole("combobox", {
      name: /select document type/i,
    });
    await user.click(docTypeSelect);

    // Should see document type options
    expect(screen.getByRole("option", { name: "domain" })).toBeInTheDocument();
    expect(
      screen.getByRole("option", { name: "preorder" }),
    ).toBeInTheDocument();
  });

  it("calls parseDocument with all required parameters", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mockCommands.parseDocument.mockResolvedValue({
      status: "ok",
      data: { json: '{"$id": "doc123"}' },
    });

    render(<DocumentVisualizerScreen />);

    await waitFor(() => {
      expect(mockCommands.contractListLocal).toHaveBeenCalled();
    });

    // Select contract
    const contractSelect = screen.getByRole("combobox", {
      name: /select contract/i,
    });
    await user.click(contractSelect);
    await user.click(screen.getByRole("option", { name: "DPNS" }));

    await waitFor(() => {
      expect(mockCommands.contractGetById).toHaveBeenCalled();
    });

    // Select doc type
    const docTypeSelect = screen.getByRole("combobox", {
      name: /select document type/i,
    });
    await user.click(docTypeSelect);
    await user.click(screen.getByRole("option", { name: "domain" }));

    // Enter hex data
    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated integers for Document/,
    );
    await user.type(input, "deadbeef");

    // Wait for debounce
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(mockCommands.parseDocument).toHaveBeenCalledWith({
        hexData: "deadbeef",
        contractId: mockContracts[0].id,
        documentTypeName: "domain",
      });
    });
  });

  it("displays error message on parse failure", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mockCommands.parseDocument.mockResolvedValue({
      status: "error",
      error: "Deserialization error: invalid document bytes",
    });

    render(<DocumentVisualizerScreen />);

    await waitFor(() => {
      expect(mockCommands.contractListLocal).toHaveBeenCalled();
    });

    // Select contract + doc type
    const contractSelect = screen.getByRole("combobox", {
      name: /select contract/i,
    });
    await user.click(contractSelect);
    await user.click(screen.getByRole("option", { name: "DPNS" }));

    await waitFor(() => {
      expect(mockCommands.contractGetById).toHaveBeenCalled();
    });

    const docTypeSelect = screen.getByRole("combobox", {
      name: /select document type/i,
    });
    await user.click(docTypeSelect);
    await user.click(screen.getByRole("option", { name: "domain" }));

    // Enter data
    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated integers for Document/,
    );
    await user.type(input, "aabbccdd");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(
        screen.getByText("Deserialization error: invalid document bytes"),
      ).toBeInTheDocument();
    });

    expect(screen.getByRole("alert")).toBeInTheDocument();
  });

  it("dismisses error when dismiss button is clicked", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mockCommands.parseDocument.mockResolvedValue({
      status: "error",
      error: "Deserialization error: bad data",
    });

    render(<DocumentVisualizerScreen />);

    await waitFor(() => {
      expect(mockCommands.contractListLocal).toHaveBeenCalled();
    });

    // Select contract + doc type
    const contractSelect = screen.getByRole("combobox", {
      name: /select contract/i,
    });
    await user.click(contractSelect);
    await user.click(screen.getByRole("option", { name: "DPNS" }));

    await waitFor(() => {
      expect(mockCommands.contractGetById).toHaveBeenCalled();
    });

    const docTypeSelect = screen.getByRole("combobox", {
      name: /select document type/i,
    });
    await user.click(docTypeSelect);
    await user.click(screen.getByRole("option", { name: "domain" }));

    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated integers for Document/,
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

  it("filters contracts by search term", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    render(<DocumentVisualizerScreen />);

    await waitFor(() => {
      expect(mockCommands.contractListLocal).toHaveBeenCalled();
    });

    // Type a search term
    const searchInput = screen.getByPlaceholderText("Filter contracts...");
    await user.type(searchInput, "DPNS");

    // Open the select to see filtered options
    const contractSelect = screen.getByRole("combobox", {
      name: /select contract/i,
    });
    await user.click(contractSelect);

    // Should show DPNS but not the other contract
    expect(screen.getByRole("option", { name: "DPNS" })).toBeInTheDocument();
    // The other contract has no alias so it shows its id — it shouldn't match "DPNS"
    const allOptions = screen.getAllByRole("option");
    expect(allOptions).toHaveLength(1);
  });

  it("shows decode error for unknown input format", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    render(<DocumentVisualizerScreen />);

    await waitFor(() => {
      expect(mockCommands.contractListLocal).toHaveBeenCalled();
    });

    // Select contract + doc type
    const contractSelect = screen.getByRole("combobox", {
      name: /select contract/i,
    });
    await user.click(contractSelect);
    await user.click(screen.getByRole("option", { name: "DPNS" }));

    await waitFor(() => {
      expect(mockCommands.contractGetById).toHaveBeenCalled();
    });

    const docTypeSelect = screen.getByRole("combobox", {
      name: /select document type/i,
    });
    await user.click(docTypeSelect);
    await user.click(screen.getByRole("option", { name: "domain" }));

    // Type invalid data
    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated integers for Document/,
    );
    await user.type(input, "xyz!@#$%");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(screen.getByRole("alert")).toBeInTheDocument();
      expect(screen.getByText(/Unable to decode input/)).toBeInTheDocument();
    });

    // parseDocument should NOT have been called
    expect(mockCommands.parseDocument).not.toHaveBeenCalled();
  });

  it("does not call parseDocument when only contract is selected (no doc type)", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    render(<DocumentVisualizerScreen />);

    await waitFor(() => {
      expect(mockCommands.contractListLocal).toHaveBeenCalled();
    });

    // Select contract only
    const contractSelect = screen.getByRole("combobox", {
      name: /select contract/i,
    });
    await user.click(contractSelect);
    await user.click(screen.getByRole("option", { name: "DPNS" }));

    // Enter data without selecting doc type
    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated integers for Document/,
    );
    await user.type(input, "deadbeef");
    vi.advanceTimersByTime(350);

    // Should still show waiting-for-selection
    await waitFor(() => {
      expect(
        screen.getByText("Select a contract and document type."),
      ).toBeInTheDocument();
    });

    expect(mockCommands.parseDocument).not.toHaveBeenCalled();
  });

  it("handles command throwing an exception", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mockCommands.parseDocument.mockRejectedValue(
      new Error("Backend unavailable"),
    );

    render(<DocumentVisualizerScreen />);

    await waitFor(() => {
      expect(mockCommands.contractListLocal).toHaveBeenCalled();
    });

    // Select contract + doc type
    const contractSelect = screen.getByRole("combobox", {
      name: /select contract/i,
    });
    await user.click(contractSelect);
    await user.click(screen.getByRole("option", { name: "DPNS" }));

    await waitFor(() => {
      expect(mockCommands.contractGetById).toHaveBeenCalled();
    });

    const docTypeSelect = screen.getByRole("combobox", {
      name: /select document type/i,
    });
    await user.click(docTypeSelect);
    await user.click(screen.getByRole("option", { name: "domain" }));

    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated integers for Document/,
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

    mockCommands.parseDocument.mockResolvedValue({
      status: "ok",
      data: { json: '{"$id": "from-base64"}' },
    });

    render(<DocumentVisualizerScreen />);

    await waitFor(() => {
      expect(mockCommands.contractListLocal).toHaveBeenCalled();
    });

    // Select contract + doc type
    const contractSelect = screen.getByRole("combobox", {
      name: /select contract/i,
    });
    await user.click(contractSelect);
    await user.click(screen.getByRole("option", { name: "DPNS" }));

    await waitFor(() => {
      expect(mockCommands.contractGetById).toHaveBeenCalled();
    });

    const docTypeSelect = screen.getByRole("combobox", {
      name: /select document type/i,
    });
    await user.click(docTypeSelect);
    await user.click(screen.getByRole("option", { name: "domain" }));

    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated integers for Document/,
    );
    await user.type(input, "SGVsbG8gV29ybGQ=");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(mockCommands.parseDocument).toHaveBeenCalledWith({
        hexData: "48656c6c6f20576f726c64",
        contractId: mockContracts[0].id,
        documentTypeName: "domain",
      });
    });
  });

  it("supports comma-separated integer input format", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    mockCommands.parseDocument.mockResolvedValue({
      status: "ok",
      data: { json: '{"$id": "from-csv"}' },
    });

    render(<DocumentVisualizerScreen />);

    await waitFor(() => {
      expect(mockCommands.contractListLocal).toHaveBeenCalled();
    });

    // Select contract + doc type
    const contractSelect = screen.getByRole("combobox", {
      name: /select contract/i,
    });
    await user.click(contractSelect);
    await user.click(screen.getByRole("option", { name: "DPNS" }));

    await waitFor(() => {
      expect(mockCommands.contractGetById).toHaveBeenCalled();
    });

    const docTypeSelect = screen.getByRole("combobox", {
      name: /select document type/i,
    });
    await user.click(docTypeSelect);
    await user.click(screen.getByRole("option", { name: "domain" }));

    const input = screen.getByLabelText(
      /Enter hex, base64, or comma-separated integers for Document/,
    );
    await user.type(input, "0,255,128");
    vi.advanceTimersByTime(350);

    await waitFor(() => {
      expect(mockCommands.parseDocument).toHaveBeenCalledWith({
        hexData: "00ff80",
        contractId: mockContracts[0].id,
        documentTypeName: "domain",
      });
    });
  });
});
