import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  TokenInfoDialog,
  truncateHex,
  formatSupply,
} from "./TokenInfoDialog";
import type { TokenInfoData, TokenInfoDialogProps } from "./TokenInfoDialog";

// ─── Mocks ──────────────────────────────────────────────────────────

vi.mock("@/components/theme/ThemeProvider", () => ({
  useTheme: () => ({
    resolvedTheme: "light",
    theme: "light",
    setTheme: () => {},
  }),
}));

// ─── Fixtures ───────────────────────────────────────────────────────

function makeToken(overrides: Partial<TokenInfoData> = {}): TokenInfoData {
  return {
    name: "DashToken",
    tokenId:
      "aaaa1111bbbb2222cccc3333dddd4444eeee5555ffff6666000011112222333344",
    contractId:
      "cccc1111dddd2222eeee3333ffff444400001111222233334444555566667777",
    tokenPosition: 0,
    decimals: 8,
    description: "A test token for Dash platform",
    baseSupply: "100000000000",
    maxSupply: null,
    hasPerpetualDistribution: false,
    hasPreprogrammedDistribution: false,
    ownerIdentityId:
      "ownr1111ownr2222ownr3333ownr4444ownr5555ownr6666ownr7777ownr8888",
    paused: false,
    configurationJson: {
      conventions: { decimals: 8, localizations: {} },
      baseSupply: "100000000000",
      maxSupply: null,
    },
    ...overrides,
  };
}

const defaultProps: TokenInfoDialogProps = {
  open: true,
  onOpenChange: vi.fn(),
  token: makeToken(),
};

function setup(props: Partial<TokenInfoDialogProps> = {}) {
  const mergedProps = { ...defaultProps, ...props };
  return {
    user: userEvent.setup(),
    ...render(<TokenInfoDialog {...mergedProps} />),
    props: mergedProps,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

// ─── Unit tests for helpers ─────────────────────────────────────────

describe("truncateHex", () => {
  it("returns short strings unchanged", () => {
    expect(truncateHex("abcdef")).toBe("abcdef");
  });

  it("truncates long hex strings", () => {
    const long =
      "aaaa1111bbbb2222cccc3333dddd4444eeee5555ffff6666000011112222333344";
    const result = truncateHex(long, 12);
    expect(result).toContain("...");
    expect(result.startsWith("aaaa1111bbbb")).toBe(true);
    expect(result.endsWith("112222333344")).toBe(true);
  });

  it("uses default char count of 12", () => {
    const long =
      "aaaa1111bbbb2222cccc3333dddd4444eeee5555ffff6666000011112222333344";
    const result = truncateHex(long);
    expect(result.length).toBeLessThan(long.length);
  });
});

describe("formatSupply", () => {
  it("returns '0' for null/undefined", () => {
    expect(formatSupply(null, 8)).toBe("0");
    expect(formatSupply(undefined, 8)).toBe("0");
  });

  it("returns '0' for '0'", () => {
    expect(formatSupply("0", 8)).toBe("0");
  });

  it("formats with decimals", () => {
    expect(formatSupply("100000000", 8)).toBe("1");
  });

  it("formats with trailing decimals", () => {
    expect(formatSupply("150000000", 8)).toBe("1.5");
  });

  it("formats with 0 decimals", () => {
    expect(formatSupply("12345", 0)).toBe("12345");
  });

  it("formats small values", () => {
    expect(formatSupply("1", 8)).toBe("0.00000001");
  });
});

// ─── Component tests ────────────────────────────────────────────────

describe("TokenInfoDialog", () => {
  it("renders nothing when token is null", () => {
    const { container } = render(
      <TokenInfoDialog open={true} onOpenChange={vi.fn()} token={null} />,
    );
    expect(container.innerHTML).toBe("");
  });

  it("renders nothing when closed", () => {
    setup({ open: false });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("renders the dialog with token name as title", () => {
    setup();
    expect(screen.getByText("DashToken")).toBeInTheDocument();
  });

  it("shows 'Unnamed Token' when name is null", () => {
    setup({ token: makeToken({ name: null }) });
    expect(screen.getByText("Unnamed Token")).toBeInTheDocument();
  });

  it("shows paused badge when token is paused", () => {
    setup({ token: makeToken({ paused: true }) });
    expect(screen.getByText("Paused")).toBeInTheDocument();
  });

  it("does not show paused badge when not paused", () => {
    setup({ token: makeToken({ paused: false }) });
    expect(screen.queryByText("Paused")).not.toBeInTheDocument();
  });

  it("shows description in basic info", () => {
    setup();
    expect(
      screen.getByText("A test token for Dash platform"),
    ).toBeInTheDocument();
  });

  it("shows 'No description' when description is null", () => {
    setup({ token: makeToken({ description: null }) });
    expect(screen.getByText("No description")).toBeInTheDocument();
  });

  it("shows base supply formatted", () => {
    setup({ token: makeToken({ baseSupply: "100000000000", decimals: 8 }) });
    expect(screen.getByText("1000")).toBeInTheDocument();
  });

  it("shows 'Unlimited' for null max supply", () => {
    setup({ token: makeToken({ maxSupply: null }) });
    expect(screen.getByText("Unlimited")).toBeInTheDocument();
  });

  it("shows formatted max supply when present", () => {
    setup({
      token: makeToken({ maxSupply: "500000000000", decimals: 8 }),
    });
    expect(screen.getByText("5000")).toBeInTheDocument();
  });

  it("shows decimals value", () => {
    setup({ token: makeToken({ decimals: 8 }) });
    const decimalsEl = screen.getByTestId("info-decimals");
    expect(decimalsEl).toHaveTextContent("8");
  });

  it("shows perpetual distribution status", () => {
    setup({ token: makeToken({ hasPerpetualDistribution: true }) });
    const el = screen.getByTestId("info-perpetual-distribution");
    expect(el).toHaveTextContent("Yes");
  });

  it("shows preprogrammed distribution status", () => {
    setup({ token: makeToken({ hasPreprogrammedDistribution: false }) });
    const el = screen.getByTestId("info-preprogrammed-distribution");
    expect(el).toHaveTextContent("No");
  });

  it("shows truncated token ID with copy button", () => {
    setup();
    const tokenIdEl = screen.getByTestId("info-token-id");
    expect(tokenIdEl.textContent).toContain("...");
    // Copy button should be near the token ID
    const row = tokenIdEl.closest("div.flex");
    expect(row).not.toBeNull();
    expect(
      within(row!).getByRole("button", { name: /copy/i }),
    ).toBeInTheDocument();
  });

  it("shows truncated contract ID with copy button", () => {
    setup();
    const contractIdEl = screen.getByTestId("info-contract-id");
    expect(contractIdEl.textContent).toContain("...");
  });

  it("shows contract owner when provided", () => {
    setup();
    expect(screen.getByText("Contract Owner")).toBeInTheDocument();
    const ownerEl = screen.getByTestId("info-contract-owner");
    expect(ownerEl.textContent).toContain("...");
  });

  it("hides contract owner when not provided", () => {
    setup({ token: makeToken({ ownerIdentityId: null }) });
    expect(screen.queryByText("Contract Owner")).not.toBeInTheDocument();
  });

  it("shows token position", () => {
    setup({ token: makeToken({ tokenPosition: 3 }) });
    const el = screen.getByTestId("info-token-position");
    expect(el).toHaveTextContent("3");
  });

  it("calls onOpenChange when Close button is clicked", async () => {
    const onOpenChange = vi.fn();
    const { user } = setup({ onOpenChange });
    const closeButtons = screen.getAllByRole("button", { name: /close/i });
    // Click the footer Close button (text button, not the X icon)
    const footerClose = closeButtons.find(
      (btn) => btn.textContent === "Close",
    );
    expect(footerClose).toBeDefined();
    await user.click(footerClose!);
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("renders Token Configuration section when configurationJson is provided", () => {
    setup();
    expect(screen.getByText("Token Configuration")).toBeInTheDocument();
  });

  it("does not render Token Configuration section when configurationJson is absent", () => {
    setup({ token: makeToken({ configurationJson: undefined }) });
    expect(
      screen.queryByText("Token Configuration"),
    ).not.toBeInTheDocument();
  });

  it("can toggle Basic Information section collapse", async () => {
    const { user } = setup();
    const basicInfoToggle = screen.getByText("Basic Information");
    // Initially open — description should be visible
    expect(
      screen.getByText("A test token for Dash platform"),
    ).toBeInTheDocument();

    // Click to collapse
    await user.click(basicInfoToggle);
    expect(
      screen.queryByText("A test token for Dash platform"),
    ).not.toBeInTheDocument();

    // Click to expand again
    await user.click(basicInfoToggle);
    expect(
      screen.getByText("A test token for Dash platform"),
    ).toBeInTheDocument();
  });

  it("Token Configuration is collapsed by default", () => {
    setup();
    // The collapsible starts closed, so the JsonViewer content is not rendered
    const configSection = screen.getByText("Token Configuration");
    expect(configSection).toBeInTheDocument();
    // The config JSON description text should not be visible
    expect(
      screen.queryByText("Full token configuration schema:"),
    ).not.toBeInTheDocument();
  });

  it("can expand Token Configuration section", async () => {
    const { user } = setup();
    const configToggle = screen.getByText("Token Configuration");
    await user.click(configToggle);
    expect(
      screen.getByText("Full token configuration schema:"),
    ).toBeInTheDocument();
  });

  it("shows 'N/A' for baseSupply when not provided", () => {
    setup({ token: makeToken({ baseSupply: null }) });
    expect(screen.getByText("N/A")).toBeInTheDocument();
  });
});
