import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { RefreshCw, PlusCircle, Search, Coins } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Island } from "@/components/layout";
import { LoadingSpinner } from "@/components/feedback";
import { MyTokensTable } from "@/components/token/MyTokensTable";
import type { TokenAction, RewardEstimate } from "@/components/token/MyTokensTable";
import { TokenInfoDialog } from "@/components/token/TokenInfoDialog";
import type { TokenInfoData } from "@/components/token/TokenInfoDialog";
import { useTokenStore } from "@/stores/tokenStore";
import type { TokenEntry } from "@/stores/tokenStore";
import { commands, events } from "@/bindings";
import type { TaskResultEvent, TaskErrorEvent } from "@/bindings";
import { toastError } from "@/lib/toastError";

/**
 * Map a TokenEntry to the TokenInfoData expected by TokenInfoDialog.
 * Only the fields available in TokenEntry are populated.
 */
function toTokenInfoData(entry: TokenEntry): TokenInfoData {
  return {
    name: entry.name,
    tokenId: entry.tokenId,
    contractId: entry.contractId,
    tokenPosition: entry.tokenPosition,
    decimals: entry.decimals,
    ownerIdentityId: entry.identityId,
  };
}

/**
 * Map a TokenAction from the table into a route path under /tokens/.
 * Returns null for actions handled inline (moreInfo, remove).
 */
function actionToRoute(action: TokenAction): string | null {
  switch (action) {
    case "transfer":
      return "/tokens/transfer";
    case "mint":
      return "/tokens/mint";
    case "burn":
      return "/tokens/burn";
    case "freeze":
      return "/tokens/freeze";
    case "unfreeze":
      return "/tokens/unfreeze";
    case "destroyFrozen":
      return "/tokens/destroy-frozen";
    case "pause":
      return "/tokens/pause";
    case "resume":
      return "/tokens/resume";
    case "claim":
      return "/tokens/claim";
    case "viewClaims":
      return "/tokens/view-claims";
    case "setPrice":
      return "/tokens/set-price";
    case "purchase":
      return "/tokens/purchase";
    case "updateConfig":
      return "/tokens/update-config";
    case "moreInfo":
    case "remove":
      return null;
    default:
      return null;
  }
}

export function TokenMyTokensScreen() {
  const navigate = useNavigate();

  // Token store
  const {
    tokens,
    loading,
    refreshing,
    error,
    sortColumn,
    sortOrder,
    loadMyTokenBalances,
    setSortColumn,
    removeToken,
    reorderTokens,
    subscribeToUpdates,
    clearError,
  } = useTokenStore();

  // Token info dialog state
  const [infoDialogOpen, setInfoDialogOpen] = useState(false);
  const [selectedTokenInfo, setSelectedTokenInfo] =
    useState<TokenInfoData | null>(null);

  // ── Rewards estimation state ──────────────────────────────────────
  const [showRewardsColumn, setShowRewardsColumn] = useState(false);
  const [rewardEstimates, setRewardEstimates] = useState<Map<string, RewardEstimate>>(new Map());
  const [estimatingRewards, setEstimatingRewards] = useState<Set<string>>(new Set());
  // Map taskId → "identityId:tokenId" for correlating async results
  const estimateTaskMapRef = useRef<Map<string, string>>(new Map());

  // Check if developer mode is enabled (always show rewards column in dev mode)
  useEffect(() => {
    commands.contextIsDeveloperMode().then((isDev) => {
      if (isDev) {
        setShowRewardsColumn(true);
      }
    }).catch(() => {});
  }, []);

  // Subscribe to estimate result/error events
  useEffect(() => {
    let cleanupResult: (() => void) | undefined;
    let cleanupError: (() => void) | undefined;

    const subscribe = async () => {
      cleanupResult = await events.taskResultEvent.listen(
        (event: { payload: TaskResultEvent }) => {
          const { taskId, result } = event.payload;
          const key = estimateTaskMapRef.current.get(taskId);
          if (!key) return;
          if (result.type !== "tokenRewardEstimate") return;

          estimateTaskMapRef.current.delete(taskId);
          setEstimatingRewards((prev) => {
            const next = new Set(prev);
            next.delete(key);
            return next;
          });

          setRewardEstimates((prev) => {
            const next = new Map(prev);
            next.set(key, {
              amount: result.amount,
              explanation: result.explanation,
            });
            return next;
          });
        },
      );

      cleanupError = await events.taskErrorEvent.listen(
        (event: { payload: TaskErrorEvent }) => {
          const { taskId, message } = event.payload;
          const key = estimateTaskMapRef.current.get(taskId);
          if (!key) return;

          estimateTaskMapRef.current.delete(taskId);
          setEstimatingRewards((prev) => {
            const next = new Set(prev);
            next.delete(key);
            return next;
          });
          toastError(message);
        },
      );
    };

    subscribe().catch(() => {});

    return () => {
      cleanupResult?.();
      cleanupError?.();
    };
  }, []);

  // Handle estimate rewards request
  const handleEstimateRewards = useCallback(async (identityId: string, tokenId: string) => {
    const key = `${identityId}:${tokenId}`;
    setEstimatingRewards((prev) => new Set(prev).add(key));

    try {
      const result = await commands.tokenEstimatePerpetualRewards({
        identityId,
        tokenId,
      });
      if (result.status === "ok") {
        estimateTaskMapRef.current.set(result.data.taskId, key);
      } else {
        setEstimatingRewards((prev) => {
          const next = new Set(prev);
          next.delete(key);
          return next;
        });
      }
    } catch {
      setEstimatingRewards((prev) => {
        const next = new Set(prev);
        next.delete(key);
        return next;
      });
    }
  }, []);

  // Load tokens on mount
  useEffect(() => {
    loadMyTokenBalances();
  }, [loadMyTokenBalances]);

  // Subscribe to real-time updates
  useEffect(() => {
    let unsubscribe: (() => void) | undefined;
    subscribeToUpdates()
      .then((unsub) => {
        unsubscribe = unsub;
      })
      .catch((e) => console.error("Failed to subscribe to token events:", e));
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

  // Handle refresh
  const handleRefresh = useCallback(() => {
    loadMyTokenBalances();
  }, [loadMyTokenBalances]);

  // Handle token action from Level 2 detail view (has full entry with identityId)
  const handleAction = useCallback(
    (entry: TokenEntry, action: TokenAction) => {
      const route = actionToRoute(action);
      if (route) {
        navigate({
          to: route,
          search: {
            tokenId: entry.tokenId,
            contractId: entry.contractId,
            tokenPosition: String(entry.tokenPosition),
            identityId: entry.identityId,
          },
        });
      }
    },
    [navigate],
  );

  // Handle "More Info" (token-level, no specific identity needed)
  const handleMoreInfo = useCallback(
    (tokenId: string) => {
      const entry = tokens.find((t) => t.tokenId === tokenId);
      if (entry) {
        setSelectedTokenInfo(toTokenInfoData(entry));
        setInfoDialogOpen(true);
      }
    },
    [tokens],
  );

  // Handle remove
  const handleRemove = useCallback(
    (tokenId: string) => {
      removeToken(tokenId);
    },
    [removeToken],
  );

  const isLoading = loading && tokens.length === 0;

  return (
    <div className="flex flex-1 flex-col gap-4 p-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">My Tokens</h1>
          <p className="text-sm text-muted-foreground">
            Manage your token balances and perform token operations.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={handleRefresh}
            disabled={loading || refreshing}
          >
            {loading || refreshing ? (
              <LoadingSpinner className="mr-2 h-4 w-4" />
            ) : (
              <RefreshCw className="mr-2 h-4 w-4" />
            )}
            Refresh
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => navigate({ to: "/tokens/add-by-id" })}
          >
            <PlusCircle className="mr-2 h-4 w-4" />
            Add Token by ID
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => navigate({ to: "/tokens/search" })}
          >
            <Search className="mr-2 h-4 w-4" />
            Search Tokens
          </Button>
          <Button
            size="sm"
            onClick={() => navigate({ to: "/tokens/creator" })}
          >
            <Coins className="mr-2 h-4 w-4" />
            Create Token
          </Button>
        </div>
      </div>

      {/* Content */}
      <Island>
        {isLoading ? (
          <div className="flex items-center justify-center py-16">
            <LoadingSpinner className="h-8 w-8" />
            <span className="ml-3 text-muted-foreground">
              Loading tokens...
            </span>
          </div>
        ) : (
          <MyTokensTable
            tokens={tokens}
            sortColumn={sortColumn}
            sortOrder={sortOrder}
            onSortChange={setSortColumn}
            onAction={handleAction}
            onMoreInfo={handleMoreInfo}
            onRemove={handleRemove}
            showRewardsColumn={showRewardsColumn}
            rewardEstimates={rewardEstimates}
            estimatingRewards={estimatingRewards}
            onEstimateRewards={handleEstimateRewards}
            onReorder={reorderTokens}
          />
        )}
      </Island>

      {/* Token Info Dialog */}
      <TokenInfoDialog
        open={infoDialogOpen}
        onOpenChange={setInfoDialogOpen}
        token={selectedTokenInfo}
      />
    </div>
  );
}
