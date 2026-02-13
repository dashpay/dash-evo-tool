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
    subscribeToUpdates()
      .then((unsub) => {
        unsubscribe = unsub;
      })
      .catch((e) => console.error("Failed to subscribe to token events:", e));
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

  // Contract detail event listener — use refs to avoid re-subscribing
  const contractDetailLoadingRef = useRef(contractDetailLoading);
  const addingTokenNameRef = useRef(addingTokenName);
  useEffect(() => { contractDetailLoadingRef.current = contractDetailLoading; }, [contractDetailLoading]);
  useEffect(() => { addingTokenNameRef.current = addingTokenName; }, [addingTokenName]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    const subscribe = async () => {
      unlisten = await events.taskResultEvent.listen((event: { payload: TaskResultEvent }) => {
        if (cancelled) return;
        const { result } = event.payload;

        if (result.type === "contractWithDescriptions" && contractDetailLoadingRef.current) {
          const contracts = result.contracts ?? [];
          if (contracts.length > 0) {
            const contract = contracts[0];
            const tokens = (contract.tokens ?? []).map(
              (tk) => ({
                tokenId: tk.tokenId ?? "",
                name: tk.name ?? "Unnamed Token",
                description: tk.description ?? null,
                tokenPosition: tk.tokenPosition ?? 0,
                configurationJson: tk.configurationJson ?? undefined,
              }),
            );

            setContractDetail({
              contractId: contract.contractId ?? "",
              description: contract.description ?? "",
              tokens,
            });
          }
          setContractDetailLoading(false);
        }

        // Token save is now synchronous — no event listener needed
      });
    };
    subscribe().catch(console.error);

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [loadMyTokenBalances]);

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
      prevCursors.length > 0 ? prevCursors[prevCursors.length - 1] ?? null : null;
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

      commands
        .tokenSaveLocally({
          tokenId: token.tokenId,
          contractId: contractDetail?.contractId ?? "",
          tokenPosition: token.tokenPosition ?? 0,
          tokenName: token.name,
        })
        .then((result) => {
          if (result.status === "ok") {
            toast.success(`"${token.name}" added to My Tokens`);
            setAddingTokenName(null);
            loadMyTokenBalances();
          } else {
            setAddingTokenName(null);
            toastError(result.error);
          }
        });
    },
    [contractDetail, loadMyTokenBalances],
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
