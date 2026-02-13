import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeAll, beforeEach } from "vitest";
import {
  VoteCastingDialog,
  formatVoteChoice,
} from "./VoteCastingDialog";
import type { VoteCastingDialogProps } from "./VoteCastingDialog";
import type { SelectedVote } from "@/stores/contestStore";
import type { QualifiedIdentityDto, VoteChoiceDto } from "@/bindings";
import { displayId } from "@/lib/utils";

// ─── Polyfills for Radix Select in jsdom ──────────────────────────

beforeAll(() => {
  if (!Element.prototype.hasPointerCapture) {
    Element.prototype.hasPointerCapture = () => false;
  }
  if (!Element.prototype.setPointerCapture) {
    Element.prototype.setPointerCapture = () => {};
  }
  if (!Element.prototype.releasePointerCapture) {
    Element.prototype.releasePointerCapture = () => {};
  }
  if (!Element.prototype.scrollIntoView) {
    Element.prototype.scrollIntoView = () => {};
  }
});

// ─── Test fixtures ─────────────────────────────────────────────────

function makeIdentity(
  overrides: Partial<QualifiedIdentityDto> = {},
): QualifiedIdentityDto {
  return {
    id: "aaaa1111bbbb2222cccc3333dddd4444",
    identityType: "masternode",
    alias: "MN-1",
    balance: 100000,
    keys: [],
    dpnsNames: [],
    associatedWalletHashes: [],
    ...overrides,
  } as QualifiedIdentityDto;
}

function makeVote(overrides: Partial<SelectedVote> = {}): SelectedVote {
  return {
    contestedName: "d4sh",
    choice: "lock" as VoteChoiceDto,
    endTime: Date.now() + 86_400_000,
    ...overrides,
  };
}

const defaultProps: VoteCastingDialogProps = {
  open: true,
  onOpenChange: vi.fn(),
  selectedVotes: [makeVote()],
  votingIdentities: [makeIdentity()],
  onApplyVotes: vi.fn().mockResolvedValue(undefined),
  onGoToActiveContests: vi.fn(),
  onGoToScheduledVotes: vi.fn(),
  votingInProgress: false,
};

function setup(props: Partial<VoteCastingDialogProps> = {}) {
  const mergedProps = { ...defaultProps, ...props };
  return {
    user: userEvent.setup(),
    ...render(<VoteCastingDialog {...mergedProps} />),
    props: mergedProps,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

// ─── formatVoteChoice ─────────────────────────────────────────────

describe("formatVoteChoice", () => {
  it("formats lock choice", () => {
    expect(formatVoteChoice("lock")).toBe("Lock");
  });

  it("formats abstain choice", () => {
    expect(formatVoteChoice("abstain")).toBe("Abstain");
  });

  it("formats towardsIdentity with truncation", () => {
    const id = "aaaa1111bbbb2222cccc3333dddd4444";
    const choice: VoteChoiceDto = {
      towardsIdentity: { identityId: id },
    };
    expect(formatVoteChoice(choice)).toBe(displayId(id));
  });

  it("formats short towardsIdentity without truncation", () => {
    const choice: VoteChoiceDto = {
      towardsIdentity: { identityId: "short123" },
    };
    expect(formatVoteChoice(choice)).toBe("short123");
  });
});

// ─── Dialog rendering ─────────────────────────────────────────────

describe("VoteCastingDialog — rendering", () => {
  it("renders the dialog title", () => {
    setup();
    expect(screen.getByText("Voting")).toBeInTheDocument();
  });

  it("renders the vote count in description", () => {
    setup({ selectedVotes: [makeVote(), makeVote({ contestedName: "other" })] });
    expect(
      screen.getByText(/Cast or schedule 2 votes/),
    ).toBeInTheDocument();
  });

  it("uses singular when 1 vote selected", () => {
    setup({ selectedVotes: [makeVote()] });
    expect(
      screen.getByText(/Cast or schedule 1 vote using/),
    ).toBeInTheDocument();
  });

  it("renders selected votes summary", () => {
    setup({
      selectedVotes: [
        makeVote({ contestedName: "d4sh", choice: "lock" }),
        makeVote({ contestedName: "alice", choice: "abstain" }),
      ],
    });
    expect(screen.getByText("d4sh")).toBeInTheDocument();
    expect(screen.getByText("alice")).toBeInTheDocument();
    expect(screen.getByText("Lock")).toBeInTheDocument();
    expect(screen.getByText("Abstain")).toBeInTheDocument();
  });

  it("renders end time for votes that have one", () => {
    const futureTime = Date.now() + 2 * 86_400_000; // 2 days
    setup({
      selectedVotes: [makeVote({ endTime: futureTime })],
    });
    expect(screen.getByText(/ends in/)).toBeInTheDocument();
  });

  it("renders identity list", () => {
    setup({
      votingIdentities: [
        makeIdentity({ id: "aaa111", alias: "MN-1" }),
        makeIdentity({ id: "bbb222", alias: "MN-2" }),
      ],
    });
    expect(screen.getByText("MN-1")).toBeInTheDocument();
    expect(screen.getByText("MN-2")).toBeInTheDocument();
  });

  it("shows identity type", () => {
    setup({
      votingIdentities: [makeIdentity({ identityType: "evonode" })],
    });
    expect(screen.getByText("evonode")).toBeInTheDocument();
  });

  it("shows truncated ID when alias is null", () => {
    setup({
      votingIdentities: [
        makeIdentity({
          id: "aaaa1111bbbb2222cccc3333dddd4444",
          alias: null,
        }),
      ],
    });
    expect(screen.getByText("aaaa1111...dddd4444")).toBeInTheDocument();
  });

  it("renders Set All controls", () => {
    setup();
    expect(
      screen.getByLabelText("Set all vote method"),
    ).toBeInTheDocument();
    expect(
      screen.getByLabelText("Apply to all identities"),
    ).toBeInTheDocument();
  });

  it("renders Apply Votes and Cancel buttons", () => {
    setup();
    expect(screen.getByLabelText("Apply votes")).toBeInTheDocument();
    expect(screen.getByText("Cancel")).toBeInTheDocument();
  });
});

// ─── Empty / error states ─────────────────────────────────────────

describe("VoteCastingDialog — validation", () => {
  it("shows error when no votes selected", () => {
    setup({ selectedVotes: [] });
    expect(screen.getByText("No votes selected.")).toBeInTheDocument();
    expect(screen.getByLabelText("Apply votes")).toBeDisabled();
  });

  it("shows error when no voting identities", () => {
    setup({ votingIdentities: [] });
    expect(
      screen.getByText("No voting identities loaded."),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Apply votes")).toBeDisabled();
  });

  it("shows error when no identity has action selected", () => {
    setup(); // default is all "noVote"
    expect(
      screen.getByText(
        "Select at least one identity to cast or schedule votes.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Apply votes")).toBeDisabled();
  });
});

// ─── Per-identity vote method selection ───────────────────────────

describe("VoteCastingDialog — identity vote method", () => {
  it("changes identity vote method via select", async () => {
    const { user } = setup({
      votingIdentities: [makeIdentity({ alias: "MN-1" })],
    });

    // Open the identity's method select
    const methodSelect = screen.getByLabelText("Vote method for MN-1");
    await user.click(methodSelect);

    // Select "Cast Now"
    const castNowOption = screen.getByRole("option", { name: "Cast Now" });
    await user.click(castNowOption);

    // Apply button should now be enabled (validation passes)
    expect(screen.getByLabelText("Apply votes")).toBeEnabled();
  });

  it("shows schedule time picker when schedule selected", async () => {
    const { user } = setup({
      votingIdentities: [makeIdentity({ alias: "MN-1" })],
    });

    const methodSelect = screen.getByLabelText("Vote method for MN-1");
    await user.click(methodSelect);
    const scheduleOption = screen.getByRole("option", { name: "Schedule" });
    await user.click(scheduleOption);

    // Time pickers should appear
    expect(screen.getByLabelText("Days")).toBeInTheDocument();
    expect(screen.getByLabelText("Hours")).toBeInTheDocument();
    expect(screen.getByLabelText("Minutes")).toBeInTheDocument();
  });

  it("shows validation error when schedule time is 0", async () => {
    const { user } = setup({
      votingIdentities: [makeIdentity({ alias: "MN-1" })],
    });

    const methodSelect = screen.getByLabelText("Vote method for MN-1");
    await user.click(methodSelect);
    const scheduleOption = screen.getByRole("option", { name: "Schedule" });
    await user.click(scheduleOption);

    // With 0 days/hours/minutes, should show error
    expect(screen.getByText(/Schedule time for MN-1 must be > 0/)).toBeInTheDocument();
    expect(screen.getByLabelText("Apply votes")).toBeDisabled();
  });
});

// ─── Set All controls ─────────────────────────────────────────────

describe("VoteCastingDialog — Set All", () => {
  it("applies Cast Now to all identities", async () => {
    const { user } = setup({
      votingIdentities: [
        makeIdentity({ id: "aaa", alias: "MN-1" }),
        makeIdentity({ id: "bbb", alias: "MN-2" }),
      ],
    });

    // Set the "Set All" select to "Cast Now"
    const setAllSelect = screen.getByLabelText("Set all vote method");
    await user.click(setAllSelect);
    const castNowOption = screen.getByRole("option", { name: "Cast Now" });
    await user.click(castNowOption);

    // Click "Apply to All"
    await user.click(screen.getByLabelText("Apply to all identities"));

    // Both identities should now show the Apply Votes button as enabled
    expect(screen.getByLabelText("Apply votes")).toBeEnabled();
  });

  it("shows schedule time picker when Set All is schedule", async () => {
    const { user } = setup({
      votingIdentities: [makeIdentity({ alias: "MN-1" })],
    });

    const setAllSelect = screen.getByLabelText("Set all vote method");
    await user.click(setAllSelect);
    const scheduleOption = screen.getByRole("option", { name: "Schedule" });
    await user.click(scheduleOption);

    // Time pickers should appear next to the Set All control
    // There should be Days/Hours/Minutes labels
    const dayInputs = screen.getAllByLabelText("Days");
    expect(dayInputs.length).toBeGreaterThanOrEqual(1);
  });
});

// ─── Schedule warning ─────────────────────────────────────────────

describe("VoteCastingDialog — schedule warning", () => {
  it("shows warning when any identity has schedule selected", async () => {
    const { user } = setup({
      votingIdentities: [makeIdentity({ alias: "MN-1" })],
    });

    // Select schedule for MN-1
    const methodSelect = screen.getByLabelText("Vote method for MN-1");
    await user.click(methodSelect);
    const scheduleOption = screen.getByRole("option", { name: "Schedule" });
    await user.click(scheduleOption);

    expect(
      screen.getByText(/Dash Evo Tool must remain running/),
    ).toBeInTheDocument();
  });

  it("does not show warning when only Cast Now selected", async () => {
    const { user } = setup({
      votingIdentities: [makeIdentity({ alias: "MN-1" })],
    });

    const methodSelect = screen.getByLabelText("Vote method for MN-1");
    await user.click(methodSelect);
    const castNowOption = screen.getByRole("option", { name: "Cast Now" });
    await user.click(castNowOption);

    expect(
      screen.queryByText(/Dash Evo Tool must remain running/),
    ).not.toBeInTheDocument();
  });
});

// ─── Apply votes ──────────────────────────────────────────────────

describe("VoteCastingDialog — applying votes", () => {
  it("calls onApplyVotes with immediate identity IDs for Cast Now", async () => {
    const onApply = vi.fn().mockResolvedValue(undefined);
    const { user } = setup({
      votingIdentities: [makeIdentity({ id: "aaa111", alias: "MN-1" })],
      onApplyVotes: onApply,
    });

    // Select Cast Now
    const methodSelect = screen.getByLabelText("Vote method for MN-1");
    await user.click(methodSelect);
    await user.click(screen.getByRole("option", { name: "Cast Now" }));

    // Apply
    await user.click(screen.getByLabelText("Apply votes"));

    expect(onApply).toHaveBeenCalledWith(
      ["aaa111"],
      [],
    );
  });

  it("calls onApplyVotes with scheduled votes for Schedule", async () => {
    const onApply = vi.fn().mockResolvedValue(undefined);
    const { user } = setup({
      votingIdentities: [makeIdentity({ id: "bbb222", alias: "MN-2" })],
      onApplyVotes: onApply,
    });

    // Select Schedule
    const methodSelect = screen.getByLabelText("Vote method for MN-2");
    await user.click(methodSelect);
    await user.click(screen.getByRole("option", { name: "Schedule" }));

    // Set 1 day schedule
    const daysInput = screen.getByLabelText("Days");
    await user.clear(daysInput);
    await user.type(daysInput, "1");

    // Apply
    await user.click(screen.getByLabelText("Apply votes"));

    expect(onApply).toHaveBeenCalledWith(
      [],
      [expect.objectContaining({ voterId: "bbb222" })],
    );
    // Check that unixTimestamp is roughly 1 day in the future
    const scheduledVotes = onApply.mock.calls[0][1];
    const delta = scheduledVotes[0].unixTimestamp - Date.now();
    expect(delta).toBeGreaterThan(85_000_000); // ~23.6 hours
    expect(delta).toBeLessThan(87_000_000); // ~24.2 hours
  });

  it("sends both immediate and scheduled when mixed", async () => {
    const onApply = vi.fn().mockResolvedValue(undefined);
    const { user } = setup({
      votingIdentities: [
        makeIdentity({ id: "aaa", alias: "MN-1" }),
        makeIdentity({ id: "bbb", alias: "MN-2" }),
      ],
      onApplyVotes: onApply,
    });

    // MN-1: Cast Now
    const mn1Select = screen.getByLabelText("Vote method for MN-1");
    await user.click(mn1Select);
    await user.click(screen.getByRole("option", { name: "Cast Now" }));

    // MN-2: Schedule
    const mn2Select = screen.getByLabelText("Vote method for MN-2");
    await user.click(mn2Select);
    await user.click(screen.getByRole("option", { name: "Schedule" }));

    // Set 2 hours schedule for MN-2
    const hoursInputs = screen.getAllByLabelText("Hours");
    // The last one should be MN-2's
    const mn2Hours = hoursInputs[hoursInputs.length - 1];
    await user.clear(mn2Hours);
    await user.type(mn2Hours, "2");

    // Apply
    await user.click(screen.getByLabelText("Apply votes"));

    expect(onApply).toHaveBeenCalledWith(
      ["aaa"],
      [expect.objectContaining({ voterId: "bbb" })],
    );
  });
});

// ─── Progress states ──────────────────────────────────────────────

describe("VoteCastingDialog — progress", () => {
  it("shows casting progress when vote is being applied", async () => {
    // Use a never-resolving promise to keep the dialog in casting state
    const onApply = vi.fn().mockReturnValue(new Promise(() => {}));
    const { user } = setup({
      votingIdentities: [makeIdentity({ id: "aaa", alias: "MN-1" })],
      onApplyVotes: onApply,
    });

    // Select Cast Now
    await user.click(screen.getByLabelText("Vote method for MN-1"));
    await user.click(screen.getByRole("option", { name: "Cast Now" }));
    await user.click(screen.getByLabelText("Apply votes"));

    expect(screen.getByText("Casting votes...")).toBeInTheDocument();
    expect(screen.getByTestId("elapsed-time")).toBeInTheDocument();
  });

  it("shows scheduling progress when only scheduling", async () => {
    const onApply = vi.fn().mockReturnValue(new Promise(() => {}));
    const { user } = setup({
      votingIdentities: [makeIdentity({ id: "aaa", alias: "MN-1" })],
      onApplyVotes: onApply,
    });

    // Select Schedule
    await user.click(screen.getByLabelText("Vote method for MN-1"));
    await user.click(screen.getByRole("option", { name: "Schedule" }));

    // Set 1 hour
    const hoursInput = screen.getByLabelText("Hours");
    await user.clear(hoursInput);
    await user.type(hoursInput, "1");

    await user.click(screen.getByLabelText("Apply votes"));

    expect(screen.getByText("Scheduling votes...")).toBeInTheDocument();
  });
});

// ─── Completed state ──────────────────────────────────────────────

describe("VoteCastingDialog — completed", () => {
  it("shows success when all votes succeeded", async () => {
    const onApply = vi.fn().mockResolvedValue(undefined);
    const { user } = setup({
      votingIdentities: [makeIdentity({ id: "aaa", alias: "MN-1" })],
      onApplyVotes: onApply,
    });

    // Cast Now
    await user.click(screen.getByLabelText("Vote method for MN-1"));
    await user.click(screen.getByRole("option", { name: "Cast Now" }));
    await user.click(screen.getByLabelText("Apply votes"));

    await waitFor(() => {
      expect(
        screen.getByText("Successfully casted and scheduled all votes"),
      ).toBeInTheDocument();
    });
  });

  it("shows Go to Active Contests button on completion", async () => {
    const onGoToActive = vi.fn();
    const onApply = vi.fn().mockResolvedValue(undefined);
    const { user } = setup({
      votingIdentities: [makeIdentity({ id: "aaa", alias: "MN-1" })],
      onApplyVotes: onApply,
      onGoToActiveContests: onGoToActive,
    });

    await user.click(screen.getByLabelText("Vote method for MN-1"));
    await user.click(screen.getByRole("option", { name: "Cast Now" }));
    await user.click(screen.getByLabelText("Apply votes"));

    await waitFor(() => {
      expect(screen.getByText("Go to Active Contests")).toBeInTheDocument();
    });

    await user.click(screen.getByText("Go to Active Contests"));
    expect(onGoToActive).toHaveBeenCalled();
  });

  it("shows Go to Scheduled Votes when votes were scheduled", async () => {
    const onGoToScheduled = vi.fn();
    const onApply = vi.fn().mockResolvedValue(undefined);
    const { user } = setup({
      votingIdentities: [makeIdentity({ id: "aaa", alias: "MN-1" })],
      onApplyVotes: onApply,
      onGoToScheduledVotes: onGoToScheduled,
    });

    // Schedule
    await user.click(screen.getByLabelText("Vote method for MN-1"));
    await user.click(screen.getByRole("option", { name: "Schedule" }));
    const hoursInput = screen.getByLabelText("Hours");
    await user.clear(hoursInput);
    await user.type(hoursInput, "1");

    await user.click(screen.getByLabelText("Apply votes"));

    await waitFor(() => {
      expect(screen.getByText("Go to Scheduled Votes")).toBeInTheDocument();
    });

    await user.click(screen.getByText("Go to Scheduled Votes"));
    expect(onGoToScheduled).toHaveBeenCalled();
  });

  it("shows cast result summary", async () => {
    const onApply = vi.fn().mockResolvedValue(undefined);
    const { user } = setup({
      votingIdentities: [makeIdentity({ id: "aaa", alias: "MN-1" })],
      onApplyVotes: onApply,
    });

    await user.click(screen.getByLabelText("Vote method for MN-1"));
    await user.click(screen.getByRole("option", { name: "Cast Now" }));
    await user.click(screen.getByLabelText("Apply votes"));

    await waitFor(() => {
      expect(screen.getByText("Votes cast:")).toBeInTheDocument();
      expect(screen.getByText("1/1 succeeded")).toBeInTheDocument();
    });
  });
});

// ─── Failed state ─────────────────────────────────────────────────

describe("VoteCastingDialog — failure", () => {
  it("shows error message when vote fails", async () => {
    const onApply = vi
      .fn()
      .mockRejectedValue(new Error("Network error"));
    const { user } = setup({
      votingIdentities: [makeIdentity({ id: "aaa", alias: "MN-1" })],
      onApplyVotes: onApply,
    });

    await user.click(screen.getByLabelText("Vote method for MN-1"));
    await user.click(screen.getByRole("option", { name: "Cast Now" }));
    await user.click(screen.getByLabelText("Apply votes"));

    await waitFor(() => {
      expect(screen.getByText("Network error")).toBeInTheDocument();
      expect(
        screen.getByText("Error casting and scheduling votes"),
      ).toBeInTheDocument();
    });
  });

  it("allows retry after failure", async () => {
    const onApply = vi
      .fn()
      .mockRejectedValueOnce(new Error("Temporary error"))
      .mockResolvedValueOnce(undefined);
    const { user } = setup({
      votingIdentities: [makeIdentity({ id: "aaa", alias: "MN-1" })],
      onApplyVotes: onApply,
    });

    // First attempt: Cast Now → fail
    await user.click(screen.getByLabelText("Vote method for MN-1"));
    await user.click(screen.getByRole("option", { name: "Cast Now" }));
    await user.click(screen.getByLabelText("Apply votes"));

    await waitFor(() => {
      expect(screen.getByText("Try Again")).toBeInTheDocument();
    });

    // Retry — goes back to form
    await user.click(screen.getByText("Try Again"));

    // Form should be visible again with the method still selected
    expect(screen.getByLabelText("Apply votes")).toBeInTheDocument();
  });

  it("allows close after failure", async () => {
    const onApply = vi.fn().mockRejectedValue(new Error("Failed"));
    const onOpenChange = vi.fn();
    const { user } = setup({
      votingIdentities: [makeIdentity({ id: "aaa", alias: "MN-1" })],
      onApplyVotes: onApply,
      onOpenChange,
    });

    await user.click(screen.getByLabelText("Vote method for MN-1"));
    await user.click(screen.getByRole("option", { name: "Cast Now" }));
    await user.click(screen.getByLabelText("Apply votes"));

    await waitFor(() => {
      expect(
        screen.getByText("Error casting and scheduling votes"),
      ).toBeInTheDocument();
    });

    // Find the Close button inside the dialog footer (not the X close)
    const closeButtons = screen.getAllByRole("button", { name: "Close" });
    const footerClose = closeButtons.find(
      (btn) => btn.getAttribute("data-variant") === "outline",
    );
    expect(footerClose).toBeTruthy();
    await user.click(footerClose!);
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });
});

// ─── Cancel ───────────────────────────────────────────────────────

describe("VoteCastingDialog — cancel", () => {
  it("calls onOpenChange(false) when Cancel is clicked", async () => {
    const onOpenChange = vi.fn();
    const { user } = setup({ onOpenChange });

    await user.click(screen.getByText("Cancel"));
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });
});

// ─── Accessibility ────────────────────────────────────────────────

describe("VoteCastingDialog — accessibility", () => {
  it("has aria-label on the dialog", () => {
    setup();
    expect(screen.getByLabelText("Vote casting dialog")).toBeInTheDocument();
  });

  it("has aria-label on Apply Votes button", () => {
    setup();
    expect(screen.getByLabelText("Apply votes")).toBeInTheDocument();
  });

  it("has role=alert on validation error", () => {
    setup({ selectedVotes: [] });
    expect(screen.getByRole("alert")).toBeInTheDocument();
  });

  it("has role=status on success message", async () => {
    const onApply = vi.fn().mockResolvedValue(undefined);
    const { user } = setup({
      votingIdentities: [makeIdentity({ id: "aaa", alias: "MN-1" })],
      onApplyVotes: onApply,
    });

    await user.click(screen.getByLabelText("Vote method for MN-1"));
    await user.click(screen.getByRole("option", { name: "Cast Now" }));
    await user.click(screen.getByLabelText("Apply votes"));

    await waitFor(() => {
      expect(screen.getByRole("status")).toBeInTheDocument();
    });
  });

  it("has role=alert on failure message", async () => {
    const onApply = vi.fn().mockRejectedValue(new Error("fail"));
    const { user } = setup({
      votingIdentities: [makeIdentity({ id: "aaa", alias: "MN-1" })],
      onApplyVotes: onApply,
    });

    await user.click(screen.getByLabelText("Vote method for MN-1"));
    await user.click(screen.getByRole("option", { name: "Cast Now" }));
    await user.click(screen.getByLabelText("Apply votes"));

    await waitFor(() => {
      const alerts = screen.getAllByRole("alert");
      expect(alerts.length).toBeGreaterThanOrEqual(1);
    });
  });

  it("each identity has labeled select", () => {
    setup({
      votingIdentities: [
        makeIdentity({ id: "aaa", alias: "MN-1" }),
        makeIdentity({ id: "bbb", alias: "MN-2" }),
      ],
    });
    expect(
      screen.getByLabelText("Vote method for MN-1"),
    ).toBeInTheDocument();
    expect(
      screen.getByLabelText("Vote method for MN-2"),
    ).toBeInTheDocument();
  });
});

// ─── Time input ───────────────────────────────────────────────────

describe("VoteCastingDialog — time input", () => {
  it("clamps days input to 0-14", async () => {
    const { user } = setup({
      votingIdentities: [makeIdentity({ alias: "MN-1" })],
    });

    // Select Schedule for MN-1
    await user.click(screen.getByLabelText("Vote method for MN-1"));
    await user.click(screen.getByRole("option", { name: "Schedule" }));

    const daysInput = screen.getByLabelText("Days") as HTMLInputElement;
    expect(daysInput).toHaveAttribute("max", "14");
    expect(daysInput).toHaveAttribute("min", "0");
  });

  it("clamps hours input to 0-23", async () => {
    const { user } = setup({
      votingIdentities: [makeIdentity({ alias: "MN-1" })],
    });

    await user.click(screen.getByLabelText("Vote method for MN-1"));
    await user.click(screen.getByRole("option", { name: "Schedule" }));

    const hoursInput = screen.getByLabelText("Hours") as HTMLInputElement;
    expect(hoursInput).toHaveAttribute("max", "23");
  });

  it("clamps minutes input to 0-59", async () => {
    const { user } = setup({
      votingIdentities: [makeIdentity({ alias: "MN-1" })],
    });

    await user.click(screen.getByLabelText("Vote method for MN-1"));
    await user.click(screen.getByRole("option", { name: "Schedule" }));

    const minutesInput = screen.getByLabelText("Minutes") as HTMLInputElement;
    expect(minutesInput).toHaveAttribute("max", "59");
  });
});

// ─── Dialog close prevention ──────────────────────────────────────

describe("VoteCastingDialog — close prevention", () => {
  it("does not show close button while processing", async () => {
    const onApply = vi.fn().mockReturnValue(new Promise(() => {}));
    const { user } = setup({
      votingIdentities: [makeIdentity({ id: "aaa", alias: "MN-1" })],
      onApplyVotes: onApply,
    });

    await user.click(screen.getByLabelText("Vote method for MN-1"));
    await user.click(screen.getByRole("option", { name: "Cast Now" }));
    await user.click(screen.getByLabelText("Apply votes"));

    // The close X button should not be present
    expect(screen.queryByText("Cancel")).not.toBeInTheDocument();
  });
});
