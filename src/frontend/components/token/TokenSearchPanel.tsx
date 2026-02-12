import { useState, useCallback, useRef, useEffect } from "react";
import {
  Search,
  X,
  ChevronLeft,
  ChevronRight,
  ExternalLink,
  PlusCircle,
  Loader2,
  FileSearch,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { EmptyState } from "@/components/feedback/EmptyState";
import { LoadingSpinner } from "@/components/feedback";
import { CopyButton } from "@/components/shared/CopyButton";
import { JsonViewer } from "@/components/shared/JsonViewer";
import { displayId, hexToBase58 } from "@/lib/utils";

// ─── Types ──────────────────────────────────────────────────────────

export interface SearchResult {
  contractId: string;
  description: string;
}

export interface ContractDetailToken {
  tokenId: string;
  name: string;
  description: string | null;
  tokenPosition?: number;
  configurationJson?: unknown;
}

export interface ContractDetail {
  contractId: string;
  description: string;
  tokens: ContractDetailToken[];
}

export interface TokenSearchPanelProps {
  /** Current search results. */
  results: SearchResult[];
  /** Whether a search is in progress. */
  searching: boolean;
  /** Whether more results are available (next page). */
  hasMore: boolean;
  /** Current page number (1-based). */
  currentPage: number;
  /** Called when user submits a keyword search. */
  onSearch: (keyword: string) => void;
  /** Called when user clicks Next page. */
  onNextPage: () => void;
  /** Called when user clicks Previous page. */
  onPreviousPage: () => void;
  /** Called when user clicks Clear. */
  onClear: () => void;
  /** Called when "More Info" is clicked for a contract. */
  onMoreInfo: (contractId: string) => void;
  /** Currently selected contract detail (null if none). */
  contractDetail: ContractDetail | null;
  /** Whether contract detail is loading. */
  contractDetailLoading: boolean;
  /** Called when user clicks "Add to My Tokens" for a token. */
  onAddToken: (token: ContractDetailToken) => void;
  /** Called to go back from contract detail to search results. */
  onBackToResults: () => void;
  /** Token name currently being added (for loading state). */
  addingTokenName: string | null;
}

// ─── Search Bar ─────────────────────────────────────────────────────

function SearchBar({
  onSearch,
  onClear,
  searching,
  hasResults,
}: {
  onSearch: (keyword: string) => void;
  onClear: () => void;
  searching: boolean;
  hasResults: boolean;
}) {
  const [keyword, setKeyword] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  const handleSubmit = useCallback(() => {
    const trimmed = keyword.trim();
    if (trimmed) {
      onSearch(trimmed);
    }
  }, [keyword, onSearch]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        handleSubmit();
      }
    },
    [handleSubmit],
  );

  const handleClear = useCallback(() => {
    setKeyword("");
    onClear();
    inputRef.current?.focus();
  }, [onClear]);

  return (
    <div className="flex items-center gap-2">
      <div className="relative flex-1">
        <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          ref={inputRef}
          type="text"
          placeholder="Enter keyword to search tokens..."
          value={keyword}
          onChange={(e) => setKeyword(e.target.value)}
          onKeyDown={handleKeyDown}
          className="pl-9"
          aria-label="Token search keyword"
        />
      </div>
      <Button
        onClick={handleSubmit}
        disabled={searching || !keyword.trim()}
        size="default"
      >
        {searching ? (
          <Loader2 className="mr-2 h-4 w-4 animate-spin" />
        ) : (
          <Search className="mr-2 h-4 w-4" />
        )}
        Search
      </Button>
      {(hasResults || keyword) && (
        <Button variant="outline" onClick={handleClear} disabled={searching}>
          <X className="mr-2 h-4 w-4" />
          Clear
        </Button>
      )}
    </div>
  );
}

// ─── Search Results Table ───────────────────────────────────────────

function SearchResultsTable({
  results,
  onMoreInfo,
}: {
  results: SearchResult[];
  onMoreInfo: (contractId: string) => void;
}) {
  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead className="w-[200px]">Contract ID</TableHead>
          <TableHead>Description</TableHead>
          <TableHead className="w-[120px] text-right">Action</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {results.map((result) => (
          <TableRow key={result.contractId}>
            <TableCell>
              <Tooltip>
                <TooltipTrigger asChild>
                  <span className="font-mono text-sm cursor-default">
                    {displayId(result.contractId)}
                  </span>
                </TooltipTrigger>
                <TooltipContent side="bottom">
                  <p className="font-mono text-xs">{hexToBase58(result.contractId)}</p>
                </TooltipContent>
              </Tooltip>
            </TableCell>
            <TableCell>
              <span className="text-sm">
                {result.description || "No description"}
              </span>
            </TableCell>
            <TableCell className="text-right">
              <Button
                variant="outline"
                size="sm"
                onClick={() => onMoreInfo(result.contractId)}
              >
                <ExternalLink className="mr-1.5 h-3.5 w-3.5" />
                More Info
              </Button>
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  );
}

// ─── Pagination Controls ────────────────────────────────────────────

function PaginationControls({
  currentPage,
  hasMore,
  searching,
  onPreviousPage,
  onNextPage,
}: {
  currentPage: number;
  hasMore: boolean;
  searching: boolean;
  onPreviousPage: () => void;
  onNextPage: () => void;
}) {
  if (currentPage <= 1 && !hasMore) return null;

  return (
    <div className="flex items-center justify-center gap-3 pt-3">
      <Button
        variant="outline"
        size="sm"
        onClick={onPreviousPage}
        disabled={currentPage <= 1 || searching}
      >
        <ChevronLeft className="mr-1 h-4 w-4" />
        Previous
      </Button>
      <span className="text-sm text-muted-foreground">
        Page {currentPage}
      </span>
      <Button
        variant="outline"
        size="sm"
        onClick={onNextPage}
        disabled={!hasMore || searching}
      >
        Next
        <ChevronRight className="ml-1 h-4 w-4" />
      </Button>
    </div>
  );
}

// ─── Contract Detail View ───────────────────────────────────────────

function ContractDetailView({
  detail,
  loading,
  onBack,
  onAddToken,
  addingTokenName,
}: {
  detail: ContractDetail | null;
  loading: boolean;
  onBack: () => void;
  onAddToken: (token: ContractDetailToken) => void;
  addingTokenName: string | null;
}) {
  const [schemaToken, setSchemaToken] = useState<ContractDetailToken | null>(
    null,
  );

  if (loading) {
    return (
      <div className="space-y-4">
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ChevronLeft className="mr-1 h-4 w-4" />
          Back to Search Results
        </Button>
        <div className="flex items-center justify-center py-12">
          <LoadingSpinner className="h-6 w-6" />
          <span className="ml-3 text-muted-foreground">
            Loading contract details...
          </span>
        </div>
      </div>
    );
  }

  if (!detail) return null;

  return (
    <div className="space-y-4">
      <Button variant="ghost" size="sm" onClick={onBack}>
        <ChevronLeft className="mr-1 h-4 w-4" />
        Back to Search Results
      </Button>

      {/* Contract Header */}
      <div className="space-y-2">
        <h3 className="text-lg font-semibold">Contract Details</h3>
        <div className="flex items-center gap-2">
          <span className="text-sm text-muted-foreground">Contract ID:</span>
          <span className="font-mono text-sm">{displayId(detail.contractId)}</span>
          <CopyButton value={hexToBase58(detail.contractId)} />
        </div>
        <p className="text-sm text-muted-foreground">
          {detail.description || "No description"}
        </p>
      </div>

      {/* Tokens Section */}
      <div className="space-y-3">
        <h4 className="text-sm font-semibold text-muted-foreground uppercase tracking-wider">
          Tokens
        </h4>
        {detail.tokens.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No tokens found in this contract.
          </p>
        ) : (
          detail.tokens.map((token) => (
            <div
              key={token.tokenId}
              className="rounded-lg border bg-card p-4 space-y-2"
            >
              <div className="flex items-center justify-between">
                <h5 className="text-base font-medium">{token.name}</h5>
                <div className="flex items-center gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => onAddToken(token)}
                    disabled={addingTokenName !== null}
                  >
                    {addingTokenName === token.name ? (
                      <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <PlusCircle className="mr-1.5 h-3.5 w-3.5" />
                    )}
                    Add to My Tokens
                  </Button>
                  {!!token.configurationJson && (
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setSchemaToken(token)}
                    >
                      View Schema
                    </Button>
                  )}
                </div>
              </div>
              <div className="flex items-center gap-2">
                <span className="text-sm text-muted-foreground">ID:</span>
                <span className="font-mono text-sm">
                  {displayId(token.tokenId)}
                </span>
                <CopyButton value={hexToBase58(token.tokenId)} />
              </div>
              <p className="text-sm text-muted-foreground">
                {token.description || "No description"}
              </p>
            </div>
          ))
        )}
      </div>

      {/* Schema Viewer (inline, below the token cards) */}
      {schemaToken && (
        <div className="space-y-2 rounded-lg border bg-muted/30 p-4">
          <div className="flex items-center justify-between">
            <h5 className="text-sm font-semibold">
              {schemaToken.name} — Configuration Schema
            </h5>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setSchemaToken(null)}
            >
              <X className="h-4 w-4" />
            </Button>
          </div>
          <JsonViewer
            data={schemaToken.configurationJson}
            defaultExpanded
            expandDepth={3}
            showCopy
            className="max-h-[400px]"
          />
        </div>
      )}
    </div>
  );
}

// ─── Elapsed Timer ──────────────────────────────────────────────────

function ElapsedTimer() {
  const [elapsed, setElapsed] = useState(0);
  const startRef = useRef<number>(0);

  useEffect(() => {
    startRef.current = Date.now();
    const interval = setInterval(() => {
      setElapsed(Math.floor((Date.now() - startRef.current) / 1000));
    }, 1000);
    return () => clearInterval(interval);
  }, []);

  if (elapsed < 1) return null;
  return (
    <span className="text-sm text-muted-foreground">
      {elapsed} {elapsed === 1 ? "second" : "seconds"}
    </span>
  );
}

// ─── Main Component ─────────────────────────────────────────────────

export function TokenSearchPanel({
  results,
  searching,
  hasMore,
  currentPage,
  onSearch,
  onNextPage,
  onPreviousPage,
  onClear,
  onMoreInfo,
  contractDetail,
  contractDetailLoading,
  onAddToken,
  onBackToResults,
  addingTokenName,
}: TokenSearchPanelProps) {
  const hasResults = results.length > 0;
  const showDetail = contractDetail !== null || contractDetailLoading;

  return (
    <div className="space-y-4">
      {/* Search Bar — always visible */}
      {!showDetail && (
        <SearchBar
          onSearch={onSearch}
          onClear={onClear}
          searching={searching}
          hasResults={hasResults}
        />
      )}

      {/* Contract Detail View */}
      {showDetail ? (
        <ContractDetailView
          detail={contractDetail}
          loading={contractDetailLoading}
          onBack={onBackToResults}
          onAddToken={onAddToken}
          addingTokenName={addingTokenName}
        />
      ) : (
        <>
          {/* Searching state */}
          {searching && (
            <div className="flex items-center justify-center gap-3 py-12">
              <LoadingSpinner className="h-6 w-6" />
              <div className="flex flex-col items-center gap-1">
                <span className="text-muted-foreground">Searching...</span>
                <ElapsedTimer />
              </div>
            </div>
          )}

          {/* Results */}
          {!searching && hasResults && (
            <>
              <SearchResultsTable results={results} onMoreInfo={onMoreInfo} />
              <PaginationControls
                currentPage={currentPage}
                hasMore={hasMore}
                searching={searching}
                onPreviousPage={onPreviousPage}
                onNextPage={onNextPage}
              />
            </>
          )}

          {/* Empty state after search completes with no results */}
          {!searching && !hasResults && currentPage > 0 && (
            <EmptyState
              icon={FileSearch}
              title="No tokens match your keyword"
              description="Try a different search term or browse tokens by ID."
            />
          )}

          {/* Initial state (no search yet) */}
          {!searching && !hasResults && currentPage === 0 && (
            <EmptyState
              icon={Search}
              title="Search for tokens"
              description="Enter a keyword above to search for tokens on the Dash Platform."
            />
          )}
        </>
      )}
    </div>
  );
}
