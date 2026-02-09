import { useCallback, useEffect } from "react";
import { RefreshCw, Trash2, CheckCircle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Island } from "@/components/layout";
import { LoadingSpinner } from "@/components/feedback";
import { ScheduledVotesTable } from "@/components/dpns/ScheduledVotesTable";
import { useContestStore } from "@/stores/contestStore";
import type { ScheduledVoteWithStatus } from "@/stores/contestStore";
import { toastError } from "@/lib/toastError";

export function DpnsScheduledVotesScreen() {
  const {
    scheduledVotes,
    loading,
    error,
    scheduledVoteCastInProgress,
    loadScheduledVotes,
    castScheduledVote,
    deleteScheduledVote,
    clearAllScheduledVotes,
    clearExecutedScheduledVotes,
    subscribeToUpdates,
    clearError,
  } = useContestStore();

  // Load scheduled votes on mount
  useEffect(() => {
    loadScheduledVotes();
  }, [loadScheduledVotes]);

  // Subscribe to real-time updates
  useEffect(() => {
    let unsubscribe: (() => void) | undefined;
    subscribeToUpdates().then((unsub) => {
      unsubscribe = unsub;
    });
    return () => {
      unsubscribe?.();
    };
  }, [subscribeToUpdates]);

  // Show error toast
  useEffect(() => {
    if (error) {
      toastError(error);
      clearError();
    }
  }, [error, clearError]);

  const handleRefresh = useCallback(() => {
    loadScheduledVotes();
  }, [loadScheduledVotes]);

  const handleCastNow = useCallback(
    (sv: ScheduledVoteWithStatus) => {
      castScheduledVote(sv.vote);
    },
    [castScheduledVote],
  );

  const handleRemove = useCallback(
    (voterId: string, contestedName: string) => {
      deleteScheduledVote(voterId, contestedName);
    },
    [deleteScheduledVote],
  );

  const handleClearAll = useCallback(() => {
    clearAllScheduledVotes();
  }, [clearAllScheduledVotes]);

  const handleClearCasted = useCallback(() => {
    clearExecutedScheduledVotes();
  }, [clearExecutedScheduledVotes]);

  const hasCompletedVotes = scheduledVotes.some(
    (sv) => sv.castingStatus === "completed",
  );

  // Initial loading state
  if (loading && scheduledVotes.length === 0) {
    return (
      <div className="flex flex-1 items-center justify-center">
        <LoadingSpinner label="Loading scheduled votes..." />
      </div>
    );
  }

  return (
    <div className="flex flex-1 flex-col gap-4 p-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">
            Scheduled Votes
          </h1>
          <p className="text-sm text-muted-foreground">
            Votes scheduled for future execution. Dash Evo Tool must remain
            running for votes to execute on time.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={handleClearCasted}
            disabled={!hasCompletedVotes}
            aria-label="Clear casted votes"
          >
            <CheckCircle className="mr-1.5 h-4 w-4" />
            Clear Casted
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={handleClearAll}
            disabled={scheduledVotes.length === 0}
            aria-label="Clear all scheduled votes"
          >
            <Trash2 className="mr-1.5 h-4 w-4" />
            Clear All
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={handleRefresh}
            aria-label="Refresh scheduled votes"
          >
            <RefreshCw className="mr-1.5 h-4 w-4" />
            Refresh
          </Button>
        </div>
      </div>

      {/* Content */}
      <Island noPadding className="flex-1 overflow-auto p-4">
        <ScheduledVotesTable
          scheduledVotes={scheduledVotes}
          castInProgress={scheduledVoteCastInProgress}
          onRemove={handleRemove}
          onCastNow={handleCastNow}
        />
      </Island>
    </div>
  );
}
