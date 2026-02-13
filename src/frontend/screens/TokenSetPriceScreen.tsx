import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useRouterState } from "@tanstack/react-router";
import { AlertTriangle, Loader2, Plus, Trash2 } from "lucide-react";
import { commands, events } from "@/bindings";
import type { TaskResultEvent, TaskErrorEvent } from "@/bindings";
import { TokenOperationForm } from "@/components/token/TokenOperationForm";
import type {
  ConfirmationConfig,
  GroupActionContext,
} from "@/components/token/TokenOperationForm";
import { estimateDocumentBatch } from "@/lib/feeEstimation";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/**
 * Pricing type for the Set Token Price screen.
 * - single: Fixed price per token
 * - tiered: Volume-based tiered pricing (BTreeMap<u64, u64>)
 * - remove: Remove pricing (make token unavailable for direct purchase)
 */
type PricingType = "single" | "tiered" | "remove";

/** Pricing schedule as returned by the backend. */
type PricingSchedule =
  | { SinglePrice: number }
  | { SetPrices: Record<string, number> };

interface TierRow {
  amount: string;
  price: string;
}

/**
 * Token Set Price screen — allows setting the direct purchase pricing schedule
 * for a token contract.
 *
 * Supports three pricing modes:
 *   1. Single Price — fixed price per token
 *   2. Tiered Pricing — volume discounts with multiple tiers
 *   3. Remove Pricing — clears the pricing schedule
 *
 * Reads token context from route search params:
 *   tokenId, contractId, tokenPosition, identityId, name, balance, decimals
 *
 * Optional group action params:
 *   groupActionId, groupPosition, details
 */
export function TokenSetPriceScreen() {
  const search = useRouterState({
    select: (s) => s.location.search as Record<string, string>,
  });

  const tokenContext = {
    tokenId: search.tokenId ?? "",
    contractId: search.contractId ?? "",
    tokenPosition: Number(search.tokenPosition ?? "0"),
    name: search.name ?? null,
    balance: search.balance ?? "0",
    decimals: Number(search.decimals ?? "8"),
    identityId: search.identityId ?? "",
  };

  const tokenDecimals = tokenContext.decimals;

  // Group action context
  const groupActionId = search.groupActionId;
  const isGroupSigning = !!groupActionId;

  const groupDetails = useMemo(() => {
    if (search.details) {
      try {
        return JSON.parse(search.details) as Record<string, unknown>;
      } catch {
        return null;
      }
    }
    return null;
  }, [search.details]);

  const groupAction: GroupActionContext | undefined = isGroupSigning
    ? { groupActionId, hasGroup: true, isUnilateral: false }
    : undefined;

  // ── Current pricing state (fetched from backend on mount) ──────────
  const [currentPricing, setCurrentPricing] = useState<PricingSchedule | null>(null);
  const [currentPricingLoading, setCurrentPricingLoading] = useState(false);
  const [currentPricingFetched, setCurrentPricingFetched] = useState(false);
  const [currentPricingError, setCurrentPricingError] = useState<string | null>(null);
  const pricingTaskIdRef = useRef<string | null>(null);

  // ── Pricing form state ──────────────────────────────────────────────
  const [pricingType, setPricingType] = useState<PricingType>("remove");
  const [singlePrice, setSinglePrice] = useState("");
  const [tiers, setTiers] = useState<TierRow[]>([
    { amount: "1", price: "" },
  ]);

  // ── Helpers ─────────────────────────────────────────────────────────

  const decimalMultiplier = Math.pow(10, tokenDecimals);

  /** Minimum price in DASH (based on token decimals). */
  const minimumPriceDash = useMemo(() => {
    if (tokenDecimals === 0) return "0.00000001";
    const divisor = Math.pow(10, tokenDecimals);
    // minimum 1 credit times the decimal divisor = minimum price per token
    return (divisor / 1e8).toFixed(8).replace(/0+$/, "").replace(/\.$/, "");
  }, [tokenDecimals]);

  // ── Fetch current pricing on mount ──────────────────────────────────
  useEffect(() => {
    if (!tokenContext.tokenId || isGroupSigning) return;

    let cleanupResult: (() => void) | undefined;
    let cleanupError: (() => void) | undefined;

    const subscribe = async () => {
      cleanupResult = await events.taskResultEvent.listen(
        (event: { payload: TaskResultEvent }) => {
          const { taskId, result } = event.payload;
          if (pricingTaskIdRef.current !== taskId) return;
          if (result.type !== "tokenPricing") return;

          pricingTaskIdRef.current = null;
          setCurrentPricingLoading(false);
          setCurrentPricingFetched(true);

          if (result.prices && typeof result.prices === "object") {
            setCurrentPricing(result.prices as PricingSchedule);
          } else {
            setCurrentPricing(null);
          }
        },
      );

      cleanupError = await events.taskErrorEvent.listen(
        (event: { payload: TaskErrorEvent }) => {
          const { taskId, message } = event.payload;
          if (pricingTaskIdRef.current !== taskId) return;

          pricingTaskIdRef.current = null;
          setCurrentPricingLoading(false);
          setCurrentPricingFetched(true);
          setCurrentPricingError(message || "Failed to fetch current pricing.");
        },
      );

      // Dispatch the query
      setCurrentPricingLoading(true);
      setCurrentPricingError(null);
      try {
        const result = await commands.tokenQueryPricing({
          tokenId: tokenContext.tokenId,
        });
        if (result.status === "ok") {
          pricingTaskIdRef.current = result.data.taskId;
        } else {
          setCurrentPricingLoading(false);
          setCurrentPricingError("Failed to dispatch pricing query.");
        }
      } catch {
        setCurrentPricingLoading(false);
        setCurrentPricingError("Failed to fetch current pricing.");
      }
    };

    subscribe().catch(() => {});

    return () => {
      cleanupResult?.();
      cleanupError?.();
    };
  }, [tokenContext.tokenId, isGroupSigning]);

  /** Format a pricing schedule for display. */
  const formatCurrentPricing = useCallback(
    (schedule: PricingSchedule): string[] => {
      if ("SinglePrice" in schedule) {
        const pricePerUnit = schedule.SinglePrice;
        if (pricePerUnit === 0) return ["Fixed price: FREE"];
        const pricePerToken = pricePerUnit * decimalMultiplier;
        const dashPrice = (pricePerToken / 1e8).toFixed(8).replace(/0+$/, "").replace(/\.$/, "");
        return [`Fixed price: ${dashPrice} DASH per token (${pricePerToken} credits)`];
      }

      const lines: string[] = [];
      const sortedTiers = Object.entries(schedule.SetPrices)
        .map(([k, v]) => [Number(k), v] as [number, number])
        .sort((a, b) => a[0] - b[0]);

      for (const [amountSmallest, pricePerUnit] of sortedTiers) {
        const tokenAmount =
          tokenDecimals > 0
            ? amountSmallest / Math.pow(10, tokenDecimals)
            : amountSmallest;

        if (pricePerUnit === 0) {
          lines.push(`${tokenAmount}+ tokens: FREE`);
        } else {
          const pricePerToken = pricePerUnit * decimalMultiplier;
          const dashPrice = (pricePerToken / 1e8).toFixed(8).replace(/0+$/, "").replace(/\.$/, "");
          lines.push(`${tokenAmount}+ tokens: ${dashPrice} DASH each (${pricePerToken} credits)`);
        }
      }

      return lines;
    },
    [tokenDecimals, decimalMultiplier],
  );

  /** Validate a price string for this token's precision. */
  const validatePrice = useCallback(
    (priceStr: string): string | null => {
      const price = parseFloat(priceStr);
      if (isNaN(price) || price <= 0) return "Price must be greater than 0.";
      // Convert to credits
      const credits = Math.round(price * 1e8);
      if (credits <= 0) return "Price too small.";

      const divisor = Math.pow(10, tokenDecimals);
      if (divisor > 1) {
        if (credits < divisor) {
          return `Price too low. Minimum is ${minimumPriceDash} DASH.`;
        }
        if (credits % divisor !== 0) {
          return `Price must be a multiple of ${minimumPriceDash} DASH to match this token's precision.`;
        }
      }
      return null;
    },
    [tokenDecimals, minimumPriceDash],
  );

  /**
   * Build the token pricing schedule JSON for the backend.
   * Returns null for remove pricing, or the pricing schedule object.
   */
  const buildPricingSchedule = useCallback((): {
    schedule: unknown;
    error: string | null;
  } => {
    if (pricingType === "remove") {
      return { schedule: null, error: null };
    }

    if (pricingType === "single") {
      const priceFloat = parseFloat(singlePrice);
      if (isNaN(priceFloat) || priceFloat <= 0) {
        return { schedule: null, error: "Please enter a valid price." };
      }
      const validationError = validatePrice(singlePrice);
      if (validationError) {
        return { schedule: null, error: validationError };
      }
      // Convert to credits per token, then divide by decimal divisor for per-smallest-unit
      const creditsPerToken = Math.round(priceFloat * 1e8);
      const divisor = Math.pow(10, tokenDecimals);
      const pricePerSmallestUnit = Math.floor(creditsPerToken / divisor);

      return {
        schedule: { SinglePrice: pricePerSmallestUnit },
        error: null,
      };
    }

    // Tiered pricing
    const map: Record<string, number> = {};
    let validTiers = 0;

    for (const tier of tiers) {
      const amount = parseInt(tier.amount, 10);
      if (isNaN(amount) || amount <= 0) continue;

      const priceFloat = parseFloat(tier.price);
      if (isNaN(priceFloat) || priceFloat <= 0) continue;

      const validationError = validatePrice(tier.price);
      if (validationError) {
        return { schedule: null, error: validationError };
      }

      const creditsPerToken = Math.round(priceFloat * 1e8);
      const divisor = Math.pow(10, tokenDecimals);
      const pricePerSmallestUnit = Math.floor(creditsPerToken / divisor);

      // Amount in smallest units
      const amountSmallest = amount * Math.pow(10, tokenDecimals);
      map[String(amountSmallest)] = pricePerSmallestUnit;
      validTiers++;
    }

    if (validTiers === 0) {
      return {
        schedule: null,
        error: "Please add at least one valid pricing tier.",
      };
    }

    return { schedule: { SetPrices: map }, error: null };
  }, [pricingType, singlePrice, tiers, tokenDecimals, validatePrice]);

  // ── Validation ────────────────────────────────────────────────────────
  const { isValid, validationMessage } = useMemo(() => {
    if (isGroupSigning) {
      return { isValid: true, validationMessage: undefined };
    }

    if (pricingType === "remove") {
      return { isValid: true, validationMessage: undefined };
    }

    if (pricingType === "single") {
      if (!singlePrice) {
        return {
          isValid: false,
          validationMessage: "Please enter a price.",
        };
      }
      const error = validatePrice(singlePrice);
      if (error) {
        return { isValid: false, validationMessage: error };
      }
      return { isValid: true, validationMessage: undefined };
    }

    // Tiered
    const { error } = buildPricingSchedule();
    if (error) {
      return { isValid: false, validationMessage: error };
    }
    return { isValid: true, validationMessage: undefined };
  }, [
    pricingType,
    singlePrice,
    validatePrice,
    buildPricingSchedule,
    isGroupSigning,
  ]);

  // ── Confirmation ──────────────────────────────────────────────────────
  const confirmation: ConfirmationConfig | undefined = useMemo(() => {
    if (!isValid) return undefined;

    if (pricingType === "remove") {
      return {
        title: isGroupSigning ? "Confirm Pricing Update" : "Remove Pricing",
        description: isGroupSigning
          ? "Are you sure you want to sign this group pricing update?"
          : "WARNING: Are you sure you want to remove the pricing schedule? This will make the token unavailable for direct purchase.",
        confirmLabel: isGroupSigning ? "Sign Set Price" : "Confirm",
        destructive: !isGroupSigning,
      };
    }

    if (pricingType === "single") {
      return {
        title: "Confirm Pricing Update",
        description: `Are you sure you want to set a fixed price of ${singlePrice} DASH per token?`,
        confirmLabel: isGroupSigning ? "Sign Set Price" : "Confirm",
      };
    }

    // Tiered
    const tierLines = tiers
      .filter(
        (t) =>
          parseInt(t.amount, 10) > 0 &&
          parseFloat(t.price) > 0,
      )
      .map((t) => `  ${t.amount}+ tokens: ${t.price} DASH each`)
      .join("\n");

    return {
      title: "Confirm Pricing Update",
      description: `Are you sure you want to set the following tiered pricing?\n${tierLines}`,
      confirmLabel: isGroupSigning ? "Sign Set Price" : "Confirm",
    };
  }, [isValid, pricingType, singlePrice, tiers, isGroupSigning]);

  // ── Group info builder ────────────────────────────────────────────────
  const buildGroupInfo = useCallback(() => {
    const groupPos = Number(search.groupPosition ?? "0");
    if (groupActionId) {
      return {
        type: "otherSigner",
        groupContractPosition: groupPos,
        actionId: groupActionId,
        actionIsProposer: false,
      };
    }
    return null;
  }, [groupActionId, search.groupPosition]);

  // ── Submit ────────────────────────────────────────────────────────────
  const handleSubmit = useCallback(
    async (params: {
      identityId: string;
      keyId: number;
      publicNote: string | null;
    }) => {
      const groupInfo = buildGroupInfo();
      const { schedule } = buildPricingSchedule();

      return commands.tokenSetDirectPurchasePrice({
        operation: {
          identityId: params.identityId,
          contractId: tokenContext.contractId,
          tokenPosition: tokenContext.tokenPosition,
          keyId: params.keyId,
          publicNote: params.publicNote,
        },
        tokenPricingSchedule: schedule as null,
        groupInfo: groupInfo,
      });
    },
    [
      tokenContext.contractId,
      tokenContext.tokenPosition,
      buildPricingSchedule,
      buildGroupInfo,
    ],
  );

  // ── Reset ─────────────────────────────────────────────────────────────
  const handleDoAnother = useCallback(() => {
    setPricingType("remove");
    setSinglePrice("");
    setTiers([{ amount: "1", price: "" }]);
  }, []);

  // ── Tier manipulation ─────────────────────────────────────────────────
  const addTier = useCallback(() => {
    setTiers((prev) => [...prev, { amount: "", price: "" }]);
  }, []);

  const removeTier = useCallback((index: number) => {
    setTiers((prev) => prev.filter((_, i) => i !== index));
  }, []);

  const updateTier = useCallback(
    (index: number, field: "amount" | "price", value: string) => {
      setTiers((prev) =>
        prev.map((t, i) => (i === index ? { ...t, [field]: value } : t)),
      );
    },
    [],
  );

  // ── Group signing display ────────────────────────────────────────────
  const groupScheduleDisplay = useMemo(() => {
    if (!groupDetails) return null;
    const schedule = groupDetails.pricingSchedule;
    if (schedule && typeof schedule === "object") {
      return JSON.stringify(schedule, null, 2);
    }
    if (typeof schedule === "string") return schedule;
    return "No schedule details available";
  }, [groupDetails]);

  return (
    <TokenOperationForm
      actionName="Set Price"
      tokenContext={tokenContext}
      estimatedFee={estimateDocumentBatch(1)}
      groupAction={groupAction}
      isValid={isValid}
      validationMessage={validationMessage}
      confirmation={confirmation}
      onSubmit={handleSubmit}
      resultEventType="tokenCompleted"
      successMessage="Token pricing schedule updated successfully."
      doAnotherLabel="Set Price Again"
      onDoAnother={handleDoAnother}
    >
      {isGroupSigning ? (
        /* ── Group signing mode — read-only display ──────────────── */
        <div className="space-y-3">
          <div className="rounded-md border border-border bg-muted/30 p-3 text-sm">
            <p className="text-muted-foreground mb-1">
              You are signing an existing group Set Price action. The pricing
              schedule is pre-determined.
            </p>
            {groupScheduleDisplay && (
              <pre
                className="mt-2 font-mono text-xs whitespace-pre-wrap"
                data-testid="group-schedule-display"
              >
                {groupScheduleDisplay}
              </pre>
            )}
          </div>
        </div>
      ) : (
        /* ── Normal pricing input ────────────────────────────────── */
        <div className="space-y-4" data-testid="pricing-section">
          {/* ── Current pricing display ────────────────────────── */}
          <div data-testid="current-pricing-section">
            <Label className="mb-2 block">Current Pricing</Label>
            {currentPricingLoading && (
              <div
                className="flex items-center gap-2 rounded-md border border-border bg-muted/30 p-3 text-sm text-muted-foreground"
                data-testid="current-pricing-loading"
              >
                <Loader2 className="h-4 w-4 animate-spin" />
                Fetching current pricing...
              </div>
            )}
            {currentPricingError && (
              <div
                className="rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive"
                data-testid="current-pricing-error"
              >
                {currentPricingError}
              </div>
            )}
            {!currentPricingLoading && !currentPricingError && currentPricing === null && currentPricingFetched && (
              <div
                className="rounded-md border border-border bg-muted/30 p-3 text-sm text-muted-foreground"
                data-testid="current-pricing-none"
              >
                No pricing schedule is currently set. This token is not available for direct purchase.
              </div>
            )}
            {!currentPricingLoading && currentPricing !== null && (
              <div
                className="rounded-md border border-border bg-muted/30 p-3 text-sm space-y-1"
                data-testid="current-pricing-display"
              >
                {"SinglePrice" in currentPricing ? (
                  <p className="font-medium text-foreground">
                    {formatCurrentPricing(currentPricing)[0]}
                  </p>
                ) : (
                  <>
                    <p className="font-medium text-foreground">Tiered pricing:</p>
                    {formatCurrentPricing(currentPricing).map((line, i) => (
                      <p key={i} className="text-muted-foreground ml-2">
                        {line}
                      </p>
                    ))}
                  </>
                )}
              </div>
            )}
          </div>

          {/* ── Pricing type selector ──────────────────────────── */}
          <div className="space-y-2">
            <Label>New Pricing</Label>
            <div
              className="flex flex-wrap gap-2"
              data-testid="pricing-type-buttons"
            >
              {(
                [
                  { value: "single", label: "Single Price" },
                  { value: "tiered", label: "Tiered Pricing" },
                  { value: "remove", label: "Remove Pricing" },
                ] as const
              ).map((opt) => (
                <Button
                  key={opt.value}
                  variant={pricingType === opt.value ? "default" : "outline"}
                  size="sm"
                  onClick={() => setPricingType(opt.value)}
                  className={cn(
                    pricingType === opt.value && "ring-2 ring-primary/30",
                  )}
                  data-testid={`pricing-type-${opt.value}`}
                >
                  {opt.label}
                </Button>
              ))}
            </div>
          </div>

          {/* ── Single price input ─────────────────────────────── */}
          {pricingType === "single" && (
            <div className="space-y-3" data-testid="single-price-section">
              <p className="text-sm text-muted-foreground">
                Set a fixed price per token:
              </p>
              {tokenDecimals > 0 && (
                <p className="text-xs text-amber-600 dark:text-amber-400">
                  Prices must be multiples of {minimumPriceDash} DASH to match
                  this token&apos;s precision.
                </p>
              )}
              <div className="space-y-1">
                <Label htmlFor="single-price-input">Price per token (DASH)</Label>
                <Input
                  id="single-price-input"
                  type="number"
                  step="any"
                  min="0"
                  placeholder="Enter price in DASH"
                  value={singlePrice}
                  onChange={(e) => setSinglePrice(e.target.value)}
                  className="font-mono max-w-xs"
                  data-testid="single-price-input"
                />
              </div>
              {singlePrice && parseFloat(singlePrice) > 0 && !validatePrice(singlePrice) && (
                <p
                  className="text-sm text-green-600 dark:text-green-400"
                  data-testid="price-preview"
                >
                  Price: {singlePrice} DASH per token (
                  {Math.round(parseFloat(singlePrice) * 1e8)} credits)
                </p>
              )}
              {singlePrice && validatePrice(singlePrice) && (
                <p className="text-sm text-destructive" data-testid="price-error">
                  {validatePrice(singlePrice)}
                </p>
              )}
            </div>
          )}

          {/* ── Tiered pricing input ───────────────────────────── */}
          {pricingType === "tiered" && (
            <div className="space-y-3" data-testid="tiered-pricing-section">
              <p className="text-sm text-muted-foreground">
                Add pricing tiers to offer volume discounts:
              </p>

              {/* Tier table */}
              <div className="space-y-2">
                {/* Header */}
                <div className="grid grid-cols-[1fr_1fr_auto] gap-2 text-xs font-medium text-muted-foreground">
                  <span>Minimum Amount (tokens)</span>
                  <span>Price per Token (DASH)</span>
                  <span className="w-9" />
                </div>

                {/* Rows */}
                {tiers.map((tier, index) => (
                  <div
                    key={index}
                    className="grid grid-cols-[1fr_1fr_auto] gap-2 items-center"
                    data-testid={`tier-row-${index}`}
                  >
                    <Input
                      type="number"
                      min="1"
                      step="1"
                      placeholder="Token amount"
                      value={tier.amount}
                      onChange={(e) =>
                        updateTier(index, "amount", e.target.value)
                      }
                      disabled={index === 0}
                      className="font-mono"
                      data-testid={`tier-amount-${index}`}
                    />
                    <Input
                      type="number"
                      step="any"
                      min="0"
                      placeholder="Price in DASH"
                      value={tier.price}
                      onChange={(e) =>
                        updateTier(index, "price", e.target.value)
                      }
                      className="font-mono"
                      data-testid={`tier-price-${index}`}
                    />
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={() => removeTier(index)}
                      disabled={index === 0 || tiers.length <= 1}
                      className="h-9 w-9"
                      data-testid={`tier-remove-${index}`}
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </div>
                ))}
              </div>

              <Button
                variant="outline"
                size="sm"
                onClick={addTier}
                data-testid="add-tier-button"
              >
                <Plus className="h-4 w-4 mr-1" />
                Add Tier
              </Button>

              {/* Tiered pricing preview */}
              {tiers.some(
                (t) =>
                  parseInt(t.amount, 10) > 0 &&
                  parseFloat(t.price) > 0 &&
                  !validatePrice(t.price),
              ) && (
                <div
                  className="rounded-md border bg-muted/30 p-3 text-sm space-y-1"
                  data-testid="tiered-preview"
                >
                  <p className="font-medium text-green-600 dark:text-green-400">
                    Pricing Structure:
                  </p>
                  {tiers
                    .filter(
                      (t) =>
                        parseInt(t.amount, 10) > 0 &&
                        parseFloat(t.price) > 0 &&
                        !validatePrice(t.price),
                    )
                    .sort((a, b) => parseInt(a.amount) - parseInt(b.amount))
                    .map((t, i) => (
                      <p key={i} className="text-muted-foreground ml-2">
                        {t.amount} or more tokens: {t.price} DASH each (
                        {Math.round(parseFloat(t.price) * 1e8)} credits)
                      </p>
                    ))}
                </div>
              )}
            </div>
          )}

          {/* ── Remove pricing warning ─────────────────────────── */}
          {pricingType === "remove" && (
            <div
              className="flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/5 p-3"
              data-testid="remove-pricing-warning"
            >
              <AlertTriangle className="h-5 w-5 mt-0.5 shrink-0 text-amber-500" />
              <div className="text-sm space-y-1">
                <p className="font-medium text-amber-700 dark:text-amber-300">
                  WARNING: This will remove the pricing schedule, making the
                  token unavailable for direct purchase.
                </p>
                <p className="text-muted-foreground">
                  Users will no longer be able to buy this token directly.
                </p>
              </div>
            </div>
          )}
        </div>
      )}
    </TokenOperationForm>
  );
}
