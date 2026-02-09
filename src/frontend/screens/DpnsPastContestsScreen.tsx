import { useCallback, useEffect } from "react";
import { RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Island } from "@/components/layout";
import { LoadingSpinner } from "@/components/feedback";
import { PastContestsTable } from "@/components/dpns/PastContestsTable";
import { useContestStore } from "@/stores/contestStore";
import { toast } from "sonner";

export function DpnsPastContestsScreen() {
  // Contest store
  const {
    contestedNames,
    loading,
    refreshing,
    error,
    pastFilterTerm,
    sortColumn,
    sortOrder,
    loadContests,
    setFilterTerm,
    setSortColumn,
    subscribeToUpdates,
    clearError,
  } = useContestStore();

  // Load data on mount
  useEffect(() => {
    loadContests();
  }, [loadContests]);

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
      toast.error(error);
      clearError();
    }
  }, [error, clearError]);

  const handleRefresh = useCallback(() => {
    loadContests();
  }, [loadContests]);

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
          <h1 className="text-2xl font-bold tracking-tight">Past Contests</h1>
          <p className="text-sm text-muted-foreground">
            Historical DPNS name contests that have ended.
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
        </div>
      </div>

      {/* Content */}
      <Island noPadding className="flex-1 overflow-auto p-4">
        <PastContestsTable
          contestedNames={contestedNames}
          filterTerm={pastFilterTerm}
          sortColumn={sortColumn}
          sortOrder={sortOrder}
          onFilterChange={(term) => setFilterTerm("past", term)}
          onSortChange={setSortColumn}
        />
      </Island>
    </div>
  );
}
