import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Sidebar, navItems, getActiveSectionFromPath } from "./Sidebar";
import { renderWithProviders } from "@/test/router-utils";

const defaultProps = {
  onNavigate: vi.fn(),
};

beforeEach(() => {
  vi.clearAllMocks();
});

describe("Sidebar", () => {
  it("renders all 7 navigation items", () => {
    renderWithProviders(<Sidebar {...defaultProps} />);
    for (const item of navItems) {
      expect(screen.getByTestId(`nav-${item.id}`)).toBeInTheDocument();
    }
  });

  it("renders navigation labels when expanded", () => {
    renderWithProviders(<Sidebar {...defaultProps} />);
    expect(screen.getByText("DashPay")).toBeInTheDocument();
    expect(screen.getByText("Identities")).toBeInTheDocument();
    expect(screen.getByText("Contracts")).toBeInTheDocument();
    expect(screen.getByText("Tokens")).toBeInTheDocument();
    expect(screen.getByText("Wallets")).toBeInTheDocument();
    expect(screen.getByText("Tools")).toBeInTheDocument();
    expect(screen.getByText("Settings")).toBeInTheDocument();
  });

  it("hides labels when collapsed", () => {
    renderWithProviders(<Sidebar {...defaultProps} collapsed={true} />);
    expect(screen.queryByText("DashPay")).not.toBeInTheDocument();
    expect(screen.queryByText("Identities")).not.toBeInTheDocument();
  });

  it("highlights active section with aria-current", () => {
    renderWithProviders(
      <Sidebar {...defaultProps} activeSection="identities" />,
    );
    const identitiesBtn = screen.getByTestId("nav-identities");
    expect(identitiesBtn).toHaveAttribute("aria-current", "page");

    // Other items should not have aria-current
    const walletsBtn = screen.getByTestId("nav-wallets");
    expect(walletsBtn).not.toHaveAttribute("aria-current");
  });

  it("calls onNavigate with correct path on click", async () => {
    const user = userEvent.setup();
    const onNavigate = vi.fn();
    renderWithProviders(<Sidebar {...defaultProps} onNavigate={onNavigate} />);

    await user.click(screen.getByTestId("nav-wallets"));
    expect(onNavigate).toHaveBeenCalledWith("/wallets");

    await user.click(screen.getByTestId("nav-tokens"));
    expect(onNavigate).toHaveBeenCalledWith("/tokens");
  });

  it("shows network badge for non-mainnet networks", () => {
    renderWithProviders(
      <Sidebar {...defaultProps} network="testnet" />,
    );
    expect(screen.getByTestId("network-badge")).toHaveTextContent("Testnet");
  });

  it("shows devnet badge", () => {
    renderWithProviders(
      <Sidebar {...defaultProps} network="devnet" />,
    );
    expect(screen.getByTestId("network-badge")).toHaveTextContent("Devnet");
  });

  it("shows local network badge", () => {
    renderWithProviders(
      <Sidebar {...defaultProps} network="regtest" />,
    );
    expect(screen.getByTestId("network-badge")).toHaveTextContent(
      "Local Network",
    );
  });

  it("does not show network badge for mainnet", () => {
    renderWithProviders(<Sidebar {...defaultProps} network="dash" />);
    expect(screen.queryByTestId("network-badge")).not.toBeInTheDocument();
  });

  it("shows developer mode badge when enabled", () => {
    renderWithProviders(
      <Sidebar {...defaultProps} developerMode={true} />,
    );
    expect(screen.getByTestId("dev-mode-badge")).toBeInTheDocument();
  });

  it("hides developer mode badge when disabled", () => {
    renderWithProviders(
      <Sidebar {...defaultProps} developerMode={false} />,
    );
    expect(screen.queryByTestId("dev-mode-badge")).not.toBeInTheDocument();
  });

  it("developer mode badge navigates to settings on click", async () => {
    const user = userEvent.setup();
    const onNavigate = vi.fn();
    renderWithProviders(
      <Sidebar
        {...defaultProps}
        onNavigate={onNavigate}
        developerMode={true}
      />,
    );
    await user.click(screen.getByTestId("dev-mode-badge"));
    expect(onNavigate).toHaveBeenCalledWith("/settings");
  });

  it("shows collapse toggle and calls onToggleCollapsed", async () => {
    const user = userEvent.setup();
    const onToggleCollapsed = vi.fn();
    renderWithProviders(
      <Sidebar
        {...defaultProps}
        onToggleCollapsed={onToggleCollapsed}
      />,
    );
    const toggle = screen.getByTestId("sidebar-collapse-toggle");
    expect(toggle).toBeInTheDocument();
    await user.click(toggle);
    expect(onToggleCollapsed).toHaveBeenCalledTimes(1);
  });

  it("collapse toggle shows correct label based on state", () => {
    const { rerender } = renderWithProviders(
      <Sidebar
        {...defaultProps}
        collapsed={false}
        onToggleCollapsed={vi.fn()}
      />,
    );
    expect(screen.getByText("Collapse")).toBeInTheDocument();
    expect(
      screen.getByLabelText("Collapse sidebar"),
    ).toBeInTheDocument();

    rerender(
      <Sidebar
        {...defaultProps}
        collapsed={true}
        onToggleCollapsed={vi.fn()}
      />,
    );
    expect(
      screen.getByLabelText("Expand sidebar"),
    ).toBeInTheDocument();
  });

  it("has correct width classes for collapsed vs expanded", () => {
    const { rerender } = renderWithProviders(
      <Sidebar {...defaultProps} collapsed={false} />,
    );
    const sidebar = screen.getByTestId("sidebar");
    expect(sidebar.className).toContain("w-[200px]");

    rerender(<Sidebar {...defaultProps} collapsed={true} />);
    expect(sidebar.className).toContain("w-[72px]");
  });

  it("has navigation landmark", () => {
    renderWithProviders(<Sidebar {...defaultProps} />);
    expect(
      screen.getByRole("navigation", { name: "Main navigation" }),
    ).toBeInTheDocument();
  });
});

describe("getActiveSectionFromPath", () => {
  it("maps root paths correctly", () => {
    expect(getActiveSectionFromPath("/identities")).toBe("identities");
    expect(getActiveSectionFromPath("/wallets")).toBe("wallets");
    expect(getActiveSectionFromPath("/dashpay")).toBe("dashpay");
    expect(getActiveSectionFromPath("/contracts")).toBe("contracts");
    expect(getActiveSectionFromPath("/tokens")).toBe("tokens");
    expect(getActiveSectionFromPath("/tools")).toBe("tools");
    expect(getActiveSectionFromPath("/settings")).toBe("settings");
  });

  it("maps sub-paths to parent section", () => {
    expect(getActiveSectionFromPath("/dashpay/contacts")).toBe("dashpay");
    expect(getActiveSectionFromPath("/dashpay/profile")).toBe("dashpay");
    expect(getActiveSectionFromPath("/contracts/dpns/active")).toBe("contracts");
    expect(getActiveSectionFromPath("/tokens/search")).toBe("tokens");
    expect(getActiveSectionFromPath("/tools/proof-log")).toBe("tools");
  });

  it("returns undefined for unknown paths", () => {
    expect(getActiveSectionFromPath("/")).toBeUndefined();
    expect(getActiveSectionFromPath("/unknown")).toBeUndefined();
  });
});
