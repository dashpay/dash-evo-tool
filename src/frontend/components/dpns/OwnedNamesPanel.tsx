import { useMemo, useState } from "react";
import {
  ArrowUpDown,
  ArrowUp,
  ArrowDown,
  Search,
  User,
  Tag,
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
import type { DpnsNameEntryDto } from "@/bindings";

// ─── Types ─────────────────────────────────────────────────────────

export type OwnedNameSortColumn = "name" | "ownerId" | "acquiredAt";

export type SortOrder = "ascending" | "descending";

// ─── Helpers ───────────────────────────────────────────────────────

/** Format a Unix timestamp (seconds) to a human-readable date. */
function formatAcquiredAt(timestamp: number): string {
  const ms = timestamp * 1000;
  const d = new Date(ms);
  if (isNaN(d.getTime())) return "Unknown";
  return d.toISOString().replace("T", " ").slice(0, 19);
}

/** Format relative time for acquired at. */
function formatRelativeTime(timestamp: number): string {
  const ms = timestamp * 1000;
  const now = Date.now();
  const diff = now - ms;

  if (diff < 60_000) return "just now";

  const minutes = Math.floor(diff / 60_000);
  if (minutes < 60) return `${minutes}m ago`;

  const hours = Math.floor(diff / 3_600_000);
  if (hours < 24) return `${hours}h ago`;

  const days = Math.floor(diff / 86_400_000);
  return `${days}d ago`;
}

/** Sort owned names by a given column. */
function sortOwnedNames(
  names: DpnsNameEntryDto[],
  column: OwnedNameSortColumn,
  order: SortOrder,
): DpnsNameEntryDto[] {
  const sorted = [...names];
  const dir = order === "ascending" ? 1 : -1;

  sorted.sort((a, b) => {
    switch (column) {
      case "name":
        return dir * a.name.localeCompare(b.name);
      case "ownerId":
        return dir * a.identityId.localeCompare(b.identityId);
      case "acquiredAt":
        return dir * (a.acquiredAt - b.acquiredAt);
      default:
        return 0;
    }
  });

  return sorted;
}

// ─── Props ─────────────────────────────────────────────────────────

export interface OwnedNamesPanelProps {
  /** Local DPNS names owned by loaded identities. */
  ownedNames: DpnsNameEntryDto[];
  /** Current filter term. */
  filterTerm: string;
  /** Called when user changes the filter text. */
  onFilterChange: (term: string) => void;
  /** Called when user clicks "Set Alias" for a name entry. */
  onSetAlias: (identityId: string, name: string) => void;
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
  column: OwnedNameSortColumn;
  currentColumn: OwnedNameSortColumn;
  currentOrder: SortOrder;
  onSort: (column: OwnedNameSortColumn) => void;
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

export function OwnedNamesPanel({
  ownedNames,
  filterTerm,
  onFilterChange,
  onSetAlias,
  className,
}: OwnedNamesPanelProps) {
  const [sortColumn, setSortColumn] = useState<OwnedNameSortColumn>("name");
  const [sortOrder, setSortOrder] = useState<SortOrder>("ascending");

  // Apply filter (case-insensitive substring match on name)
  const filteredNames = useMemo(() => {
    if (!filterTerm) return ownedNames;
    const filterLc = filterTerm.toLowerCase();
    return ownedNames.filter((entry) =>
      entry.name.toLowerCase().includes(filterLc),
    );
  }, [ownedNames, filterTerm]);

  // Apply sort
  const sortedNames = useMemo(() => {
    return sortOwnedNames(filteredNames, sortColumn, sortOrder);
  }, [filteredNames, sortColumn, sortOrder]);

  const handleSort = (column: OwnedNameSortColumn) => {
    if (column === sortColumn) {
      setSortOrder((prev) =>
        prev === "ascending" ? "descending" : "ascending",
      );
    } else {
      setSortColumn(column);
      setSortOrder("ascending");
    }
  };

  if (ownedNames.length === 0 && !filterTerm) {
    return (
      <EmptyState
        icon={User}
        title="No owned usernames."
        description="Register a DPNS name to see it here. Names owned by your loaded identities will appear in this list."
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
          placeholder="Filter by name"
          value={filterTerm}
          onChange={(e) => onFilterChange(e.target.value)}
          className="pl-9"
          aria-label="Filter owned names"
        />
      </div>

      {sortedNames.length === 0 ? (
        <EmptyState
          icon={Search}
          title="No matching names."
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
                    label="Owner ID"
                    column="ownerId"
                    currentColumn={sortColumn}
                    currentOrder={sortOrder}
                    onSort={handleSort}
                  />
                </TableHead>
                <TableHead>
                  <SortableHeader
                    label="Acquired At"
                    column="acquiredAt"
                    currentColumn={sortColumn}
                    currentOrder={sortOrder}
                    onSort={handleSort}
                  />
                </TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {sortedNames.map((entry) => {
                const acquiredDate = formatAcquiredAt(entry.acquiredAt);
                const relativeTime = formatRelativeTime(entry.acquiredAt);

                return (
                  <TableRow key={`${entry.identityId}-${entry.name}`}>
                    {/* Name */}
                    <TableCell className="font-medium">
                      {entry.name}
                    </TableCell>

                    {/* Owner ID */}
                    <TableCell>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Badge
                            variant="outline"
                            className="cursor-default font-mono"
                          >
                            {displayId(entry.identityId)}
                          </Badge>
                        </TooltipTrigger>
                        <TooltipContent>
                          <p className="font-mono text-xs">
                            {entry.identityId}
                          </p>
                        </TooltipContent>
                      </Tooltip>
                    </TableCell>

                    {/* Acquired At */}
                    <TableCell>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <span className="cursor-default text-sm">
                            {relativeTime}
                          </span>
                        </TooltipTrigger>
                        <TooltipContent>
                          <p>{acquiredDate}</p>
                        </TooltipContent>
                      </Tooltip>
                    </TableCell>

                    {/* Actions */}
                    <TableCell className="text-right">
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() =>
                          onSetAlias(entry.identityId, entry.name)
                        }
                        aria-label={`Set alias for ${entry.name}`}
                      >
                        <Tag className="mr-1.5 h-3.5 w-3.5" />
                        Set Alias
                      </Button>
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
