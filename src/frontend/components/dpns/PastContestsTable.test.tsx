import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { PastContestsTable } from "./PastContestsTable";
import type { PastContestsTableProps } from "./PastContestsTable";
import type {
  ContestedName,
  ContestSortColumn,
  SortOrder,
} from "@/stores/contestStore";
import { TooltipProvider } from "@/components/ui/tooltip";
import { displayId } from "@/lib/utils";

// ─── Test fixtures ─────────────────────────────────────────────────

function makeContest(
  overrides: Partial<ContestedName> = {},
): ContestedName {
  return {
    normalizedContestedName: "d4sh",
    contestants: null,
    lockedVotes: 5,
    abstainVotes: 2,
    awardedTo: "winner1111222233334444",
    endTime: Math.floor(Date.now() / 1000) - 86400, // 1 day ago
    state: { wonBy: "winner1111222233334444" },
    lastUpdated: Math.floor(Date.now() / 1000) - 60, // 1 min ago
    ...overrides,
  };
}

const defaultProps: PastContestsTableProps = {
  contestedNames: [makeContest()],
  filterTerm: "",
  sortColumn: "name" as ContestSortColumn,
  sortOrder: "ascending" as SortOrder,
  onFilterChange: vi.fn(),
  onSortChange: vi.fn(),
};

function setup(props: Partial<PastContestsTableProps> = {}) {
  const mergedProps = { ...defaultProps, ...props };
  return {
    user: userEvent.setup(),
    ...render(
      <TooltipProvider>
        <PastContestsTable {...mergedProps} />
      </TooltipProvider>,
    ),
    props: mergedProps,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

// ─── Empty state ─────────────────────────────────────────────────

describe("PastContestsTable — empty state", () => {
  it("shows empty state when no contests", () => {
    setup({ contestedNames: [] });
    expect(
      screen.getByText("No past contests yet."),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Contests will appear here once they have ended."),
    ).toBeInTheDocument();
  });

  it("shows 'no matching' message when filter matches nothing", () => {
    setup({ filterTerm: "zzzznonexistent" });
    expect(
      screen.getByText("No matching contests."),
    ).toBeInTheDocument();
  });

  it("filters out active contests (ongoing state)", () => {
    setup({
      contestedNames: [makeContest({ state: "ongoing" })],
    });
    expect(
      screen.getByText("No past contests yet."),
    ).toBeInTheDocument();
  });

  it("filters out active contests (joinable state)", () => {
    setup({
      contestedNames: [makeContest({ state: "joinable" })],
    });
    expect(
      screen.getByText("No past contests yet."),
    ).toBeInTheDocument();
  });

  it("filters out active contests (unknown state)", () => {
    setup({
      contestedNames: [makeContest({ state: "unknown" })],
    });
    expect(
      screen.getByText("No past contests yet."),
    ).toBeInTheDocument();
  });
});

// ─── Table rendering ─────────────────────────────────────────────

describe("PastContestsTable — rendering", () => {
  it("renders the contest name in the table", () => {
    setup();
    expect(screen.getByText("d4sh")).toBeInTheDocument();
  });

  it("renders multiple past contests", () => {
    setup({
      contestedNames: [
        makeContest({ normalizedContestedName: "alpha" }),
        makeContest({ normalizedContestedName: "bravo", state: "locked" }),
      ],
    });
    expect(screen.getByText("alpha")).toBeInTheDocument();
    expect(screen.getByText("bravo")).toBeInTheDocument();
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

  it("renders the awarded-to truncated identity for wonBy state", () => {
    setup({
      contestedNames: [
        makeContest({
          state: { wonBy: "abcdef1234567890abcdef1234567890" },
        }),
      ],
    });
    expect(screen.getByText(displayId("abcdef1234567890abcdef1234567890"))).toBeInTheDocument();
  });

  it("renders 'Locked' badge for locked state", () => {
    setup({
      contestedNames: [makeContest({ state: "locked" })],
    });
    expect(screen.getByText("Locked")).toBeInTheDocument();
  });
});

// ─── Contest state filtering ─────────────────────────────────────

describe("PastContestsTable — contest filtering by state", () => {
  it("only shows past contests (wonBy and locked)", () => {
    setup({
      contestedNames: [
        makeContest({
          normalizedContestedName: "past1",
          state: { wonBy: "winner" },
        }),
        makeContest({
          normalizedContestedName: "locked1",
          state: "locked",
        }),
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
      ],
    });
    expect(screen.getByText("past1")).toBeInTheDocument();
    expect(screen.getByText("locked1")).toBeInTheDocument();
    expect(screen.queryByText("active1")).not.toBeInTheDocument();
    expect(screen.queryByText("active2")).not.toBeInTheDocument();
    expect(screen.queryByText("active3")).not.toBeInTheDocument();
  });
});

// ─── Sortable headers ────────────────────────────────────────────

describe("PastContestsTable — sorting", () => {
  it("renders all sortable column headers", () => {
    setup();
    expect(
      screen.getByRole("button", { name: /sort by name/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /sort by ended time/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /sort by last updated/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /sort by awarded to/i }),
    ).toBeInTheDocument();
  });

  it("calls onSortChange when clicking a column header", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", { name: /sort by ended time/i }),
    );
    expect(props.onSortChange).toHaveBeenCalledWith("endingTime");
  });

  it("calls onSortChange with 'name' column", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", { name: /sort by name/i }),
    );
    expect(props.onSortChange).toHaveBeenCalledWith("name");
  });

  it("calls onSortChange with 'awardedTo' column", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", { name: /sort by awarded to/i }),
    );
    expect(props.onSortChange).toHaveBeenCalledWith("awardedTo");
  });

  it("calls onSortChange with 'lastUpdated' column", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", { name: /sort by last updated/i }),
    );
    expect(props.onSortChange).toHaveBeenCalledWith("lastUpdated");
  });
});

// ─── Filter ──────────────────────────────────────────────────────

describe("PastContestsTable — filtering", () => {
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

  it("smart filters: shows 'd4sh' when filtering with 'd4sh'", () => {
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

// ─── Awarded To display ─────────────────────────────────────────

describe("PastContestsTable — awarded to display", () => {
  it("shows trophy icon badge for wonBy contests", () => {
    setup({
      contestedNames: [
        makeContest({
          state: { wonBy: "abcdef1234567890" },
        }),
      ],
    });
    expect(screen.getByText(displayId("abcdef1234567890"))).toBeInTheDocument();
  });

  it("shows lock icon badge for locked contests", () => {
    setup({
      contestedNames: [makeContest({ state: "locked" })],
    });
    expect(screen.getByText("Locked")).toBeInTheDocument();
  });

  it("differentiates wonBy and locked in mixed list", () => {
    setup({
      contestedNames: [
        makeContest({
          normalizedContestedName: "won-name",
          state: { wonBy: "id1234567890abcd" },
        }),
        makeContest({
          normalizedContestedName: "locked-name",
          state: "locked",
        }),
      ],
    });
    expect(screen.getByText(displayId("id1234567890abcd"))).toBeInTheDocument();
    expect(screen.getByText("Locked")).toBeInTheDocument();
  });
});

// ─── Read-only (no voting) ──────────────────────────────────────

describe("PastContestsTable — read-only", () => {
  it("does not render vote buttons", () => {
    setup();
    expect(
      screen.queryByRole("button", { name: /lock vote/i }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /abstain vote/i }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /vote for contestant/i }),
    ).not.toBeInTheDocument();
  });

  it("does not render selection count badge", () => {
    setup();
    expect(
      screen.queryByText(/votes? selected/),
    ).not.toBeInTheDocument();
  });
});

// ─── Accessibility ───────────────────────────────────────────────

describe("PastContestsTable — accessibility", () => {
  it("filter input has accessible label", () => {
    setup();
    expect(screen.getByLabelText("Filter contests")).toBeInTheDocument();
  });

  it("sort buttons have aria-label", () => {
    setup();
    const sortButtons = screen.getAllByRole("button", {
      name: /sort by/i,
    });
    expect(sortButtons.length).toBe(4); // Name, Ended Time, Last Updated, Awarded To
  });

  it("empty state has role=status", () => {
    setup({ contestedNames: [] });
    expect(screen.getByRole("status")).toBeInTheDocument();
  });

  it("no-match empty state has role=status", () => {
    setup({ filterTerm: "zzzznotfound" });
    expect(screen.getByRole("status")).toBeInTheDocument();
  });
});
