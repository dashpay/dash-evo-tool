import { useMemo } from "react";
import {
  ArrowUpDown,
  ArrowUp,
  ArrowDown,
  Search,
  Trophy,
  Lock,
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
  ContestSortColumn,
  SortOrder,
} from "@/stores/contestStore";
import { matchesDpnsFilter } from "@/stores/contestStore";

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

/** Get the display text for an awarded-to field. */
function formatAwardedTo(contest: ContestedName): string {
  const state = contest.state;
  if (state === "locked") return "Locked";
  if (typeof state === "object" && "wonBy" in state) {
    return displayId(state.wonBy);
  }
  return "Fetching";
}

/** Get the full awarded-to identifier for tooltip. */
function getFullAwardedTo(contest: ContestedName): string | null {
  const state = contest.state;
  if (typeof state === "object" && "wonBy" in state) {
    return state.wonBy;
  }
  return null;
}

// ─── Props ────────────────────────────────────────────────────────

export interface PastContestsTableProps {
  /** All contested names (past — will be filtered internally). */
  contestedNames: ContestedName[];
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

// ─── Main component ──────────────────────────────────────────────

export function PastContestsTable({
  contestedNames,
  filterTerm,
  sortColumn,
  sortOrder,
  onFilterChange,
  onSortChange,
  className,
}: PastContestsTableProps) {
  // Filter to past contests only (awarded or locked)
  const pastContests = useMemo(() => {
    return contestedNames.filter((cn) => {
      const state = cn.state;
      if (state === "locked") return true;
      if (typeof state === "object" && "wonBy" in state) return true;
      return false;
    });
  }, [contestedNames]);

  // Apply smart filter
  const filteredContests = useMemo(() => {
    if (!filterTerm) return pastContests;
    return pastContests.filter((c) =>
      matchesDpnsFilter(c.normalizedContestedName, filterTerm),
    );
  }, [pastContests, filterTerm]);

  if (filteredContests.length === 0 && !filterTerm) {
    return (
      <EmptyState
        icon={Trophy}
        title="No past contests yet."
        description="Contests will appear here once they have ended."
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
                    label="Ended Time"
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
                <TableHead>
                  <SortableHeader
                    label="Awarded To"
                    column="awardedTo"
                    currentColumn={sortColumn}
                    currentOrder={sortOrder}
                    onSort={onSortChange}
                  />
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {filteredContests.map((contest) => {
                const endTime = formatEndTime(contest.endTime);
                const lastUpdated = formatLastUpdated(contest.lastUpdated);
                const awardedToText = formatAwardedTo(contest);
                const fullAwardedTo = getFullAwardedTo(contest);
                const isLocked = contest.state === "locked";

                return (
                  <TableRow key={contest.normalizedContestedName}>
                    {/* Name */}
                    <TableCell className="font-medium">
                      {contest.normalizedContestedName}
                    </TableCell>

                    {/* Ended Time */}
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

                    {/* Awarded To */}
                    <TableCell>
                      {isLocked ? (
                        <Badge
                          variant="outline"
                          className="gap-1 text-amber-600 dark:text-amber-400"
                        >
                          <Lock className="h-3 w-3" />
                          Locked
                        </Badge>
                      ) : fullAwardedTo ? (
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <Badge
                              variant="outline"
                              className="gap-1 font-mono text-green-700 dark:text-green-400"
                            >
                              <Trophy className="h-3 w-3" />
                              {awardedToText}
                            </Badge>
                          </TooltipTrigger>
                          <TooltipContent>
                            <p className="font-mono text-xs">
                              {fullAwardedTo}
                            </p>
                          </TooltipContent>
                        </Tooltip>
                      ) : (
                        <span className="text-sm text-muted-foreground">
                          {awardedToText}
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
    </div>
  );
}
