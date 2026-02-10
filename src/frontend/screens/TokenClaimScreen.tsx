import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useRouterState, useNavigate } from "@tanstack/react-router";
import { Info, Loader2, Eye } from "lucide-react";
import { commands, events } from "@/bindings";
import type {
  TaskResultEvent,
  TaskErrorEvent,
} from "@/bindings";
import { TokenOperationForm } from "@/components/token/TokenOperationForm";
import type { ConfirmationConfig } from "@/components/token/TokenOperationForm";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Button } from "@/components/ui/button";

/**
 * Token Claim screen — claims tokens from a distribution.
 *
 * Supports both Perpetual and PreProgrammed distribution types.
 * The user selects the distribution type, optionally adds a public note,
 * and broadcasts the claim.
 *
 * Also provides an estimate of perpetual rewards when that distribution
 * type is selected.
 *
 * Reads token context from route search params:
 *   tokenId, contractId, tokenPosition, identityId, name, balance, decimals
 */
export function TokenClaimScreen() {
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

  // ── Distribution type selection ─────────────────────────────────────
  const [distributionType, setDistributionType] = useState<string>("");

  // ── Perpetual rewards estimation ────────────────────────────────────
  const [estimatedRewards, setEstimatedRewards] = useState<string | null>(null);
  const [estimating, setEstimating] = useState(false);
  const estimateTaskIdRef = useRef<string | null>(null);

  // Subscribe to estimate result events
  useEffect(() => {
    let cleanupResult: (() => void) | undefined;
    let cleanupError: (() => void) | undefined;

    const subscribe = async () => {
      cleanupResult = await events.taskResultEvent.listen(
        (event: { payload: TaskResultEvent }) => {
          const { taskId, resultType, payload } = event.payload;
          if (estimateTaskIdRef.current !== taskId) return;
          if (resultType !== "Token") return;

          estimateTaskIdRef.current = null;
          setEstimating(false);

          // Payload contains the estimate explanation
          if (payload && typeof payload === "string") {
            setEstimatedRewards(payload);
          } else if (payload && typeof payload === "object") {
            setEstimatedRewards(JSON.stringify(payload, null, 2));
          } else {
            setEstimatedRewards("Estimate received (no details)");
          }
        },
      );

      cleanupError = await events.taskErrorEvent.listen(
        (event: { payload: TaskErrorEvent }) => {
          const { taskId } = event.payload;
          if (estimateTaskIdRef.current !== taskId) return;

          estimateTaskIdRef.current = null;
          setEstimating(false);
          setEstimatedRewards(null);
        },
      );
    };

    subscribe().catch(() => {});

    return () => {
      cleanupResult?.();
      cleanupError?.();
    };
  }, []);

  // Request estimate when Perpetual is selected
  const handleEstimate = useCallback(async () => {
    if (!tokenContext.identityId || !tokenContext.tokenId) return;

    setEstimating(true);
    setEstimatedRewards(null);

    try {
      const result = await commands.tokenEstimatePerpetualRewards({
        identityId: tokenContext.identityId,
        tokenId: tokenContext.tokenId,
      });
      if (result.status === "ok") {
        estimateTaskIdRef.current = result.data.taskId;
      } else {
        setEstimating(false);
      }
    } catch {
      setEstimating(false);
    }
  }, [tokenContext.identityId, tokenContext.tokenId]);

  // ── Validation ──────────────────────────────────────────────────────
  const isValid = distributionType !== "";

  const validationMessage = useMemo(() => {
    if (distributionType === "") {
      return "Please select a distribution type.";
    }
    return undefined;
  }, [distributionType]);

  // ── Confirmation ────────────────────────────────────────────────────
  const confirmation: ConfirmationConfig = {
    title: "Confirm Claim",
    description:
      "Are you sure you want to claim tokens for this contract?",
    confirmLabel: "Claim",
  };

  // ── Submit ──────────────────────────────────────────────────────────
  const handleSubmit = useCallback(
    async (params: {
      identityId: string;
      keyId: number;
      publicNote: string | null;
    }) => {
      return commands.tokenClaim({
        operation: {
          identityId: params.identityId,
          contractId: tokenContext.contractId,
          tokenPosition: tokenContext.tokenPosition,
          keyId: params.keyId,
          publicNote: params.publicNote,
        },
        distributionType,
      });
    },
    [tokenContext.contractId, tokenContext.tokenPosition, distributionType],
  );

  // ── View Claims navigation ──────────────────────────────────────────
  const handleViewClaims = useCallback(() => {
    navigate({
      to: "/tokens/view-claims",
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

  return (
    <TokenOperationForm
      actionName="Claim"
      tokenContext={tokenContext}
      isValid={isValid}
      validationMessage={validationMessage}
      confirmation={confirmation}
      onSubmit={handleSubmit}
      resultType="Token"
      successMessage="Tokens claimed successfully!"
      doAnotherLabel="Claim More"
      onDoAnother={() => {
        setDistributionType("");
        setEstimatedRewards(null);
      }}
    >
      {/* ── Distribution Type Selector ──────────────────────────────── */}
      <div className="space-y-2" data-testid="distribution-type-section">
        <label className="text-sm font-medium">Distribution Type</label>
        <Select
          value={distributionType}
          onValueChange={setDistributionType}
        >
          <SelectTrigger data-testid="distribution-type-select">
            <SelectValue placeholder="Select distribution type..." />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="Perpetual">Perpetual</SelectItem>
            <SelectItem value="PreProgrammed">Pre-Programmed</SelectItem>
          </SelectContent>
        </Select>
      </div>

      {/* ── Perpetual Distribution Info ─────────────────────────────── */}
      {distributionType === "Perpetual" && (
        <div
          className="rounded-lg border border-blue-500/30 bg-blue-500/5 p-4 space-y-3"
          data-testid="perpetual-info"
        >
          <div className="flex items-start gap-2">
            <Info className="h-5 w-5 mt-0.5 shrink-0 text-blue-500" />
            <div className="space-y-2 text-sm">
              <p className="font-medium">Understanding Claim Limitations</p>
              <p className="text-muted-foreground">
                A perpetual distribution can only claim 128 cycles at a time,
                except for fixed amount distributions where you can claim
                32,767 cycles.
              </p>
              <p className="text-muted-foreground">
                If your token would pay out every hour 1 Token, then you could
                only claim 128 hours worth of tokens in one claim. You can
                issue multiple claims back to back until you have nothing left
                to claim.
              </p>
            </div>
          </div>

          {/* Estimate rewards button */}
          <div className="flex items-center gap-3 pt-1">
            <Button
              variant="outline"
              size="sm"
              onClick={handleEstimate}
              disabled={estimating}
              data-testid="estimate-rewards-button"
            >
              {estimating && (
                <Loader2 className="h-3.5 w-3.5 mr-1 animate-spin" />
              )}
              Estimate Rewards
            </Button>
            {estimating && (
              <span className="text-xs text-muted-foreground">
                Estimating...
              </span>
            )}
          </div>

          {estimatedRewards && (
            <div
              className="rounded-md border bg-muted/30 p-3 text-sm whitespace-pre-wrap font-mono"
              data-testid="estimated-rewards"
            >
              {estimatedRewards}
            </div>
          )}
        </div>
      )}

      {/* ── View Claims link ────────────────────────────────────────── */}
      <div className="flex items-center">
        <Button
          variant="ghost"
          size="sm"
          onClick={handleViewClaims}
          className="text-muted-foreground hover:text-foreground"
          data-testid="view-claims-link"
        >
          <Eye className="h-4 w-4 mr-1" />
          View Previous Claims
        </Button>
      </div>
    </TokenOperationForm>
  );
}
