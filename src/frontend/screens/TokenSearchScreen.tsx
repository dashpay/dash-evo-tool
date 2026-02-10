import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { ArrowLeft } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Island } from "@/components/layout";
import { TokenSearchPanel } from "@/components/token/TokenSearchPanel";
import type {
  ContractDetail,
  ContractDetailToken,
} from "@/components/token/TokenSearchPanel";
import { useTokenStore } from "@/stores/tokenStore";
import { commands, events } from "@/bindings";
import type { TaskResultEvent } from "@/bindings";
import { toastError } from "@/lib/toastError";
import { toast } from "sonner";

/**
 * TokenSearchScreen — keyword search for tokens on the Dash Platform.
 *
 * Features:
 * - Keyword search with pagination
 * - "More Info" expands to contract detail with token list
 * - "Add to My Tokens" saves a token locally
 * - "View Schema" shows token configuration as JSON
 */
export function TokenSearchScreen() {
  const navigate = useNavigate();

  // Token store state
  const {
    searchResults,
    searching,
    searchHasMore,
    error,
    searchByKeyword,
    searchNextPage,
    clearSearch,
    subscribeToUpdates,
    clearError,
    loadMyTokenBalances,
  } = useTokenStore();

  // Local state for pagination tracking
  const [currentPage, setCurrentPage] = useState(0);
  const previousCursorsRef = useRef<(string | null)[]>([]);

  // Contract detail state
  const [contractDetail, setContractDetail] = useState<ContractDetail | null>(
    null,
  );
  const [contractDetailLoading, setContractDetailLoading] = useState(false);
  const [addingTokenName, setAddingTokenName] = useState<string | null>(null);

  // Subscribe to updates
  useEffect(() => {
    let unsubscribe: (() => void) | undefined;
    subscribeToUpdates().then((unsub) => {
      unsubscribe = unsub;
    });
    return () => {
      unsubscribe?.();
    };
  }, [subscribeToUpdates]);

  // Error handling
  useEffect(() => {
    if (error) {
      toastError(error);
      clearError();
    }
  }, [error, clearError]);

  // Contract detail event listener
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    events.taskResultEvent
      .listen((event: { payload: TaskResultEvent }) => {
        if (cancelled) return;
        const { resultType, payload } = event.payload;

        // Handle contract fetch results for the detail view
        if (resultType === "Contract" && contractDetailLoading) {
          const p = payload as Record<string, unknown>;
          // Extract contracts from the payload
          const contracts = (p.contracts ?? p.data ?? []) as unknown[];
          if (contracts.length > 0) {
            const contract = contracts[0] as Record<string, unknown>;
            const tokens = ((contract.tokens ?? []) as unknown[]).map(
              (t: unknown) => {
                const tk = t as Record<string, unknown>;
                return {
                  tokenId: (tk.tokenId ?? tk.token_id ?? "") as string,
                  name: (tk.name ?? tk.token_name ?? "Unnamed Token") as string,
                  description: (tk.description ?? null) as string | null,
                  configurationJson:
                    tk.configurationJson ??
                    tk.configuration_json ??
                    tk.token_configuration ??
                    undefined,
                };
              },
            );

            setContractDetail({
              contractId: (contract.contractId ??
                contract.contract_id ??
                "") as string,
              description: (contract.description ?? "") as string,
              tokens,
            });
          }
          setContractDetailLoading(false);
        }

        // Handle token save results
        if (resultType === "Token" && addingTokenName !== null) {
          toast.success(`"${addingTokenName}" added to My Tokens`);
          setAddingTokenName(null);
          // Refresh token list
          loadMyTokenBalances();
        }
      })
      .then((unsub) => {
        unlisten = unsub;
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [contractDetailLoading, addingTokenName, loadMyTokenBalances]);

  // Handlers
  const handleSearch = useCallback(
    (keyword: string) => {
      setCurrentPage(1);
      previousCursorsRef.current = [];
      setContractDetail(null);
      setContractDetailLoading(false);
      searchByKeyword(keyword);
    },
    [searchByKeyword],
  );

  const handleNextPage = useCallback(() => {
    // Save current cursor for going back
    const currentCursor = useTokenStore.getState().searchCursor;
    previousCursorsRef.current.push(currentCursor);
    setCurrentPage((p) => p + 1);
    searchNextPage();
  }, [searchNextPage]);

  const handlePreviousPage = useCallback(() => {
    const prevCursors = previousCursorsRef.current;
    if (prevCursors.length > 0) {
      prevCursors.pop();
    }
    setCurrentPage((p) => Math.max(1, p - 1));
    // Re-search from appropriate cursor
    const keyword = useTokenStore.getState().searchKeyword;
    const cursor =
      prevCursors.length > 0 ? prevCursors[prevCursors.length - 1] : null;
    if (keyword) {
      commands.tokenQueryDescriptionsByKeyword({
        keyword,
        startAfter: cursor,
      });
    }
  }, []);

  const handleClear = useCallback(() => {
    setCurrentPage(0);
    previousCursorsRef.current = [];
    setContractDetail(null);
    setContractDetailLoading(false);
    setAddingTokenName(null);
    clearSearch();
  }, [clearSearch]);

  const handleMoreInfo = useCallback((contractId: string) => {
    setContractDetailLoading(true);
    setContractDetail(null);
    commands.contractFetchWithDescriptions({ contractIds: [contractId] });
  }, []);

  const handleBackToResults = useCallback(() => {
    setContractDetail(null);
    setContractDetailLoading(false);
  }, []);

  const handleAddToken = useCallback(
    (token: ContractDetailToken) => {
      // Check if already in My Tokens
      const myTokens = useTokenStore.getState().tokens;
      const exists = myTokens.some((t) => t.tokenId === token.tokenId);
      if (exists) {
        toast.info("Token already in My Tokens");
        return;
      }

      setAddingTokenName(token.name);

      // Build token info JSON matching the backend's expected structure
      const tokenInfo = {
        token_id: token.tokenId,
        token_name: token.name,
        data_contract_id: contractDetail?.contractId ?? "",
        token_position: 0,
        token_configuration: token.configurationJson ?? {},
        description: token.description,
      };

      commands.tokenSaveLocally({ tokenInfoJson: tokenInfo }).then((result) => {
        if (result.status !== "ok") {
          setAddingTokenName(null);
          toastError(result.error);
        }
      });
    },
    [contractDetail],
  );

  return (
    <div className="flex flex-1 flex-col gap-4 p-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <Button
            variant="ghost"
            size="icon"
            onClick={() => navigate({ to: "/tokens" })}
            aria-label="Back to My Tokens"
          >
            <ArrowLeft className="h-5 w-5" />
          </Button>
          <div>
            <h1 className="text-2xl font-bold tracking-tight">
              Search Tokens
            </h1>
            <p className="text-sm text-muted-foreground">
              Search for tokens by keyword on the Dash Platform.
            </p>
          </div>
        </div>
      </div>

      {/* Content */}
      <Island>
        <TokenSearchPanel
          results={searchResults}
          searching={searching}
          hasMore={searchHasMore}
          currentPage={currentPage}
          onSearch={handleSearch}
          onNextPage={handleNextPage}
          onPreviousPage={handlePreviousPage}
          onClear={handleClear}
          onMoreInfo={handleMoreInfo}
          contractDetail={contractDetail}
          contractDetailLoading={contractDetailLoading}
          onAddToken={handleAddToken}
          onBackToResults={handleBackToResults}
          addingTokenName={addingTokenName}
        />
      </Island>
    </div>
  );
}
