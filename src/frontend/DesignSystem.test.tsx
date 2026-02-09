import { render, screen, within } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { DesignSystem } from "./DesignSystem";
import { ThemeProvider } from "@/components/theme";

vi.mock("@/bindings", () => ({
  commands: {
    settingsGet: vi.fn().mockResolvedValue({
      status: "ok",
      data: { themeMode: "light" },
    }),
    systemUpdateTheme: vi.fn().mockResolvedValue({ taskId: "1" }),
  },
}));

function renderDesignSystem(defaultTheme: "light" | "dark" | "system" = "light") {
  return render(
    <ThemeProvider defaultTheme={defaultTheme}>
      <DesignSystem />
    </ThemeProvider>,
  );
}

describe("DesignSystem", () => {
  beforeEach(() => {
    document.documentElement.classList.remove("light", "dark");
  });

  it("renders the design system page", () => {
    renderDesignSystem();
    expect(screen.getByTestId("design-system")).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: /design system/i }),
    ).toBeInTheDocument();
  });

  it("displays the current theme in the subtitle", () => {
    renderDesignSystem("light");
    expect(screen.getByText(/current theme: light/i)).toBeInTheDocument();
  });

  // === Colors Section ===
  it("renders brand color swatches", () => {
    renderDesignSystem();
    expect(screen.getByText("Dash Blue")).toBeInTheDocument();
    expect(screen.getByText("Deep Blue")).toBeInTheDocument();
    expect(screen.getByText("Midnight")).toBeInTheDocument();
  });

  it("renders semantic color swatches", () => {
    renderDesignSystem();
    const colorSection = screen.getByText("Semantic Colors").closest("div")!;
    expect(within(colorSection).getByText("Primary")).toBeInTheDocument();
    expect(within(colorSection).getByText("Destructive")).toBeInTheDocument();
    expect(within(colorSection).getByText("Success")).toBeInTheDocument();
    expect(within(colorSection).getByText("Warning")).toBeInTheDocument();
    expect(within(colorSection).getByText("Info")).toBeInTheDocument();
  });

  it("renders network accent swatches", () => {
    renderDesignSystem();
    const networkSection = screen.getByText("Network Accents").closest("div")!;
    expect(within(networkSection).getByText("Mainnet")).toBeInTheDocument();
    expect(within(networkSection).getByText("Testnet")).toBeInTheDocument();
    expect(within(networkSection).getByText("Devnet")).toBeInTheDocument();
    expect(within(networkSection).getByText("Local")).toBeInTheDocument();
  });

  // === Buttons Section ===
  it("renders all button variants", () => {
    renderDesignSystem();
    expect(
      screen.getByRole("button", { name: "Primary" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Secondary" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Outline" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Ghost" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Destructive" }),
    ).toBeInTheDocument();
  });

  it("renders button size variants", () => {
    renderDesignSystem();
    expect(
      screen.getByRole("button", { name: "Extra Small" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Small" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Large" }),
    ).toBeInTheDocument();
  });

  it("renders a disabled button", () => {
    renderDesignSystem();
    expect(
      screen.getByRole("button", { name: "Disabled" }),
    ).toBeDisabled();
  });

  // === Inputs Section ===
  it("renders input variants", () => {
    renderDesignSystem();
    expect(
      screen.getByPlaceholderText("Default input"),
    ).toBeInTheDocument();
    expect(screen.getByPlaceholderText("Disabled input")).toBeDisabled();
    expect(
      screen.getByPlaceholderText("Password input"),
    ).toHaveAttribute("type", "password");
    expect(
      screen.getByPlaceholderText("Invalid input"),
    ).toHaveAttribute("aria-invalid", "true");
  });

  // === Badges Section ===
  it("renders badge variants", () => {
    renderDesignSystem();
    const badgeSection = screen.getByText("Badges").closest("div")!;
    expect(within(badgeSection).getByText("Default")).toBeInTheDocument();
    expect(within(badgeSection).getByText("Outline")).toBeInTheDocument();
  });

  it("renders network badges", () => {
    renderDesignSystem();
    const mainnetBadge = screen
      .getAllByText("Mainnet")
      .find((el) => el.classList.contains("network-badge"));
    expect(mainnetBadge).toBeInTheDocument();
    expect(mainnetBadge).toHaveClass("bg-network-mainnet");
  });

  // === Cards Section ===
  it("renders card components", () => {
    renderDesignSystem();
    expect(screen.getByText("Wallet Balance")).toBeInTheDocument();
    expect(screen.getByText("Identity Status")).toBeInTheDocument();
    // "1.234 DASH" appears in both Card and Table — verify at least one exists
    expect(screen.getAllByText("1.234 DASH").length).toBeGreaterThanOrEqual(1);
  });

  // === Tabs Section ===
  it("renders tabs with correct trigger labels", () => {
    renderDesignSystem();
    const tabsDemo = screen.getByTestId("tabs-demo");
    expect(within(tabsDemo).getByText("Active Contests")).toBeInTheDocument();
    expect(within(tabsDemo).getByText("Past Contests")).toBeInTheDocument();
    expect(within(tabsDemo).getByText("My Usernames")).toBeInTheDocument();
  });

  // === Table Section ===
  it("renders table with sample data", () => {
    renderDesignSystem();
    expect(screen.getByText("alice")).toBeInTheDocument();
    expect(screen.getByText("bob")).toBeInTheDocument();
    expect(screen.getByText("masternode-1")).toBeInTheDocument();
    expect(screen.getByText("Sample identity list")).toBeInTheDocument();
  });

  it("renders table column headers", () => {
    renderDesignSystem();
    expect(screen.getByText("Alias")).toBeInTheDocument();
    expect(screen.getByText("Identity ID")).toBeInTheDocument();
    expect(screen.getByText("Type")).toBeInTheDocument();
    expect(screen.getByText("Balance")).toBeInTheDocument();
  });

  // === Feedback Components Section ===
  it("renders empty state components", () => {
    renderDesignSystem();
    expect(screen.getByText("No wallets")).toBeInTheDocument();
    expect(screen.getByText("No identities")).toBeInTheDocument();
    expect(screen.getByText("No tokens")).toBeInTheDocument();
  });

  it("renders loading spinners at all sizes", () => {
    renderDesignSystem();
    const spinners = screen.getAllByText("Loading...");
    expect(spinners).toHaveLength(3);
  });

  it("renders progress bars with labels", () => {
    renderDesignSystem();
    expect(screen.getByText("SPV Sync")).toBeInTheDocument();
    expect(screen.getByText("25%")).toBeInTheDocument();
  });

  // === Connection Indicators Section ===
  it("renders connection status indicators", () => {
    renderDesignSystem();
    const connected = screen.getAllByText("Connected");
    expect(connected.length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText("Disconnected")).toBeInTheDocument();
  });

  // === Typography Section ===
  it("renders typography samples", () => {
    renderDesignSystem();
    expect(screen.getByText("Display (36px)")).toBeInTheDocument();
    expect(screen.getByText("Body (16px)")).toBeInTheDocument();
    expect(screen.getByText(/JetBrains Mono/)).toBeInTheDocument();
  });

  // === Shadows Section ===
  it("renders shadow examples", () => {
    renderDesignSystem();
    expect(screen.getByText("shadow-sm")).toBeInTheDocument();
    expect(screen.getByText("shadow-elevated")).toBeInTheDocument();
    expect(screen.getByText("shadow-glow")).toBeInTheDocument();
  });

  // === Theme Integration ===
  it("applies light theme class to document root", () => {
    renderDesignSystem("light");
    expect(document.documentElement).toHaveClass("light");
  });

  it("applies dark theme class to document root", () => {
    renderDesignSystem("dark");
    expect(document.documentElement).toHaveClass("dark");
  });
});
