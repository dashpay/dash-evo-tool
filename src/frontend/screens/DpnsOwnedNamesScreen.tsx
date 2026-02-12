import { useCallback, useEffect } from "react";
import { RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Island } from "@/components/layout";
import { LoadingSpinner } from "@/components/feedback";
import { OwnedNamesPanel } from "@/components/dpns/OwnedNamesPanel";
import { useContestStore } from "@/stores/contestStore";
import { commands } from "@/bindings";
import { useIdentityStore } from "@/stores/identityStore";
import { toast } from "sonner";
import { toastError } from "@/lib/toastError";

export function DpnsOwnedNamesScreen() {
  const {
    localDpnsNames,
    loading,
    refreshing,
    error,
    ownedFilterTerm,
    loadLocalNames,
    refreshDpnsNames,
    setFilterTerm,
    subscribeToUpdates,
    clearError,
  } = useContestStore();

  // Load local names on mount
  useEffect(() => {
    loadLocalNames();
  }, [loadLocalNames]);

  // Subscribe to real-time updates (Identity results reload local names)
  useEffect(() => {
    let unsubscribe: (() => void) | undefined;
    subscribeToUpdates()
      .then((unsub) => {
        unsubscribe = unsub;
      })
      .catch((e) => console.error("Failed to subscribe to DPNS events:", e));
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
    refreshDpnsNames();
  }, [refreshDpnsNames]);

  const handleSetAlias = useCallback(
    async (identityId: string, name: string) => {
      // Ensure .dash suffix
      const alias = name.endsWith(".dash") ? name : `${name}.dash`;
      try {
        const result = await commands.identitySetAlias({
          identityId,
          alias,
        });
        if (result.status === "ok") {
          // Also update the identity store's in-memory state so the alias
          // is immediately visible in the Identities screen without refresh.
          useIdentityStore.setState((state) => ({
            identities: state.identities.map((i) =>
              i.id === identityId ? { ...i, alias } : i,
            ),
          }));
          toast.success(`Alias set to "${alias}"`);
        } else {
          toastError(result.error);
        }
      } catch (e) {
        toastError(e instanceof Error ? e.message : String(e));
      }
    },
    [],
  );

  // Initial loading state
  if (loading && localDpnsNames.length === 0) {
    return (
      <div className="flex flex-1 items-center justify-center">
        <LoadingSpinner label="Loading owned names..." />
      </div>
    );
  }

  return (
    <div className="flex flex-1 flex-col gap-4 p-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">My Usernames</h1>
          <p className="text-sm text-muted-foreground">
            DPNS names owned by your loaded identities.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={handleRefresh}
            disabled={refreshing}
            aria-label="Refresh owned names"
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
        <OwnedNamesPanel
          ownedNames={localDpnsNames}
          filterTerm={ownedFilterTerm}
          onFilterChange={(term) => setFilterTerm("owned", term)}
          onSetAlias={handleSetAlias}
        />
      </Island>
    </div>
  );
}
