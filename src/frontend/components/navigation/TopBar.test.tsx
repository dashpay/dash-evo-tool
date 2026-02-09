import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TopBar } from "./TopBar";

// Mock ThemeToggle to avoid provider dependency
vi.mock("@/components/theme", () => ({
  ThemeToggle: () => <button data-testid="theme-toggle">Theme</button>,
}));

const defaultProps = {};

beforeEach(() => {
  vi.clearAllMocks();
});

describe("TopBar", () => {
  it("renders with banner role", () => {
    render(<TopBar {...defaultProps} />);
    expect(screen.getByRole("banner")).toBeInTheDocument();
  });

  it("renders connection indicator", () => {
    render(<TopBar {...defaultProps} />);
    expect(screen.getByTestId("connection-indicator")).toBeInTheDocument();
  });

  it("shows connected state", () => {
    render(<TopBar connected={true} />);
    const indicator = screen.getByTestId("connection-indicator");
    expect(indicator.querySelector(".connection-dot-connected")).toBeInTheDocument();
    expect(indicator).toHaveAccessibleName(
      "Connected to Dash Core Wallet",
    );
  });

  it("shows disconnected state", () => {
    render(<TopBar connected={false} />);
    const indicator = screen.getByTestId("connection-indicator");
    expect(
      indicator.querySelector(".connection-dot-disconnected"),
    ).toBeInTheDocument();
    expect(indicator).toHaveAccessibleName(
      "Disconnected from Dash Core Wallet. Click to start.",
    );
  });

  it("calls onConnectionClick when disconnected indicator is clicked", async () => {
    const user = userEvent.setup();
    const onConnectionClick = vi.fn();
    render(
      <TopBar connected={false} onConnectionClick={onConnectionClick} />,
    );
    await user.click(screen.getByTestId("connection-indicator"));
    expect(onConnectionClick).toHaveBeenCalledTimes(1);
  });

  it("renders breadcrumbs", () => {
    render(
      <TopBar
        breadcrumbs={[
          { label: "Wallets", onClick: vi.fn() },
          { label: "HD Wallet #1" },
        ]}
      />,
    );
    expect(screen.getByText("Wallets")).toBeInTheDocument();
    expect(screen.getByText("HD Wallet #1")).toBeInTheDocument();
  });

  it("makes clickable breadcrumbs interactive", async () => {
    const user = userEvent.setup();
    const onClick = vi.fn();
    render(
      <TopBar
        breadcrumbs={[
          { label: "Wallets", onClick },
          { label: "HD Wallet #1" },
        ]}
      />,
    );
    await user.click(screen.getByText("Wallets"));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("renders non-clickable breadcrumb as text", () => {
    render(
      <TopBar breadcrumbs={[{ label: "Identities" }]} />,
    );
    const text = screen.getByText("Identities");
    expect(text.tagName).toBe("SPAN");
  });

  it("renders network badge", () => {
    render(<TopBar network="testnet" />);
    const badge = screen.getByTestId("top-bar-network-badge");
    expect(badge).toHaveTextContent("Testnet");
  });

  it("renders mainnet badge", () => {
    render(<TopBar network="dash" />);
    const badge = screen.getByTestId("top-bar-network-badge");
    expect(badge).toHaveTextContent("Mainnet");
  });

  it("hides network badge when no network", () => {
    render(<TopBar />);
    expect(screen.queryByTestId("top-bar-network-badge")).not.toBeInTheDocument();
  });

  it("renders individual action buttons", async () => {
    const user = userEvent.setup();
    const onClick = vi.fn();
    render(
      <TopBar
        actions={[
          { label: "Add Identity", onClick },
          { label: "Refresh", onClick: vi.fn() },
        ]}
      />,
    );
    expect(screen.getByTestId("action-add-identity")).toBeInTheDocument();
    expect(screen.getByTestId("action-refresh")).toBeInTheDocument();

    await user.click(screen.getByTestId("action-add-identity"));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("renders action group dropdown menus", async () => {
    const user = userEvent.setup();
    const createDoc = vi.fn();
    render(
      <TopBar
        actionGroups={[
          {
            label: "Documents",
            actions: [
              { label: "Create Document", onClick: createDoc },
              { label: "Delete Document", onClick: vi.fn() },
            ],
          },
        ]}
      />,
    );
    const trigger = screen.getByTestId("action-group-documents");
    expect(trigger).toHaveTextContent("Documents");

    // Open the dropdown
    await user.click(trigger);
    expect(screen.getByText("Create Document")).toBeInTheDocument();
    expect(screen.getByText("Delete Document")).toBeInTheDocument();

    // Click an item
    await user.click(screen.getByText("Create Document"));
    expect(createDoc).toHaveBeenCalledTimes(1);
  });

  it("renders theme toggle", () => {
    render(<TopBar />);
    expect(screen.getByTestId("theme-toggle")).toBeInTheDocument();
  });

  it("has breadcrumb navigation landmark", () => {
    render(
      <TopBar breadcrumbs={[{ label: "Test" }]} />,
    );
    expect(
      screen.getByRole("navigation", { name: "Breadcrumb" }),
    ).toBeInTheDocument();
  });
});
