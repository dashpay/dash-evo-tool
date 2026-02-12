import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { RefreshCw, Vote, PenLine } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Island } from "@/components/layout";
import { LoadingSpinner } from "@/components/feedback";
import { ActiveContestsTable } from "@/components/dpns/ActiveContestsTable";
import { VoteCastingDialog } from "@/components/dpns/VoteCastingDialog";
import { useContestStore } from "@/stores/contestStore";
import { commands } from "@/bindings";
import type { QualifiedIdentityDto, ScheduledVoteDto } from "@/bindings";
import { toast } from "sonner";
import { toastError } from "@/lib/toastError";

export function DpnsActiveContestsScreen() {
  const navigate = useNavigate();

  // Contest store
  const {
    contestedNames,
    selectedVotes,
    loading,
    refreshing,
    votingInProgress,
    error,
    activeFilterTerm,
    sortColumn,
    sortOrder,
    loadContests,
    toggleVote,
    clearSelectedVotes,
    castVotes,
    scheduleVotes,
    setFilterTerm,
    setSortColumn,
    subscribeToUpdates,
    clearError,
  } = useContestStore();

  // Voting identities (loaded separately from identity commands)
  const [votingIdentities, setVotingIdentities] = useState<
    QualifiedIdentityDto[]
  >([]);

  // Dialog state
  const [voteCastingOpen, setVoteCastingOpen] = useState(false);

  // Load data on mount
  useEffect(() => {
    loadContests();
    commands
      .identityListVoting()
      .then((result) => {
        if (result.status === "ok") {
          setVotingIdentities(result.data);
        }
      })
      .catch(() => {
        // Silently fail — voting identities are optional
      });
  }, [loadContests]);

  // Subscribe to real-time updates
  useEffect(() => {
    let unsubscribe: (() => void) | undefined;
    subscribeToUpdates()
      .then((unsub) => {
        unsubscribe = unsub;
      })
      .catch((e) => console.error("Failed to subscribe to contest events:", e));
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
    loadContests();
    commands
      .identityListVoting()
      .then((result) => {
        if (result.status === "ok") {
          setVotingIdentities(result.data);
        }
      })
      .catch(() => {});
  }, [loadContests]);

  const handleRegisterName = useCallback(() => {
    navigate({ to: "/contracts/dpns-register" });
  }, [navigate]);

  const handleOpenVoteDialog = useCallback(() => {
    if (selectedVotes.length === 0) {
      toast.info("Select at least one vote from the table first.");
      return;
    }
    setVoteCastingOpen(true);
  }, [selectedVotes.length]);

  const handleApplyVotes = useCallback(
    async (
      immediateIdentityIds: string[],
      scheduledEntries: Array<{ voterId: string; unixTimestamp: number }>,
    ) => {
      // Cast immediate votes
      if (immediateIdentityIds.length > 0) {
        await castVotes(immediateIdentityIds);
      }

      // Schedule future votes
      if (scheduledEntries.length > 0) {
        const scheduledVoteDtos: ScheduledVoteDto[] = selectedVotes.flatMap(
          (sv) =>
            scheduledEntries.map((entry) => ({
              voterId: entry.voterId,
              contestedName: sv.contestedName,
              choice: sv.choice,
              unixTimestamp: Math.floor(entry.unixTimestamp / 1000),
              executedSuccessfully: false,
            })),
        );
        await scheduleVotes(scheduledVoteDtos);
      }

      // Clear selections after voting
      clearSelectedVotes();
    },
    [castVotes, scheduleVotes, selectedVotes, clearSelectedVotes],
  );

  const handleGoToScheduledVotes = useCallback(() => {
    navigate({ to: "/contracts/dpns-scheduled" });
  }, [navigate]);

  // Initial loading state
  if (loading && contestedNames.length === 0) {
    return (
      <div className="flex flex-1 items-center justify-center">
        <LoadingSpinner label="Loading contests..." />
      </div>
    );
  }

  return (
    <div className="flex flex-1 flex-col gap-4 p-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">
            Active Contests
          </h1>
          <p className="text-sm text-muted-foreground">
            Vote on contested DPNS names using your voting identities.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={handleRefresh}
            disabled={refreshing}
            aria-label="Refresh contests"
          >
            <RefreshCw
              className={`h-4 w-4 mr-1.5 ${refreshing ? "animate-spin" : ""}`}
            />
            Refresh
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={handleRegisterName}
            aria-label="Register DPNS name"
          >
            <PenLine className="h-4 w-4 mr-1.5" />
            Register Name
          </Button>
          <Button
            size="sm"
            onClick={handleOpenVoteDialog}
            disabled={selectedVotes.length === 0}
            aria-label="Cast or schedule votes"
          >
            <Vote className="h-4 w-4 mr-1.5" />
            Cast / Schedule Votes
            {selectedVotes.length > 0 && (
              <span className="ml-1.5 rounded-full bg-primary-foreground/20 px-1.5 py-0.5 text-xs font-medium">
                {selectedVotes.length}
              </span>
            )}
          </Button>
        </div>
      </div>

      {/* Content */}
      <Island noPadding className="flex-1 overflow-auto p-4">
        <ActiveContestsTable
          contestedNames={contestedNames}
          selectedVotes={selectedVotes}
          filterTerm={activeFilterTerm}
          sortColumn={sortColumn}
          sortOrder={sortOrder}
          onFilterChange={(term) => setFilterTerm("active", term)}
          onSortChange={setSortColumn}
          onToggleVote={toggleVote}
        />
      </Island>

      {/* Vote Casting Dialog */}
      <VoteCastingDialog
        open={voteCastingOpen}
        onOpenChange={setVoteCastingOpen}
        selectedVotes={selectedVotes}
        votingIdentities={votingIdentities}
        onApplyVotes={handleApplyVotes}
        onGoToActiveContests={() => setVoteCastingOpen(false)}
        onGoToScheduledVotes={handleGoToScheduledVotes}
        votingInProgress={votingInProgress}
      />
    </div>
  );
}
