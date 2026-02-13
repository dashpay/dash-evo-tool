import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  ScheduledVotesTable,
  formatScheduledTime,
  formatRelativeTime,
  getStatusDisplay,
} from "./ScheduledVotesTable";
import type { ScheduledVotesTableProps } from "./ScheduledVotesTable";
import type { ScheduledVoteWithStatus } from "@/stores/contestStore";
import type { VoteChoiceDto } from "@/bindings";
import { TooltipProvider } from "@/components/ui/tooltip";
import { displayId } from "@/lib/utils";

// ─── Test fixtures ────────────────────────────────────────────────

function makeVote(
  overrides: Partial<{
    contestedName: string;
    voterId: string;
    choice: VoteChoiceDto;
    unixTimestamp: number;
    executedSuccessfully: boolean;
    castingStatus: "notStarted" | "inProgress" | "completed" | "failed";
  }> = {},
): ScheduledVoteWithStatus {
  return {
    vote: {
      contestedName: overrides.contestedName ?? "test-name",
      voterId:
        overrides.voterId ?? "abcdef1234567890abcdef1234567890",
      choice: overrides.choice ?? "lock",
      unixTimestamp:
        overrides.unixTimestamp ?? Date.now() + 3_600_000, // 1 hour from now
      executedSuccessfully: overrides.executedSuccessfully ?? false,
    },
    castingStatus: overrides.castingStatus ?? "notStarted",
  };
}

const defaultProps: ScheduledVotesTableProps = {
  scheduledVotes: [makeVote()],
  onRemove: vi.fn(),
  onCastNow: vi.fn(),
};

function setup(props: Partial<ScheduledVotesTableProps> = {}) {
  const mergedProps = { ...defaultProps, ...props };
  return {
    user: userEvent.setup(),
    ...render(
      <TooltipProvider>
        <ScheduledVotesTable {...mergedProps} />
      </TooltipProvider>,
    ),
    props: mergedProps,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

// ─── Helper function tests ───────────────────────────────────────

describe("formatScheduledTime", () => {
  it("formats a valid timestamp to ISO-like string", () => {
    const ts = new Date("2025-06-15T14:30:00Z").getTime();
    expect(formatScheduledTime(ts)).toBe("2025-06-15 14:30:00");
  });

  it("returns Unknown for invalid timestamp", () => {
    expect(formatScheduledTime(NaN)).toBe("Unknown");
  });
});

describe("formatRelativeTime", () => {
  it("returns 'now' for very near future", () => {
    expect(formatRelativeTime(Date.now() + 5_000)).toBe("now");
  });

  it("returns 'in Xm' for future minutes", () => {
    expect(formatRelativeTime(Date.now() + 300_000)).toBe("in 5m");
  });

  it("returns 'in Xh' for future hours", () => {
    expect(formatRelativeTime(Date.now() + 7_200_000)).toBe("in 2h");
  });

  it("returns 'in Xd' for future days", () => {
    expect(formatRelativeTime(Date.now() + 172_800_000)).toBe("in 2d");
  });

  it("returns 'just now' for very recent past", () => {
    expect(formatRelativeTime(Date.now() - 5_000)).toBe("just now");
  });

  it("returns 'Xm ago' for past minutes", () => {
    expect(formatRelativeTime(Date.now() - 300_000)).toBe("5m ago");
  });

  it("returns 'Xh ago' for past hours", () => {
    expect(formatRelativeTime(Date.now() - 7_200_000)).toBe("2h ago");
  });

  it("returns 'Xd ago' for past days", () => {
    expect(formatRelativeTime(Date.now() - 172_800_000)).toBe("2d ago");
  });
});

describe("getStatusDisplay", () => {
  it("returns Pending for notStarted", () => {
    const result = getStatusDisplay("notStarted");
    expect(result.label).toBe("Pending");
    expect(result.variant).toBe("secondary");
  });

  it("returns Casting... for inProgress", () => {
    const result = getStatusDisplay("inProgress");
    expect(result.label).toBe("Casting...");
    expect(result.variant).toBe("default");
  });

  it("returns Casted for completed", () => {
    const result = getStatusDisplay("completed");
    expect(result.label).toBe("Casted");
    expect(result.variant).toBe("outline");
  });

  it("returns Failed for failed", () => {
    const result = getStatusDisplay("failed");
    expect(result.label).toBe("Failed");
    expect(result.variant).toBe("destructive");
  });
});

// ─── Empty state ──────────────────────────────────────────────────

describe("ScheduledVotesTable — empty state", () => {
  it("shows empty state when no scheduled votes", () => {
    setup({ scheduledVotes: [] });
    expect(screen.getByText("No scheduled votes.")).toBeInTheDocument();
    expect(
      screen.getByText(/Schedule votes from the Active Contests tab/),
    ).toBeInTheDocument();
  });

  it("empty state has role=status", () => {
    setup({ scheduledVotes: [] });
    expect(screen.getByRole("status")).toBeInTheDocument();
  });

  it("shows 'no matching' when filter matches nothing", async () => {
    const { user } = setup();
    const input = screen.getByLabelText("Filter scheduled votes");
    await user.type(input, "zzzznonexistent");
    expect(screen.getByText("No matching votes.")).toBeInTheDocument();
  });
});

// ─── Table rendering ──────────────────────────────────────────────

describe("ScheduledVotesTable — rendering", () => {
  it("renders the contested name", () => {
    setup();
    expect(screen.getByText("test-name")).toBeInTheDocument();
  });

  it("renders the truncated voter ID", () => {
    setup();
    expect(screen.getByText(displayId("abcdef1234567890abcdef1234567890"))).toBeInTheDocument();
  });

  it("renders the vote choice as Lock", () => {
    setup();
    expect(screen.getByText("Lock")).toBeInTheDocument();
  });

  it("renders Abstain vote choice", () => {
    setup({ scheduledVotes: [makeVote({ choice: "abstain" })] });
    expect(screen.getByText("Abstain")).toBeInTheDocument();
  });

  it("renders towardsIdentity vote choice", () => {
    setup({
      scheduledVotes: [
        makeVote({
          choice: {
            towardsIdentity: {
              identityId: "aaaa1111bbbb2222cccc3333dddd4444",
            },
          },
        }),
      ],
    });
    expect(screen.getByText(displayId("aaaa1111bbbb2222cccc3333dddd4444"))).toBeInTheDocument();
  });

  it("renders Pending status for notStarted", () => {
    setup({
      scheduledVotes: [makeVote({ castingStatus: "notStarted" })],
    });
    expect(screen.getByText("Pending")).toBeInTheDocument();
  });

  it("renders Casting... status for inProgress", () => {
    setup({
      scheduledVotes: [makeVote({ castingStatus: "inProgress" })],
    });
    expect(screen.getByText("Casting...")).toBeInTheDocument();
  });

  it("renders Casted status for completed", () => {
    setup({
      scheduledVotes: [makeVote({ castingStatus: "completed" })],
    });
    expect(screen.getByText("Casted")).toBeInTheDocument();
  });

  it("renders Failed status for failed", () => {
    setup({
      scheduledVotes: [makeVote({ castingStatus: "failed" })],
    });
    expect(screen.getByText("Failed")).toBeInTheDocument();
  });

  it("renders relative time for scheduled time", () => {
    setup({
      scheduledVotes: [
        makeVote({ unixTimestamp: Date.now() + 7_500_000 }), // 2h 5m to avoid boundary
      ],
    });
    expect(screen.getByText("in 2h")).toBeInTheDocument();
  });

  it("renders multiple scheduled votes", () => {
    setup({
      scheduledVotes: [
        makeVote({ contestedName: "alice" }),
        makeVote({ contestedName: "bob", voterId: "111122223333" }),
      ],
    });
    expect(screen.getByText("alice")).toBeInTheDocument();
    expect(screen.getByText("bob")).toBeInTheDocument();
  });

  it("renders all column headers", () => {
    setup();
    expect(
      screen.getByRole("button", { name: /sort by name/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /sort by voter/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /sort by vote choice/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /sort by scheduled time/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /sort by status/i }),
    ).toBeInTheDocument();
    expect(screen.getByText("Actions")).toBeInTheDocument();
  });
});

// ─── Filtering ───────────────────────────────────────────────────

describe("ScheduledVotesTable — filtering", () => {
  it("renders filter input", () => {
    setup();
    expect(
      screen.getByLabelText("Filter scheduled votes"),
    ).toBeInTheDocument();
  });

  it("filters by name", async () => {
    const { user } = setup({
      scheduledVotes: [
        makeVote({ contestedName: "alice" }),
        makeVote({ contestedName: "bob", voterId: "111122223333" }),
      ],
    });
    const input = screen.getByLabelText("Filter scheduled votes");
    await user.type(input, "alice");

    expect(screen.getByText("alice")).toBeInTheDocument();
    expect(screen.queryByText("bob")).not.toBeInTheDocument();
  });

  it("filters by voter ID", async () => {
    const { user } = setup({
      scheduledVotes: [
        makeVote({ contestedName: "alice", voterId: "aaaa1111" }),
        makeVote({ contestedName: "bob", voterId: "bbbb2222" }),
      ],
    });
    const input = screen.getByLabelText("Filter scheduled votes");
    await user.type(input, "aaaa");

    expect(screen.getByText("alice")).toBeInTheDocument();
    expect(screen.queryByText("bob")).not.toBeInTheDocument();
  });

  it("filter is case-insensitive", async () => {
    const { user } = setup({
      scheduledVotes: [makeVote({ contestedName: "Alice" })],
    });
    const input = screen.getByLabelText("Filter scheduled votes");
    await user.type(input, "alice");

    expect(screen.getByText("Alice")).toBeInTheDocument();
  });
});

// ─── Sorting ─────────────────────────────────────────────────────

describe("ScheduledVotesTable — sorting", () => {
  it("sorts by name ascending by default", () => {
    setup({
      scheduledVotes: [
        makeVote({ contestedName: "zorro" }),
        makeVote({ contestedName: "alice", voterId: "111122223333" }),
      ],
    });
    const rows = screen.getAllByRole("row");
    expect(rows[1]).toHaveTextContent("alice");
    expect(rows[2]).toHaveTextContent("zorro");
  });

  it("toggles sort order when clicking same column", async () => {
    const { user } = setup({
      scheduledVotes: [
        makeVote({ contestedName: "alice" }),
        makeVote({ contestedName: "zorro", voterId: "111122223333" }),
      ],
    });

    await user.click(
      screen.getByRole("button", { name: /sort by name/i }),
    );

    const rows = screen.getAllByRole("row");
    expect(rows[1]).toHaveTextContent("zorro");
    expect(rows[2]).toHaveTextContent("alice");
  });

  it("sorts by scheduled time when clicking that header", async () => {
    const now = Date.now();
    const { user } = setup({
      scheduledVotes: [
        makeVote({
          contestedName: "later",
          unixTimestamp: now + 172_800_000,
        }),
        makeVote({
          contestedName: "sooner",
          unixTimestamp: now + 3_600_000,
          voterId: "111122223333",
        }),
      ],
    });

    await user.click(
      screen.getByRole("button", { name: /sort by scheduled time/i }),
    );

    const rows = screen.getAllByRole("row");
    expect(rows[1]).toHaveTextContent("sooner");
    expect(rows[2]).toHaveTextContent("later");
  });

  it("sorts by status when clicking that header", async () => {
    const { user } = setup({
      scheduledVotes: [
        makeVote({
          contestedName: "completed-vote",
          castingStatus: "completed",
        }),
        makeVote({
          contestedName: "pending-vote",
          castingStatus: "notStarted",
          voterId: "111122223333",
        }),
      ],
    });

    await user.click(
      screen.getByRole("button", { name: /sort by status/i }),
    );

    const rows = screen.getAllByRole("row");
    expect(rows[1]).toHaveTextContent("pending-vote");
    expect(rows[2]).toHaveTextContent("completed-vote");
  });
});

// ─── Actions ─────────────────────────────────────────────────────

describe("ScheduledVotesTable — actions", () => {
  it("calls onRemove when clicking Remove button", async () => {
    const onRemove = vi.fn();
    const { user } = setup({ onRemove });

    await user.click(
      screen.getByRole("button", { name: /remove test-name/i }),
    );

    expect(onRemove).toHaveBeenCalledWith(
      "abcdef1234567890abcdef1234567890",
      "test-name",
    );
  });

  it("calls onCastNow when clicking Cast Now button", async () => {
    const onCastNow = vi.fn();
    const vote = makeVote({ castingStatus: "notStarted" });
    const { user } = setup({
      scheduledVotes: [vote],
      onCastNow,
    });

    await user.click(
      screen.getByRole("button", { name: /cast now test-name/i }),
    );

    expect(onCastNow).toHaveBeenCalledWith(vote);
  });

  it("Cast Now is enabled for notStarted votes", () => {
    setup({
      scheduledVotes: [makeVote({ castingStatus: "notStarted" })],
    });
    expect(
      screen.getByRole("button", { name: /cast now test-name/i }),
    ).not.toBeDisabled();
  });

  it("Cast Now is enabled for failed votes", () => {
    setup({
      scheduledVotes: [makeVote({ castingStatus: "failed" })],
    });
    expect(
      screen.getByRole("button", { name: /cast now test-name/i }),
    ).not.toBeDisabled();
  });

  it("Cast Now is disabled for inProgress votes", () => {
    setup({
      scheduledVotes: [makeVote({ castingStatus: "inProgress" })],
    });
    expect(
      screen.getByRole("button", { name: /cast now test-name/i }),
    ).toBeDisabled();
  });

  it("Cast Now is disabled for completed votes", () => {
    setup({
      scheduledVotes: [makeVote({ castingStatus: "completed" })],
    });
    expect(
      screen.getByRole("button", { name: /cast now test-name/i }),
    ).toBeDisabled();
  });

  it("Remove button is always enabled", () => {
    setup({
      scheduledVotes: [makeVote({ castingStatus: "inProgress" })],
    });
    expect(
      screen.getByRole("button", { name: /remove test-name/i }),
    ).not.toBeDisabled();
  });

  it("renders Cast Now and Remove for each row", () => {
    setup({
      scheduledVotes: [
        makeVote({ contestedName: "alice" }),
        makeVote({ contestedName: "bob", voterId: "111122223333" }),
      ],
    });
    expect(
      screen.getByRole("button", { name: /cast now alice/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /cast now bob/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /remove alice/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /remove bob/i }),
    ).toBeInTheDocument();
  });
});

// ─── Accessibility ───────────────────────────────────────────────

describe("ScheduledVotesTable — accessibility", () => {
  it("filter input has accessible label", () => {
    setup();
    expect(
      screen.getByLabelText("Filter scheduled votes"),
    ).toBeInTheDocument();
  });

  it("sort buttons have aria-label", () => {
    setup();
    const sortButtons = screen.getAllByRole("button", {
      name: /sort by/i,
    });
    expect(sortButtons.length).toBe(5); // Name, Voter, Vote Choice, Scheduled Time, Status
  });

  it("action buttons have aria-labels with name", () => {
    setup({
      scheduledVotes: [makeVote({ contestedName: "test-name" })],
    });
    expect(
      screen.getByRole("button", { name: /cast now test-name/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /remove test-name/i }),
    ).toBeInTheDocument();
  });
});
