import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useRouterState, useNavigate } from "@tanstack/react-router";
import {
  ArrowLeft,
  Loader2,
  RefreshCw,
  Gift,
  Inbox,
} from "lucide-react";
import { commands, events } from "@/bindings";
import type { TaskResultEvent, TaskErrorEvent } from "@/bindings";
import { Button } from "@/components/ui/button";
import { formatTokenBalance } from "@/components/token/MyTokensTable";
import { toastError } from "@/lib/toastError";

// ─── Types ──────────────────────────────────────────────────────────────────

interface ClaimRecord {
  amount: string;
  timestamp: string;
  blockHeight: string;
  note: string;
}

type FetchStatus = "idle" | "fetching" | "done" | "error";

// ─── Helpers ────────────────────────────────────────────────────────────────

function truncateId(id: string, chars = 8): string {
  if (id.length <= chars * 2 + 3) return id;
  return `${id.slice(0, chars)}...${id.slice(-chars)}`;
}

/**
 * Parse claim documents from the task result payload.
 *
 * The payload is an array of [key, document|null] tuples from
 * DocumentResult::Fetched. Each document is a JSON object with properties
 * like "amount", "$createdAt", "$createdAtBlockHeight", "note".
 */
function parseClaimDocuments(payload: unknown): ClaimRecord[] {
  if (!Array.isArray(payload)) return [];

  const claims: ClaimRecord[] = [];
  for (const entry of payload) {
    // Each entry is [key, document | null]
    const doc = Array.isArray(entry) ? entry[1] : entry;
    if (!doc || typeof doc !== "object") continue;

    const d = doc as Record<string, unknown>;

    // Amount
    let amount = "0";
    if (typeof d.amount === "number") {
      amount = String(d.amount);
    } else if (typeof d.amount === "string") {
      amount = d.amount;
    }

    // Timestamp from $createdAt (milliseconds since epoch)
    let timestamp = "Unknown";
    const createdAt = d["$createdAt"];
    if (typeof createdAt === "number" && createdAt > 0) {
      try {
        const date = new Date(createdAt);
        timestamp = date.toISOString().replace("T", " ").replace(/\..*$/, "");
      } catch {
        timestamp = String(createdAt);
      }
    }

    // Block height from $createdAtBlockHeight
    let blockHeight = "Unknown";
    const bh = d["$createdAtBlockHeight"];
    if (typeof bh === "number") {
      blockHeight = String(bh);
    } else if (typeof bh === "string") {
      blockHeight = bh;
    }

    // Note
    let note = "";
    if (typeof d.note === "string") {
      note = d.note;
    }

    claims.push({ amount, timestamp, blockHeight, note });
  }

  return claims;
}

// ─── Component ──────────────────────────────────────────────────────────────

/**
 * View Token Claims screen — fetches and displays claim history
 * from the token history contract.
 *
 * Reads token context from route search params:
 *   tokenId, contractId, tokenPosition, identityId, name, balance, decimals
 */
export function TokenViewClaimsScreen() {
  const navigate = useNavigate();
  const search = useRouterState({
    select: (s) => s.location.search as Record<string, string>,
  });

  const tokenContext = useMemo(
    () => ({
      tokenId: search.tokenId ?? "",
      contractId: search.contractId ?? "",
      tokenPosition: Number(search.tokenPosition ?? "0"),
      name: search.name ?? null,
      balance: search.balance ?? "0",
      decimals: Number(search.decimals ?? "8"),
      identityId: search.identityId ?? "",
    }),
    [search.tokenId, search.contractId, search.tokenPosition, search.name, search.balance, search.decimals, search.identityId],
  );

  // ── Fetch state ─────────────────────────────────────────────────────
  const [claims, setClaims] = useState<ClaimRecord[]>([]);
  const [fetchStatus, setFetchStatus] = useState<FetchStatus>("idle");
  const [fetchStartTime, setFetchStartTime] = useState<number>(0);
  const [elapsedMs, setElapsedMs] = useState(0);
  const [message, setMessage] = useState<{
    text: string;
    type: "info" | "error" | "success";
  } | null>(null);
  const activeTaskIdRef = useRef<string | null>(null);

  // ── Elapsed timer ───────────────────────────────────────────────────
  useEffect(() => {
    if (fetchStatus !== "fetching") return;
    const interval = setInterval(() => {
      setElapsedMs(Date.now() - fetchStartTime);
    }, 100);
    return () => clearInterval(interval);
  }, [fetchStatus, fetchStartTime]);

  // ── Task result/error listeners ─────────────────────────────────────
  useEffect(() => {
    let cleanupResult: (() => void) | undefined;
    let cleanupError: (() => void) | undefined;

    const subscribe = async () => {
      cleanupResult = await events.taskResultEvent.listen(
        (event: { payload: TaskResultEvent }) => {
          const { taskId, resultType, payload } = event.payload;
          if (activeTaskIdRef.current !== taskId) return;
          if (resultType !== "Document") return;

          activeTaskIdRef.current = null;
          const parsed = parseClaimDocuments(payload);
          setClaims(parsed);
          setFetchStatus("done");

          if (parsed.length === 0) {
            setMessage({ text: "No claims found", type: "info" });
          } else {
            setMessage({
              text: `Found ${parsed.length} claim${parsed.length === 1 ? "" : "s"}`,
              type: "success",
            });
          }
        },
      );

      cleanupError = await events.taskErrorEvent.listen(
        (event: { payload: TaskErrorEvent }) => {
          const { taskId, message: errMsg } = event.payload;
          if (activeTaskIdRef.current !== taskId) return;

          activeTaskIdRef.current = null;
          setFetchStatus("error");
          setMessage({ text: errMsg, type: "error" });
          toastError(errMsg);
        },
      );
    };

    subscribe().catch(() => {});

    return () => {
      cleanupResult?.();
      cleanupError?.();
    };
  }, []);

  // ── Fetch claims ────────────────────────────────────────────────────
  const handleFetch = useCallback(async () => {
    if (!tokenContext.tokenId || !tokenContext.identityId) return;

    setFetchStatus("fetching");
    setFetchStartTime(Date.now());
    setElapsedMs(0);
    setMessage(null);

    try {
      const result = await commands.tokenQueryClaims({
        tokenId: tokenContext.tokenId,
        recipientId: tokenContext.identityId,
      });
      if (result.status === "ok") {
        activeTaskIdRef.current = result.data.taskId;
      } else {
        setFetchStatus("error");
        setMessage({ text: result.error, type: "error" });
      }
    } catch (e) {
      setFetchStatus("error");
      const msg = e instanceof Error ? e.message : String(e);
      setMessage({ text: msg, type: "error" });
    }
  }, [tokenContext.tokenId, tokenContext.identityId]);

  // ── Navigation ──────────────────────────────────────────────────────
  const handleBack = useCallback(() => {
    navigate({ to: "/tokens" });
  }, [navigate]);

  const handleClaim = useCallback(() => {
    navigate({
      to: "/tokens/claim",
      search: {
        tokenId: tokenContext.tokenId,
        contractId: tokenContext.contractId,
        tokenPosition: String(tokenContext.tokenPosition),
        name: tokenContext.name ?? "",
        balance: tokenContext.balance,
        decimals: String(tokenContext.decimals),
        identityId: tokenContext.identityId,
      },
    });
  }, [navigate, tokenContext]);

  const formattedBalance = formatTokenBalance(
    tokenContext.balance,
    tokenContext.decimals,
  );

  return (
    <div className="space-y-6" data-testid="view-claims-screen">
      {/* ── Token context header ──────────────────────────────────── */}
      <div className="flex items-start justify-between rounded-lg border bg-muted/30 p-4">
        <div className="space-y-1">
          <h3 className="text-lg font-semibold">
            {tokenContext.name || "Unnamed Token"}
          </h3>
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <span className="font-mono">
              {truncateId(tokenContext.tokenId)}
            </span>
          </div>
        </div>
        <div className="text-right">
          <p className="text-sm text-muted-foreground">Balance</p>
          <p className="text-lg font-semibold tabular-nums">
            {formattedBalance}
          </p>
        </div>
      </div>

      {/* ── Header with actions ───────────────────────────────────── */}
      <div className="flex items-center justify-between">
        <h2 className="text-xl font-semibold">Token Claims</h2>
        <div className="flex gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={handleClaim}
            data-testid="claim-tokens-button"
          >
            <Gift className="h-4 w-4 mr-1" />
            Claim Tokens
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={handleFetch}
            disabled={fetchStatus === "fetching"}
            data-testid="refresh-claims-button"
          >
            <RefreshCw
              className={`h-4 w-4 mr-1 ${fetchStatus === "fetching" ? "animate-spin" : ""}`}
            />
            Refresh
          </Button>
        </div>
      </div>

      {/* ── Fetch button ──────────────────────────────────────────── */}
      <div className="flex items-center gap-3">
        <Button
          onClick={handleFetch}
          disabled={fetchStatus === "fetching"}
          data-testid="fetch-claims-button"
        >
          {fetchStatus === "fetching" && (
            <Loader2 className="h-4 w-4 mr-1 animate-spin" />
          )}
          Fetch Claims
        </Button>
        {fetchStatus === "fetching" && (
          <span className="text-sm text-muted-foreground tabular-nums">
            Fetching... ({(elapsedMs / 1000).toFixed(1)}s)
          </span>
        )}
      </div>

      {/* ── Status message ────────────────────────────────────────── */}
      {message && (
        <p
          className={`text-sm ${
            message.type === "error"
              ? "text-destructive"
              : message.type === "success"
                ? "text-green-600 dark:text-green-400"
                : "text-muted-foreground"
          }`}
          data-testid="claims-message"
        >
          {message.text}
        </p>
      )}

      {/* ── Claims table ──────────────────────────────────────────── */}
      {claims.length > 0 && (
        <div
          className="rounded-lg border overflow-hidden"
          data-testid="claims-table"
        >
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b bg-muted/50">
                <th className="px-4 py-3 text-left font-medium">Amount</th>
                <th className="px-4 py-3 text-left font-medium">Timestamp</th>
                <th className="px-4 py-3 text-left font-medium">
                  Block Height
                </th>
                <th className="px-4 py-3 text-left font-medium">Note</th>
              </tr>
            </thead>
            <tbody>
              {claims.map((claim, i) => (
                <tr
                  key={i}
                  className="border-b last:border-b-0 hover:bg-muted/30 transition-colors"
                  data-testid={`claim-row-${i}`}
                >
                  <td className="px-4 py-3 font-mono tabular-nums">
                    {claim.amount}
                  </td>
                  <td className="px-4 py-3 text-muted-foreground">
                    {claim.timestamp}
                  </td>
                  <td className="px-4 py-3 font-mono tabular-nums">
                    {claim.blockHeight}
                  </td>
                  <td className="px-4 py-3 text-muted-foreground">
                    {claim.note || (
                      <span className="italic text-muted-foreground/50">
                        —
                      </span>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* ── Empty state ───────────────────────────────────────────── */}
      {fetchStatus === "done" && claims.length === 0 && (
        <div
          className="flex flex-col items-center justify-center py-12 text-center"
          data-testid="claims-empty"
        >
          <div className="rounded-full bg-muted p-4 mb-4">
            <Inbox className="h-8 w-8 text-muted-foreground" />
          </div>
          <h3 className="text-lg font-medium">No Claims Found</h3>
          <p className="text-sm text-muted-foreground mt-1 max-w-sm">
            No token claims have been recorded for this identity and token.
          </p>
          <Button
            variant="outline"
            size="sm"
            className="mt-4"
            onClick={handleClaim}
          >
            <Gift className="h-4 w-4 mr-1" />
            Claim Tokens
          </Button>
        </div>
      )}

      {/* ── Initial state ─────────────────────────────────────────── */}
      {fetchStatus === "idle" && claims.length === 0 && (
        <div
          className="flex flex-col items-center justify-center py-12 text-center"
          data-testid="claims-initial"
        >
          <div className="rounded-full bg-muted p-4 mb-4">
            <Inbox className="h-8 w-8 text-muted-foreground" />
          </div>
          <p className="text-sm text-muted-foreground max-w-sm">
            Click &ldquo;Fetch Claims&rdquo; to load token claim history
            from the platform.
          </p>
        </div>
      )}

      {/* ── Footer ────────────────────────────────────────────────── */}
      <div className="flex items-center border-t pt-4">
        <Button
          variant="ghost"
          onClick={handleBack}
          data-testid="back-to-tokens"
        >
          <ArrowLeft className="h-4 w-4 mr-1" />
          Back to Tokens
        </Button>
      </div>
    </div>
  );
}
