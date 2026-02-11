import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useRouterState } from "@tanstack/react-router";
import { DollarSign, Loader2, RefreshCw } from "lucide-react";
import { commands, events } from "@/bindings";
import type { TaskResultEvent, TaskErrorEvent } from "@/bindings";
import { TokenOperationForm } from "@/components/token/TokenOperationForm";
import type { ConfirmationConfig } from "@/components/token/TokenOperationForm";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";

/** Pricing schedule as returned by the backend. */
type PricingSchedule =
  | { SinglePrice: number }
  | { SetPrices: Record<string, number> };

/**
 * Token Purchase screen — allows purchasing tokens at the contract's
 * configured pricing schedule.
 *
 * Flow:
 * 1. User clicks "Fetch Token Price" to retrieve the current pricing schedule
 * 2. User enters the amount of tokens to purchase
 * 3. Total price is calculated from the pricing schedule
 * 4. User confirms and broadcasts the purchase transaction
 *
 * Reads token context from route search params:
 *   tokenId, contractId, tokenPosition, identityId, name, balance, decimals
 */
export function TokenPurchaseScreen() {
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
    [
      search.tokenId,
      search.contractId,
      search.tokenPosition,
      search.name,
      search.balance,
      search.decimals,
      search.identityId,
    ],
  );

  const tokenDecimals = tokenContext.decimals;

  // ── Pricing state ───────────────────────────────────────────────────
  const [pricingSchedule, setPricingSchedule] =
    useState<PricingSchedule | null>(null);
  const [pricingFetched, setPricingFetched] = useState(false);
  const [fetchingPricing, setFetchingPricing] = useState(false);
  const pricingTaskIdRef = useRef<string | null>(null);

  // ── Amount / price state ────────────────────────────────────────────
  const [amount, setAmount] = useState("");
  const [pricingError, setPricingError] = useState<string | null>(null);

  // ── Pricing event listener ──────────────────────────────────────────
  useEffect(() => {
    let cleanupResult: (() => void) | undefined;
    let cleanupError: (() => void) | undefined;

    const subscribe = async () => {
      cleanupResult = await events.taskResultEvent.listen(
        (event: { payload: TaskResultEvent }) => {
          const { taskId, resultType, payload } = event.payload;
          if (pricingTaskIdRef.current !== taskId) return;
          if (resultType !== "Token") return;

          pricingTaskIdRef.current = null;
          setFetchingPricing(false);
          setPricingFetched(true);

          // Payload is { token_id, prices: <schedule or null> }
          if (payload && typeof payload === "object" && !Array.isArray(payload)) {
            const obj = payload as Record<string, unknown>;
            const prices = obj.prices;
            if (prices && typeof prices === "object") {
              setPricingSchedule(prices as PricingSchedule);
              setPricingError(null);
            } else {
              setPricingSchedule(null);
              setPricingError(
                "This token is not available for direct purchase. No pricing has been set.",
              );
            }
          } else {
            setPricingSchedule(null);
            setPricingError("Unexpected pricing response format.");
          }
        },
      );

      cleanupError = await events.taskErrorEvent.listen(
        (event: { payload: TaskErrorEvent }) => {
          const { taskId, message } = event.payload;
          if (pricingTaskIdRef.current !== taskId) return;

          pricingTaskIdRef.current = null;
          setFetchingPricing(false);
          setPricingFetched(true);
          setPricingError(message || "Failed to fetch pricing.");
        },
      );
    };

    subscribe().catch(() => {});

    return () => {
      cleanupResult?.();
      cleanupError?.();
    };
  }, []);

  // ── Fetch pricing ───────────────────────────────────────────────────
  const handleFetchPricing = useCallback(async () => {
    if (!tokenContext.tokenId) return;

    setFetchingPricing(true);
    setPricingError(null);

    try {
      const result = await commands.tokenQueryPricing({
        tokenId: tokenContext.tokenId,
      });
      if (result.status === "ok") {
        pricingTaskIdRef.current = result.data.taskId;
      } else {
        setFetchingPricing(false);
        setPricingError("Failed to dispatch pricing query.");
      }
    } catch {
      setFetchingPricing(false);
      setPricingError("Failed to fetch token pricing.");
    }
  }, [tokenContext.tokenId]);

  // ── Recalculate price when amount or pricing changes ────────────────
  const calculatedPriceCredits = useMemo(() => {
    if (!pricingSchedule || !amount) return null;

    const amountNum = parseFloat(amount);
    if (isNaN(amountNum) || amountNum <= 0) return null;

    // Convert user's token amount to smallest units
    const amountSmallestUnits = Math.round(
      amountNum * Math.pow(10, tokenDecimals),
    );

    let pricePerSmallestUnit = 0;

    if ("SinglePrice" in pricingSchedule) {
      pricePerSmallestUnit = pricingSchedule.SinglePrice;
    } else if ("SetPrices" in pricingSchedule) {
      const sortedTiers = Object.entries(pricingSchedule.SetPrices)
        .map(([k, v]) => [Number(k), v] as [number, number])
        .sort((a, b) => a[0] - b[0]);

      for (const [tierAmount, tierPrice] of sortedTiers) {
        if (amountSmallestUnits >= tierAmount) {
          pricePerSmallestUnit = tierPrice;
        }
      }
    }

    if (pricePerSmallestUnit > 0) {
      // Total price = amount_in_smallest_units * price_per_smallest_unit
      const total = amountSmallestUnits * pricePerSmallestUnit;
      return Math.min(total, Number.MAX_SAFE_INTEGER);
    }
    return null;
  }, [pricingSchedule, amount, tokenDecimals]);

  // ── Display helpers ─────────────────────────────────────────────────
  const decimalMultiplier = Math.pow(10, tokenDecimals);

  const formatPricingDisplay = useCallback(
    (schedule: PricingSchedule): string[] => {
      if ("SinglePrice" in schedule) {
        const pricePerUnit = schedule.SinglePrice;
        if (pricePerUnit === 0) return ["Fixed price: FREE"];
        // Convert price per smallest unit to price per whole token for display
        const pricePerToken = pricePerUnit * decimalMultiplier;
        const dashPrice = (pricePerToken / 1e8).toFixed(8).replace(/0+$/, "").replace(/\.$/, "");
        return [`Fixed price: ${dashPrice} DASH per token`];
      }

      const lines: string[] = ["Tiered pricing:"];
      const sortedTiers = Object.entries(schedule.SetPrices)
        .map(([k, v]) => [Number(k), v] as [number, number])
        .sort((a, b) => a[0] - b[0]);

      for (const [amountSmallest, pricePerUnit] of sortedTiers) {
        const tokenAmount =
          tokenDecimals > 0
            ? amountSmallest / Math.pow(10, tokenDecimals)
            : amountSmallest;

        if (pricePerUnit === 0) {
          lines.push(`  ${tokenAmount} tokens: FREE`);
        } else {
          const pricePerToken = pricePerUnit * decimalMultiplier;
          const dashPrice = (pricePerToken / 1e8).toFixed(8).replace(/0+$/, "").replace(/\.$/, "");
          lines.push(`  ${tokenAmount}+ tokens: ${dashPrice} DASH each`);
        }
      }

      return lines;
    },
    [tokenDecimals, decimalMultiplier],
  );

  const calculatedPriceDash = useMemo(() => {
    if (calculatedPriceCredits === null) return null;
    return (calculatedPriceCredits / 1e8)
      .toFixed(8)
      .replace(/0+$/, "")
      .replace(/\.$/, "");
  }, [calculatedPriceCredits]);

  // ── Validation ──────────────────────────────────────────────────────
  const amountNum = parseFloat(amount);
  const isAmountValid = !isNaN(amountNum) && amountNum > 0;
  const canPurchase =
    pricingSchedule !== null &&
    isAmountValid &&
    calculatedPriceCredits !== null &&
    calculatedPriceCredits > 0;

  const validationMessage = useMemo(() => {
    if (!pricingFetched && !fetchingPricing) {
      return 'Click "Fetch Token Price" to retrieve current pricing.';
    }
    if (pricingError) return pricingError;
    if (!pricingSchedule) return "No pricing available for this token.";
    if (amount && !isAmountValid) return "Please enter a valid amount.";
    if (isAmountValid && calculatedPriceCredits === null) {
      return "Please enter a valid amount to see the price.";
    }
    return undefined;
  }, [
    pricingFetched,
    fetchingPricing,
    pricingError,
    pricingSchedule,
    amount,
    isAmountValid,
    calculatedPriceCredits,
  ]);

  // ── Confirmation ────────────────────────────────────────────────────
  const confirmation: ConfirmationConfig | undefined = useMemo(() => {
    if (!canPurchase || calculatedPriceDash === null) return undefined;
    return {
      title: "Confirm Purchase",
      description: `Are you sure you want to purchase ${amount} tokens for ${calculatedPriceDash} DASH (${calculatedPriceCredits} credits)?`,
      confirmLabel: "Purchase",
    };
  }, [canPurchase, amount, calculatedPriceDash, calculatedPriceCredits]);

  // ── Submit ──────────────────────────────────────────────────────────
  const handleSubmit = useCallback(
    async (params: {
      identityId: string;
      keyId: number;
      publicNote: string | null;
    }) => {
      if (calculatedPriceCredits === null) {
        return {
          status: "error" as const,
          error: "Price not calculated",
        };
      }

      // Amount in smallest units as string (u128)
      const amountSmallestUnits = String(
        Math.round(parseFloat(amount) * Math.pow(10, tokenDecimals)),
      );

      return commands.tokenPurchase({
        operation: {
          identityId: params.identityId,
          contractId: tokenContext.contractId,
          tokenPosition: tokenContext.tokenPosition,
          keyId: params.keyId,
          publicNote: params.publicNote,
        },
        amount: amountSmallestUnits,
        totalAgreedPrice: calculatedPriceCredits,
      });
    },
    [
      tokenContext.contractId,
      tokenContext.tokenPosition,
      amount,
      tokenDecimals,
      calculatedPriceCredits,
    ],
  );

  // ── Reset ───────────────────────────────────────────────────────────
  const handleDoAnother = useCallback(() => {
    setAmount("");
  }, []);

  return (
    <TokenOperationForm
      actionName="Purchase"
      tokenContext={tokenContext}
      isValid={canPurchase}
      validationMessage={validationMessage}
      confirmation={confirmation}
      onSubmit={handleSubmit}
      resultType="Token"
      successMessage="Tokens purchased successfully!"
      doAnotherLabel="Purchase More"
      onDoAnother={handleDoAnother}
    >
      {/* ── Amount to purchase ─────────────────────────────────── */}
      <div className="space-y-3" data-testid="purchase-amount-section">
        <div className="flex items-end gap-3">
          <div className="flex-1 space-y-1">
            <Label htmlFor="purchase-amount">Amount to Purchase</Label>
            <Input
              id="purchase-amount"
              type="number"
              step="any"
              min="0"
              placeholder="Enter token amount"
              value={amount}
              onChange={(e) => setAmount(e.target.value)}
              className="font-mono"
              data-testid="purchase-amount-input"
            />
          </div>
          <Button
            variant="outline"
            onClick={handleFetchPricing}
            disabled={fetchingPricing}
            data-testid="fetch-pricing-button"
          >
            {fetchingPricing ? (
              <Loader2 className="h-4 w-4 mr-1 animate-spin" />
            ) : (
              <RefreshCw className="h-4 w-4 mr-1" />
            )}
            Fetch Token Price
          </Button>
        </div>
      </div>

      {/* ── Pricing schedule display ───────────────────────────── */}
      {pricingSchedule && (
        <div
          className="rounded-md border bg-muted/30 p-3 text-sm space-y-0.5"
          data-testid="pricing-display"
        >
          <p className="font-medium text-muted-foreground mb-1">
            Current pricing:
          </p>
          {formatPricingDisplay(pricingSchedule).map((line, i) => (
            <p
              key={i}
              className={
                i === 0 && line.startsWith("Tiered")
                  ? "font-medium"
                  : "text-muted-foreground"
              }
            >
              {line}
            </p>
          ))}
        </div>
      )}

      {/* ── Pricing error ──────────────────────────────────────── */}
      {pricingError && (
        <p
          className="text-sm text-destructive"
          data-testid="pricing-error"
        >
          {pricingError}
        </p>
      )}

      {/* ── Calculated total price ─────────────────────────────── */}
      {calculatedPriceCredits !== null && calculatedPriceDash !== null && (
        <div
          className="rounded-lg border border-green-500/30 bg-green-500/5 p-4 space-y-1"
          data-testid="calculated-price"
        >
          <div className="flex items-center gap-2">
            <DollarSign className="h-5 w-5 text-green-600 dark:text-green-400" />
            <span className="font-medium">Calculated Total Price</span>
          </div>
          <p className="text-lg font-mono font-semibold">
            {calculatedPriceDash} DASH
          </p>
          <p className="text-sm text-muted-foreground">
            {calculatedPriceCredits.toLocaleString()} credits
          </p>
          <p className="text-xs text-muted-foreground mt-1">
            Based on the current pricing schedule.
          </p>
        </div>
      )}

      {/* ── No pricing fetched hint ────────────────────────────── */}
      {!pricingFetched && !fetchingPricing && !pricingError && (
        <p
          className="text-sm text-muted-foreground"
          data-testid="fetch-hint"
        >
          Click &quot;Fetch Token Price&quot; to retrieve current pricing.
        </p>
      )}

      {/* ── Fetching indicator ─────────────────────────────────── */}
      {fetchingPricing && (
        <div
          className="flex items-center gap-2 text-sm text-muted-foreground"
          data-testid="fetching-indicator"
        >
          <Loader2 className="h-4 w-4 animate-spin" />
          Fetching pricing schedule...
        </div>
      )}
    </TokenOperationForm>
  );
}
