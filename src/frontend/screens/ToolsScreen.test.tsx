import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { ToolsScreen } from "./ToolsScreen";

const mockNavigate = vi.fn();
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

describe("ToolsScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders the page title", () => {
    render(<ToolsScreen />);
    expect(
      screen.getByRole("heading", { name: "Tools" }),
    ).toBeInTheDocument();
  });

  it("renders the subtitle", () => {
    render(<ToolsScreen />);
    expect(
      screen.getByText("Platform utilities and data inspection tools"),
    ).toBeInTheDocument();
  });

  // Category headings
  it("renders category headings", () => {
    render(<ToolsScreen />);
    expect(screen.getByText("Query & Inspection")).toBeInTheDocument();
    expect(screen.getByText("Deserializers")).toBeInTheDocument();
    expect(screen.getByText("Advanced")).toBeInTheDocument();
  });

  // All 9 tool cards
  it("renders all tool cards", () => {
    render(<ToolsScreen />);
    expect(
      screen.getByRole("button", { name: /platform info/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /address balance/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /proof log/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /masternode list diff/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /transition visualizer/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /contract visualizer/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /document visualizer/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /proof visualizer/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /grovestark/i }),
    ).toBeInTheDocument();
  });

  // Tool descriptions
  it("renders tool descriptions", () => {
    render(<ToolsScreen />);
    expect(
      screen.getByText(/fetch platform data/i),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/look up the balance/i),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/zero-knowledge proofs/i),
    ).toBeInTheDocument();
  });

  // Navigation
  it("navigates to Platform Info when clicked", async () => {
    const user = userEvent.setup();
    render(<ToolsScreen />);
    await user.click(screen.getByRole("button", { name: /platform info/i }));
    expect(mockNavigate).toHaveBeenCalledWith({
      to: "/tools/platform-info",
    });
  });

  it("navigates to Address Balance when clicked", async () => {
    const user = userEvent.setup();
    render(<ToolsScreen />);
    await user.click(
      screen.getByRole("button", { name: /address balance/i }),
    );
    expect(mockNavigate).toHaveBeenCalledWith({
      to: "/tools/address-balance",
    });
  });

  it("navigates to Proof Log when clicked", async () => {
    const user = userEvent.setup();
    render(<ToolsScreen />);
    await user.click(screen.getByRole("button", { name: /proof log/i }));
    expect(mockNavigate).toHaveBeenCalledWith({
      to: "/tools/proof-log",
    });
  });

  it("navigates to Transition Visualizer when clicked", async () => {
    const user = userEvent.setup();
    render(<ToolsScreen />);
    await user.click(
      screen.getByRole("button", { name: /transition visualizer/i }),
    );
    expect(mockNavigate).toHaveBeenCalledWith({
      to: "/tools/transition-visualizer",
    });
  });

  it("navigates to GroveSTARK when clicked", async () => {
    const user = userEvent.setup();
    render(<ToolsScreen />);
    await user.click(screen.getByRole("button", { name: /grovestark/i }));
    expect(mockNavigate).toHaveBeenCalledWith({
      to: "/tools/grovestark",
    });
  });

  it("navigates to Masternode List Diff when clicked", async () => {
    const user = userEvent.setup();
    render(<ToolsScreen />);
    await user.click(
      screen.getByRole("button", { name: /masternode list diff/i }),
    );
    expect(mockNavigate).toHaveBeenCalledWith({
      to: "/tools/masternode-list",
    });
  });

  // Card structure
  it("renders tool cards as interactive buttons", () => {
    render(<ToolsScreen />);
    const cards = screen.getAllByRole("button");
    expect(cards.length).toBe(9);
  });

  // Wrapped in Island
  it("wraps content in an Island", () => {
    const { container } = render(<ToolsScreen />);
    expect(container.querySelector(".island")).toBeInTheDocument();
  });
});
