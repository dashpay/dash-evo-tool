/**
 * Test fixture factories for DPNS contest and voting DTOs.
 *
 * Usage:
 *   import { createMockContestedName, createMockScheduledVote } from "@/test/fixtures";
 *
 *   const contest = createMockContestedName({ normalizedContestedName: "bob" });
 *   const vote = createMockScheduledVote({ contestedName: "bob" });
 */

import type {
  DpnsNameEntryDto,
  ScheduledVoteDto,
  VoteChoiceDto,
  VoteEntry,
} from "@/bindings";
import type {
  ContestedName,
  Contestant,
  ContestState,
  SelectedVote,
} from "@/stores/contestStore";

// ─── Atomic factories ──────────────────────────────────────────────

export function createMockContestant(
  overrides?: Partial<Contestant>,
): Contestant {
  return {
    id: "4EfA9Jrvv3nnCFdSf7fad59851kYrUWF21pYPKhq6Ycis",
    name: "alice",
    info: "Identity: 4EfA9J...Ycis",
    votes: 5,
    createdAt: 1707500000,
    createdAtBlockHeight: 1_920_000,
    createdAtCoreBlockHeight: 2_100_000,
    documentId: "BZkLq39rhYNtwpmmFhHjuNXMZq5SnMf39DpE9miFDxpk",
    ...overrides,
  };
}

export function createMockDpnsNameEntry(
  overrides?: Partial<DpnsNameEntryDto>,
): DpnsNameEntryDto {
  return {
    identityId: "4EfA9Jrvv3nnCFdSf7fad59851kYrUWF21pYPKhq6Ycis",
    name: "alice.dash",
    acquiredAt: 1707500000,
    ...overrides,
  };
}

export function createMockVoteChoice(
  type?: "towards" | "abstain" | "lock",
  identityId?: string,
): VoteChoiceDto {
  switch (type) {
    case "abstain":
      return "abstain";
    case "lock":
      return "lock";
    case "towards":
    default:
      return {
        towardsIdentity: {
          identityId:
            identityId ?? "4EfA9Jrvv3nnCFdSf7fad59851kYrUWF21pYPKhq6Ycis",
        },
      };
  }
}

export function createMockVoteEntry(
  overrides?: Partial<VoteEntry>,
): VoteEntry {
  return {
    contestedName: "alice",
    choice: createMockVoteChoice("towards"),
    ...overrides,
  };
}

// ─── Scheduled vote factory ────────────────────────────────────────

export function createMockScheduledVote(
  overrides?: Partial<ScheduledVoteDto>,
): ScheduledVoteDto {
  return {
    contestedName: "alice",
    voterId: "7BfX2Kqv8npRFdTe952mYsTWF31qZQNhq6Kdm5Zabc12",
    choice: createMockVoteChoice("towards"),
    unixTimestamp: Math.floor(Date.now() / 1000) + 3600, // 1 hour from now
    executedSuccessfully: false,
    ...overrides,
  };
}

// ─── Contested name factory ────────────────────────────────────────

export function createMockContestedName(
  overrides?: Partial<ContestedName>,
): ContestedName {
  return {
    normalizedContestedName: "alice",
    contestants: [
      createMockContestant(),
      createMockContestant({
        id: "7BfX2Kqv8npRFdTe952mYsTWF31qZQNhq6Kdm5Zabc12",
        name: "bob",
        info: "Identity: 7BfX2K...bc12",
        votes: 3,
        documentId: "CZlMr49siZOuxqnnGiIkuONYZr6ToNg49EqF0njHExql",
      }),
    ],
    lockedVotes: 2,
    abstainVotes: 1,
    awardedTo: null,
    endTime: Math.floor(Date.now() / 1000) + 86400, // 24 hours from now
    state: "joinable" as ContestState,
    lastUpdated: Math.floor(Date.now() / 1000),
    ...overrides,
  };
}

export function createMockPastContest(
  overrides?: Partial<ContestedName>,
): ContestedName {
  return createMockContestedName({
    normalizedContestedName: "carol",
    awardedTo: "4EfA9Jrvv3nnCFdSf7fad59851kYrUWF21pYPKhq6Ycis",
    endTime: Math.floor(Date.now() / 1000) - 86400, // 24 hours ago
    state: { wonBy: "4EfA9Jrvv3nnCFdSf7fad59851kYrUWF21pYPKhq6Ycis" },
    ...overrides,
  });
}

export function createMockLockedContest(
  overrides?: Partial<ContestedName>,
): ContestedName {
  return createMockContestedName({
    normalizedContestedName: "dave",
    awardedTo: null,
    endTime: Math.floor(Date.now() / 1000) - 86400,
    state: "locked" as ContestState,
    lockedVotes: 10,
    abstainVotes: 0,
    ...overrides,
  });
}

// ─── Selected vote factory ─────────────────────────────────────────

export function createMockSelectedVote(
  overrides?: Partial<SelectedVote>,
): SelectedVote {
  return {
    contestedName: "alice",
    choice: createMockVoteChoice("towards"),
    endTime: Math.floor(Date.now() / 1000) + 86400,
    ...overrides,
  };
}
