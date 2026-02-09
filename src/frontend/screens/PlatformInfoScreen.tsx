import { useCallback, useEffect, useRef, useState } from "react";
import { ToolPageLayout } from "@/components/tools/ToolPageLayout";
import { MonospaceOutput } from "@/components/tools/MonospaceOutput";
import { commands, events } from "@/bindings";
import type { TaskResultEvent, TaskErrorEvent } from "@/bindings";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { toast } from "sonner";
import {
  Info,
  Clock,
  Coins,
  Vote,
  Users,
  ArrowDownToLine,
  CheckCircle,
  Loader2,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";

/** A platform info query type with its display metadata and dispatch function. */
interface QueryType {
  id: string;
  title: string;
  description: string;
  icon: LucideIcon;
  dispatch: () => Promise<{ taskId: string }>;
}

const QUERY_TYPES: QueryType[] = [
  {
    id: "basic_info",
    title: "Basic Platform Info",
    description:
      "Protocol version, fee version, drive versions, system data contracts, network, and chain lock height.",
    icon: Info,
    dispatch: () => commands.platformBasicInfo(),
  },
  {
    id: "epoch_info",
    title: "Current Epoch Info",
    description:
      "Current epoch protocol version, index, block heights, start time, estimated end time, and fee multiplier.",
    icon: Clock,
    dispatch: () => commands.platformCurrentEpochInfo(),
  },
  {
    id: "total_credits",
    title: "Total Credits on Platform",
    description: "Total credits held on the platform with Dash equivalent.",
    icon: Coins,
    dispatch: () => commands.platformTotalCredits(),
  },
  {
    id: "version_voting",
    title: "Version Voting State",
    description:
      "Current protocol version voting counts from validators.",
    icon: Vote,
    dispatch: () => commands.platformVersionVotingState(),
  },
  {
    id: "validator_set",
    title: "Validator Set Info",
    description:
      "Current quorums, validator ProTx hashes, node IPs, and last proposer.",
    icon: Users,
    dispatch: () => commands.platformValidatorSetInfo(),
  },
  {
    id: "withdrawals_queue",
    title: "Withdrawals in Queue",
    description:
      "Pending withdrawals with daily limit calculations and remaining today.",
    icon: ArrowDownToLine,
    dispatch: () => commands.platformWithdrawalsInQueue(),
  },
  {
    id: "recent_withdrawals",
    title: "Recently Completed Withdrawals",
    description:
      "Most recent completed or expired withdrawals with transaction details.",
    icon: CheckCircle,
    dispatch: () => commands.platformRecentlyCompletedWithdrawals(),
  },
];

/**
 * Platform Info screen — fetch and display various platform data queries.
 *
 * Two-column layout: left shows query-type cards that dispatch IPC commands,
 * right shows formatted results with copy-to-clipboard. Loading states are
 * tracked per-query to prevent duplicate concurrent requests.
 */
export function PlatformInfoScreen() {
  const [loadingId, setLoadingId] = useState<string | null>(null);
  const [resultTitle, setResultTitle] = useState<string | null>(null);
  const [resultText, setResultText] = useState<string | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  // Track active task ID and pending query as refs (not needed for rendering)
  const activeTaskIdRef = useRef<string | null>(null);
  const pendingQueryRef = useRef<string | null>(null);

  // Subscribe to task result / error events
  useEffect(() => {
    let cleanupResult: (() => void) | undefined;
    let cleanupError: (() => void) | undefined;

    const subscribe = async () => {
      cleanupResult = await events.taskResultEvent.listen(
        (event: { payload: TaskResultEvent }) => {
          const { taskId, resultType, payload } = event.payload;

          // Only handle Platform results for our active task
          if (resultType !== "Platform") return;
          if (activeTaskIdRef.current !== taskId) return;

          // Parse the platform result payload
          if (payload && typeof payload === "object") {
            const p = payload as Record<string, unknown>;
            if (p.type === "text") {
              setResultTitle((p.title as string) || "Platform Information");
              setResultText((p.data as string) || "");
            }
          }

          setLoadingId(null);
          setErrorMessage(null);
          activeTaskIdRef.current = null;
          pendingQueryRef.current = null;
        },
      );

      cleanupError = await events.taskErrorEvent.listen(
        (event: { payload: TaskErrorEvent }) => {
          const { taskId, message } = event.payload;
          if (activeTaskIdRef.current !== taskId) return;

          setErrorMessage(message);
          setLoadingId(null);
          activeTaskIdRef.current = null;
          pendingQueryRef.current = null;
          toast.error(message);
        },
      );
    };

    subscribe().catch(() => {});

    return () => {
      cleanupResult?.();
      cleanupError?.();
    };
  }, []);

  const handleQuery = useCallback(
    async (query: QueryType) => {
      // Prevent duplicate requests
      if (loadingId) return;

      setLoadingId(query.id);
      setErrorMessage(null);
      pendingQueryRef.current = query.id;

      try {
        const response = await query.dispatch();
        activeTaskIdRef.current = response.taskId;
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        setErrorMessage(msg);
        setLoadingId(null);
        pendingQueryRef.current = null;
        toast.error(msg);
      }
    },
    [loadingId],
  );

  return (
    <ToolPageLayout
      title="Platform Information"
      subtitle="Fetch and inspect platform data, epoch info, credits, validators, and withdrawals"
    >
      <div className="flex gap-6 min-h-0 h-full">
        {/* Left column: query cards */}
        <div className="w-72 flex-shrink-0 space-y-2 overflow-y-auto">
          {QUERY_TYPES.map((query) => {
            const Icon = query.icon;
            const isLoading = loadingId === query.id;
            const isDisabled = loadingId !== null;

            return (
              <button
                key={query.id}
                type="button"
                onClick={() => handleQuery(query)}
                disabled={isDisabled}
                className={cn(
                  "w-full flex items-start gap-3 rounded-lg border p-4 text-left",
                  "transition-all",
                  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
                  "disabled:cursor-not-allowed disabled:opacity-50",
                  !isDisabled &&
                    "hover:border-primary/30 hover:bg-accent/50",
                  isLoading && "border-primary/40 bg-accent/30",
                )}
                aria-busy={isLoading}
              >
                <div
                  className={cn(
                    "mt-0.5 flex size-8 items-center justify-center rounded-md",
                    "bg-primary/10 text-primary",
                  )}
                >
                  {isLoading ? (
                    <Loader2 className="size-4 animate-spin" />
                  ) : (
                    <Icon className="size-4" />
                  )}
                </div>
                <div className="min-w-0 flex-1">
                  <h3 className="text-sm font-semibold leading-tight">
                    {query.title}
                  </h3>
                  <p className="mt-1 text-xs text-muted-foreground leading-relaxed">
                    {query.description}
                  </p>
                </div>
              </button>
            );
          })}
        </div>

        {/* Right column: results */}
        <div className="flex-1 min-w-0 flex flex-col gap-4">
          {/* Error banner */}
          {errorMessage && (
            <div className="flex items-start gap-3 rounded-lg border border-destructive/30 bg-destructive/5 p-4">
              <div className="flex-1 min-w-0">
                <p className="text-sm font-medium text-destructive">Error</p>
                <p className="mt-1 text-sm text-destructive/80">
                  {errorMessage}
                </p>
              </div>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setErrorMessage(null)}
                className="flex-shrink-0 text-destructive hover:text-destructive"
              >
                Dismiss
              </Button>
            </div>
          )}

          {/* Loading state */}
          {loadingId && !resultText && !errorMessage && (
            <div className="flex flex-1 items-center justify-center">
              <div className="flex flex-col items-center gap-3 text-muted-foreground">
                <Loader2 className="size-8 animate-spin" />
                <p className="text-sm">Loading...</p>
              </div>
            </div>
          )}

          {/* Result display */}
          {resultText && (
            <MonospaceOutput
              value={resultText}
              label={resultTitle || "Result"}
              maxHeight={600}
              className="flex-1"
            />
          )}

          {/* Empty state */}
          {!loadingId && !resultText && !errorMessage && (
            <div className="flex flex-1 items-center justify-center">
              <p className="text-sm text-muted-foreground">
                Select a query from the left to fetch platform data.
              </p>
            </div>
          )}
        </div>
      </div>
    </ToolPageLayout>
  );
}
