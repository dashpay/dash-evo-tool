import { useMemo } from "react";
import {
  ArrowUpDown,
  ArrowUp,
  ArrowDown,
  Lock,
  Ban,
  Search,
} from "lucide-react";
import { cn, displayId } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { EmptyState } from "@/components/feedback/EmptyState";
import type {
  ContestedName,
  Contestant,
  SelectedVote,
  ContestSortColumn,
  SortOrder,
} from "@/stores/contestStore";
import { matchesDpnsFilter } from "@/stores/contestStore";
import type { VoteChoiceDto } from "@/bindings";

// ─── Helpers ──────────────────────────────────────────────────────

/** Format a Unix timestamp (seconds) to a human-readable date + relative time. */
function formatEndTime(timestamp: number | null): {
  date: string;
  relative: string;
} {
  if (timestamp == null) return { date: "Fetching", relative: "" };
  const ms = timestamp * 1000;
  const d = new Date(ms);
  if (isNaN(d.getTime())) return { date: "Invalid timestamp", relative: "" };

  const date = d.toISOString().replace("T", " ").slice(0, 19);
  const now = Date.now();
  const diff = ms - now;

  if (Math.abs(diff) < 60_000) {
    return { date, relative: diff >= 0 ? "in < 1 min" : "< 1 min ago" };
  }

  const absDiff = Math.abs(diff);
  const minutes = Math.floor(absDiff / 60_000);
  const hours = Math.floor(absDiff / 3_600_000);
  const days = Math.floor(absDiff / 86_400_000);

  let relative: string;
  if (days > 0) {
    relative = `${days}d ${hours % 24}h`;
  } else if (hours > 0) {
    relative = `${hours}h ${minutes % 60}m`;
  } else {
    relative = `${minutes}m`;
  }

  return { date, relative: diff >= 0 ? `in ${relative}` : `${relative} ago` };
}

/** Format a relative time for "last updated" timestamps. */
function formatLastUpdated(timestamp: number | null): string {
  if (timestamp == null) return "Fetching";
  const ms = timestamp * 1000;
  const now = Date.now();
  const diff = now - ms;

  if (diff < 5_000) return "now";
  if (diff < 60_000) return `${Math.floor(diff / 1_000)}s ago`;

  const minutes = Math.floor(diff / 60_000);
  if (minutes < 60) return `${minutes}m ago`;

  const hours = Math.floor(diff / 3_600_000);
  if (hours < 24) return `${hours}h ago`;

  const days = Math.floor(diff / 86_400_000);
  return `${days}d ago`;
}

/** Check if a VoteChoiceDto matches a specific type. */
function isVoteChoice(
  choice: VoteChoiceDto,
  type: "lock" | "abstain" | { identityId: string },
): boolean {
  if (type === "lock") return choice === "lock";
  if (type === "abstain") return choice === "abstain";
  return (
    typeof choice === "object" &&
    "towardsIdentity" in choice &&
    choice.towardsIdentity.identityId === type.identityId
  );
}

// ─── Props ────────────────────────────────────────────────────────

export interface ActiveContestsTableProps {
  /** All contested names (active — will be filtered internally). */
  contestedNames: ContestedName[];
  /** Currently selected votes for highlighting. */
  selectedVotes: SelectedVote[];
  /** Current filter term for smart filtering. */
  filterTerm: string;
  /** Current sort column. */
  sortColumn: ContestSortColumn;
  /** Current sort order. */
  sortOrder: SortOrder;
  /** Called when user changes the filter text. */
  onFilterChange: (term: string) => void;
  /** Called when user clicks a sortable column header. */
  onSortChange: (column: ContestSortColumn) => void;
  /** Called when user clicks a vote button (Lock, Abstain, or contestant). */
  onToggleVote: (vote: SelectedVote) => void;
  className?: string;
}

// ─── Sort header component ───────────────────────────────────────

function SortableHeader({
  label,
  column,
  currentColumn,
  currentOrder,
  onSort,
  className,
}: {
  label: string;
  column: ContestSortColumn;
  currentColumn: ContestSortColumn;
  currentOrder: SortOrder;
  onSort: (column: ContestSortColumn) => void;
  className?: string;
}) {
  const isActive = column === currentColumn;
  return (
    <Button
      variant="ghost"
      size="sm"
      className={cn("h-8 gap-1 px-1 font-medium", className)}
      onClick={() => onSort(column)}
      aria-label={`Sort by ${label}`}
    >
      {label}
      {isActive ? (
        currentOrder === "ascending" ? (
          <ArrowUp className="h-3.5 w-3.5" />
        ) : (
          <ArrowDown className="h-3.5 w-3.5" />
        )
      ) : (
        <ArrowUpDown className="h-3.5 w-3.5 opacity-40" />
      )}
    </Button>
  );
}

// ─── Contestant button ───────────────────────────────────────────

function ContestantButton({
  contestant,
  isSelected,
  hasHighestVotes,
  lockedVotesAreHighest,
  onToggle,
}: {
  contestant: Contestant;
  isSelected: boolean;
  hasHighestVotes: boolean;
  lockedVotesAreHighest: boolean;
  onToggle: () => void;
}) {
  const label = `${displayId(contestant.id)} — ${contestant.votes} votes`;
  const showBold = hasHighestVotes && !lockedVotesAreHighest;

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant={isSelected ? "default" : "ghost"}
          size="sm"
          className={cn(
            "h-7 gap-1 px-2 text-xs",
            isSelected && "bg-blue-600 text-white hover:bg-blue-700",
            showBold && !isSelected && "font-bold text-green-700 dark:text-green-400",
          )}
          onClick={onToggle}
          aria-label={`Vote for contestant ${displayId(contestant.id)}`}
          aria-pressed={isSelected}
        >
          {label}
        </Button>
      </TooltipTrigger>
      <TooltipContent>
        <p className="font-mono text-xs">{contestant.id}</p>
        {contestant.name && <p>{contestant.name}</p>}
      </TooltipContent>
    </Tooltip>
  );
}

// ─── Main component ──────────────────────────────────────────────

export function ActiveContestsTable({
  contestedNames,
  selectedVotes,
  filterTerm,
  sortColumn,
  sortOrder,
  onFilterChange,
  onSortChange,
  onToggleVote,
  className,
}: ActiveContestsTableProps) {
  // Filter to active contests only (no awardedTo, state is ongoing/joinable/unknown)
  const activeContests = useMemo(() => {
    return contestedNames.filter((cn) => {
      // Active = not awarded and not locked
      const state = cn.state;
      if (state === "locked") return false;
      if (typeof state === "object" && "wonBy" in state) return false;
      return true;
    });
  }, [contestedNames]);

  // Apply smart filter
  const filteredContests = useMemo(() => {
    if (!filterTerm) return activeContests;
    return activeContests.filter((c) =>
      matchesDpnsFilter(c.normalizedContestedName, filterTerm),
    );
  }, [activeContests, filterTerm]);

  // Find selected vote for a contested name
  const getSelectedVote = (contestedName: string) =>
    selectedVotes.find((sv) => sv.contestedName === contestedName);

  // Create a vote to toggle
  const makeVoteChoice = (
    contest: ContestedName,
    choice: VoteChoiceDto,
  ): SelectedVote => ({
    contestedName: contest.normalizedContestedName,
    choice,
    endTime: contest.endTime,
  });

  if (filteredContests.length === 0 && !filterTerm) {
    return (
      <EmptyState
        icon={Search}
        title="No active contests at the moment."
        description="Please check back later or try refreshing the list."
        className={className}
      />
    );
  }

  return (
    <div className={cn("space-y-3", className)}>
      {/* Filter input */}
      <div className="relative max-w-sm">
        <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          placeholder="Filter contests (smart: o→0, l→1)"
          value={filterTerm}
          onChange={(e) => onFilterChange(e.target.value)}
          className="pl-9"
          aria-label="Filter contests"
        />
      </div>

      {filteredContests.length === 0 ? (
        <EmptyState
          icon={Search}
          title="No matching contests."
          description="Try adjusting your filter. Smart filter converts: o→0, l→1."
        />
      ) : (
        <div className="rounded-md border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>
                  <SortableHeader
                    label="Name"
                    column="name"
                    currentColumn={sortColumn}
                    currentOrder={sortOrder}
                    onSort={onSortChange}
                  />
                </TableHead>
                <TableHead>
                  <SortableHeader
                    label="Locked Votes"
                    column="lockedVotes"
                    currentColumn={sortColumn}
                    currentOrder={sortOrder}
                    onSort={onSortChange}
                  />
                </TableHead>
                <TableHead>
                  <SortableHeader
                    label="Abstain Votes"
                    column="abstainVotes"
                    currentColumn={sortColumn}
                    currentOrder={sortOrder}
                    onSort={onSortChange}
                  />
                </TableHead>
                <TableHead>
                  <SortableHeader
                    label="Ending Time"
                    column="endingTime"
                    currentColumn={sortColumn}
                    currentOrder={sortOrder}
                    onSort={onSortChange}
                  />
                </TableHead>
                <TableHead>
                  <SortableHeader
                    label="Last Updated"
                    column="lastUpdated"
                    currentColumn={sortColumn}
                    currentOrder={sortOrder}
                    onSort={onSortChange}
                  />
                </TableHead>
                <TableHead className="min-w-[200px]">Contestants</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {filteredContests.map((contest) => {
                const selected = getSelectedVote(
                  contest.normalizedContestedName,
                );
                const isLockSelected =
                  selected != null && isVoteChoice(selected.choice, "lock");
                const isAbstainSelected =
                  selected != null && isVoteChoice(selected.choice, "abstain");

                // Determine highest vote count among contestants
                const maxContestantVotes = Math.max(
                  0,
                  ...(contest.contestants?.map((c) => c.votes) ?? []),
                );
                const lockedVotesAreHighest =
                  (contest.lockedVotes ?? 0) > maxContestantVotes;

                const endTime = formatEndTime(contest.endTime);
                const lastUpdated = formatLastUpdated(contest.lastUpdated);

                return (
                  <TableRow
                    key={contest.normalizedContestedName}
                    data-state={selected ? "selected" : undefined}
                  >
                    {/* Name */}
                    <TableCell className="font-medium">
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <span className="cursor-default">
                            {contest.normalizedContestedName}
                          </span>
                        </TooltipTrigger>
                        <TooltipContent>
                          <p>
                            Normalized: {contest.normalizedContestedName}
                          </p>
                          {contest.contestants &&
                            contest.contestants.length > 0 && (
                              <p>
                                {contest.contestants.length} contestant
                                {contest.contestants.length !== 1 ? "s" : ""}
                              </p>
                            )}
                        </TooltipContent>
                      </Tooltip>
                    </TableCell>

                    {/* Locked Votes */}
                    <TableCell>
                      <Button
                        variant={isLockSelected ? "default" : "ghost"}
                        size="sm"
                        className={cn(
                          "h-7 gap-1.5 px-2",
                          isLockSelected &&
                            "bg-blue-600 text-white hover:bg-blue-700",
                          lockedVotesAreHighest &&
                            !isLockSelected &&
                            "font-bold text-green-700 dark:text-green-400",
                        )}
                        onClick={() =>
                          onToggleVote(makeVoteChoice(contest, "lock"))
                        }
                        aria-label={`Lock vote for ${contest.normalizedContestedName}`}
                        aria-pressed={isLockSelected}
                      >
                        <Lock className="h-3.5 w-3.5" />
                        {contest.lockedVotes ?? 0}
                      </Button>
                    </TableCell>

                    {/* Abstain Votes */}
                    <TableCell>
                      <Button
                        variant={isAbstainSelected ? "default" : "ghost"}
                        size="sm"
                        className={cn(
                          "h-7 gap-1.5 px-2",
                          isAbstainSelected &&
                            "bg-blue-600 text-white hover:bg-blue-700",
                        )}
                        onClick={() =>
                          onToggleVote(makeVoteChoice(contest, "abstain"))
                        }
                        aria-label={`Abstain vote for ${contest.normalizedContestedName}`}
                        aria-pressed={isAbstainSelected}
                      >
                        <Ban className="h-3.5 w-3.5" />
                        {contest.abstainVotes ?? 0}
                      </Button>
                    </TableCell>

                    {/* Ending Time */}
                    <TableCell>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <span className="cursor-default text-sm">
                            {endTime.relative || endTime.date}
                          </span>
                        </TooltipTrigger>
                        <TooltipContent>
                          <p>{endTime.date}</p>
                          {endTime.relative && <p>{endTime.relative}</p>}
                        </TooltipContent>
                      </Tooltip>
                    </TableCell>

                    {/* Last Updated */}
                    <TableCell>
                      <Badge variant="secondary" className="font-normal">
                        {lastUpdated}
                      </Badge>
                    </TableCell>

                    {/* Contestants */}
                    <TableCell>
                      {contest.contestants &&
                      contest.contestants.length > 0 ? (
                        <div className="flex flex-wrap gap-1">
                          {contest.contestants.map((contestant) => {
                            const isContestantSelected =
                              selected != null &&
                              isVoteChoice(selected.choice, {
                                identityId: contestant.id,
                              });
                            const hasHighestVotes =
                              contestant.votes === maxContestantVotes &&
                              maxContestantVotes > 0;

                            return (
                              <ContestantButton
                                key={contestant.id}
                                contestant={contestant}
                                isSelected={isContestantSelected}
                                hasHighestVotes={hasHighestVotes}
                                lockedVotesAreHighest={lockedVotesAreHighest}
                                onToggle={() =>
                                  onToggleVote(
                                    makeVoteChoice(contest, {
                                      towardsIdentity: {
                                        identityId: contestant.id,
                                      },
                                    }),
                                  )
                                }
                              />
                            );
                          })}
                        </div>
                      ) : (
                        <span className="text-sm text-muted-foreground">
                          No contestants
                        </span>
                      )}
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        </div>
      )}

      {/* Selection summary */}
      {selectedVotes.length > 0 && (
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <Badge variant="outline">
            {selectedVotes.length} vote{selectedVotes.length !== 1 ? "s" : ""}{" "}
            selected
          </Badge>
        </div>
      )}
    </div>
  );
}
