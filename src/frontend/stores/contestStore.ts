import { create } from "zustand";
import { commands, events } from "../bindings";
import type {
  DpnsNameEntryDto,
  ScheduledVoteDto,
  VoteChoiceDto,
  VoteEntry,
  TaskResultEvent,
  ScheduledVoteExecutedEvent,
  ScheduledVoteInProgressEvent,
} from "../bindings";
import { TaskTimeoutManager, TIMEOUT_ERROR_MESSAGE } from "../lib/taskTimeout";

// ─── Local types (contest data not yet in auto-generated bindings) ───

/** State of a DPNS name contest. */
export type ContestState =
  | "unknown"
  | "joinable"
  | "ongoing"
  | { wonBy: string }
  | "locked";

/** A contestant vying for a contested DPNS name. */
export interface Contestant {
  id: string;
  name: string;
  info: string;
  votes: number;
  createdAt: number | null;
  createdAtBlockHeight: number | null;
  createdAtCoreBlockHeight: number | null;
  documentId: string;
}

/** A contested DPNS name with its voting state. */
export interface ContestedName {
  normalizedContestedName: string;
  contestants: Contestant[] | null;
  lockedVotes: number | null;
  abstainVotes: number | null;
  awardedTo: string | null;
  endTime: number | null;
  state: ContestState;
  lastUpdated: number | null;
}

/** A vote selected by the user in the UI (not yet submitted). */
export interface SelectedVote {
  contestedName: string;
  choice: VoteChoiceDto;
  endTime: number | null;
}

/** Execution status for a scheduled vote in the UI. */
export type ScheduledVoteCastingStatus =
  | "notStarted"
  | "inProgress"
  | "completed"
  | "failed";

/** A scheduled vote paired with its UI casting status. */
export interface ScheduledVoteWithStatus {
  vote: ScheduledVoteDto;
  castingStatus: ScheduledVoteCastingStatus;
}

/** Sort columns for the contests table. */
export type ContestSortColumn =
  | "name"
  | "lockedVotes"
  | "abstainVotes"
  | "endingTime"
  | "lastUpdated"
  | "awardedTo";

export type SortOrder = "ascending" | "descending";

// ─── Store state ────────────────────────────────────────────────────

interface ContestState_ {
  /** All contested names (active + past). */
  contestedNames: ContestedName[];

  /** Local DPNS names owned by loaded identities. */
  localDpnsNames: DpnsNameEntryDto[];

  /** Scheduled votes with their casting status. */
  scheduledVotes: ScheduledVoteWithStatus[];

  /** Currently selected votes for bulk voting. */
  selectedVotes: SelectedVote[];

  /** Loading (initial fetch). */
  loading: boolean;

  /** Whether a refresh query is in progress. */
  refreshing: boolean;

  /** Whether a bulk vote cast/schedule operation is in progress. */
  votingInProgress: boolean;

  /** Error message. */
  error: string | null;

  /** Active tab filter term. */
  activeFilterTerm: string;

  /** Past tab filter term. */
  pastFilterTerm: string;

  /** Owned tab filter term. */
  ownedFilterTerm: string;

  /** Current sort column. */
  sortColumn: ContestSortColumn;

  /** Current sort direction. */
  sortOrder: SortOrder;

}

// ─── Store actions ──────────────────────────────────────────────────

interface ContestActions {
  /** Dispatch a query to refresh contested names from Platform. */
  loadContests: () => Promise<void>;

  /** Load contested names from local database (after Platform query completes). */
  loadContestedNames: () => Promise<void>;

  /** Load local DPNS names from backend. */
  loadLocalNames: () => Promise<void>;

  /** Load scheduled votes from local database. */
  loadScheduledVotes: () => Promise<void>;

  /** Select a vote for a contested name (add or replace). */
  selectVote: (vote: SelectedVote) => void;

  /** Deselect a vote for a contested name. */
  deselectVote: (contestedName: string) => void;

  /** Toggle a vote: select if not selected or different choice, deselect if same. */
  toggleVote: (vote: SelectedVote) => void;

  /** Clear all selected votes. */
  clearSelectedVotes: () => void;

  /** Cast selected votes immediately for given voter identities. */
  castVotes: (voterIdentityIds: string[]) => Promise<void>;

  /** Schedule selected votes for future execution. */
  scheduleVotes: (votes: ScheduledVoteDto[]) => Promise<void>;

  /** Cast a single scheduled vote immediately. */
  castScheduledVote: (vote: ScheduledVoteDto) => Promise<void>;

  /** Delete a single scheduled vote. */
  deleteScheduledVote: (voterId: string, contestedName: string) => Promise<void>;

  /** Clear all scheduled votes. */
  clearAllScheduledVotes: () => Promise<void>;

  /** Clear only executed (completed) scheduled votes. */
  clearExecutedScheduledVotes: () => Promise<void>;

  /** Refresh owned DPNS names from Platform. */
  refreshDpnsNames: () => Promise<void>;

  /** Set the filter term for a specific tab. */
  setFilterTerm: (tab: "active" | "past" | "owned", term: string) => void;

  /** Set sort column (toggles direction if same column). */
  setSortColumn: (column: ContestSortColumn) => void;

  /** Subscribe to contest-related Tauri events. Returns unsubscribe fn. */
  subscribeToUpdates: () => Promise<() => void>;

  /** Reset all state (used on network switch). */
  resetState: () => void;

  /** Clear error state. */
  clearError: () => void;
}

export type ContestStore = ContestState_ & ContestActions;

// ─── Helpers ────────────────────────────────────────────────────────

/** Check if two VoteChoiceDto values are equal. */
function voteChoicesEqual(a: VoteChoiceDto, b: VoteChoiceDto): boolean {
  if (a === b) return true;
  if (typeof a === "object" && typeof b === "object") {
    if ("towardsIdentity" in a && "towardsIdentity" in b) {
      return a.towardsIdentity.identityId === b.towardsIdentity.identityId;
    }
  }
  return false;
}

/** Sort contested names by a column. Returns a new sorted array. */
function sortContestedNames(
  names: ContestedName[],
  column: ContestSortColumn,
  order: SortOrder,
): ContestedName[] {
  const sorted = [...names];
  const dir = order === "ascending" ? 1 : -1;

  sorted.sort((a, b) => {
    switch (column) {
      case "name":
        return (
          dir *
          a.normalizedContestedName.localeCompare(b.normalizedContestedName)
        );
      case "lockedVotes":
        return dir * ((a.lockedVotes ?? 0) - (b.lockedVotes ?? 0));
      case "abstainVotes":
        return dir * ((a.abstainVotes ?? 0) - (b.abstainVotes ?? 0));
      case "endingTime":
        return dir * ((a.endTime ?? 0) - (b.endTime ?? 0));
      case "lastUpdated":
        return dir * ((a.lastUpdated ?? 0) - (b.lastUpdated ?? 0));
      case "awardedTo":
        return dir * (a.awardedTo ?? "").localeCompare(b.awardedTo ?? "");
      default:
        return 0;
    }
  });

  return sorted;
}

/**
 * Apply the "smart filter" for DPNS names.
 * Converts ambiguous lookalike characters: 'o'/'O' → '0', 'l' → '1'.
 */
export function normalizeDpnsFilter(term: string): string {
  return term.replace(/[oO]/g, "0").replace(/l/g, "1").toLowerCase();
}

/** Check if a contested name matches the smart filter term. */
export function matchesDpnsFilter(
  name: string,
  filterTerm: string,
): boolean {
  if (!filterTerm) return true;
  const normalizedName = normalizeDpnsFilter(name);
  const normalizedFilter = normalizeDpnsFilter(filterTerm);
  return normalizedName.includes(normalizedFilter);
}

/** Convert a ContestStateDto from the backend to the frontend ContestState type. */
function convertContestState(dto: { wonBy?: { identityId: string } } | string): ContestState {
  if (typeof dto === "string") {
    const lower = dto.toLowerCase();
    if (lower === "unknown") return "unknown";
    if (lower === "joinable") return "joinable";
    if (lower === "ongoing") return "ongoing";
    if (lower === "locked") return "locked";
    return "unknown";
  }
  if (typeof dto === "object" && dto !== null) {
    if ("wonBy" in dto && dto.wonBy) {
      return { wonBy: dto.wonBy.identityId };
    }
  }
  return "unknown";
}

// ─── Task timeout manager ────────────────────────────────────────────

const timeouts = new TaskTimeoutManager();

// ─── Store ──────────────────────────────────────────────────────────

export const useContestStore = create<ContestStore>((set, get) => ({
  // Initial state
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

  loadContests: async () => {
    set({ refreshing: true, error: null });
    try {
      // Dispatch async query — result arrives via TaskResultEvent
      await commands.contestedQueryDpnsContests();
      timeouts.start("contest", () => {
        set({ refreshing: false, votingInProgress: false, error: TIMEOUT_ERROR_MESSAGE });
      });
      // refreshing will be cleared when the "Contest" result event arrives
    } catch (e) {
      set({
        error: e instanceof Error ? e.message : String(e),
        refreshing: false,
      });
    }
  },

  loadContestedNames: async () => {
    try {
      const result = await commands.contestedGetAllNames();
      if (result.status === "ok") {
        const names: ContestedName[] = result.data.map((dto) => ({
          normalizedContestedName: dto.normalizedContestedName,
          contestants: dto.contestants
            ? dto.contestants.map((c) => ({
                id: c.id,
                name: c.name,
                info: c.info,
                votes: c.votes,
                createdAt: c.createdAt ?? null,
                createdAtBlockHeight: c.createdAtBlockHeight ?? null,
                createdAtCoreBlockHeight: c.createdAtCoreBlockHeight ?? null,
                documentId: c.documentId,
              }))
            : null,
          lockedVotes: dto.lockedVotes ?? null,
          abstainVotes: dto.abstainVotes ?? null,
          awardedTo: dto.awardedTo ?? null,
          endTime: dto.endTime ?? null,
          state: convertContestState(dto.state),
          lastUpdated: dto.lastUpdated ?? null,
        }));
        const { sortColumn, sortOrder } = get();
        set({ contestedNames: sortContestedNames(names, sortColumn, sortOrder) });
      } else {
        set({ error: result.error });
      }
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
    }
  },

  loadLocalNames: async () => {
    try {
      const result = await commands.identityLocalDpnsNames();
      if (result.status === "ok") {
        set({ localDpnsNames: result.data });
      } else {
        set({ error: result.error });
      }
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
    }
  },

  loadScheduledVotes: async () => {
    try {
      const result = await commands.contestedGetScheduledVotes();
      if (result.status === "ok") {
        set({
          scheduledVotes: result.data.map((vote) => ({
            vote,
            castingStatus: vote.executedSuccessfully
              ? "completed"
              : "notStarted",
          })),
        });
      } else {
        set({ error: result.error });
      }
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
    }
  },

  selectVote: (vote) => {
    set((state) => {
      const existing = state.selectedVotes.findIndex(
        (v) => v.contestedName === vote.contestedName,
      );
      if (existing >= 0) {
        // Replace existing vote for this name
        const updated = [...state.selectedVotes];
        updated[existing] = vote;
        return { selectedVotes: updated };
      }
      return { selectedVotes: [...state.selectedVotes, vote] };
    });
  },

  deselectVote: (contestedName) => {
    set((state) => ({
      selectedVotes: state.selectedVotes.filter(
        (v) => v.contestedName !== contestedName,
      ),
    }));
  },

  toggleVote: (vote) => {
    const { selectedVotes } = get();
    const existing = selectedVotes.find(
      (v) => v.contestedName === vote.contestedName,
    );
    if (existing && voteChoicesEqual(existing.choice, vote.choice)) {
      // Same choice — deselect
      get().deselectVote(vote.contestedName);
    } else {
      // Different or new — select
      get().selectVote(vote);
    }
  },

  clearSelectedVotes: () => {
    set({ selectedVotes: [] });
  },

  castVotes: async (voterIdentityIds) => {
    const { selectedVotes } = get();
    if (selectedVotes.length === 0 || voterIdentityIds.length === 0) return;

    set({ votingInProgress: true, error: null });
    try {
      const votes: VoteEntry[] = selectedVotes.map((sv) => ({
        contestedName: sv.contestedName,
        choice: sv.choice,
      }));
      const result = await commands.contestedVoteOnDpnsNames({
        votes,
        voterIdentityIds,
      });
      if (result.status === "error") {
        set({ error: result.error, votingInProgress: false });
      } else {
        timeouts.start("castVotes", () => {
          set({ refreshing: false, votingInProgress: false, error: TIMEOUT_ERROR_MESSAGE });
        });
      }
      // votingInProgress will be cleared when the "Contest" result event arrives
    } catch (e) {
      set({
        error: e instanceof Error ? e.message : String(e),
        votingInProgress: false,
      });
    }
  },

  scheduleVotes: async (votes) => {
    if (votes.length === 0) return;

    set({ votingInProgress: true, error: null });
    try {
      const result = await commands.contestedScheduleDpnsVotes({ votes });
      if (result.status === "error") {
        set({ error: result.error, votingInProgress: false });
      } else {
        timeouts.start("scheduleVotes", () => {
          set({ refreshing: false, votingInProgress: false, error: TIMEOUT_ERROR_MESSAGE });
        });
      }
      // votingInProgress cleared by event
    } catch (e) {
      set({
        error: e instanceof Error ? e.message : String(e),
        votingInProgress: false,
      });
    }
  },

  castScheduledVote: async (vote) => {
    // Per-vote guard: skip if this specific vote is already in-flight
    const currentVote = get().scheduledVotes.find(
      (sv) =>
        sv.vote.voterId === vote.voterId &&
        sv.vote.contestedName === vote.contestedName,
    );
    if (currentVote?.castingStatus === "inProgress") return;

    set({ error: null });

    // Optimistically mark the vote as in-progress
    set((state) => ({
      scheduledVotes: state.scheduledVotes.map((sv) =>
        sv.vote.voterId === vote.voterId &&
        sv.vote.contestedName === vote.contestedName
          ? { ...sv, castingStatus: "inProgress" as const }
          : sv,
      ),
    }));

    try {
      const result = await commands.contestedCastScheduledVote({ vote });
      if (result.status === "error") {
        // Revert optimistic update
        set((state) => ({
          scheduledVotes: state.scheduledVotes.map((sv) =>
            sv.vote.voterId === vote.voterId &&
            sv.vote.contestedName === vote.contestedName
              ? { ...sv, castingStatus: "failed" as const }
              : sv,
          ),
          error: result.error,
        }));
      } else {
        timeouts.start("castScheduled", () => {
          set({ refreshing: false, votingInProgress: false, error: TIMEOUT_ERROR_MESSAGE });
        });
      }
    } catch (e) {
      set((state) => ({
        scheduledVotes: state.scheduledVotes.map((sv) =>
          sv.vote.voterId === vote.voterId &&
          sv.vote.contestedName === vote.contestedName
            ? { ...sv, castingStatus: "failed" as const }
            : sv,
        ),
        error: e instanceof Error ? e.message : String(e),
      }));
    }
  },

  deleteScheduledVote: async (voterId, contestedName) => {
    try {
      const result = await commands.contestedDeleteScheduledVote({
        voterId,
        contestedName,
      });
      if (result.status === "ok") {
        set((state) => ({
          scheduledVotes: state.scheduledVotes.filter(
            (sv) =>
              !(
                sv.vote.voterId === voterId &&
                sv.vote.contestedName === contestedName
              ),
          ),
        }));
      } else {
        set({ error: result.error });
      }
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
    }
  },

  clearAllScheduledVotes: async () => {
    try {
      await commands.contestedClearAllScheduledVotes();
      set({ scheduledVotes: [] });
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
    }
  },

  clearExecutedScheduledVotes: async () => {
    try {
      await commands.contestedClearExecutedScheduledVotes();
      set((state) => ({
        scheduledVotes: state.scheduledVotes.filter(
          (sv) => sv.castingStatus !== "completed",
        ),
      }));
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
    }
  },

  refreshDpnsNames: async () => {
    try {
      await commands.identityRefreshDpnsNames();
      // Result arrives via TaskResultEvent — reload local names after
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
    }
  },

  setFilterTerm: (tab, term) => {
    switch (tab) {
      case "active":
        set({ activeFilterTerm: term });
        break;
      case "past":
        set({ pastFilterTerm: term });
        break;
      case "owned":
        set({ ownedFilterTerm: term });
        break;
    }
  },

  setSortColumn: (column) => {
    const { sortColumn, sortOrder } = get();
    if (column === sortColumn) {
      const newOrder =
        sortOrder === "ascending" ? "descending" : "ascending";
      set((state) => ({
        sortOrder: newOrder,
        contestedNames: sortContestedNames(
          state.contestedNames,
          column,
          newOrder,
        ),
      }));
    } else {
      set((state) => ({
        sortColumn: column,
        sortOrder: "ascending",
        contestedNames: sortContestedNames(
          state.contestedNames,
          column,
          "ascending",
        ),
      }));
    }
  },

  subscribeToUpdates: async () => {
    const unlistenResult = await events.taskResultEvent.listen(
      (event: { payload: TaskResultEvent }) => {
        const { result } = event.payload;

        if (result.type === "contestCompleted") {
          timeouts.clearAll();

          const state = get();

          // Contest query completed — clear refreshing/voting state
          set({
            refreshing: false,
            votingInProgress: false,
          });

          // Load contested names from DB (they were just saved by the backend)
          state.loadContestedNames();

          // Also reload scheduled votes (may have been updated)
          state.loadScheduledVotes();
        }

        // Also handle Identity results that may contain DPNS name updates
        if (result.type === "identityCompleted") {
          get().loadLocalNames();
        }
      },
    );

    const unlistenError = await events.taskErrorEvent.listen(
      (event: { payload: { taskId: string; domain: string; message: string } }) => {
        if (event.payload.domain !== "contest") return;

        timeouts.clearAll();

        set({
          refreshing: false,
          votingInProgress: false,
          error: event.payload.message,
        });
      },
    );

    const unlistenInProgress = await events.scheduledVoteInProgressEvent.listen(
      (event: { payload: ScheduledVoteInProgressEvent }) => {
        const { contestedName, voterId } = event.payload;

        set((state) => ({
          scheduledVotes: state.scheduledVotes.map((sv) =>
            sv.vote.voterId === voterId &&
            sv.vote.contestedName === contestedName
              ? { ...sv, castingStatus: "inProgress" as const }
              : sv,
          ),
        }));
      },
    );

    const unlistenScheduled = await events.scheduledVoteExecutedEvent.listen(
      (event: { payload: ScheduledVoteExecutedEvent }) => {
        timeouts.clear("castScheduled");

        const { contestedName, voterId, success, error: errMsg } = event.payload;

        set((state) => ({
          scheduledVotes: state.scheduledVotes.map((sv) =>
            sv.vote.voterId === voterId &&
            sv.vote.contestedName === contestedName
              ? {
                  ...sv,
                  castingStatus: success
                    ? ("completed" as const)
                    : ("failed" as const),
                  vote: success
                    ? { ...sv.vote, executedSuccessfully: true }
                    : sv.vote,
                }
              : sv,
          ),
          error: errMsg ?? state.error,
        }));
      },
    );

    return () => {
      unlistenResult();
      unlistenError();
      unlistenInProgress();
      unlistenScheduled();
    };
  },

  resetState: () => {
    timeouts.clearAll();
    set({
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
    });
  },

  clearError: () => {
    set({ error: null });
  },
}));
