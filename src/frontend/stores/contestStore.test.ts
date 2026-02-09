import { describe, it, expect, vi, beforeEach, type Mock } from "vitest";
import {
  useContestStore,
  normalizeDpnsFilter,
  matchesDpnsFilter,
} from "./contestStore";
import type {
  ContestedName,
  SelectedVote,
} from "./contestStore";
import type {
  DpnsNameEntryDto,
  ScheduledVoteDto,
  TaskResultEvent,
  ScheduledVoteExecutedEvent,
} from "../bindings";

// ─── Mock bindings ──────────────────────────────────────────────────

vi.mock("../bindings", () => ({
  commands: {
    contestedQueryDpnsContests: vi.fn(),
    contestedVoteOnDpnsNames: vi.fn(),
    contestedScheduleDpnsVotes: vi.fn(),
    contestedCastScheduledVote: vi.fn(),
    contestedDeleteScheduledVote: vi.fn(),
    contestedClearAllScheduledVotes: vi.fn(),
    contestedClearExecutedScheduledVotes: vi.fn(),
    contestedGetScheduledVotes: vi.fn(),
    identityLocalDpnsNames: vi.fn(),
    identityRefreshDpnsNames: vi.fn(),
  },
  events: {
    taskResultEvent: {
      listen: vi.fn().mockResolvedValue(() => {}),
    },
    taskErrorEvent: {
      listen: vi.fn().mockResolvedValue(() => {}),
    },
    scheduledVoteExecutedEvent: {
      listen: vi.fn().mockResolvedValue(() => {}),
    },
  },
}));

import { commands, events } from "../bindings";

// ─── Test fixtures ──────────────────────────────────────────────────

function makeContest(
  overrides?: Partial<ContestedName>,
): ContestedName {
  return {
    normalizedContestedName: "alice",
    contestants: [
      {
        id: "contestant1",
        name: "alice",
        info: "",
        votes: 5,
        createdAt: null,
        createdAtBlockHeight: null,
        createdAtCoreBlockHeight: null,
        documentId: "doc1",
      },
    ],
    lockedVotes: 3,
    abstainVotes: 1,
    awardedTo: null,
    endTime: 1700000000,
    state: "ongoing",
    lastUpdated: 1699000000,
    ...overrides,
  };
}

function makeScheduledVote(
  overrides?: Partial<ScheduledVoteDto>,
): ScheduledVoteDto {
  return {
    contestedName: "alice",
    voterId: "voter1",
    choice: "lock",
    unixTimestamp: 1700050000,
    executedSuccessfully: false,
    ...overrides,
  };
}

function makeDpnsName(
  overrides?: Partial<DpnsNameEntryDto>,
): DpnsNameEntryDto {
  return {
    identityId: "id1",
    name: "myname.dash",
    acquiredAt: 1699000000,
    ...overrides,
  };
}

// ─── Helpers ────────────────────────────────────────────────────────

function resetStore() {
  useContestStore.setState({
    contestedNames: [],
    localDpnsNames: [],
    scheduledVotes: [],
    selectedVotes: [],
    loading: false,
    refreshing: false,
    votingInProgress: false,
    error: null,
    activeFilterTerm: "",
    pastFilterTerm: "",
    ownedFilterTerm: "",
    sortColumn: "name",
    sortOrder: "ascending",
    scheduledVoteCastInProgress: false,
  });
}

// ─── Tests ──────────────────────────────────────────────────────────

describe("contestStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore();
  });

  describe("initial state", () => {
    it("starts with empty data and default values", () => {
      const state = useContestStore.getState();
      expect(state.contestedNames).toEqual([]);
      expect(state.localDpnsNames).toEqual([]);
      expect(state.scheduledVotes).toEqual([]);
      expect(state.selectedVotes).toEqual([]);
      expect(state.loading).toBe(false);
      expect(state.refreshing).toBe(false);
      expect(state.votingInProgress).toBe(false);
      expect(state.error).toBeNull();
      expect(state.activeFilterTerm).toBe("");
      expect(state.pastFilterTerm).toBe("");
      expect(state.ownedFilterTerm).toBe("");
      expect(state.sortColumn).toBe("name");
      expect(state.sortOrder).toBe("ascending");
      expect(state.scheduledVoteCastInProgress).toBe(false);
    });
  });

  describe("loadContests", () => {
    it("dispatches query and sets refreshing=true", async () => {
      (commands.contestedQueryDpnsContests as Mock).mockResolvedValue({
        taskId: "t1",
      });

      await useContestStore.getState().loadContests();

      expect(commands.contestedQueryDpnsContests).toHaveBeenCalled();
      // refreshing stays true until event arrives
      expect(useContestStore.getState().refreshing).toBe(true);
    });

    it("clears previous error on new load", async () => {
      useContestStore.setState({ error: "old error" });
      (commands.contestedQueryDpnsContests as Mock).mockResolvedValue({
        taskId: "t1",
      });

      await useContestStore.getState().loadContests();
      expect(useContestStore.getState().error).toBeNull();
    });

    it("sets error and clears refreshing on exception", async () => {
      (commands.contestedQueryDpnsContests as Mock).mockRejectedValue(
        new Error("Network error"),
      );

      await useContestStore.getState().loadContests();

      expect(useContestStore.getState().error).toBe("Network error");
      expect(useContestStore.getState().refreshing).toBe(false);
    });
  });

  describe("loadLocalNames", () => {
    it("loads DPNS names from backend", async () => {
      const names = [makeDpnsName(), makeDpnsName({ name: "other.dash" })];
      (commands.identityLocalDpnsNames as Mock).mockResolvedValue({
        status: "ok",
        data: names,
      });

      await useContestStore.getState().loadLocalNames();

      expect(useContestStore.getState().localDpnsNames).toEqual(names);
    });

    it("sets error on backend error", async () => {
      (commands.identityLocalDpnsNames as Mock).mockResolvedValue({
        status: "error",
        error: "DB error",
      });

      await useContestStore.getState().loadLocalNames();

      expect(useContestStore.getState().error).toBe("DB error");
    });

    it("sets error on exception", async () => {
      (commands.identityLocalDpnsNames as Mock).mockRejectedValue(
        new Error("IPC error"),
      );

      await useContestStore.getState().loadLocalNames();

      expect(useContestStore.getState().error).toBe("IPC error");
    });
  });

  describe("loadScheduledVotes", () => {
    it("loads scheduled votes with casting status", async () => {
      const votes = [
        makeScheduledVote(),
        makeScheduledVote({
          contestedName: "bob",
          executedSuccessfully: true,
        }),
      ];
      (commands.contestedGetScheduledVotes as Mock).mockResolvedValue({
        status: "ok",
        data: votes,
      });

      await useContestStore.getState().loadScheduledVotes();

      const scheduled = useContestStore.getState().scheduledVotes;
      expect(scheduled).toHaveLength(2);
      expect(scheduled[0].castingStatus).toBe("notStarted");
      expect(scheduled[1].castingStatus).toBe("completed");
    });

    it("sets error on backend error", async () => {
      (commands.contestedGetScheduledVotes as Mock).mockResolvedValue({
        status: "error",
        error: "DB locked",
      });

      await useContestStore.getState().loadScheduledVotes();

      expect(useContestStore.getState().error).toBe("DB locked");
    });
  });

  describe("selectVote / deselectVote / toggleVote", () => {
    it("adds a new vote selection", () => {
      const vote: SelectedVote = {
        contestedName: "alice",
        choice: "lock",
        endTime: 1700000000,
      };

      useContestStore.getState().selectVote(vote);

      expect(useContestStore.getState().selectedVotes).toEqual([vote]);
    });

    it("replaces existing vote for same name", () => {
      useContestStore.setState({
        selectedVotes: [
          {
            contestedName: "alice",
            choice: "lock",
            endTime: 1700000000,
          },
        ],
      });

      useContestStore.getState().selectVote({
        contestedName: "alice",
        choice: "abstain",
        endTime: 1700000000,
      });

      const votes = useContestStore.getState().selectedVotes;
      expect(votes).toHaveLength(1);
      expect(votes[0].choice).toBe("abstain");
    });

    it("deselects a vote by name", () => {
      useContestStore.setState({
        selectedVotes: [
          { contestedName: "alice", choice: "lock", endTime: null },
          { contestedName: "bob", choice: "abstain", endTime: null },
        ],
      });

      useContestStore.getState().deselectVote("alice");

      const votes = useContestStore.getState().selectedVotes;
      expect(votes).toHaveLength(1);
      expect(votes[0].contestedName).toBe("bob");
    });

    it("toggleVote selects when not selected", () => {
      const vote: SelectedVote = {
        contestedName: "alice",
        choice: "lock",
        endTime: null,
      };

      useContestStore.getState().toggleVote(vote);

      expect(useContestStore.getState().selectedVotes).toEqual([vote]);
    });

    it("toggleVote deselects when same choice", () => {
      useContestStore.setState({
        selectedVotes: [
          { contestedName: "alice", choice: "lock", endTime: null },
        ],
      });

      useContestStore.getState().toggleVote({
        contestedName: "alice",
        choice: "lock",
        endTime: null,
      });

      expect(useContestStore.getState().selectedVotes).toEqual([]);
    });

    it("toggleVote replaces when different choice", () => {
      useContestStore.setState({
        selectedVotes: [
          { contestedName: "alice", choice: "lock", endTime: null },
        ],
      });

      useContestStore.getState().toggleVote({
        contestedName: "alice",
        choice: "abstain",
        endTime: null,
      });

      const votes = useContestStore.getState().selectedVotes;
      expect(votes).toHaveLength(1);
      expect(votes[0].choice).toBe("abstain");
    });

    it("toggleVote handles towardsIdentity correctly", () => {
      const choice1 = { towardsIdentity: { identityId: "id1" } };
      const choice2 = { towardsIdentity: { identityId: "id2" } };

      useContestStore.setState({
        selectedVotes: [
          { contestedName: "alice", choice: choice1, endTime: null },
        ],
      });

      // Same identity — should deselect
      useContestStore.getState().toggleVote({
        contestedName: "alice",
        choice: choice1,
        endTime: null,
      });
      expect(useContestStore.getState().selectedVotes).toEqual([]);

      // Different identity — should replace
      useContestStore.setState({
        selectedVotes: [
          { contestedName: "alice", choice: choice1, endTime: null },
        ],
      });
      useContestStore.getState().toggleVote({
        contestedName: "alice",
        choice: choice2,
        endTime: null,
      });
      expect(useContestStore.getState().selectedVotes[0].choice).toEqual(
        choice2,
      );
    });

    it("clearSelectedVotes removes all selections", () => {
      useContestStore.setState({
        selectedVotes: [
          { contestedName: "alice", choice: "lock", endTime: null },
          { contestedName: "bob", choice: "abstain", endTime: null },
        ],
      });

      useContestStore.getState().clearSelectedVotes();

      expect(useContestStore.getState().selectedVotes).toEqual([]);
    });
  });

  describe("castVotes", () => {
    it("dispatches vote command with selected votes", async () => {
      useContestStore.setState({
        selectedVotes: [
          { contestedName: "alice", choice: "lock", endTime: null },
          { contestedName: "bob", choice: "abstain", endTime: null },
        ],
      });
      (commands.contestedVoteOnDpnsNames as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t1" },
      });

      await useContestStore.getState().castVotes(["voter1", "voter2"]);

      expect(commands.contestedVoteOnDpnsNames).toHaveBeenCalledWith({
        votes: [
          { contestedName: "alice", choice: "lock" },
          { contestedName: "bob", choice: "abstain" },
        ],
        voterIdentityIds: ["voter1", "voter2"],
      });
      expect(useContestStore.getState().votingInProgress).toBe(true);
    });

    it("does nothing with no selected votes", async () => {
      await useContestStore.getState().castVotes(["voter1"]);
      expect(commands.contestedVoteOnDpnsNames).not.toHaveBeenCalled();
    });

    it("does nothing with no voter IDs", async () => {
      useContestStore.setState({
        selectedVotes: [
          { contestedName: "alice", choice: "lock", endTime: null },
        ],
      });

      await useContestStore.getState().castVotes([]);
      expect(commands.contestedVoteOnDpnsNames).not.toHaveBeenCalled();
    });

    it("sets error on failure", async () => {
      useContestStore.setState({
        selectedVotes: [
          { contestedName: "alice", choice: "lock", endTime: null },
        ],
      });
      (commands.contestedVoteOnDpnsNames as Mock).mockResolvedValue({
        status: "error",
        error: "Voter not found",
      });

      await useContestStore.getState().castVotes(["voter1"]);

      expect(useContestStore.getState().error).toBe("Voter not found");
      expect(useContestStore.getState().votingInProgress).toBe(false);
    });

    it("sets error on exception", async () => {
      useContestStore.setState({
        selectedVotes: [
          { contestedName: "alice", choice: "lock", endTime: null },
        ],
      });
      (commands.contestedVoteOnDpnsNames as Mock).mockRejectedValue(
        new Error("Network"),
      );

      await useContestStore.getState().castVotes(["voter1"]);

      expect(useContestStore.getState().error).toBe("Network");
      expect(useContestStore.getState().votingInProgress).toBe(false);
    });
  });

  describe("scheduleVotes", () => {
    it("dispatches schedule command", async () => {
      const votes = [makeScheduledVote()];
      (commands.contestedScheduleDpnsVotes as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t1" },
      });

      await useContestStore.getState().scheduleVotes(votes);

      expect(commands.contestedScheduleDpnsVotes).toHaveBeenCalledWith({
        votes,
      });
      expect(useContestStore.getState().votingInProgress).toBe(true);
    });

    it("does nothing with empty array", async () => {
      await useContestStore.getState().scheduleVotes([]);
      expect(commands.contestedScheduleDpnsVotes).not.toHaveBeenCalled();
    });

    it("sets error on failure", async () => {
      (commands.contestedScheduleDpnsVotes as Mock).mockResolvedValue({
        status: "error",
        error: "Invalid vote",
      });

      await useContestStore.getState().scheduleVotes([makeScheduledVote()]);

      expect(useContestStore.getState().error).toBe("Invalid vote");
      expect(useContestStore.getState().votingInProgress).toBe(false);
    });
  });

  describe("castScheduledVote", () => {
    it("dispatches cast command and sets optimistic status", async () => {
      const vote = makeScheduledVote();
      useContestStore.setState({
        scheduledVotes: [{ vote, castingStatus: "notStarted" }],
      });
      (commands.contestedCastScheduledVote as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t1" },
      });

      await useContestStore.getState().castScheduledVote(vote);

      expect(commands.contestedCastScheduledVote).toHaveBeenCalledWith({
        vote,
      });
      expect(useContestStore.getState().scheduledVoteCastInProgress).toBe(
        true,
      );
      expect(
        useContestStore.getState().scheduledVotes[0].castingStatus,
      ).toBe("inProgress");
    });

    it("does nothing if a cast is already in progress", async () => {
      useContestStore.setState({ scheduledVoteCastInProgress: true });

      await useContestStore.getState().castScheduledVote(makeScheduledVote());

      expect(commands.contestedCastScheduledVote).not.toHaveBeenCalled();
    });

    it("reverts to failed on error", async () => {
      const vote = makeScheduledVote();
      useContestStore.setState({
        scheduledVotes: [{ vote, castingStatus: "notStarted" }],
      });
      (commands.contestedCastScheduledVote as Mock).mockResolvedValue({
        status: "error",
        error: "Platform unavailable",
      });

      await useContestStore.getState().castScheduledVote(vote);

      expect(
        useContestStore.getState().scheduledVotes[0].castingStatus,
      ).toBe("failed");
      expect(useContestStore.getState().error).toBe("Platform unavailable");
      expect(useContestStore.getState().scheduledVoteCastInProgress).toBe(
        false,
      );
    });

    it("reverts to failed on exception", async () => {
      const vote = makeScheduledVote();
      useContestStore.setState({
        scheduledVotes: [{ vote, castingStatus: "notStarted" }],
      });
      (commands.contestedCastScheduledVote as Mock).mockRejectedValue(
        new Error("Timeout"),
      );

      await useContestStore.getState().castScheduledVote(vote);

      expect(
        useContestStore.getState().scheduledVotes[0].castingStatus,
      ).toBe("failed");
      expect(useContestStore.getState().error).toBe("Timeout");
    });
  });

  describe("deleteScheduledVote", () => {
    it("removes the vote on success", async () => {
      const vote1 = makeScheduledVote();
      const vote2 = makeScheduledVote({
        voterId: "voter2",
        contestedName: "bob",
      });
      useContestStore.setState({
        scheduledVotes: [
          { vote: vote1, castingStatus: "notStarted" },
          { vote: vote2, castingStatus: "notStarted" },
        ],
      });
      (commands.contestedDeleteScheduledVote as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t1" },
      });

      await useContestStore.getState().deleteScheduledVote("voter1", "alice");

      expect(useContestStore.getState().scheduledVotes).toHaveLength(1);
      expect(
        useContestStore.getState().scheduledVotes[0].vote.contestedName,
      ).toBe("bob");
    });

    it("sets error on failure", async () => {
      (commands.contestedDeleteScheduledVote as Mock).mockResolvedValue({
        status: "error",
        error: "Not found",
      });

      await useContestStore
        .getState()
        .deleteScheduledVote("voter1", "alice");

      expect(useContestStore.getState().error).toBe("Not found");
    });
  });

  describe("clearAllScheduledVotes", () => {
    it("clears all scheduled votes on success", async () => {
      useContestStore.setState({
        scheduledVotes: [
          { vote: makeScheduledVote(), castingStatus: "notStarted" },
          {
            vote: makeScheduledVote({ contestedName: "bob" }),
            castingStatus: "completed",
          },
        ],
      });
      (commands.contestedClearAllScheduledVotes as Mock).mockResolvedValue({
        taskId: "t1",
      });

      await useContestStore.getState().clearAllScheduledVotes();

      expect(useContestStore.getState().scheduledVotes).toEqual([]);
    });
  });

  describe("clearExecutedScheduledVotes", () => {
    it("removes only completed votes", async () => {
      useContestStore.setState({
        scheduledVotes: [
          { vote: makeScheduledVote(), castingStatus: "notStarted" },
          {
            vote: makeScheduledVote({ contestedName: "bob" }),
            castingStatus: "completed",
          },
          {
            vote: makeScheduledVote({ contestedName: "charlie" }),
            castingStatus: "failed",
          },
        ],
      });
      (
        commands.contestedClearExecutedScheduledVotes as Mock
      ).mockResolvedValue({ taskId: "t1" });

      await useContestStore.getState().clearExecutedScheduledVotes();

      const remaining = useContestStore.getState().scheduledVotes;
      expect(remaining).toHaveLength(2);
      expect(remaining.map((sv) => sv.vote.contestedName)).toEqual([
        "alice",
        "charlie",
      ]);
    });
  });

  describe("refreshDpnsNames", () => {
    it("dispatches refresh command", async () => {
      (commands.identityRefreshDpnsNames as Mock).mockResolvedValue({
        taskId: "t1",
      });

      await useContestStore.getState().refreshDpnsNames();

      expect(commands.identityRefreshDpnsNames).toHaveBeenCalled();
    });

    it("sets error on exception", async () => {
      (commands.identityRefreshDpnsNames as Mock).mockRejectedValue(
        new Error("Failed"),
      );

      await useContestStore.getState().refreshDpnsNames();

      expect(useContestStore.getState().error).toBe("Failed");
    });
  });

  describe("setFilterTerm", () => {
    it("sets active filter term", () => {
      useContestStore.getState().setFilterTerm("active", "alice");
      expect(useContestStore.getState().activeFilterTerm).toBe("alice");
    });

    it("sets past filter term", () => {
      useContestStore.getState().setFilterTerm("past", "bob");
      expect(useContestStore.getState().pastFilterTerm).toBe("bob");
    });

    it("sets owned filter term", () => {
      useContestStore.getState().setFilterTerm("owned", "charlie");
      expect(useContestStore.getState().ownedFilterTerm).toBe("charlie");
    });
  });

  describe("setSortColumn", () => {
    it("sorts by name ascending when switching from different column", () => {
      useContestStore.setState({
        contestedNames: [
          makeContest({ normalizedContestedName: "charlie" }),
          makeContest({ normalizedContestedName: "alice" }),
          makeContest({ normalizedContestedName: "bob" }),
        ],
        sortColumn: "lockedVotes",
      });

      useContestStore.getState().setSortColumn("name");

      const names = useContestStore
        .getState()
        .contestedNames.map((c) => c.normalizedContestedName);
      expect(names).toEqual(["alice", "bob", "charlie"]);
      expect(useContestStore.getState().sortOrder).toBe("ascending");
    });

    it("toggles to descending on same column click", () => {
      useContestStore.setState({
        contestedNames: [
          makeContest({ normalizedContestedName: "alice" }),
          makeContest({ normalizedContestedName: "charlie" }),
          makeContest({ normalizedContestedName: "bob" }),
        ],
        sortColumn: "name",
        sortOrder: "ascending",
      });

      useContestStore.getState().setSortColumn("name");

      expect(useContestStore.getState().sortOrder).toBe("descending");
      const names = useContestStore
        .getState()
        .contestedNames.map((c) => c.normalizedContestedName);
      expect(names).toEqual(["charlie", "bob", "alice"]);
    });

    it("sorts by lockedVotes", () => {
      useContestStore.setState({
        contestedNames: [
          makeContest({ normalizedContestedName: "a", lockedVotes: 10 }),
          makeContest({ normalizedContestedName: "b", lockedVotes: 3 }),
          makeContest({ normalizedContestedName: "c", lockedVotes: 7 }),
        ],
      });

      useContestStore.getState().setSortColumn("lockedVotes");

      const votes = useContestStore
        .getState()
        .contestedNames.map((c) => c.lockedVotes);
      expect(votes).toEqual([3, 7, 10]);
    });

    it("sorts by endingTime", () => {
      useContestStore.setState({
        contestedNames: [
          makeContest({ normalizedContestedName: "a", endTime: 300 }),
          makeContest({ normalizedContestedName: "b", endTime: 100 }),
          makeContest({ normalizedContestedName: "c", endTime: 200 }),
        ],
      });

      useContestStore.getState().setSortColumn("endingTime");

      const times = useContestStore
        .getState()
        .contestedNames.map((c) => c.endTime);
      expect(times).toEqual([100, 200, 300]);
    });

    it("resets to ascending when switching columns", () => {
      useContestStore.setState({
        sortColumn: "name",
        sortOrder: "descending",
        contestedNames: [makeContest()],
      });

      useContestStore.getState().setSortColumn("lockedVotes");

      expect(useContestStore.getState().sortColumn).toBe("lockedVotes");
      expect(useContestStore.getState().sortOrder).toBe("ascending");
    });
  });

  describe("subscribeToUpdates", () => {
    it("subscribes to all three event types", async () => {
      const unlistenResult = vi.fn();
      const unlistenError = vi.fn();
      const unlistenScheduled = vi.fn();
      (events.taskResultEvent.listen as Mock).mockResolvedValue(
        unlistenResult,
      );
      (events.taskErrorEvent.listen as Mock).mockResolvedValue(unlistenError);
      (events.scheduledVoteExecutedEvent.listen as Mock).mockResolvedValue(
        unlistenScheduled,
      );

      const unlisten = await useContestStore
        .getState()
        .subscribeToUpdates();

      expect(events.taskResultEvent.listen).toHaveBeenCalled();
      expect(events.taskErrorEvent.listen).toHaveBeenCalled();
      expect(events.scheduledVoteExecutedEvent.listen).toHaveBeenCalled();
      expect(typeof unlisten).toBe("function");
    });

    it("calls all unsubscribe functions when unsubscribed", async () => {
      const unlistenResult = vi.fn();
      const unlistenError = vi.fn();
      const unlistenScheduled = vi.fn();
      (events.taskResultEvent.listen as Mock).mockResolvedValue(
        unlistenResult,
      );
      (events.taskErrorEvent.listen as Mock).mockResolvedValue(unlistenError);
      (events.scheduledVoteExecutedEvent.listen as Mock).mockResolvedValue(
        unlistenScheduled,
      );

      const unlisten = await useContestStore
        .getState()
        .subscribeToUpdates();
      unlisten();

      expect(unlistenResult).toHaveBeenCalled();
      expect(unlistenError).toHaveBeenCalled();
      expect(unlistenScheduled).toHaveBeenCalled();
    });

    it("clears refreshing/voting state on Contest result", async () => {
      useContestStore.setState({ refreshing: true, votingInProgress: true });

      // Mock scheduled votes reload
      (commands.contestedGetScheduledVotes as Mock).mockResolvedValue({
        status: "ok",
        data: [],
      });

      let resultCallback: (event: { payload: TaskResultEvent }) => void;
      (events.taskResultEvent.listen as Mock).mockImplementation(
        (cb: typeof resultCallback) => {
          resultCallback = cb;
          return Promise.resolve(() => {});
        },
      );
      (events.taskErrorEvent.listen as Mock).mockResolvedValue(() => {});
      (events.scheduledVoteExecutedEvent.listen as Mock).mockResolvedValue(
        () => {},
      );

      await useContestStore.getState().subscribeToUpdates();
      resultCallback!({
        payload: {
          taskId: "t1",
          resultType: "Contest",
          payload: null,
        },
      });

      expect(useContestStore.getState().refreshing).toBe(false);
      expect(useContestStore.getState().votingInProgress).toBe(false);
    });

    it("reloads local names on Identity result", async () => {
      (commands.identityLocalDpnsNames as Mock).mockResolvedValue({
        status: "ok",
        data: [makeDpnsName()],
      });

      let resultCallback: (event: { payload: TaskResultEvent }) => void;
      (events.taskResultEvent.listen as Mock).mockImplementation(
        (cb: typeof resultCallback) => {
          resultCallback = cb;
          return Promise.resolve(() => {});
        },
      );
      (events.taskErrorEvent.listen as Mock).mockResolvedValue(() => {});
      (events.scheduledVoteExecutedEvent.listen as Mock).mockResolvedValue(
        () => {},
      );

      await useContestStore.getState().subscribeToUpdates();
      resultCallback!({
        payload: {
          taskId: "t1",
          resultType: "Identity",
          payload: null,
        },
      });

      await vi.waitFor(() => {
        expect(commands.identityLocalDpnsNames).toHaveBeenCalled();
      });
    });

    it("ignores non-Contest/non-Identity result types", async () => {
      let resultCallback: (event: { payload: TaskResultEvent }) => void;
      (events.taskResultEvent.listen as Mock).mockImplementation(
        (cb: typeof resultCallback) => {
          resultCallback = cb;
          return Promise.resolve(() => {});
        },
      );
      (events.taskErrorEvent.listen as Mock).mockResolvedValue(() => {});
      (events.scheduledVoteExecutedEvent.listen as Mock).mockResolvedValue(
        () => {},
      );

      await useContestStore.getState().subscribeToUpdates();
      resultCallback!({
        payload: {
          taskId: "t1",
          resultType: "Wallet",
          payload: null,
        },
      });

      expect(commands.contestedGetScheduledVotes).not.toHaveBeenCalled();
      expect(commands.identityLocalDpnsNames).not.toHaveBeenCalled();
    });

    it("clears state and sets error on task error event", async () => {
      useContestStore.setState({
        refreshing: true,
        votingInProgress: true,
        scheduledVoteCastInProgress: true,
      });

      let errorCallback: (event: {
        payload: { taskId: string; message: string };
      }) => void;
      (events.taskResultEvent.listen as Mock).mockResolvedValue(() => {});
      (events.taskErrorEvent.listen as Mock).mockImplementation(
        (cb: typeof errorCallback) => {
          errorCallback = cb;
          return Promise.resolve(() => {});
        },
      );
      (events.scheduledVoteExecutedEvent.listen as Mock).mockResolvedValue(
        () => {},
      );

      await useContestStore.getState().subscribeToUpdates();
      errorCallback!({
        payload: { taskId: "t1", message: "Platform error" },
      });

      const state = useContestStore.getState();
      expect(state.refreshing).toBe(false);
      expect(state.votingInProgress).toBe(false);
      expect(state.scheduledVoteCastInProgress).toBe(false);
      expect(state.error).toBe("Platform error");
    });

    it("updates scheduled vote status on ScheduledVoteExecutedEvent (success)", async () => {
      const vote = makeScheduledVote();
      useContestStore.setState({
        scheduledVotes: [{ vote, castingStatus: "inProgress" }],
        scheduledVoteCastInProgress: true,
      });

      let scheduledCallback: (event: {
        payload: ScheduledVoteExecutedEvent;
      }) => void;
      (events.taskResultEvent.listen as Mock).mockResolvedValue(() => {});
      (events.taskErrorEvent.listen as Mock).mockResolvedValue(() => {});
      (events.scheduledVoteExecutedEvent.listen as Mock).mockImplementation(
        (cb: typeof scheduledCallback) => {
          scheduledCallback = cb;
          return Promise.resolve(() => {});
        },
      );

      await useContestStore.getState().subscribeToUpdates();
      scheduledCallback!({
        payload: {
          contestedName: "alice",
          voterId: "voter1",
          success: true,
          error: null,
        },
      });

      const sv = useContestStore.getState().scheduledVotes[0];
      expect(sv.castingStatus).toBe("completed");
      expect(sv.vote.executedSuccessfully).toBe(true);
      expect(useContestStore.getState().scheduledVoteCastInProgress).toBe(
        false,
      );
    });

    it("updates scheduled vote status on ScheduledVoteExecutedEvent (failure)", async () => {
      const vote = makeScheduledVote();
      useContestStore.setState({
        scheduledVotes: [{ vote, castingStatus: "inProgress" }],
        scheduledVoteCastInProgress: true,
      });

      let scheduledCallback: (event: {
        payload: ScheduledVoteExecutedEvent;
      }) => void;
      (events.taskResultEvent.listen as Mock).mockResolvedValue(() => {});
      (events.taskErrorEvent.listen as Mock).mockResolvedValue(() => {});
      (events.scheduledVoteExecutedEvent.listen as Mock).mockImplementation(
        (cb: typeof scheduledCallback) => {
          scheduledCallback = cb;
          return Promise.resolve(() => {});
        },
      );

      await useContestStore.getState().subscribeToUpdates();
      scheduledCallback!({
        payload: {
          contestedName: "alice",
          voterId: "voter1",
          success: false,
          error: "Vote rejected",
        },
      });

      const sv = useContestStore.getState().scheduledVotes[0];
      expect(sv.castingStatus).toBe("failed");
      expect(sv.vote.executedSuccessfully).toBe(false);
      expect(useContestStore.getState().error).toBe("Vote rejected");
    });
  });

  describe("clearError", () => {
    it("clears the error state", () => {
      useContestStore.setState({ error: "some error" });

      useContestStore.getState().clearError();

      expect(useContestStore.getState().error).toBeNull();
    });
  });
});

// ─── Helper function tests ──────────────────────────────────────────

describe("normalizeDpnsFilter", () => {
  it("converts o/O to 0", () => {
    expect(normalizeDpnsFilter("hero")).toBe("her0");
    expect(normalizeDpnsFilter("OCEAN")).toBe("0cean");
  });

  it("converts l to 1", () => {
    expect(normalizeDpnsFilter("label")).toBe("1abe1");
  });

  it("converts mixed lookalikes", () => {
    // o/O → 0, l → 1, L stays L then lowercased to l
    expect(normalizeDpnsFilter("oOlLl")).toBe("001l1");
  });

  it("lowercases the result", () => {
    // ALICE → A1ICE (l→1) then lowercase → a1ice? No, uppercase L not converted
    // ALICE → toLower first: alice, then: a1ice? No, regex runs first.
    // Actually: ALICE → replace o/O: ALICE → replace l: ALICE → toLowerCase: alice
    // Capital L is not matched by /l/g
    expect(normalizeDpnsFilter("ALICE")).toBe("alice");
  });

  it("leaves non-lookalike chars unchanged", () => {
    expect(normalizeDpnsFilter("dash123")).toBe("dash123");
  });

  it("handles empty string", () => {
    expect(normalizeDpnsFilter("")).toBe("");
  });
});

describe("matchesDpnsFilter", () => {
  it("returns true with empty filter", () => {
    expect(matchesDpnsFilter("anything", "")).toBe(true);
  });

  it("matches exact name", () => {
    expect(matchesDpnsFilter("alice", "alice")).toBe(true);
  });

  it("matches partial name", () => {
    expect(matchesDpnsFilter("alice", "ali")).toBe(true);
  });

  it("matches with lookalike normalization", () => {
    // 'o' in filter matches '0' in name
    expect(matchesDpnsFilter("a0ice", "aoice")).toBe(true);
    // 'l' in filter matches '1' in name
    expect(matchesDpnsFilter("a1ice", "alice")).toBe(true);
  });

  it("returns false for non-matching filter", () => {
    expect(matchesDpnsFilter("alice", "bob")).toBe(false);
  });
});
