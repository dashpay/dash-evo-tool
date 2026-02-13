import { useMemo, useState } from "react";
import {
  ArrowUpDown,
  ArrowUp,
  ArrowDown,
  Search,
  Clock,
  Trash2,
  Play,
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
import { formatVoteChoice } from "@/components/dpns/VoteCastingDialog";
import type { ScheduledVoteWithStatus } from "@/stores/contestStore";
import type { ScheduledVoteCastingStatus } from "@/stores/contestStore";

// ─── Types ─────────────────────────────────────────────────────────

export type ScheduledVoteSortColumn =
  | "name"
  | "voter"
  | "choice"
  | "scheduledTime"
  | "status";

export type SortOrder = "ascending" | "descending";

// ─── Helpers ───────────────────────────────────────────────────────

/** Format a Unix timestamp (milliseconds) to ISO-like date string. */
export function formatScheduledTime(timestampMs: number): string {
  const d = new Date(timestampMs);
  if (isNaN(d.getTime())) return "Unknown";
  return d.toISOString().replace("T", " ").slice(0, 19);
}

/** Format relative time from a Unix timestamp in milliseconds. */
export function formatRelativeTime(timestampMs: number): string {
  const now = Date.now();
  const diff = timestampMs - now;

  // Past times
  if (diff < 0) {
    const absDiff = -diff;
    if (absDiff < 60_000) return "just now";
    const minutes = Math.floor(absDiff / 60_000);
    if (minutes < 60) return `${minutes}m ago`;
    const hours = Math.floor(absDiff / 3_600_000);
    if (hours < 24) return `${hours}h ago`;
    const days = Math.floor(absDiff / 86_400_000);
    return `${days}d ago`;
  }

  // Future times
  if (diff < 60_000) return "now";
  const minutes = Math.floor(diff / 60_000);
  if (minutes < 60) return `in ${minutes}m`;
  const hours = Math.floor(diff / 3_600_000);
  if (hours < 24) return `in ${hours}h`;
  const days = Math.floor(diff / 86_400_000);
  return `in ${days}d`;
}

/** Get display label and variant for a casting status. */
export function getStatusDisplay(status: ScheduledVoteCastingStatus): {
  label: string;
  variant: "default" | "secondary" | "destructive" | "outline";
} {
  switch (status) {
    case "notStarted":
      return { label: "Pending", variant: "secondary" };
    case "inProgress":
      return { label: "Casting...", variant: "default" };
    case "completed":
      return { label: "Casted", variant: "outline" };
    case "failed":
      return { label: "Failed", variant: "destructive" };
  }
}

/** Sort scheduled votes by a given column. */
function sortScheduledVotes(
  votes: ScheduledVoteWithStatus[],
  column: ScheduledVoteSortColumn,
  order: SortOrder,
): ScheduledVoteWithStatus[] {
  const sorted = [...votes];
  const dir = order === "ascending" ? 1 : -1;

  sorted.sort((a, b) => {
    switch (column) {
      case "name":
        return (
          dir *
          a.vote.contestedName.localeCompare(b.vote.contestedName)
        );
      case "voter":
        return dir * a.vote.voterId.localeCompare(b.vote.voterId);
      case "choice":
        return (
          dir *
          formatVoteChoice(a.vote.choice).localeCompare(
            formatVoteChoice(b.vote.choice),
          )
        );
      case "scheduledTime":
        return dir * (a.vote.unixTimestamp - b.vote.unixTimestamp);
      case "status": {
        const statusOrder: Record<ScheduledVoteCastingStatus, number> = {
          notStarted: 0,
          inProgress: 1,
          failed: 2,
          completed: 3,
        };
        return (
          dir * (statusOrder[a.castingStatus] - statusOrder[b.castingStatus])
        );
      }
      default:
        return 0;
    }
  });

  return sorted;
}

// ─── Props ─────────────────────────────────────────────────────────

export interface ScheduledVotesTableProps {
  /** Scheduled votes with their casting status. */
  scheduledVotes: ScheduledVoteWithStatus[];
  /** Whether any scheduled vote cast is currently in progress. */
  castInProgress: boolean;
  /** Called when user clicks "Remove" for a vote. */
  onRemove: (voterId: string, contestedName: string) => void;
  /** Called when user clicks "Cast Now" for a vote. */
  onCastNow: (vote: ScheduledVoteWithStatus) => void;
  className?: string;
}

// ─── Sort header component ────────────────────────────────────────

function SortableHeader({
  label,
  column,
  currentColumn,
  currentOrder,
  onSort,
  className,
}: {
  label: string;
  column: ScheduledVoteSortColumn;
  currentColumn: ScheduledVoteSortColumn;
  currentOrder: SortOrder;
  onSort: (column: ScheduledVoteSortColumn) => void;
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

// ─── Main component ───────────────────────────────────────────────

export function ScheduledVotesTable({
  scheduledVotes,
  castInProgress,
  onRemove,
  onCastNow,
  className,
}: ScheduledVotesTableProps) {
  const [filterTerm, setFilterTerm] = useState("");
  const [sortColumn, setSortColumn] =
    useState<ScheduledVoteSortColumn>("name");
  const [sortOrder, setSortOrder] = useState<SortOrder>("ascending");

  // Apply filter (case-insensitive on name or voter ID)
  const filteredVotes = useMemo(() => {
    if (!filterTerm) return scheduledVotes;
    const filterLc = filterTerm.toLowerCase();
    return scheduledVotes.filter(
      (sv) =>
        sv.vote.contestedName.toLowerCase().includes(filterLc) ||
        sv.vote.voterId.toLowerCase().includes(filterLc),
    );
  }, [scheduledVotes, filterTerm]);

  // Apply sort
  const sortedVotes = useMemo(() => {
    return sortScheduledVotes(filteredVotes, sortColumn, sortOrder);
  }, [filteredVotes, sortColumn, sortOrder]);

  const handleSort = (column: ScheduledVoteSortColumn) => {
    if (column === sortColumn) {
      setSortOrder((prev) =>
        prev === "ascending" ? "descending" : "ascending",
      );
    } else {
      setSortColumn(column);
      setSortOrder("ascending");
    }
  };

  if (scheduledVotes.length === 0 && !filterTerm) {
    return (
      <EmptyState
        icon={Clock}
        title="No scheduled votes."
        description="Schedule votes from the Active Contests tab. Scheduled votes will be cast automatically when their time arrives. Dash Evo Tool must remain running for scheduled votes to execute."
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
          placeholder="Filter by name or voter"
          value={filterTerm}
          onChange={(e) => setFilterTerm(e.target.value)}
          className="pl-9"
          aria-label="Filter scheduled votes"
        />
      </div>

      {sortedVotes.length === 0 ? (
        <EmptyState
          icon={Search}
          title="No matching votes."
          description="Try adjusting your filter."
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
                    onSort={handleSort}
                  />
                </TableHead>
                <TableHead>
                  <SortableHeader
                    label="Voter"
                    column="voter"
                    currentColumn={sortColumn}
                    currentOrder={sortOrder}
                    onSort={handleSort}
                  />
                </TableHead>
                <TableHead>
                  <SortableHeader
                    label="Vote Choice"
                    column="choice"
                    currentColumn={sortColumn}
                    currentOrder={sortOrder}
                    onSort={handleSort}
                  />
                </TableHead>
                <TableHead>
                  <SortableHeader
                    label="Scheduled Time"
                    column="scheduledTime"
                    currentColumn={sortColumn}
                    currentOrder={sortOrder}
                    onSort={handleSort}
                  />
                </TableHead>
                <TableHead>
                  <SortableHeader
                    label="Status"
                    column="status"
                    currentColumn={sortColumn}
                    currentOrder={sortOrder}
                    onSort={handleSort}
                  />
                </TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {sortedVotes.map((sv) => {
                const { label: statusLabel, variant: statusVariant } =
                  getStatusDisplay(sv.castingStatus);
                const scheduledDate = formatScheduledTime(
                  sv.vote.unixTimestamp,
                );
                const relativeTime = formatRelativeTime(
                  sv.vote.unixTimestamp,
                );
                const canCastNow =
                  !castInProgress &&
                  (sv.castingStatus === "notStarted" ||
                    sv.castingStatus === "failed");

                return (
                  <TableRow
                    key={`${sv.vote.voterId}-${sv.vote.contestedName}`}
                  >
                    {/* Contested Name */}
                    <TableCell className="font-medium">
                      {sv.vote.contestedName}
                    </TableCell>

                    {/* Voter ID */}
                    <TableCell>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Badge
                            variant="outline"
                            className="cursor-default font-mono"
                          >
                            {displayId(sv.vote.voterId)}
                          </Badge>
                        </TooltipTrigger>
                        <TooltipContent>
                          <p className="font-mono text-xs">
                            {sv.vote.voterId}
                          </p>
                        </TooltipContent>
                      </Tooltip>
                    </TableCell>

                    {/* Vote Choice */}
                    <TableCell>
                      <Badge variant="secondary">
                        {formatVoteChoice(sv.vote.choice)}
                      </Badge>
                    </TableCell>

                    {/* Scheduled Time */}
                    <TableCell>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <span className="cursor-default text-sm">
                            {relativeTime}
                          </span>
                        </TooltipTrigger>
                        <TooltipContent>
                          <p>{scheduledDate}</p>
                        </TooltipContent>
                      </Tooltip>
                    </TableCell>

                    {/* Status */}
                    <TableCell>
                      <Badge variant={statusVariant}>{statusLabel}</Badge>
                    </TableCell>

                    {/* Actions */}
                    <TableCell className="text-right">
                      <div className="flex items-center justify-end gap-1">
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => onCastNow(sv)}
                          disabled={!canCastNow}
                          aria-label={`Cast now ${sv.vote.contestedName}`}
                        >
                          <Play className="mr-1 h-3.5 w-3.5" />
                          Cast Now
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() =>
                            onRemove(
                              sv.vote.voterId,
                              sv.vote.contestedName,
                            )
                          }
                          aria-label={`Remove ${sv.vote.contestedName}`}
                        >
                          <Trash2 className="mr-1 h-3.5 w-3.5" />
                          Remove
                        </Button>
                      </div>
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
