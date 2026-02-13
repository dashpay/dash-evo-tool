import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { ArrowLeft, Search, X, Loader2, Coins, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Island } from "@/components/layout";
import { Badge } from "@/components/ui/badge";
import { CopyButton } from "@/components/shared/CopyButton";
import { TokenInfoDialog } from "@/components/token/TokenInfoDialog";
import type { TokenInfoData } from "@/components/token/TokenInfoDialog";
import { useTokenStore } from "@/stores/tokenStore";
import { commands, events } from "@/bindings";
import type { TaskResultEvent } from "@/bindings";
import { toastError } from "@/lib/toastError";
import { toast } from "sonner";
import { displayId, formatElapsed, hexToBase58 } from "@/lib/utils";

// ─── Types ──────────────────────────────────────────────────────────

type SearchStatus = "idle" | "searching" | "found" | "error";

interface FoundToken {
  tokenId: string;
  contractId: string;
  name: string | null;
  description: string | null;
  decimals: number;
  tokenPosition: number;
  baseSupply: string | null;
  maxSupply: string | null;
  ownerIdentityId: string | null;
  paused: boolean;
  configurationJson: unknown;
}

// ─── Component ──────────────────────────────────────────────────────

/**
 * TokenAddByIdScreen — add a token to "My Tokens" by contract ID or token ID.
 *
 * Features:
 * - Input for contract ID or token ID
 * - Search button dispatches fetch by contract ID or token ID
 * - Elapsed time counter during search
 * - Results display with token info
 * - "Add to My Tokens" button per found token
 * - "More Info" opens TokenInfoDialog
 * - Clear button resets to idle
 */
export function TokenAddByIdScreen() {
  const navigate = useNavigate();

  const { loadMyTokenBalances } = useTokenStore();

  // Input state
  const [inputValue, setInputValue] = useState("");

  // Search state
  const [status, setStatus] = useState<SearchStatus>("idle");
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [foundTokens, setFoundTokens] = useState<FoundToken[]>([]);
  const [elapsedMs, setElapsedMs] = useState(0);
  const searchStartRef = useRef<number>(0);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Adding state
  const [addingTokenId, setAddingTokenId] = useState<string | null>(null);

  // Info dialog state
  const [infoDialogOpen, setInfoDialogOpen] = useState(false);
  const [infoDialogToken, setInfoDialogToken] = useState<TokenInfoData | null>(
    null,
  );

  // Clean up timer on unmount
  useEffect(() => {
    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, []);

  // Start elapsed timer
  const startTimer = useCallback(() => {
    searchStartRef.current = Date.now();
    setElapsedMs(0);
    if (timerRef.current) clearInterval(timerRef.current);
    timerRef.current = setInterval(() => {
      setElapsedMs(Date.now() - searchStartRef.current);
    }, 100);
  }, []);

  // Stop elapsed timer
  const stopTimer = useCallback(() => {
    if (timerRef.current) {
      clearInterval(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  // Track mutable state in refs so the listener doesn't re-subscribe
  const statusRef = useRef(status);
  const addingTokenIdRef = useRef(addingTokenId);
  const foundTokensRef = useRef(foundTokens);
  useEffect(() => { statusRef.current = status; }, [status]);
  useEffect(() => { addingTokenIdRef.current = addingTokenId; }, [addingTokenId]);
  useEffect(() => { foundTokensRef.current = foundTokens; }, [foundTokens]);

  // Listen for task result events (subscribe once)
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    let unlistenError: (() => void) | undefined;

    const subscribe = async () => {
      unlisten = await events.taskResultEvent.listen((event: { payload: TaskResultEvent }) => {
        if (cancelled) return;
        const { result } = event.payload;

        // Handle fetched contract with token descriptions (from FetchTokenByContractId / FetchTokenByTokenId)
        if (result.type === "contractWithDescriptions" && statusRef.current === "searching") {
          stopTimer();
          const tokens = mapContractResultToFoundTokens(result.contracts);
          if (tokens.length > 0) {
            setFoundTokens(tokens);
            setStatus("found");
          } else {
            setErrorMessage("No tokens found for the given ID.");
            setStatus("error");
          }
        }

        // Handle token not found
        if (result.type === "tokenNotFound" && statusRef.current === "searching") {
          stopTimer();
          setErrorMessage("No token or contract found for the given ID.");
          setStatus("error");
        }

        // Handle generic token completion (e.g. after saving a token locally)
        if (result.type === "tokenCompleted" && addingTokenIdRef.current !== null) {
          const token = foundTokensRef.current.find((t) => t.tokenId === addingTokenIdRef.current);
          toast.success(`"${token?.name ?? "Token"}" added to My Tokens`);
          setAddingTokenId(null);
          loadMyTokenBalances();
        }
      });

      unlistenError = await events.taskErrorEvent.listen(
        (event: { payload: { taskId: string; message: string } }) => {
          if (cancelled) return;
          if (statusRef.current === "searching") {
            stopTimer();
            setErrorMessage(event.payload.message);
            setStatus("error");
          }
        },
      );
    };
    subscribe().catch(console.error);

    return () => {
      cancelled = true;
      unlisten?.();
      unlistenError?.();
    };
  }, [stopTimer, loadMyTokenBalances]);

  // Handlers
  const handleSearch = useCallback(() => {
    const trimmed = inputValue.trim();
    if (!trimmed) return;

    setStatus("searching");
    setFoundTokens([]);
    setErrorMessage(null);
    startTimer();

    // Pass input directly to backend — parse_identifier handles both hex and base58
    commands
      .tokenFetchByContractId({ contractId: trimmed })
      .then((result) => {
        if (result.status !== "ok") {
          // Try as token ID instead
          return commands.tokenFetchByTokenId({ tokenId: trimmed });
        }
        return result;
      })
      .then((result) => {
        if (result && result.status !== "ok") {
          stopTimer();
          setErrorMessage(result.error);
          setStatus("error");
        }
      })
      .catch((e) => {
        stopTimer();
        setErrorMessage(e instanceof Error ? e.message : String(e));
        setStatus("error");
      });
  }, [inputValue, startTimer, stopTimer]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        handleSearch();
      }
    },
    [handleSearch],
  );

  const handleClear = useCallback(() => {
    setInputValue("");
    setStatus("idle");
    setFoundTokens([]);
    setErrorMessage(null);
    setAddingTokenId(null);
    stopTimer();
  }, [stopTimer]);

  const handleAddToken = useCallback(
    (token: FoundToken) => {
      // Check if already in My Tokens
      const myTokens = useTokenStore.getState().tokens;
      const exists = myTokens.some((t) => t.tokenId === token.tokenId);
      if (exists) {
        toast.info("Token already in My Tokens");
        return;
      }

      setAddingTokenId(token.tokenId);

      commands
        .tokenSaveLocally({
          tokenId: token.tokenId,
          contractId: token.contractId,
          tokenPosition: token.tokenPosition,
          tokenName: token.name ?? "Unnamed Token",
        })
        .then((result) => {
          if (result.status === "ok") {
            toast.success(`"${token.name ?? "Unnamed Token"}" added to My Tokens`);
            setAddingTokenId(null);
            loadMyTokenBalances();
          } else {
            setAddingTokenId(null);
            toastError(result.error);
          }
        });
    },
    [loadMyTokenBalances],
  );

  const handleMoreInfo = useCallback((token: FoundToken) => {
    setInfoDialogToken({
      name: token.name,
      tokenId: token.tokenId,
      contractId: token.contractId,
      tokenPosition: token.tokenPosition,
      description: token.description,
      decimals: token.decimals,
      baseSupply: token.baseSupply,
      maxSupply: token.maxSupply,
      ownerIdentityId: token.ownerIdentityId,
      paused: token.paused,
      configurationJson: token.configurationJson,
    });
    setInfoDialogOpen(true);
  }, []);

  return (
    <div className="flex flex-1 flex-col gap-4 p-4">
      {/* Header */}
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
            Add Token by ID
          </h1>
          <p className="text-sm text-muted-foreground">
            Look up a token by its contract ID or token ID and add it to your
            list.
          </p>
        </div>
      </div>

      {/* Search input */}
      <Island>
        <div className="space-y-4">
          <div className="flex items-center gap-2">
            <Input
              value={inputValue}
              onChange={(e) => setInputValue(e.target.value)}
              onKeyDown={handleKeyDown}
              placeholder="Enter contract ID or token ID..."
              className="font-mono flex-1"
              aria-label="Contract or token ID"
              disabled={status === "searching"}
            />
            <Button
              onClick={handleSearch}
              disabled={
                !inputValue.trim() || status === "searching"
              }
              aria-label="Search"
            >
              {status === "searching" ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <Search className="h-4 w-4" />
              )}
              <span className="ml-1.5">Search</span>
            </Button>
            {(inputValue || status !== "idle") && (
              <Button
                variant="ghost"
                onClick={handleClear}
                aria-label="Clear"
              >
                <X className="h-4 w-4" />
                <span className="ml-1.5">Clear</span>
              </Button>
            )}
          </div>

          {/* Status area */}
          {status === "idle" && (
            <div className="py-8 text-center text-muted-foreground">
              <Coins className="mx-auto mb-3 size-10 opacity-40" />
              <p className="text-sm">
                Enter a contract ID or token ID to search for a token on the
                Dash Platform.
              </p>
            </div>
          )}

          {status === "searching" && (
            <div className="py-8 text-center">
              <Loader2 className="mx-auto mb-3 size-8 animate-spin text-dash-blue" />
              <p className="text-sm text-muted-foreground">
                Searching for token...
              </p>
              <p
                className="mt-1 text-xs text-muted-foreground"
                data-testid="elapsed-time"
              >
                {formatElapsed(elapsedMs)}
              </p>
            </div>
          )}

          {status === "error" && (
            <div className="rounded-lg border border-destructive/50 bg-destructive/5 p-4 text-center">
              <p className="text-sm text-destructive" data-testid="error-message">
                {errorMessage}
              </p>
              <Button
                variant="outline"
                size="sm"
                className="mt-3"
                onClick={handleClear}
              >
                Try Again
              </Button>
            </div>
          )}

          {status === "found" && foundTokens.length > 0 && (
            <div className="space-y-3">
              <p className="text-sm text-muted-foreground">
                Found {foundTokens.length}{" "}
                {foundTokens.length === 1 ? "token" : "tokens"}:
              </p>
              {foundTokens.map((token) => (
                <TokenResultCard
                  key={token.tokenId}
                  token={token}
                  adding={addingTokenId === token.tokenId}
                  onAdd={() => handleAddToken(token)}
                  onMoreInfo={() => handleMoreInfo(token)}
                />
              ))}
            </div>
          )}
        </div>
      </Island>

      {/* Token info dialog */}
      <TokenInfoDialog
        open={infoDialogOpen}
        onOpenChange={setInfoDialogOpen}
        token={infoDialogToken}
      />
    </div>
  );
}

// ─── Token result card ──────────────────────────────────────────────

function TokenResultCard({
  token,
  adding,
  onAdd,
  onMoreInfo,
}: {
  token: FoundToken;
  adding: boolean;
  onAdd: () => void;
  onMoreInfo: () => void;
}) {
  const displayName = token.name ?? "Unnamed Token";

  return (
    <div className="flex items-start justify-between gap-4 rounded-lg border bg-card p-4">
      <div className="min-w-0 flex-1 space-y-1.5">
        <div className="flex items-center gap-2">
          <Coins className="size-4 text-dash-blue shrink-0" />
          <span className="font-semibold text-sm truncate">{displayName}</span>
          {token.paused && (
            <Badge variant="secondary" className="text-xs">
              Paused
            </Badge>
          )}
        </div>
        {token.description && (
          <p className="text-xs text-muted-foreground line-clamp-2">
            {token.description}
          </p>
        )}
        <div className="flex items-center gap-2">
          <span className="text-xs text-muted-foreground">Token ID:</span>
          <code className="text-xs font-mono" data-testid="token-id-display">
            {displayId(token.tokenId)}
          </code>
          <CopyButton value={hexToBase58(token.tokenId)} />
        </div>
        <div className="flex items-center gap-2">
          <span className="text-xs text-muted-foreground">Contract ID:</span>
          <code className="text-xs font-mono" data-testid="contract-id-display">
            {displayId(token.contractId)}
          </code>
          <CopyButton value={hexToBase58(token.contractId)} />
        </div>
      </div>
      <div className="flex flex-col gap-1.5 shrink-0">
        <Button
          size="sm"
          onClick={onAdd}
          disabled={adding}
          aria-label={`Add ${displayName} to My Tokens`}
        >
          {adding ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Plus className="h-3.5 w-3.5" />
          )}
          <span className="ml-1">Add</span>
        </Button>
        <Button
          variant="outline"
          size="sm"
          onClick={onMoreInfo}
          aria-label={`More info about ${displayName}`}
        >
          More Info
        </Button>
      </div>
    </div>
  );
}

// ─── Payload extraction ─────────────────────────────────────────────

/**
 * Map contractWithDescriptions result payload to FoundToken[].
 */
function mapContractResultToFoundTokens(
  contracts: Array<{
    contractId: string;
    description: string | null;
    tokens: Array<{
      tokenId: string;
      name: string;
      description: string | null;
      tokenPosition: number;
      configurationJson: unknown;
    }>;
  }>,
): FoundToken[] {
  const result: FoundToken[] = [];
  for (const contract of contracts) {
    for (const token of contract.tokens) {
      const config = parseTokenConfig(token.configurationJson);
      result.push({
        tokenId: token.tokenId,
        contractId: contract.contractId,
        name: token.name || null,
        description: token.description,
        decimals: config.decimals,
        tokenPosition: token.tokenPosition,
        baseSupply: config.baseSupply,
        maxSupply: config.maxSupply,
        ownerIdentityId: config.ownerIdentityId,
        paused: config.paused,
        configurationJson: token.configurationJson,
      });
    }
  }
  return result;
}

/**
 * Parse the versioned TokenConfiguration JSON to extract display fields.
 * The structure is typically: { V0: { conventions: { V0: { decimals, ... } }, base_supply, max_supply, ... } }
 */
function parseTokenConfig(configJson: unknown): {
  decimals: number;
  baseSupply: string | null;
  maxSupply: string | null;
  ownerIdentityId: string | null;
  paused: boolean;
} {
  const defaults = {
    decimals: 8,
    baseSupply: null as string | null,
    maxSupply: null as string | null,
    ownerIdentityId: null as string | null,
    paused: false,
  };

  if (!configJson || typeof configJson !== "object") return defaults;

  // Unwrap versioned envelope (e.g. { V0: { ... } })
  const inner = unwrapVersioned(configJson);
  if (!inner) return defaults;

  // Extract conventions → decimals
  const conventions = unwrapVersioned(inner.conventions);
  if (conventions && typeof conventions.decimals === "number") {
    defaults.decimals = conventions.decimals;
  }

  // Extract base_supply
  if (inner.base_supply != null) {
    defaults.baseSupply = String(inner.base_supply);
  }

  // Extract max_supply
  if (inner.max_supply != null) {
    defaults.maxSupply = String(inner.max_supply);
  }

  // Extract new_tokens_destination_identity
  if (inner.new_tokens_destination_identity) {
    const destId = typeof inner.new_tokens_destination_identity === "string"
      ? inner.new_tokens_destination_identity
      : null;
    if (destId) defaults.ownerIdentityId = destId;
  }

  // Extract paused state from manual_minting/manual_burning or a direct `paused` flag
  if (typeof inner.paused === "boolean") {
    defaults.paused = inner.paused;
  }

  return defaults;
}

/** Unwrap a versioned enum like { V0: { ... } } to its inner object. */
function unwrapVersioned(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== "object") return null;
  const obj = value as Record<string, unknown>;
  // Try V0, V1, etc.
  for (const key of Object.keys(obj)) {
    if (/^V\d+$/.test(key) && obj[key] && typeof obj[key] === "object") {
      return obj[key] as Record<string, unknown>;
    }
  }
  // If no version wrapper, return the object itself
  return obj;
}
