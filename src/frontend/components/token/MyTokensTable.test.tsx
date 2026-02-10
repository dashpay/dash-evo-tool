import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { MyTokensTable, formatTokenBalance, truncateId, groupTokens } from "./MyTokensTable";
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
  onMoreInfo: vi.fn(),
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

// ─── Level 1: Token List ────────────────────────────────────────────

describe("MyTokensTable — Level 1 (Token List)", () => {
  it("renders table with Level 1 headers", () => {
    setup();
    expect(screen.getByRole("table")).toBeInTheDocument();
    expect(screen.getByText("Token Name")).toBeInTheDocument();
    expect(screen.getByText("Token ID")).toBeInTheDocument();
    expect(screen.getByText("Identities")).toBeInTheDocument();
  });

  it("renders token name as clickable link", () => {
    setup();
    const link = screen.getByRole("button", { name: "TestToken" });
    expect(link).toBeInTheDocument();
  });

  it("renders 'Unnamed Token' when name is null", () => {
    setup({ tokens: [makeToken({ name: null })] });
    expect(screen.getByRole("button", { name: "Unnamed Token" })).toBeInTheDocument();
  });

  it("renders truncated token ID", () => {
    setup();
    expect(screen.getByText(/token111\.\.\.ddddeeee/)).toBeInTheDocument();
  });

  it("renders identity count", () => {
    setup();
    expect(screen.getByText("1")).toBeInTheDocument();
  });

  it("groups multiple entries for same token", () => {
    const tokens = [
      makeToken({ identityId: "id1aaaaabbbbccccddddeeeeffffffff1111222233334444555566667777" }),
      makeToken({ identityId: "id2aaaaabbbbccccddddeeeeffffffff1111222233334444555566667777", ownerAlias: "Bob" }),
    ];
    setup({ tokens });
    // Should show one row with identity count = 2
    const rows = screen.getAllByRole("row");
    // 1 header row + 1 data row (grouped)
    expect(rows).toHaveLength(2);
    expect(screen.getByText("2")).toBeInTheDocument();
  });

  it("shows multiple token rows for different tokens", () => {
    const tokens = [
      makeToken({ tokenId: "t1aaaaabbbbccccddddeeeeffffffff1111222233334444555566667777", name: "Alpha" }),
      makeToken({ tokenId: "t2aaaaabbbbccccddddeeeeffffffff1111222233334444555566667777", name: "Beta" }),
    ];
    setup({ tokens });
    expect(screen.getByRole("button", { name: "Alpha" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Beta" })).toBeInTheDocument();
  });

  it("renders More Info button for each token", () => {
    setup();
    expect(
      screen.getByRole("button", { name: /more info for testtoken/i }),
    ).toBeInTheDocument();
  });

  it("renders Remove button for each token", () => {
    setup();
    expect(
      screen.getByRole("button", { name: /remove testtoken/i }),
    ).toBeInTheDocument();
  });

  it("calls onMoreInfo when More Info button is clicked", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", { name: /more info for testtoken/i }),
    );
    expect(props.onMoreInfo).toHaveBeenCalledWith(
      "token1111222233334444555566667777888899990000aaaabbbbccccddddeeee",
    );
  });
});

// ─── Drill-down navigation ─────────────────────────────────────────

describe("MyTokensTable — drill-down", () => {
  it("drills down to Level 2 when token name is clicked", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "TestToken" }));
    // Should now show Level 2 with Back button and identity columns
    expect(screen.getByRole("button", { name: /back/i })).toBeInTheDocument();
    expect(screen.getByText("Identity")).toBeInTheDocument();
    expect(screen.getByText("Identity ID")).toBeInTheDocument();
  });

  it("shows Back button in Level 2", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "TestToken" }));
    expect(screen.getByRole("button", { name: /back/i })).toBeInTheDocument();
  });

  it("shows token name in Level 2 header", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "TestToken" }));
    expect(screen.getByText("TestToken")).toBeInTheDocument();
  });

  it("returns to Level 1 when Back is clicked", async () => {
    const { user } = setup();
    // Drill down
    await user.click(screen.getByRole("button", { name: "TestToken" }));
    expect(screen.getByRole("button", { name: /back/i })).toBeInTheDocument();
    // Go back
    await user.click(screen.getByRole("button", { name: /back/i }));
    // Should be back at Level 1
    expect(screen.queryByRole("button", { name: /back/i })).not.toBeInTheDocument();
    expect(screen.getByText("Token Name")).toBeInTheDocument();
  });
});

// ─── Level 2: Per-Identity Balances ────────────────────────────────

describe("MyTokensTable — Level 2 (Identity Balances)", () => {
  const multiIdentityTokens = [
    makeToken({
      identityId: "id1aaaaabbbbccccddddeeeeffffffff1111222233334444555566667777",
      ownerAlias: "Alice",
      balance: "100000000",
    }),
    makeToken({
      identityId: "id2aaaaabbbbccccddddeeeeffffffff1111222233334444555566667777",
      ownerAlias: "Bob",
      balance: "200000000",
    }),
  ];

  it("shows all identity entries for the selected token", async () => {
    const { user } = setup({ tokens: multiIdentityTokens });
    await user.click(screen.getByRole("button", { name: "TestToken" }));
    expect(screen.getByText("Alice")).toBeInTheDocument();
    expect(screen.getByText("Bob")).toBeInTheDocument();
  });

  it("shows formatted balances for each identity", async () => {
    const { user } = setup({ tokens: multiIdentityTokens });
    await user.click(screen.getByRole("button", { name: "TestToken" }));
    expect(screen.getByText("1")).toBeInTheDocument(); // 100000000 / 10^8 = 1
    expect(screen.getByText("2")).toBeInTheDocument(); // 200000000 / 10^8 = 2
  });

  it("shows truncated identity IDs", async () => {
    const { user } = setup({ tokens: multiIdentityTokens });
    await user.click(screen.getByRole("button", { name: "TestToken" }));
    expect(screen.getByText(/id1aaaaa\.\.\.66667777/)).toBeInTheDocument();
    expect(screen.getByText(/id2aaaaa\.\.\.66667777/)).toBeInTheDocument();
  });

  it("shows dash for identity with no alias", async () => {
    const tokens = [makeToken({ ownerAlias: null })];
    const { user } = setup({ tokens });
    await user.click(screen.getByRole("button", { name: "TestToken" }));
    expect(screen.getByText("—")).toBeInTheDocument();
  });

  it("renders action dropdown for each identity row", async () => {
    const { user } = setup({ tokens: multiIdentityTokens });
    await user.click(screen.getByRole("button", { name: "TestToken" }));
    const actionButtons = screen.getAllByLabelText(/^Actions for/);
    expect(actionButtons).toHaveLength(2);
  });

  it("opens action menu with all 15 actions", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "TestToken" }));
    const actionButton = screen.getByLabelText(/^Actions for/);
    await user.click(actionButton);
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

  it("calls onAction with full entry when Transfer is clicked", async () => {
    const { user, props } = setup();
    await user.click(screen.getByRole("button", { name: "TestToken" }));
    const actionButton = screen.getByLabelText(/^Actions for/);
    await user.click(actionButton);
    await user.click(screen.getByText("Transfer"));
    expect(props.onAction).toHaveBeenCalledWith(
      expect.objectContaining({
        tokenId: "token1111222233334444555566667777888899990000aaaabbbbccccddddeeee",
        identityId: "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234",
      }),
      "transfer",
    );
  });

  it("calls onMoreInfo when More Info is clicked in Level 2 actions", async () => {
    const { user, props } = setup();
    await user.click(screen.getByRole("button", { name: "TestToken" }));
    const actionButton = screen.getByLabelText(/^Actions for/);
    await user.click(actionButton);
    await user.click(screen.getByText("More Info"));
    expect(props.onMoreInfo).toHaveBeenCalledWith(
      "token1111222233334444555566667777888899990000aaaabbbbccccddddeeee",
    );
  });
});

// ─── Sorting (Level 2) ─────────────────────────────────────────────

describe("MyTokensTable — sorting (Level 2)", () => {
  it("calls onSortChange with 'ownerAlias' when Identity column clicked", async () => {
    const { user, props } = setup();
    await user.click(screen.getByRole("button", { name: "TestToken" }));
    await user.click(screen.getByRole("button", { name: /identity$/i }));
    expect(props.onSortChange).toHaveBeenCalledWith("ownerAlias");
  });

  it("calls onSortChange with 'balance' when Balance column clicked", async () => {
    const { user, props } = setup();
    await user.click(screen.getByRole("button", { name: "TestToken" }));
    await user.click(screen.getByRole("button", { name: /balance/i }));
    expect(props.onSortChange).toHaveBeenCalledWith("balance");
  });

  it("shows sort indicator on active column", async () => {
    const { user } = setup({ sortColumn: "balance", sortOrder: "ascending" });
    await user.click(screen.getByRole("button", { name: "TestToken" }));
    const balanceButton = screen.getByRole("button", { name: /balance/i });
    const svg = balanceButton.querySelector("svg");
    expect(svg).toBeInTheDocument();
  });
});

// ─── Remove confirmation ────────────────────────────────────────────

describe("MyTokensTable — remove confirmation", () => {
  it("shows confirmation dialog when Remove is clicked in Level 1", async () => {
    const { user } = setup();
    await user.click(
      screen.getByRole("button", { name: /remove testtoken/i }),
    );
    expect(screen.getByText("Confirm Remove Token")).toBeInTheDocument();
    expect(
      screen.getByText(/Are you sure you want to stop tracking the token "TestToken"/),
    ).toBeInTheDocument();
  });

  it("does not call onRemove before confirmation", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", { name: /remove testtoken/i }),
    );
    expect(props.onRemove).not.toHaveBeenCalled();
  });

  it("calls onRemove after confirmation", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", { name: /remove testtoken/i }),
    );
    const dialog = screen.getByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Confirm" }));
    expect(props.onRemove).toHaveBeenCalledWith(
      "token1111222233334444555566667777888899990000aaaabbbbccccddddeeee",
    );
  });

  it("does not call onRemove when cancel is clicked", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", { name: /remove testtoken/i }),
    );
    const dialog = screen.getByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(props.onRemove).not.toHaveBeenCalled();
  });

  it("shows 'Unknown' in remove dialog when token has no name", async () => {
    const { user } = setup({ tokens: [makeToken({ name: null })] });
    await user.click(
      screen.getByRole("button", { name: /remove unnamed token/i }),
    );
    expect(
      screen.getByText(/stop tracking the token "Unknown"/),
    ).toBeInTheDocument();
  });

  it("shows confirmation from Level 2 Remove action", async () => {
    const { user } = setup();
    // Drill down first
    await user.click(screen.getByRole("button", { name: "TestToken" }));
    // Open actions and click Remove
    const actionButton = screen.getByLabelText(/^Actions for/);
    await user.click(actionButton);
    await user.click(screen.getByText("Remove"));
    expect(screen.getByText("Confirm Remove Token")).toBeInTheDocument();
  });

  it("returns to Level 1 when confirmed remove from Level 2", async () => {
    const { user } = setup();
    // Drill down
    await user.click(screen.getByRole("button", { name: "TestToken" }));
    // Open actions and click Remove
    const actionButton = screen.getByLabelText(/^Actions for/);
    await user.click(actionButton);
    await user.click(screen.getByText("Remove"));
    // Confirm
    const dialog = screen.getByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Confirm" }));
    // Should be back at Level 1 (no Back button)
    expect(screen.queryByRole("button", { name: /back/i })).not.toBeInTheDocument();
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

// ─── groupTokens ────────────────────────────────────────────────────

describe("groupTokens", () => {
  it("groups entries by tokenId", () => {
    const tokens = [
      makeToken({ tokenId: "t1", identityId: "id1" }),
      makeToken({ tokenId: "t1", identityId: "id2" }),
      makeToken({ tokenId: "t2", identityId: "id3" }),
    ];
    const summaries = groupTokens(tokens);
    expect(summaries).toHaveLength(2);
    const t1 = summaries.find((s) => s.tokenId === "t1");
    const t2 = summaries.find((s) => s.tokenId === "t2");
    expect(t1?.identityCount).toBe(2);
    expect(t2?.identityCount).toBe(1);
  });

  it("preserves token metadata from first entry", () => {
    const tokens = [
      makeToken({ tokenId: "t1", name: "Alpha", decimals: 6 }),
      makeToken({ tokenId: "t1", name: "Alpha", identityId: "id2" }),
    ];
    const summaries = groupTokens(tokens);
    expect(summaries[0].name).toBe("Alpha");
    expect(summaries[0].decimals).toBe(6);
  });

  it("returns empty array for no tokens", () => {
    expect(groupTokens([])).toHaveLength(0);
  });
});

// ─── Accessibility ──────────────────────────────────────────────────

describe("MyTokensTable — accessibility", () => {
  it("has sr-only label for actions column in Level 1", () => {
    setup();
    expect(screen.getByText("Actions")).toHaveClass("sr-only");
  });

  it("has sr-only label for actions column in Level 2", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "TestToken" }));
    expect(screen.getByText("Actions")).toHaveClass("sr-only");
  });

  it("each Level 2 row action button has accessible name", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "TestToken" }));
    expect(
      screen.getByLabelText(/^Actions for Alice/),
    ).toBeInTheDocument();
  });

  it("uses semantic table markup in Level 1", () => {
    setup();
    expect(screen.getByRole("table")).toBeInTheDocument();
    const rowGroups = screen.getAllByRole("rowgroup");
    expect(rowGroups.length).toBe(2); // thead + tbody
  });

  it("uses semantic table markup in Level 2", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "TestToken" }));
    expect(screen.getByRole("table")).toBeInTheDocument();
    const rowGroups = screen.getAllByRole("rowgroup");
    expect(rowGroups.length).toBe(2);
  });
});
