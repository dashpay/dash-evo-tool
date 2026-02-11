import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { commands } from "@/bindings";
import type { ProofLogItemDto, RequestTypeDto } from "@/bindings";
import { ToolPageLayout } from "@/components/tools/ToolPageLayout";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/shared/CopyButton";
import {
  ChevronLeft,
  ChevronRight,
  Loader2,
  AlertCircle,
  FileSearch,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { toastError } from "@/lib/toastError";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type DisplayMode = "hex" | "json" | "pathQuery";

type SortColumn = "requestType" | "height" | "timeMs" | "error";

interface SortState {
  column: SortColumn;
  ascending: boolean;
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const ITEMS_PER_PAGE = 100;

/** Map camelCase RequestTypeDto to a human-readable label. */
function formatRequestType(rt: RequestTypeDto): string {
  // Insert spaces before uppercase letters, trim, then capitalize first letter
  const spaced = rt.replace(/([A-Z])/g, " $1").trim();
  return spaced.charAt(0).toUpperCase() + spaced.slice(1);
}

/**
 * Extract all 64-character hex strings from an error message.
 * Used for gold-highlighting matching hashes in proof/hex output.
 */
function extractHashes(error: string | null): string[] {
  if (!error) return [];
  const matches = error.match(/[a-fA-F0-9]{64}/g);
  return matches ? [...new Set(matches)] : [];
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function ProofLogScreen() {
  // --- Data state ---
  const [items, setItems] = useState<ProofLogItemDto[]>([]);
  const [page, setPage] = useState(0);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // --- Selection & display ---
  const [selectedIndex, setSelectedIndex] = useState<number | null>(null);
  const [displayMode, setDisplayMode] = useState<DisplayMode>("hex");
  const [detailText, setDetailText] = useState<string | null>(null);
  const [isDetailLoading, setIsDetailLoading] = useState(false);
  const [detailError, setDetailError] = useState<string | null>(null);

  // --- Sort state (client-side on current page) ---
  const [sort, setSort] = useState<SortState>({
    column: "timeMs",
    ascending: false,
  });

  // Ref for cancelling stale detail parse requests
  const detailSeqRef = useRef(0);

  // ------------------------------------------------------------------
  // Data fetching
  // ------------------------------------------------------------------

  const fetchItems = useCallback(async (pageNum: number) => {
    setIsLoading(true);
    setError(null);
    try {
      const result = await commands.proofLogGetItems({
        onlyErrored: false,
        page: pageNum,
        itemsPerPage: ITEMS_PER_PAGE,
      });
      if (result.status === "ok") {
        setItems(result.data.items);
        setSelectedIndex(null);
        setDetailText(null);
        setDetailError(null);
      } else {
        setError(result.error);
        toastError(result.error);
        setItems([]);
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(msg);
      toastError(msg);
      setItems([]);
    } finally {
      setIsLoading(false);
    }
  }, []);

  // Fetch on mount and when page changes
  useEffect(() => {
    fetchItems(page);
  }, [page, fetchItems]);

  // ------------------------------------------------------------------
  // Sorting (client-side within current page)
  // ------------------------------------------------------------------

  const sortedItems = useMemo(() => {
    const sorted = [...items];
    sorted.sort((a, b) => {
      let cmp = 0;
      switch (sort.column) {
        case "requestType":
          cmp = a.requestType.localeCompare(b.requestType);
          break;
        case "height":
          cmp = a.height - b.height;
          break;
        case "timeMs":
          cmp = a.timeMs - b.timeMs;
          break;
        case "error":
          cmp = (a.error ?? "").localeCompare(b.error ?? "");
          break;
      }
      return sort.ascending ? cmp : -cmp;
    });
    return sorted;
  }, [items, sort]);

  const handleHeaderClick = useCallback((col: SortColumn) => {
    setSort((prev) =>
      prev.column === col
        ? { column: col, ascending: !prev.ascending }
        : { column: col, ascending: true },
    );
  }, []);

  // ------------------------------------------------------------------
  // Detail panel: parse proof bytes in the selected display mode
  // ------------------------------------------------------------------

  const selectedItem =
    selectedIndex !== null ? sortedItems[selectedIndex] ?? null : null;

  const loadDetail = useCallback(
    async (item: ProofLogItemDto, mode: DisplayMode) => {
      const seq = ++detailSeqRef.current;

      if (mode === "hex") {
        // Hex mode is synchronous — just show proof_bytes_hex directly
        setDetailText(item.proofBytesHex);
        setDetailError(null);
        setIsDetailLoading(false);
        return;
      }

      setIsDetailLoading(true);
      setDetailError(null);
      setDetailText(null);

      try {
        if (mode === "json") {
          if (!item.proofBytesHex) {
            setDetailText("(empty proof bytes)");
            return;
          }
          const result = await commands.parseGrovedbProof({
            hexData: item.proofBytesHex,
          });
          if (seq !== detailSeqRef.current) return; // stale
          if (result.status === "ok") {
            setDetailText(result.data.text);
          } else {
            setDetailError(result.error);
          }
        } else if (mode === "pathQuery") {
          if (!item.verificationPathQueryHex) {
            setDetailText("(empty path query bytes)");
            return;
          }
          const result = await commands.parsePathQuery({
            hexData: item.verificationPathQueryHex,
          });
          if (seq !== detailSeqRef.current) return; // stale
          if (result.status === "ok") {
            setDetailText(result.data.text);
          } else {
            setDetailError(result.error);
          }
        }
      } catch (err) {
        if (seq !== detailSeqRef.current) return;
        const msg = err instanceof Error ? err.message : String(err);
        setDetailError(msg);
        toastError(msg);
      } finally {
        if (seq === detailSeqRef.current) {
          setIsDetailLoading(false);
        }
      }
    },
    [],
  );

  // Re-parse when selection or display mode changes
  useEffect(() => {
    if (selectedItem) {
      loadDetail(selectedItem, displayMode);
    } else {
      setDetailText(null);
      setDetailError(null);
    }
  }, [selectedItem, displayMode, loadDetail]);

  // ------------------------------------------------------------------
  // Pagination handlers
  // ------------------------------------------------------------------

  const handlePrev = useCallback(() => {
    setPage((p) => Math.max(0, p - 1));
  }, []);

  const handleNext = useCallback(() => {
    setPage((p) => p + 1);
  }, []);

  const rangeStart = page * ITEMS_PER_PAGE + 1;
  const rangeEnd = page * ITEMS_PER_PAGE + items.length;

  // ------------------------------------------------------------------
  // Hash highlighting for proof output
  // ------------------------------------------------------------------

  const hashes = useMemo(
    () => (selectedItem ? extractHashes(selectedItem.error) : []),
    [selectedItem],
  );

  // ------------------------------------------------------------------
  // Render
  // ------------------------------------------------------------------

  return (
    <ToolPageLayout
      title="Proof Log"
      subtitle="Browse and inspect historical proof log entries with sorting, filtering, and detail views"
    >
      {/* Empty state */}
      {!isLoading && items.length === 0 && !error && (
        <div className="flex flex-col items-center justify-center py-16 text-center">
          <FileSearch className="mb-4 size-12 text-muted-foreground/50" />
          <h2 className="text-lg font-semibold text-foreground">
            No proof items to display
          </h2>
          <p className="mt-1 text-sm text-muted-foreground">
            Proof log entries are recorded when Platform proofs are received.
          </p>
        </div>
      )}

      {/* Error */}
      {error && (
        <div className="mb-4 flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive">
          <AlertCircle className="mt-0.5 size-4 shrink-0" />
          <span>{error}</span>
        </div>
      )}

      {/* Loading */}
      {isLoading && (
        <div className="flex items-center justify-center py-16">
          <Loader2 className="size-6 animate-spin text-muted-foreground" />
          <span className="ml-2 text-sm text-muted-foreground">
            Loading proof log...
          </span>
        </div>
      )}

      {/* Main content: table + detail panel */}
      {!isLoading && items.length > 0 && (
        <div className="flex gap-4 min-h-0 flex-1">
          {/* Left: table */}
          <div className="flex flex-col min-w-0 flex-1">
            <div className="overflow-auto rounded-md border">
              <table className="w-full text-sm" role="grid">
                <thead>
                  <tr className="border-b bg-muted/40">
                    <SortHeader
                      label="Request Type"
                      column="requestType"
                      sort={sort}
                      onClick={handleHeaderClick}
                    />
                    <SortHeader
                      label="Height"
                      column="height"
                      sort={sort}
                      onClick={handleHeaderClick}
                    />
                    <SortHeader
                      label="Time (ms)"
                      column="timeMs"
                      sort={sort}
                      onClick={handleHeaderClick}
                    />
                    <SortHeader
                      label="Error"
                      column="error"
                      sort={sort}
                      onClick={handleHeaderClick}
                    />
                  </tr>
                </thead>
                <tbody>
                  {sortedItems.map((item, idx) => (
                    <tr
                      key={`${item.timeMs}-${idx}`}
                      role="row"
                      tabIndex={0}
                      className={cn(
                        "cursor-pointer border-b transition-colors",
                        "hover:bg-muted/30",
                        selectedIndex === idx && "bg-primary/10",
                      )}
                      onClick={() =>
                        setSelectedIndex(selectedIndex === idx ? null : idx)
                      }
                      onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault();
                          setSelectedIndex(selectedIndex === idx ? null : idx);
                        }
                      }}
                    >
                      <td className="whitespace-nowrap px-3 py-2 font-medium">
                        {formatRequestType(item.requestType)}
                      </td>
                      <td className="whitespace-nowrap px-3 py-2 font-mono text-xs">
                        {item.height}
                      </td>
                      <td className="whitespace-nowrap px-3 py-2 font-mono text-xs">
                        {item.timeMs}
                      </td>
                      <td
                        className="max-w-[260px] truncate px-3 py-2 text-xs"
                        title={item.error ?? "No Error"}
                      >
                        {item.error ? (
                          <span className="text-destructive">
                            {item.error.length > 40
                              ? `${item.error.slice(0, 40)}...`
                              : item.error}
                          </span>
                        ) : (
                          <span className="text-muted-foreground">
                            No Error
                          </span>
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>

            {/* Pagination */}
            <div className="mt-2 flex items-center justify-between">
              <span className="text-xs text-muted-foreground">
                Showing items {rangeStart} to {rangeEnd}
              </span>
              <div className="flex items-center gap-1">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={handlePrev}
                  disabled={page === 0}
                  aria-label="Previous page"
                >
                  <ChevronLeft className="size-4" />
                  Previous
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={handleNext}
                  disabled={items.length < ITEMS_PER_PAGE}
                  aria-label="Next page"
                >
                  Next
                  <ChevronRight className="size-4" />
                </Button>
              </div>
            </div>
          </div>

          {/* Right: detail panel */}
          <div className="flex flex-col min-w-0 w-[420px] shrink-0">
            {selectedItem ? (
              <DetailPanel
                item={selectedItem}
                displayMode={displayMode}
                onDisplayModeChange={setDisplayMode}
                detailText={detailText}
                detailError={detailError}
                isDetailLoading={isDetailLoading}
                hashes={hashes}
              />
            ) : (
              <div className="flex flex-1 items-center justify-center rounded-md border border-dashed p-6 text-center text-sm text-muted-foreground">
                Select a proof log entry to view details
              </div>
            )}
          </div>
        </div>
      )}
    </ToolPageLayout>
  );
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function SortHeader({
  label,
  column,
  sort,
  onClick,
}: {
  label: string;
  column: SortColumn;
  sort: SortState;
  onClick: (col: SortColumn) => void;
}) {
  const isActive = sort.column === column;
  return (
    <th className="px-3 py-2 text-left">
      <button
        className="inline-flex items-center gap-1 text-xs font-medium uppercase tracking-wider text-muted-foreground hover:text-foreground"
        onClick={() => onClick(column)}
        title={`Sort by ${label}`}
      >
        {label}
        {isActive && (
          <span className="text-foreground">
            {sort.ascending ? "▲" : "▼"}
          </span>
        )}
      </button>
    </th>
  );
}

function DetailPanel({
  item,
  displayMode,
  onDisplayModeChange,
  detailText,
  detailError,
  isDetailLoading,
  hashes,
}: {
  item: ProofLogItemDto;
  displayMode: DisplayMode;
  onDisplayModeChange: (mode: DisplayMode) => void;
  detailText: string | null;
  detailError: string | null;
  isDetailLoading: boolean;
  hashes: string[];
}) {
  return (
    <div className="flex flex-col gap-3 overflow-auto rounded-md border p-4">
      {/* Metadata */}
      <div className="space-y-1 text-sm">
        <InfoRow
          label="Request Type"
          value={formatRequestType(item.requestType)}
        />
        <InfoRow label="Height" value={String(item.height)} mono />
        <InfoRow label="Time (ms)" value={String(item.timeMs)} mono />
        <InfoRow
          label="Error"
          value={item.error ?? "None"}
          className={item.error ? "text-destructive" : "text-muted-foreground"}
        />
      </div>

      {/* Display mode radios */}
      <div className="flex items-center gap-3 border-t pt-3">
        <span className="text-xs font-medium text-muted-foreground">
          Display:
        </span>
        {(["hex", "json", "pathQuery"] as const).map((mode) => (
          <label
            key={mode}
            className="flex items-center gap-1.5 text-sm cursor-pointer"
          >
            <input
              type="radio"
              name="displayMode"
              value={mode}
              checked={displayMode === mode}
              onChange={() => onDisplayModeChange(mode)}
              className="accent-primary"
            />
            {mode === "hex" ? "Hex" : mode === "json" ? "JSON" : "Path Query"}
          </label>
        ))}
      </div>

      {/* Detail content */}
      <div className="flex-1 min-h-0">
        {isDetailLoading ? (
          <div className="flex items-center justify-center py-8">
            <Loader2 className="size-5 animate-spin text-muted-foreground" />
            <span className="ml-2 text-sm text-muted-foreground">
              Parsing...
            </span>
          </div>
        ) : detailError ? (
          <div className="rounded-md border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive">
            {detailError}
          </div>
        ) : detailText ? (
          <div className="space-y-2">
            <div className="flex items-center justify-end">
              <CopyButton value={detailText} label="Copy" size="sm" />
            </div>
            <div
              className="overflow-auto rounded-md border bg-muted/30 p-3 font-mono text-xs select-text"
              style={{ maxHeight: 400, minHeight: 200 }}
              role="log"
              aria-label={`Proof ${displayMode} content`}
              tabIndex={0}
            >
              <HighlightedText
                text={detailText}
                hashes={displayMode !== "pathQuery" ? hashes : []}
              />
            </div>
          </div>
        ) : (
          <div className="flex items-center justify-center py-8 text-sm text-muted-foreground">
            No data available
          </div>
        )}
      </div>
    </div>
  );
}

function InfoRow({
  label,
  value,
  mono,
  className,
}: {
  label: string;
  value: string;
  mono?: boolean;
  className?: string;
}) {
  return (
    <div className="flex items-start gap-2">
      <span className="w-24 shrink-0 text-xs font-medium text-muted-foreground">
        {label}:
      </span>
      <span
        className={cn(
          "text-xs break-all",
          mono && "font-mono",
          className,
        )}
      >
        {value}
      </span>
    </div>
  );
}

/**
 * Renders text with 64-char hex strings highlighted in gold when they match
 * hashes extracted from the error message.
 */
function HighlightedText({
  text,
  hashes,
}: {
  text: string;
  hashes: string[];
}) {
  if (hashes.length === 0) {
    return <pre className="m-0 whitespace-pre-wrap break-all">{text}</pre>;
  }

  // Build a regex to match any of the hashes (case insensitive)
  const pattern = new RegExp(`(${hashes.map(escapeRegExp).join("|")})`, "gi");
  const parts = text.split(pattern);

  return (
    <pre className="m-0 whitespace-pre-wrap break-all">
      {parts.map((part, i) => {
        const isHighlight = hashes.some(
          (h) => h.toLowerCase() === part.toLowerCase(),
        );
        return isHighlight ? (
          <span key={i} className="text-amber-500 font-bold">
            {part}
          </span>
        ) : (
          <span key={i}>{part}</span>
        );
      })}
    </pre>
  );
}

function escapeRegExp(str: string): string {
  return str.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
