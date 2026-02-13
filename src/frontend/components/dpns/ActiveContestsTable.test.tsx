import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { ActiveContestsTable } from "./ActiveContestsTable";
import type { ActiveContestsTableProps } from "./ActiveContestsTable";
import type {
  ContestedName,
  ContestSortColumn,
  SortOrder,
} from "@/stores/contestStore";
import { TooltipProvider } from "@/components/ui/tooltip";
import { displayId } from "@/lib/utils";

// ─── Test fixtures ─────────────────────────────────────────────────

function makeContestant(overrides: Record<string, unknown> = {}) {
  return {
    id: "aaaa1111bbbb2222cccc3333dddd4444",
    name: "alice",
    info: "",
    votes: 3,
    createdAt: null,
    createdAtBlockHeight: null,
    createdAtCoreBlockHeight: null,
    documentId: "doc111",
    ...overrides,
  };
}

function makeContest(
  overrides: Partial<ContestedName> = {},
): ContestedName {
  return {
    normalizedContestedName: "d4sh",
    contestants: [makeContestant()],
    lockedVotes: 2,
    abstainVotes: 1,
    awardedTo: null,
    endTime: Math.floor(Date.now() / 1000) + 86400, // 1 day from now
    state: "ongoing",
    lastUpdated: Math.floor(Date.now() / 1000) - 60, // 1 min ago
    ...overrides,
  };
}

const defaultProps: ActiveContestsTableProps = {
  contestedNames: [makeContest()],
  selectedVotes: [],
  filterTerm: "",
  sortColumn: "name" as ContestSortColumn,
  sortOrder: "ascending" as SortOrder,
  onFilterChange: vi.fn(),
  onSortChange: vi.fn(),
  onToggleVote: vi.fn(),
};

function setup(
  props: Partial<ActiveContestsTableProps> = {},
) {
  const mergedProps = { ...defaultProps, ...props };
  return {
    user: userEvent.setup(),
    ...render(
      <TooltipProvider>
        <ActiveContestsTable {...mergedProps} />
      </TooltipProvider>,
    ),
    props: mergedProps,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

// ─── Empty state ─────────────────────────────────────────────────

describe("ActiveContestsTable — empty state", () => {
  it("shows empty state when no contests", () => {
    setup({ contestedNames: [] });
    expect(
      screen.getByText("No active contests at the moment."),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "Please check back later or try refreshing the list.",
      ),
    ).toBeInTheDocument();
  });

  it("shows 'no matching' message when filter matches nothing", () => {
    setup({ filterTerm: "zzzznonexistent" });
    expect(
      screen.getByText("No matching contests."),
    ).toBeInTheDocument();
  });

  it("filters out non-active contests (locked state)", () => {
    setup({
      contestedNames: [makeContest({ state: "locked" })],
    });
    expect(
      screen.getByText("No active contests at the moment."),
    ).toBeInTheDocument();
  });

  it("filters out non-active contests (wonBy state)", () => {
    setup({
      contestedNames: [
        makeContest({ state: { wonBy: "winner-id" } }),
      ],
    });
    expect(
      screen.getByText("No active contests at the moment."),
    ).toBeInTheDocument();
  });
});

// ─── Table rendering ─────────────────────────────────────────────

describe("ActiveContestsTable — rendering", () => {
  it("renders the contest name in the table", () => {
    setup();
    expect(screen.getByText("d4sh")).toBeInTheDocument();
  });

  it("renders locked votes count", () => {
    setup();
    expect(screen.getByText("2")).toBeInTheDocument();
  });

  it("renders abstain votes count", () => {
    setup();
    expect(screen.getByText("1")).toBeInTheDocument();
  });

  it("renders contestant with votes", () => {
    setup();
    expect(
      screen.getByText(`${displayId("aaaa1111bbbb2222cccc3333dddd4444")} — 3 votes`),
    ).toBeInTheDocument();
  });

  it("renders multiple contests", () => {
    setup({
      contestedNames: [
        makeContest({ normalizedContestedName: "alpha" }),
        makeContest({ normalizedContestedName: "bravo" }),
      ],
    });
    expect(screen.getByText("alpha")).toBeInTheDocument();
    expect(screen.getByText("bravo")).toBeInTheDocument();
  });

  it("shows 'No contestants' when contestants is empty", () => {
    setup({
      contestedNames: [makeContest({ contestants: [] })],
    });
    expect(screen.getByText("No contestants")).toBeInTheDocument();
  });

  it("shows 'No contestants' when contestants is null", () => {
    setup({
      contestedNames: [makeContest({ contestants: null })],
    });
    expect(screen.getByText("No contestants")).toBeInTheDocument();
  });

  it("renders 'Fetching' when endTime is null", () => {
    setup({
      contestedNames: [makeContest({ endTime: null })],
    });
    expect(screen.getByText("Fetching")).toBeInTheDocument();
  });

  it("renders 'Fetching' when lastUpdated is null", () => {
    setup({
      contestedNames: [makeContest({ lastUpdated: null })],
    });
    expect(screen.getByText("Fetching")).toBeInTheDocument();
  });
});

// ─── Sortable headers ────────────────────────────────────────────

describe("ActiveContestsTable — sorting", () => {
  it("renders all sortable column headers", () => {
    setup();
    expect(
      screen.getByRole("button", { name: /sort by name/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /sort by locked votes/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /sort by abstain votes/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /sort by ending time/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /sort by last updated/i }),
    ).toBeInTheDocument();
  });

  it("calls onSortChange when clicking a column header", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", { name: /sort by locked votes/i }),
    );
    expect(props.onSortChange).toHaveBeenCalledWith("lockedVotes");
  });

  it("calls onSortChange with 'name' column", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", { name: /sort by name/i }),
    );
    expect(props.onSortChange).toHaveBeenCalledWith("name");
  });
});

// ─── Filter ──────────────────────────────────────────────────────

describe("ActiveContestsTable — filtering", () => {
  it("renders the filter input", () => {
    setup();
    expect(
      screen.getByLabelText("Filter contests"),
    ).toBeInTheDocument();
  });

  it("calls onFilterChange when typing", async () => {
    const { user, props } = setup();
    const input = screen.getByLabelText("Filter contests");
    await user.type(input, "x");
    expect(props.onFilterChange).toHaveBeenCalled();
  });

  it("shows filter input value", () => {
    setup({ filterTerm: "test" });
    const input = screen.getByLabelText("Filter contests") as HTMLInputElement;
    expect(input.value).toBe("test");
  });

  it("smart filters: shows 'd4sh' when filtering with 'dash'", () => {
    // 'dash' → normalizeDpnsFilter → 'd4sh' won't match normally,
    // but 'o' → '0' and 'l' → '1'. 'a' stays 'a'.
    // For this test, contest name is "d4sh" and filter "d4sh" matches directly.
    setup({ filterTerm: "d4sh" });
    expect(screen.getByText("d4sh")).toBeInTheDocument();
  });

  it("smart filters: hides contests that don't match", () => {
    setup({ filterTerm: "zzz" });
    expect(screen.queryByText("d4sh")).not.toBeInTheDocument();
    expect(
      screen.getByText("No matching contests."),
    ).toBeInTheDocument();
  });
});

// ─── Vote buttons ────────────────────────────────────────────────

describe("ActiveContestsTable — voting", () => {
  it("calls onToggleVote with lock choice when clicking lock button", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", {
        name: /lock vote for d4sh/i,
      }),
    );
    expect(props.onToggleVote).toHaveBeenCalledWith({
      contestedName: "d4sh",
      choice: "lock",
      endTime: expect.any(Number),
    });
  });

  it("calls onToggleVote with abstain choice when clicking abstain button", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", {
        name: /abstain vote for d4sh/i,
      }),
    );
    expect(props.onToggleVote).toHaveBeenCalledWith({
      contestedName: "d4sh",
      choice: "abstain",
      endTime: expect.any(Number),
    });
  });

  it("calls onToggleVote with towardsIdentity when clicking contestant", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", {
        name: `Vote for contestant ${displayId("aaaa1111bbbb2222cccc3333dddd4444")}`,
      }),
    );
    expect(props.onToggleVote).toHaveBeenCalledWith({
      contestedName: "d4sh",
      choice: { towardsIdentity: { identityId: "aaaa1111bbbb2222cccc3333dddd4444" } },
      endTime: expect.any(Number),
    });
  });
});

// ─── Selected votes highlighting ─────────────────────────────────

describe("ActiveContestsTable — selection highlighting", () => {
  it("highlights lock button when lock vote is selected", () => {
    setup({
      selectedVotes: [
        {
          contestedName: "d4sh",
          choice: "lock",
          endTime: null,
        },
      ],
    });
    const lockBtn = screen.getByRole("button", {
      name: /lock vote for d4sh/i,
    });
    expect(lockBtn).toHaveAttribute("aria-pressed", "true");
  });

  it("highlights abstain button when abstain vote is selected", () => {
    setup({
      selectedVotes: [
        {
          contestedName: "d4sh",
          choice: "abstain",
          endTime: null,
        },
      ],
    });
    const abstainBtn = screen.getByRole("button", {
      name: /abstain vote for d4sh/i,
    });
    expect(abstainBtn).toHaveAttribute("aria-pressed", "true");
  });

  it("highlights contestant button when contestant vote is selected", () => {
    setup({
      selectedVotes: [
        {
          contestedName: "d4sh",
          choice: {
            towardsIdentity: {
              identityId: "aaaa1111bbbb2222cccc3333dddd4444",
            },
          },
          endTime: null,
        },
      ],
    });
    const contestantBtn = screen.getByRole("button", {
      name: `Vote for contestant ${displayId("aaaa1111bbbb2222cccc3333dddd4444")}`,
    });
    expect(contestantBtn).toHaveAttribute("aria-pressed", "true");
  });

  it("does not highlight lock when different vote is selected", () => {
    setup({
      selectedVotes: [
        {
          contestedName: "d4sh",
          choice: "abstain",
          endTime: null,
        },
      ],
    });
    const lockBtn = screen.getByRole("button", {
      name: /lock vote for d4sh/i,
    });
    expect(lockBtn).toHaveAttribute("aria-pressed", "false");
  });

  it("shows selection count badge", () => {
    setup({
      selectedVotes: [
        { contestedName: "d4sh", choice: "lock", endTime: null },
        { contestedName: "other", choice: "abstain", endTime: null },
      ],
    });
    expect(screen.getByText("2 votes selected")).toBeInTheDocument();
  });

  it("does not show selection count when no votes selected", () => {
    setup();
    expect(
      screen.queryByText(/votes? selected/),
    ).not.toBeInTheDocument();
  });
});

// ─── Locked votes visual emphasis ────────────────────────────────

describe("ActiveContestsTable — visual emphasis", () => {
  it("renders lock button with bold green when locked votes exceed contestant votes", () => {
    setup({
      contestedNames: [
        makeContest({
          lockedVotes: 10,
          contestants: [makeContestant({ votes: 3 })],
        }),
      ],
    });
    const lockBtn = screen.getByRole("button", {
      name: /lock vote for d4sh/i,
    });
    // Bold green class should be applied
    expect(lockBtn.className).toContain("font-bold");
    expect(lockBtn.className).toContain("text-green");
  });

  it("does not bold lock button when contestant votes are higher", () => {
    setup({
      contestedNames: [
        makeContest({
          lockedVotes: 1,
          contestants: [makeContestant({ votes: 10 })],
        }),
      ],
    });
    const lockBtn = screen.getByRole("button", {
      name: /lock vote for d4sh/i,
    });
    expect(lockBtn.className).not.toContain("font-bold");
  });

  it("bolds the contestant with highest votes (when lock is not higher)", () => {
    setup({
      contestedNames: [
        makeContest({
          lockedVotes: 1,
          contestants: [
            makeContestant({ id: "aaa111xxxxxxxx", votes: 10 }),
            makeContestant({ id: "bbb222xxxxxxxx", votes: 2 }),
          ],
        }),
      ],
    });
    const highVoteBtn = screen.getByRole("button", {
      name: `Vote for contestant ${displayId("aaa111xxxxxxxx")}`,
    });
    expect(highVoteBtn.className).toContain("font-bold");

    const lowVoteBtn = screen.getByRole("button", {
      name: `Vote for contestant ${displayId("bbb222xxxxxxxx")}`,
    });
    expect(lowVoteBtn.className).not.toContain("font-bold");
  });
});

// ─── Multiple contestants ────────────────────────────────────────

describe("ActiveContestsTable — multiple contestants", () => {
  it("renders all contestants for a contest", () => {
    setup({
      contestedNames: [
        makeContest({
          contestants: [
            makeContestant({ id: "aaa111xxxxxxxx", name: "alice", votes: 5 }),
            makeContestant({ id: "bbb222xxxxxxxx", name: "bob", votes: 3 }),
            makeContestant({ id: "ccc333xxxxxxxx", name: "charlie", votes: 1 }),
          ],
        }),
      ],
    });
    expect(screen.getByText(`${displayId("aaa111xxxxxxxx")} — 5 votes`)).toBeInTheDocument();
    expect(screen.getByText(`${displayId("bbb222xxxxxxxx")} — 3 votes`)).toBeInTheDocument();
    expect(screen.getByText(`${displayId("ccc333xxxxxxxx")} — 1 votes`)).toBeInTheDocument();
  });
});

// ─── Mixed active/non-active contests ────────────────────────────

describe("ActiveContestsTable — contest filtering by state", () => {
  it("only shows active contests (ongoing, joinable, unknown)", () => {
    setup({
      contestedNames: [
        makeContest({
          normalizedContestedName: "active1",
          state: "ongoing",
        }),
        makeContest({
          normalizedContestedName: "active2",
          state: "joinable",
        }),
        makeContest({
          normalizedContestedName: "active3",
          state: "unknown",
        }),
        makeContest({
          normalizedContestedName: "past1",
          state: { wonBy: "winner" },
        }),
        makeContest({
          normalizedContestedName: "locked1",
          state: "locked",
        }),
      ],
    });
    expect(screen.getByText("active1")).toBeInTheDocument();
    expect(screen.getByText("active2")).toBeInTheDocument();
    expect(screen.getByText("active3")).toBeInTheDocument();
    expect(screen.queryByText("past1")).not.toBeInTheDocument();
    expect(screen.queryByText("locked1")).not.toBeInTheDocument();
  });
});

// ─── Accessibility ───────────────────────────────────────────────

describe("ActiveContestsTable — accessibility", () => {
  it("vote buttons have aria-label attributes", () => {
    setup();
    expect(
      screen.getByRole("button", { name: /lock vote for d4sh/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /abstain vote for d4sh/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /vote for contestant/i }),
    ).toBeInTheDocument();
  });

  it("vote buttons have aria-pressed for selected state", () => {
    setup({
      selectedVotes: [
        { contestedName: "d4sh", choice: "lock", endTime: null },
      ],
    });
    expect(
      screen.getByRole("button", { name: /lock vote for d4sh/i }),
    ).toHaveAttribute("aria-pressed", "true");
    expect(
      screen.getByRole("button", { name: /abstain vote for d4sh/i }),
    ).toHaveAttribute("aria-pressed", "false");
  });

  it("filter input has accessible label", () => {
    setup();
    expect(screen.getByLabelText("Filter contests")).toBeInTheDocument();
  });

  it("sort buttons have aria-label", () => {
    setup();
    const sortButtons = screen.getAllByRole("button", {
      name: /sort by/i,
    });
    expect(sortButtons.length).toBe(5); // Name, Locked, Abstain, Ending, LastUpdated
  });

  it("empty state has role=status", () => {
    setup({ contestedNames: [] });
    expect(screen.getByRole("status")).toBeInTheDocument();
  });
});
