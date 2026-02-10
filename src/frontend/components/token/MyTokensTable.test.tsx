import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { MyTokensTable, formatTokenBalance, truncateId } from "./MyTokensTable";
import type { MyTokensTableProps } from "./MyTokensTable";
import type { TokenEntry } from "@/stores/tokenStore";
import type { TokenSortColumn, TokenSortOrder } from "@/stores/tokenStore";
import { TooltipProvider } from "@/components/ui/tooltip";

// ─── Test fixtures ──────────────────────────────────────────────────

function makeToken(overrides: Partial<TokenEntry> = {}): TokenEntry {
  return {
    identityId: "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234",
    tokenId: "token1111222233334444555566667777888899990000aaaabbbbccccddddeeee",
    contractId: "contract111122223333444455556666777788889999",
    tokenPosition: 0,
    name: "TestToken",
    ownerAlias: "Alice",
    balance: "100000000",
    decimals: 8,
    ...overrides,
  };
}

const defaultProps: MyTokensTableProps = {
  tokens: [makeToken()],
  sortColumn: "name" as TokenSortColumn,
  sortOrder: "ascending" as TokenSortOrder,
  onSortChange: vi.fn(),
  onAction: vi.fn(),
  onRemove: vi.fn(),
};

function setup(props: Partial<MyTokensTableProps> = {}) {
  const mergedProps = { ...defaultProps, ...props };
  return {
    user: userEvent.setup(),
    ...render(
      <TooltipProvider>
        <MyTokensTable {...mergedProps} />
      </TooltipProvider>,
    ),
    props: mergedProps,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

// ─── Empty state ────────────────────────────────────────────────────

describe("MyTokensTable — empty state", () => {
  it("shows empty state when no tokens", () => {
    setup({ tokens: [] });
    expect(screen.getByText("No tokens yet")).toBeInTheDocument();
    expect(
      screen.getByText("Add a token by ID or create a new one to get started."),
    ).toBeInTheDocument();
  });

  it("does not render table when no tokens", () => {
    setup({ tokens: [] });
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
  });
});

// ─── Table rendering ────────────────────────────────────────────────

describe("MyTokensTable — rendering", () => {
  it("renders table with headers", () => {
    setup();
    expect(screen.getByRole("table")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /owner identity/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /token name/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /balance/i }),
    ).toBeInTheDocument();
  });

  it("renders token name", () => {
    setup();
    expect(screen.getByText("TestToken")).toBeInTheDocument();
  });

  it("renders owner alias", () => {
    setup();
    expect(screen.getByText("Alice")).toBeInTheDocument();
  });

  it("renders formatted balance", () => {
    setup({ tokens: [makeToken({ balance: "100000000", decimals: 8 })] });
    expect(screen.getByText("1")).toBeInTheDocument();
  });

  it("renders 'Unnamed Token' when name is null", () => {
    setup({ tokens: [makeToken({ name: null })] });
    expect(screen.getByText("Unnamed Token")).toBeInTheDocument();
  });

  it("renders truncated identity ID when no alias", () => {
    setup({ tokens: [makeToken({ ownerAlias: null })] });
    // Should show truncated ID
    expect(screen.getByText(/abcd1234\.\.\.abcd1234/)).toBeInTheDocument();
  });

  it("renders multiple tokens", () => {
    const tokens = [
      makeToken({ tokenId: "t1", name: "Alpha" }),
      makeToken({ tokenId: "t2", name: "Beta", identityId: "id2" }),
      makeToken({ tokenId: "t3", name: "Gamma", identityId: "id3" }),
    ];
    setup({ tokens });
    expect(screen.getByText("Alpha")).toBeInTheDocument();
    expect(screen.getByText("Beta")).toBeInTheDocument();
    expect(screen.getByText("Gamma")).toBeInTheDocument();
  });

  it("renders truncated ID with alias present", () => {
    setup();
    // When alias is present, both alias and truncated ID should appear
    expect(screen.getByText("Alice")).toBeInTheDocument();
    expect(screen.getByText(/abcd1234\.\.\.abcd1234/)).toBeInTheDocument();
  });
});

// ─── Sorting ────────────────────────────────────────────────────────

describe("MyTokensTable — sorting", () => {
  it("calls onSortChange with 'ownerAlias' when Owner column clicked", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", { name: /owner identity/i }),
    );
    expect(props.onSortChange).toHaveBeenCalledWith("ownerAlias");
  });

  it("calls onSortChange with 'name' when Token Name column clicked", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", { name: /token name/i }),
    );
    expect(props.onSortChange).toHaveBeenCalledWith("name");
  });

  it("calls onSortChange with 'balance' when Balance column clicked", async () => {
    const { user, props } = setup();
    await user.click(screen.getByRole("button", { name: /balance/i }));
    expect(props.onSortChange).toHaveBeenCalledWith("balance");
  });

  it("shows active sort indicator on sorted column", () => {
    setup({ sortColumn: "name", sortOrder: "ascending" });
    // The Name column button should have an ArrowUp indicator (ascending)
    const nameButton = screen.getByRole("button", { name: /token name/i });
    // ArrowUp should be present (svg has class h-3.5)
    const svg = nameButton.querySelector("svg");
    expect(svg).toBeInTheDocument();
  });
});

// ─── Action dropdown ────────────────────────────────────────────────

describe("MyTokensTable — actions dropdown", () => {
  it("renders actions button for each token", () => {
    setup();
    expect(
      screen.getByRole("button", { name: /actions for testtoken/i }),
    ).toBeInTheDocument();
  });

  it("opens action menu on click", async () => {
    const { user } = setup();
    await user.click(
      screen.getByRole("button", { name: /actions for testtoken/i }),
    );
    // All 15 actions should appear
    expect(screen.getByText("Transfer")).toBeInTheDocument();
    expect(screen.getByText("Mint")).toBeInTheDocument();
    expect(screen.getByText("Burn")).toBeInTheDocument();
    expect(screen.getByText("Freeze")).toBeInTheDocument();
    expect(screen.getByText("Unfreeze")).toBeInTheDocument();
    expect(screen.getByText("Destroy Frozen Funds")).toBeInTheDocument();
    expect(screen.getByText("Pause")).toBeInTheDocument();
    expect(screen.getByText("Resume")).toBeInTheDocument();
    expect(screen.getByText("Claim")).toBeInTheDocument();
    expect(screen.getByText("View Claims")).toBeInTheDocument();
    expect(screen.getByText("Set Price")).toBeInTheDocument();
    expect(screen.getByText("Purchase")).toBeInTheDocument();
    expect(screen.getByText("Update Config")).toBeInTheDocument();
    expect(screen.getByText("More Info")).toBeInTheDocument();
    expect(screen.getByText("Remove")).toBeInTheDocument();
  });

  it("calls onAction with 'transfer' when Transfer clicked", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", { name: /actions for testtoken/i }),
    );
    await user.click(screen.getByText("Transfer"));
    expect(props.onAction).toHaveBeenCalledWith(
      "token1111222233334444555566667777888899990000aaaabbbbccccddddeeee",
      "transfer",
    );
  });

  it("calls onAction with 'mint' when Mint clicked", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", { name: /actions for testtoken/i }),
    );
    await user.click(screen.getByText("Mint"));
    expect(props.onAction).toHaveBeenCalledWith(
      "token1111222233334444555566667777888899990000aaaabbbbccccddddeeee",
      "mint",
    );
  });

  it("calls onAction with 'burn' when Burn clicked", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", { name: /actions for testtoken/i }),
    );
    await user.click(screen.getByText("Burn"));
    expect(props.onAction).toHaveBeenCalledWith(
      expect.any(String),
      "burn",
    );
  });

  it("calls onAction with 'moreInfo' when More Info clicked", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", { name: /actions for testtoken/i }),
    );
    await user.click(screen.getByText("More Info"));
    expect(props.onAction).toHaveBeenCalledWith(
      expect.any(String),
      "moreInfo",
    );
  });

  it("calls onAction with 'freeze' when Freeze clicked", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", { name: /actions for testtoken/i }),
    );
    await user.click(screen.getByText("Freeze"));
    expect(props.onAction).toHaveBeenCalledWith(
      expect.any(String),
      "freeze",
    );
  });

  it("calls onAction with 'purchase' when Purchase clicked", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", { name: /actions for testtoken/i }),
    );
    await user.click(screen.getByText("Purchase"));
    expect(props.onAction).toHaveBeenCalledWith(
      expect.any(String),
      "purchase",
    );
  });
});

// ─── Remove confirmation ────────────────────────────────────────────

describe("MyTokensTable — remove confirmation", () => {
  it("shows confirmation dialog when Remove is clicked", async () => {
    const { user } = setup();
    await user.click(
      screen.getByRole("button", { name: /actions for testtoken/i }),
    );
    await user.click(screen.getByText("Remove"));
    expect(
      screen.getByText("Confirm Remove Token"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/Are you sure you want to stop tracking the token "TestToken"/),
    ).toBeInTheDocument();
  });

  it("does not call onRemove before confirmation", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", { name: /actions for testtoken/i }),
    );
    await user.click(screen.getByText("Remove"));
    expect(props.onRemove).not.toHaveBeenCalled();
  });

  it("calls onRemove after confirmation", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", { name: /actions for testtoken/i }),
    );
    await user.click(screen.getByText("Remove"));
    // Click confirm button in dialog
    const dialog = screen.getByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Confirm" }));
    expect(props.onRemove).toHaveBeenCalledWith(
      "token1111222233334444555566667777888899990000aaaabbbbccccddddeeee",
    );
  });

  it("does not call onRemove when cancel is clicked", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", { name: /actions for testtoken/i }),
    );
    await user.click(screen.getByText("Remove"));
    const dialog = screen.getByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(props.onRemove).not.toHaveBeenCalled();
  });

  it("shows 'Unknown' in remove dialog when token has no name", async () => {
    const { user } = setup({ tokens: [makeToken({ name: null })] });
    await user.click(
      screen.getByRole("button", { name: /actions for token/i }),
    );
    await user.click(screen.getByText("Remove"));
    expect(
      screen.getByText(/stop tracking the token "Unknown"/),
    ).toBeInTheDocument();
  });
});

// ─── Balance formatting ─────────────────────────────────────────────

describe("formatTokenBalance", () => {
  it("formats zero balance", () => {
    expect(formatTokenBalance("0", 8)).toBe("0");
  });

  it("formats empty balance as zero", () => {
    expect(formatTokenBalance("", 8)).toBe("0");
  });

  it("formats 1 DASH (100000000 duffs, 8 decimals)", () => {
    expect(formatTokenBalance("100000000", 8)).toBe("1");
  });

  it("formats fractional amount", () => {
    expect(formatTokenBalance("123456789", 8)).toBe("1.23456789");
  });

  it("trims trailing zeros", () => {
    expect(formatTokenBalance("150000000", 8)).toBe("1.5");
  });

  it("formats with 0 decimals", () => {
    expect(formatTokenBalance("42", 0)).toBe("42");
  });

  it("formats very small amount", () => {
    expect(formatTokenBalance("1", 8)).toBe("0.00000001");
  });

  it("formats large amount", () => {
    expect(formatTokenBalance("999999999999999999", 8)).toBe("9999999999.99999999");
  });
});

// ─── ID truncation ──────────────────────────────────────────────────

describe("truncateId", () => {
  it("truncates long IDs", () => {
    const id = "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";
    expect(truncateId(id)).toBe("abcd1234...abcd1234");
  });

  it("does not truncate short IDs", () => {
    const id = "short";
    expect(truncateId(id)).toBe("short");
  });

  it("supports custom truncation length", () => {
    const id = "aabbccddee1234567890aabbccddee1234567890";
    expect(truncateId(id, 4)).toBe("aabb...7890");
  });
});

// ─── Accessibility ──────────────────────────────────────────────────

describe("MyTokensTable — accessibility", () => {
  it("has sr-only label for actions column", () => {
    setup();
    expect(screen.getByText("Actions")).toHaveClass("sr-only");
  });

  it("each row action button has accessible name", () => {
    setup();
    expect(
      screen.getByRole("button", { name: /actions for testtoken/i }),
    ).toBeInTheDocument();
  });

  it("uses semantic table markup", () => {
    setup();
    expect(screen.getByRole("table")).toBeInTheDocument();
    const rowGroups = screen.getAllByRole("rowgroup");
    expect(rowGroups.length).toBe(2); // thead + tbody
  });
});
