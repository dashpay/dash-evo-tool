import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  AlertTriangle,
  Check,
  ChevronDown,
  ChevronUp,
  Loader2,
  Lock,
  ShieldAlert,
  Unlock,
} from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { cn } from "@/lib/utils";
import { commands, events } from "@/bindings";
import type {
  QualifiedIdentityDto,
  WalletDto,
  SingleKeyWalletDto,
  TaskResultEvent,
  TaskErrorEvent,
} from "@/bindings";
import { useIdentityStore } from "@/stores/identityStore";
import { useWalletStore } from "@/stores/walletStore";
import { useTokenStore } from "@/stores/tokenStore";
import { WalletUnlockDialog } from "@/components/shared/WalletUnlockDialog";
import type { WalletUnlockResult } from "@/components/shared/WalletUnlockDialog";
import { toastError } from "@/lib/toastError";

import type { BasicInfoState } from "./TokenCreatorWizard";
import { TOKEN_LANGUAGES, TOKEN_PRESETS } from "./TokenCreatorWizard";
import type { DistributionState } from "./DistributionStep";
import type { ControlRulesState } from "./ControlRulesStep";
import type { GroupsState } from "./GroupsStep";
import type { HistoryState } from "./HistoryStep";
import type { KeywordsState } from "./KeywordsStep";
import type { ActionTaker } from "./ControlRulesStep";

// ─── Types ──────────────────────────────────────────────────────────────────

export type ReviewStatus =
  | { type: "idle" }
  | { type: "broadcasting"; startTime: number }
  | { type: "success" }
  | { type: "error"; message: string };

export interface ReviewStepProps {
  basicInfo: BasicInfoState;
  distribution: DistributionState;
  controlRules: ControlRulesState;
  groups: GroupsState;
  history: HistoryState;
  keywords: KeywordsState;
  onReset: () => void;
}

// ─── Helpers ────────────────────────────────────────────────────────────────

interface WalletInfo {
  seedHash: string;
  alias: string | null;
  usesPassword: boolean;
  passwordHint: string | null;
}

function findAssociatedWallet(
  identity: QualifiedIdentityDto | null,
  hdWallets: WalletDto[],
  singleKeyWallets: SingleKeyWalletDto[],
): WalletInfo | null {
  if (!identity) return null;
  const hashes = identity.associatedWalletHashes;
  if (hashes.length === 0) return null;

  for (const hash of hashes) {
    const hd = hdWallets.find((w) => w.seedHash === hash);
    if (hd) {
      return {
        seedHash: hd.seedHash,
        alias: hd.alias,
        usesPassword: hd.usesPassword,
        passwordHint: hd.passwordHint,
      };
    }
  }
  for (const hash of hashes) {
    const sk = singleKeyWallets.find((w) => w.keyHash === hash);
    if (sk) {
      return {
        seedHash: sk.keyHash,
        alias: sk.alias,
        usesPassword: sk.usesPassword,
        passwordHint: sk.passwordHint,
      };
    }
  }
  return null;
}

function autoSelectKey(
  identity: QualifiedIdentityDto | null,
): number | null {
  if (!identity) return null;
  const authKeys = identity.keys.filter(
    (k) => !k.isDisabled && k.purpose === "AUTHENTICATION",
  );
  const highKey = authKeys.find((k) => k.securityLevel === "HIGH");
  if (highKey) return highKey.keyId;
  const criticalKey = authKeys.find((k) => k.securityLevel === "CRITICAL");
  if (criticalKey) return criticalKey.keyId;
  if (authKeys.length > 0) return authKeys[0].keyId;
  return null;
}

export function getActionTakerLabel(taker: ActionTaker): string {
  switch (taker) {
    case "noOne":
      return "No One";
    case "contractOwner":
      return "Contract Owner";
    case "identity":
      return "Specific Identity";
    case "mainGroup":
      return "Main Group";
    case "group":
      return "Specific Group";
  }
}

export function getDistributionFunctionLabel(fn: string): string {
  const labels: Record<string, string> = {
    fixedAmount: "Fixed Amount",
    stepDecreasing: "Step Decreasing",
    stepwise: "Stepwise",
    linear: "Linear",
    polynomial: "Polynomial",
    exponential: "Exponential",
    logarithmic: "Logarithmic",
    invertedLogarithmic: "Inverted Logarithmic",
  };
  return labels[fn] ?? fn;
}

export function getIntervalTypeLabel(type: string): string {
  const labels: Record<string, string> = {
    timeBased: "Time-based",
    blockBased: "Block-based",
    epochBased: "Epoch-based",
  };
  return labels[type] ?? type;
}

export function formatCreditsAsDash(credits: number): string {
  const dash = credits / 100_000_000_000;
  return `~${dash.toFixed(6)} DASH`;
}

/**
 * Rough client-side fee estimation for token contract creation.
 * Based on egui's estimate_registration_cost().
 */
export function estimateRegistrationCost(
  basicInfo: BasicInfoState,
  distribution: DistributionState,
  keywords: KeywordsState,
): number {
  // Base contract registration fee (~50M credits)
  let credits = 50_000_000;

  // Token registration fee (~100M credits)
  credits += 100_000_000;

  // Distribution fees
  if (distribution.perpetual.enabled) {
    credits += 50_000_000; // perpetual distribution fee
  }
  if (distribution.preProgrammed.enabled) {
    credits += 50_000_000; // pre-programmed distribution fee
  }

  // Search keyword fees (~10M credits per keyword)
  const contractKeywords = basicInfo.contractKeywords
    .split(",")
    .map((k) => k.trim())
    .filter((k) => k.length >= 3 && k.length <= 50);
  credits += contractKeywords.length * 10_000_000;

  // Searchable token name fees
  const searchableNames = basicInfo.names.filter(
    (n) => n.singular.trim().length >= 3,
  );
  credits += searchableNames.length * 10_000_000;

  // Token keywords fees
  credits += keywords.keywords.length * 10_000_000;

  // Estimation buffer
  credits += 200_000_000;

  return credits;
}

/**
 * Check if token is permanently set to non-tradeable.
 * This is the "danger mode" from the egui implementation.
 */
export function isPermanentlyNonTradeable(
  controlRules: ControlRulesState,
): boolean {
  return (
    controlRules.marketplace.authorizedActionTaker === "noOne" &&
    !controlRules.marketplace.changingAuthorizedToNoOneAllowed
  );
}

// ─── ReviewStep ─────────────────────────────────────────────────────────────

export function ReviewStep({
  basicInfo,
  distribution,
  controlRules,
  groups,
  history,
  keywords,
  onReset,
}: ReviewStepProps) {
  // ── Store state ──────────────────────────────────────────────────────
  const { identities, loadIdentities } = useIdentityStore();
  const { hdWallets, singleKeyWallets, loadWallets } = useWalletStore();
  const { loadMyTokenBalances } = useTokenStore();

  // ── Identity & key selection ─────────────────────────────────────────
  const [rawIdentityId, setRawIdentityId] = useState<string>("");
  const [rawKeyId, setRawKeyId] = useState<string>("");

  const selectedIdentity = useMemo(() => {
    if (!rawIdentityId && identities.length > 0) return identities[0];
    return identities.find((id) => id.id === rawIdentityId) ?? null;
  }, [rawIdentityId, identities]);

  const selectedIdentityId = selectedIdentity?.id ?? null;

  const autoKeyId = useMemo(
    () => autoSelectKey(selectedIdentity),
    [selectedIdentity],
  );

  const selectedKeyId = useMemo(() => {
    if (rawKeyId) {
      const parsed = parseInt(rawKeyId, 10);
      if (!isNaN(parsed)) return parsed;
    }
    return autoKeyId;
  }, [rawKeyId, autoKeyId]);

  const availableKeys = useMemo(() => {
    if (!selectedIdentity) return [];
    return selectedIdentity.keys.filter(
      (k) => !k.isDisabled && k.purpose === "AUTHENTICATION",
    );
  }, [selectedIdentity]);

  // ── Wallet ───────────────────────────────────────────────────────────
  const associatedWallet = useMemo(
    () => findAssociatedWallet(selectedIdentity, hdWallets, singleKeyWallets),
    [selectedIdentity, hdWallets, singleKeyWallets],
  );

  const [walletUnlockOpen, setWalletUnlockOpen] = useState(false);
  const [walletUnlockError, setWalletUnlockError] = useState<string | null>(null);
  const [walletUnlockedHashes, setWalletUnlockedHashes] = useState<Set<string>>(
    new Set(),
  );

  const walletLocked = useMemo(
    () =>
      !!associatedWallet &&
      associatedWallet.usesPassword &&
      !walletUnlockedHashes.has(associatedWallet.seedHash),
    [associatedWallet, walletUnlockedHashes],
  );

  // ── Confirmation dialog ──────────────────────────────────────────────
  const [confirmOpen, setConfirmOpen] = useState(false);

  // ── Broadcast status ─────────────────────────────────────────────────
  const [status, setStatus] = useState<ReviewStatus>({ type: "idle" });
  const [elapsedMs, setElapsedMs] = useState(0);
  const activeTaskIdRef = useRef<string | null>(null);

  // ── Collapsible sections ─────────────────────────────────────────────
  const [expandedSections, setExpandedSections] = useState<Set<string>>(
    new Set(["basicInfo"]),
  );

  const toggleSection = useCallback((section: string) => {
    setExpandedSections((prev) => {
      const next = new Set(prev);
      if (next.has(section)) next.delete(section);
      else next.add(section);
      return next;
    });
  }, []);

  // ── Fee estimation ───────────────────────────────────────────────────
  const estimatedCost = useMemo(
    () => estimateRegistrationCost(basicInfo, distribution, keywords),
    [basicInfo, distribution, keywords],
  );

  const dangerMode = useMemo(
    () => isPermanentlyNonTradeable(controlRules),
    [controlRules],
  );

  // ── Load identities and wallets on mount ─────────────────────────────
  useEffect(() => {
    loadIdentities();
    loadWallets();
  }, [loadIdentities, loadWallets]);

  // ── Elapsed timer ────────────────────────────────────────────────────
  useEffect(() => {
    if (status.type !== "broadcasting") return;
    const interval = setInterval(() => {
      setElapsedMs(Date.now() - status.startTime);
    }, 100);
    return () => clearInterval(interval);
  }, [status]);

  // ── Task result/error listeners ──────────────────────────────────────
  useEffect(() => {
    let cleanupResult: (() => void) | undefined;
    let cleanupError: (() => void) | undefined;

    const subscribe = async () => {
      cleanupResult = await events.taskResultEvent.listen(
        (event: { payload: TaskResultEvent }) => {
          const { taskId, resultType } = event.payload;
          if (resultType !== "Token" && resultType !== "Contract") return;
          if (activeTaskIdRef.current !== taskId) return;

          activeTaskIdRef.current = null;
          setStatus({ type: "success" });
          toast.success("Token contract created successfully!");
          loadMyTokenBalances();
        },
      );

      cleanupError = await events.taskErrorEvent.listen(
        (event: { payload: TaskErrorEvent }) => {
          const { taskId, message } = event.payload;
          if (activeTaskIdRef.current !== taskId) return;

          activeTaskIdRef.current = null;
          setStatus({ type: "error", message });
          toastError(message);
        },
      );
    };

    subscribe().catch(() => {});

    return () => {
      cleanupResult?.();
      cleanupError?.();
    };
  }, [loadMyTokenBalances]);

  // ── Wallet unlock handler ────────────────────────────────────────────
  const handleWalletUnlockResult = useCallback(
    async (result: WalletUnlockResult) => {
      if (result.status === "unlocked" && associatedWallet) {
        setWalletUnlockError(null);
        try {
          await commands.walletNotifyUnlocked(associatedWallet.seedHash);
          setWalletUnlockedHashes(
            (prev) => new Set([...prev, associatedWallet.seedHash]),
          );
        } catch (e) {
          setWalletUnlockError(
            e instanceof Error ? e.message : String(e),
          );
          return;
        }
      }
      setWalletUnlockOpen(false);
    },
    [associatedWallet],
  );

  // ── Create token handler ─────────────────────────────────────────────
  const handleCreateToken = useCallback(async () => {
    if (!selectedIdentityId || selectedKeyId === null) return;

    setConfirmOpen(false);
    setStatus({ type: "broadcasting", startTime: Date.now() });
    setElapsedMs(0);

    // Build the config JSON from all wizard state
    const configJson = buildConfigJson(
      basicInfo,
      distribution,
      controlRules,
      groups,
      history,
      keywords,
    );

    try {
      const result = await commands.tokenRegisterContract({
        configJson: configJson as unknown as import("@/bindings").JsonValue,
        identityId: selectedIdentityId,
        keyId: selectedKeyId,
      });
      if (result.status === "ok") {
        activeTaskIdRef.current = result.data.taskId;
      } else {
        setStatus({ type: "error", message: result.error });
        toastError(result.error);
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setStatus({ type: "error", message: msg });
      toastError(msg);
    }
  }, [
    selectedIdentityId,
    selectedKeyId,
    basicInfo,
    distribution,
    controlRules,
    groups,
    history,
    keywords,
  ]);

  // ── Derived state ────────────────────────────────────────────────────
  const canCreate =
    !!selectedIdentityId && selectedKeyId !== null && !walletLocked;

  const tokenName =
    basicInfo.names[0]?.singular?.trim() || "Unnamed Token";

  // ── Success screen ───────────────────────────────────────────────────
  if (status.type === "success") {
    return (
      <div
        className="flex flex-col items-center justify-center gap-6 py-12"
        data-testid="review-success"
      >
        <div className="rounded-full bg-green-100 dark:bg-green-900/30 p-4">
          <Check className="h-8 w-8 text-green-600 dark:text-green-400" />
        </div>
        <h2 className="text-xl font-semibold">
          Token Contract Created Successfully!
        </h2>
        <p className="text-muted-foreground text-center max-w-md">
          Your token &ldquo;{tokenName}&rdquo; has been registered on the Dash
          Platform. It may take a moment for the token to appear in your
          portfolio.
        </p>
        <Button onClick={onReset} data-testid="review-create-another">
          Create Another Token
        </Button>
      </div>
    );
  }

  // ── Error screen ─────────────────────────────────────────────────────
  if (status.type === "error") {
    return (
      <div
        className="flex flex-col items-center justify-center gap-6 py-12"
        data-testid="review-error"
      >
        <div className="rounded-full bg-destructive/10 p-4">
          <AlertTriangle className="h-8 w-8 text-destructive" />
        </div>
        <h2 className="text-xl font-semibold">Registration Failed</h2>
        <p className="text-muted-foreground text-center max-w-md">
          {status.message}
        </p>
        <div className="flex gap-3">
          <Button
            variant="outline"
            onClick={() => setStatus({ type: "idle" })}
            data-testid="review-try-again"
          >
            Try Again
          </Button>
          <Button onClick={onReset}>Start Over</Button>
        </div>
      </div>
    );
  }

  // ── Broadcasting screen ──────────────────────────────────────────────
  if (status.type === "broadcasting") {
    return (
      <div
        className="flex flex-col items-center justify-center gap-6 py-12"
        data-testid="review-broadcasting"
      >
        <Loader2 className="h-10 w-10 animate-spin text-primary" />
        <h2 className="text-xl font-semibold">
          Registering Token Contract...
        </h2>
        <p className="text-muted-foreground">
          Broadcasting to Dash Platform. This may take a moment.
        </p>
        <span className="text-sm text-muted-foreground tabular-nums">
          {(elapsedMs / 1000).toFixed(1)}s elapsed
        </span>
      </div>
    );
  }

  // ── Main review UI ───────────────────────────────────────────────────
  return (
    <div className="space-y-6" data-testid="review-step">
      {/* Danger mode warning */}
      {dangerMode && (
        <div
          className="flex items-start gap-3 rounded-lg border border-destructive/50 bg-destructive/5 p-4"
          data-testid="danger-warning"
        >
          <ShieldAlert className="h-5 w-5 text-destructive mt-0.5 shrink-0" />
          <div>
            <p className="font-medium text-destructive">
              Permanently Non-Tradeable
            </p>
            <p className="text-sm text-destructive/80 mt-1">
              This token will be permanently set to NotTradeable and can NEVER
              be made tradeable in the future! This cannot be reversed.
            </p>
          </div>
        </div>
      )}

      {/* Identity selector */}
      <div className="space-y-3">
        <h3 className="text-sm font-semibold uppercase tracking-wider text-muted-foreground">
          Signing Identity
        </h3>
        <div className="grid gap-3 sm:grid-cols-2">
          <div className="space-y-1">
            <label className="text-sm font-medium" htmlFor="review-identity">
              Identity
            </label>
            <Select
              value={rawIdentityId || selectedIdentity?.id || ""}
              onValueChange={setRawIdentityId}
            >
              <SelectTrigger data-testid="review-identity-select">
                <SelectValue placeholder="Select identity..." />
              </SelectTrigger>
              <SelectContent>
                {identities.map((id) => (
                  <SelectItem key={id.id} value={id.id}>
                    {id.alias || id.id.slice(0, 12) + "..."}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="space-y-1">
            <label className="text-sm font-medium" htmlFor="review-key">
              Signing Key
            </label>
            <Select
              value={rawKeyId || (selectedKeyId !== null ? String(selectedKeyId) : "")}
              onValueChange={setRawKeyId}
            >
              <SelectTrigger data-testid="review-key-select">
                <SelectValue placeholder="Auto-selected" />
              </SelectTrigger>
              <SelectContent>
                {availableKeys.map((k) => (
                  <SelectItem key={k.keyId} value={String(k.keyId)}>
                    Key {k.keyId} ({k.securityLevel} / {k.keyType})
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>

        {/* Wallet lock status */}
        {associatedWallet && (
          <div className="flex items-center gap-2">
            {walletLocked ? (
              <>
                <Lock className="h-4 w-4 text-amber-500" />
                <span className="text-sm text-amber-600 dark:text-amber-400">
                  Wallet &ldquo;{associatedWallet.alias || "Unnamed"}&rdquo; is
                  locked
                </span>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => setWalletUnlockOpen(true)}
                  data-testid="review-unlock-wallet"
                >
                  <Unlock className="h-3.5 w-3.5 mr-1" />
                  Unlock
                </Button>
              </>
            ) : (
              <>
                <Check className="h-4 w-4 text-green-500" />
                <span className="text-sm text-green-600 dark:text-green-400">
                  Wallet unlocked
                </span>
              </>
            )}
          </div>
        )}

        {identities.length === 0 && (
          <p className="text-sm text-destructive">
            No identities loaded. Please load an identity before creating a
            token.
          </p>
        )}
      </div>

      {/* Configuration summary */}
      <div className="space-y-2">
        <h3 className="text-sm font-semibold uppercase tracking-wider text-muted-foreground">
          Configuration Summary
        </h3>

        {/* Basic Info section */}
        <SummarySection
          title="Basic Info"
          sectionKey="basicInfo"
          expanded={expandedSections.has("basicInfo")}
          onToggle={toggleSection}
        >
          <SummaryRow label="Token Name" value={tokenName} />
          {basicInfo.names[0]?.plural && (
            <SummaryRow label="Plural Name" value={basicInfo.names[0].plural} />
          )}
          {basicInfo.names.length > 1 && (
            <SummaryRow
              label="Additional Languages"
              value={basicInfo.names
                .slice(1)
                .map(
                  (n) =>
                    `${TOKEN_LANGUAGES.find((l) => l.code === n.languageCode)?.label || n.languageCode}: ${n.singular}`,
                )
                .join(", ")}
            />
          )}
          {basicInfo.description && (
            <SummaryRow label="Description" value={basicInfo.description} />
          )}
          <SummaryRow label="Base Supply" value={basicInfo.baseSupply || "0"} />
          <SummaryRow
            label="Max Supply"
            value={basicInfo.maxSupply || "Unlimited"}
          />
          <SummaryRow label="Decimals" value={basicInfo.decimals} />
          {basicInfo.preset && (
            <SummaryRow
              label="Preset"
              value={
                TOKEN_PRESETS.find((p) => p.value === basicInfo.preset)
                  ?.label || basicInfo.preset
              }
            />
          )}
          {basicInfo.shouldCapitalize && (
            <SummaryRow label="Capitalize Name" value="Yes" />
          )}
          {basicInfo.startPaused && (
            <SummaryRow label="Start Paused" value="Yes" />
          )}
          {basicInfo.allowTransfersToFrozen && (
            <SummaryRow label="Allow Transfers to Frozen" value="Yes" />
          )}
        </SummarySection>

        {/* Distribution section */}
        <SummarySection
          title="Distribution"
          sectionKey="distribution"
          expanded={expandedSections.has("distribution")}
          onToggle={toggleSection}
        >
          {!distribution.perpetual.enabled &&
          !distribution.preProgrammed.enabled ? (
            <p className="text-sm text-muted-foreground italic">
              No distribution configured
            </p>
          ) : (
            <>
              {distribution.perpetual.enabled && (
                <>
                  <SummaryRow label="Perpetual Distribution" value="Enabled" />
                  <SummaryRow
                    label="Interval Type"
                    value={getIntervalTypeLabel(
                      distribution.perpetual.intervalType,
                    )}
                  />
                  <SummaryRow
                    label="Distribution Function"
                    value={getDistributionFunctionLabel(
                      distribution.perpetual.distributionFunction,
                    )}
                  />
                  <SummaryRow
                    label="Recipient"
                    value={
                      distribution.perpetual.recipient === "contractOwner"
                        ? "Contract Owner"
                        : distribution.perpetual.recipient === "identity"
                          ? `Identity: ${distribution.perpetual.recipientIdentityId || "(not set)"}`
                          : "Evonodes (by participation)"
                    }
                  />
                </>
              )}
              {distribution.preProgrammed.enabled && (
                <>
                  <SummaryRow
                    label="Pre-programmed Distribution"
                    value="Enabled"
                  />
                  <SummaryRow
                    label="Entries"
                    value={`${distribution.preProgrammed.entries.length} entry/entries`}
                  />
                </>
              )}
            </>
          )}
        </SummarySection>

        {/* Control Rules section */}
        <SummarySection
          title="Control Rules"
          sectionKey="controlRules"
          expanded={expandedSections.has("controlRules")}
          onToggle={toggleSection}
        >
          <ControlRuleSummary
            label="Manual Minting"
            taker={controlRules.manualMinting.authorizedActionTaker}
          />
          <ControlRuleSummary
            label="Manual Burning"
            taker={controlRules.manualBurning.authorizedActionTaker}
          />
          <ControlRuleSummary
            label="Freeze"
            taker={controlRules.freeze.authorizedActionTaker}
          />
          <ControlRuleSummary
            label="Unfreeze"
            taker={controlRules.unfreeze.authorizedActionTaker}
          />
          <ControlRuleSummary
            label="Destroy Frozen Funds"
            taker={controlRules.destroyFrozenFunds.authorizedActionTaker}
          />
          <ControlRuleSummary
            label="Emergency Action"
            taker={controlRules.emergencyAction.authorizedActionTaker}
          />
          <ControlRuleSummary
            label="Max Supply Change"
            taker={controlRules.maxSupplyChange.authorizedActionTaker}
          />
          <ControlRuleSummary
            label="Conventions Change"
            taker={controlRules.conventionsChange.authorizedActionTaker}
          />
          <ControlRuleSummary
            label="Marketplace"
            taker={controlRules.marketplace.authorizedActionTaker}
          />
          <ControlRuleSummary
            label="Direct Purchase Pricing"
            taker={controlRules.directPurchasePricing.authorizedActionTaker}
          />
          <SummaryRow
            label="Main Control Group Change"
            value={getActionTakerLabel(controlRules.mainControlGroupChange)}
          />
        </SummarySection>

        {/* Groups section */}
        <SummarySection
          title="Groups"
          sectionKey="groups"
          expanded={expandedSections.has("groups")}
          onToggle={toggleSection}
        >
          {groups.groups.length === 0 ? (
            <p className="text-sm text-muted-foreground italic">
              No groups configured
            </p>
          ) : (
            groups.groups.map((g, i) => (
              <SummaryRow
                key={i}
                label={`Group ${i + 1}`}
                value={`Required Power: ${g.requiredPower || "0"}, Members: ${g.members.length}`}
              />
            ))
          )}
        </SummarySection>

        {/* History section */}
        <SummarySection
          title="History"
          sectionKey="history"
          expanded={expandedSections.has("history")}
          onToggle={toggleSection}
        >
          <SummaryRow
            label="Transfers"
            value={history.keepTransferHistory ? "Keep" : "Skip"}
          />
          <SummaryRow
            label="Freezes/Unfreezes"
            value={history.keepFreezingHistory ? "Keep" : "Skip"}
          />
          <SummaryRow
            label="Mints"
            value={history.keepMintingHistory ? "Keep" : "Skip"}
          />
          <SummaryRow
            label="Burns"
            value={history.keepBurningHistory ? "Keep" : "Skip"}
          />
          <SummaryRow
            label="Direct Pricing"
            value={history.keepDirectPricingHistory ? "Keep" : "Skip"}
          />
          <SummaryRow
            label="Direct Purchases"
            value={history.keepDirectPurchaseHistory ? "Keep" : "Skip"}
          />
        </SummarySection>

        {/* Keywords section */}
        <SummarySection
          title="Keywords"
          sectionKey="keywords"
          expanded={expandedSections.has("keywords")}
          onToggle={toggleSection}
        >
          {keywords.keywords.length === 0 ? (
            <p className="text-sm text-muted-foreground italic">
              No keywords added
            </p>
          ) : (
            <div className="flex flex-wrap gap-1.5">
              {keywords.keywords.map((kw) => (
                <Badge key={kw} variant="secondary">
                  {kw}
                </Badge>
              ))}
            </div>
          )}
        </SummarySection>
      </div>

      {/* Fee estimation */}
      <div
        className="rounded-lg border bg-muted/50 p-4 space-y-2"
        data-testid="fee-estimation"
      >
        <div className="flex items-center justify-between">
          <span className="text-sm font-medium">
            Estimated Registration Cost
          </span>
          <span className="font-mono text-sm font-semibold">
            {formatCreditsAsDash(estimatedCost)}
          </span>
        </div>
        <p className="text-xs text-muted-foreground">
          This is a rough estimate. Actual cost may vary slightly based on
          network conditions and contract size.
        </p>
      </div>

      {/* Create button */}
      <div className="flex justify-end gap-3">
        <Button
          size="lg"
          disabled={!canCreate}
          onClick={() => setConfirmOpen(true)}
          className={cn(
            dangerMode &&
              "bg-destructive hover:bg-destructive/90 text-destructive-foreground",
          )}
          data-testid="review-create-button"
        >
          {walletLocked ? (
            <>
              <Lock className="h-4 w-4 mr-2" />
              Unlock Wallet to Create
            </>
          ) : (
            "Create Token Contract"
          )}
        </Button>
      </div>

      {/* Confirmation dialog */}
      <Dialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        <DialogContent data-testid="review-confirm-dialog">
          <DialogHeader>
            <DialogTitle>
              {dangerMode
                ? "⚠️ Confirm Token Contract Registration"
                : "Confirm Token Contract Registration"}
            </DialogTitle>
            <DialogDescription>
              <span className="block mt-2 space-y-1">
                <span className="block">
                  <strong>Token Name:</strong> {tokenName}
                </span>
                <span className="block">
                  <strong>Base Supply:</strong>{" "}
                  {basicInfo.baseSupply || "0"}
                </span>
                <span className="block">
                  <strong>Max Supply:</strong>{" "}
                  {basicInfo.maxSupply || "None (unlimited)"}
                </span>
                <span className="block">
                  <strong>Estimated Cost:</strong>{" "}
                  {formatCreditsAsDash(estimatedCost)}
                </span>
              </span>
              {dangerMode && (
                <span className="block mt-3 text-destructive font-medium">
                  WARNING: This token will be permanently set to NotTradeable
                  and can NEVER be made tradeable in the future!
                </span>
              )}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setConfirmOpen(false)}
              data-testid="review-confirm-cancel"
            >
              Cancel
            </Button>
            <Button
              onClick={handleCreateToken}
              className={cn(
                dangerMode &&
                  "bg-destructive hover:bg-destructive/90 text-destructive-foreground",
              )}
              data-testid="review-confirm-submit"
            >
              Confirm
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Wallet unlock dialog */}
      {associatedWallet && (
        <WalletUnlockDialog
          open={walletUnlockOpen}
          onOpenChange={setWalletUnlockOpen}
          walletAlias={associatedWallet.alias || "Unnamed Wallet"}
          passwordHint={associatedWallet.passwordHint}
          error={walletUnlockError}
          onResult={handleWalletUnlockResult}
        />
      )}
    </div>
  );
}

// ─── SummarySection ─────────────────────────────────────────────────────────

function SummarySection({
  title,
  sectionKey,
  expanded,
  onToggle,
  children,
}: {
  title: string;
  sectionKey: string;
  expanded: boolean;
  onToggle: (key: string) => void;
  children: React.ReactNode;
}) {
  return (
    <div
      className="rounded-lg border"
      data-testid={`summary-section-${sectionKey}`}
    >
      <button
        type="button"
        className="flex items-center justify-between w-full px-4 py-2.5 text-left text-sm font-medium hover:bg-muted/50 transition-colors"
        onClick={() => onToggle(sectionKey)}
        data-testid={`summary-toggle-${sectionKey}`}
      >
        {title}
        {expanded ? (
          <ChevronUp className="h-4 w-4 text-muted-foreground" />
        ) : (
          <ChevronDown className="h-4 w-4 text-muted-foreground" />
        )}
      </button>
      {expanded && (
        <div className="px-4 pb-3 space-y-1.5 border-t pt-2">
          {children}
        </div>
      )}
    </div>
  );
}

// ─── SummaryRow ─────────────────────────────────────────────────────────────

function SummaryRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between text-sm gap-4">
      <span className="text-muted-foreground shrink-0">{label}</span>
      <span className="font-medium text-right break-all">{value}</span>
    </div>
  );
}

// ─── ControlRuleSummary ─────────────────────────────────────────────────────

function ControlRuleSummary({
  label,
  taker,
}: {
  label: string;
  taker: ActionTaker;
}) {
  return (
    <SummaryRow
      label={label}
      value={getActionTakerLabel(taker)}
    />
  );
}

// ─── buildConfigJson ────────────────────────────────────────────────────────

/**
 * Serialize all wizard state into the JSON structure expected by
 * the Tauri `tokenRegisterContract` IPC command.
 */
export function buildConfigJson(
  basicInfo: BasicInfoState,
  distribution: DistributionState,
  controlRules: ControlRulesState,
  groups: GroupsState,
  history: HistoryState,
  keywords: KeywordsState,
): Record<string, unknown> {
  return {
    // Basic info
    names: basicInfo.names.map((n) => ({
      singular: n.singular.trim(),
      plural: n.plural.trim(),
      languageCode: n.languageCode,
    })),
    description: basicInfo.description.trim() || null,
    baseSupply: basicInfo.baseSupply.trim(),
    maxSupply: basicInfo.maxSupply.trim() || null,
    decimals: parseInt(basicInfo.decimals, 10),
    shouldCapitalize: basicInfo.shouldCapitalize,
    startPaused: basicInfo.startPaused,
    allowTransfersToFrozen: basicInfo.allowTransfersToFrozen,
    contractKeywords: basicInfo.contractKeywords
      .split(",")
      .map((k) => k.trim())
      .filter(Boolean),

    // Distribution
    perpetualDistribution: distribution.perpetual.enabled
      ? {
          intervalType: distribution.perpetual.intervalType,
          intervalAmount: distribution.perpetual.intervalAmount,
          intervalTimeUnit: distribution.perpetual.intervalTimeUnit,
          distributionFunction: distribution.perpetual.distributionFunction,
          recipient: distribution.perpetual.recipient,
          recipientIdentityId:
            distribution.perpetual.recipientIdentityId || null,
          params: {
            fixedAmount: distribution.perpetual.fixedAmount,
            stepCount: distribution.perpetual.stepCount,
            decreaseNumerator: distribution.perpetual.decreaseNumerator,
            decreaseDenominator: distribution.perpetual.decreaseDenominator,
            startPeriodOffset: distribution.perpetual.startPeriodOffset,
            initialEmission: distribution.perpetual.initialEmission,
            stepDecreasingMinValue:
              distribution.perpetual.stepDecreasingMinValue,
            stepDecreasingMaxIntervalCount:
              distribution.perpetual.stepDecreasingMaxIntervalCount,
            trailingAmount: distribution.perpetual.trailingAmount,
            stepwiseEntries: distribution.perpetual.stepwiseEntries,
            linearA: distribution.perpetual.linearA,
            linearD: distribution.perpetual.linearD,
            linearStartStep: distribution.perpetual.linearStartStep,
            linearStartingAmount:
              distribution.perpetual.linearStartingAmount,
            linearMinValue: distribution.perpetual.linearMinValue,
            linearMaxValue: distribution.perpetual.linearMaxValue,
          },
        }
      : null,
    preProgrammedDistribution: distribution.preProgrammed.enabled
      ? {
          entries: distribution.preProgrammed.entries.map((e) => ({
            days: e.days,
            hours: e.hours,
            minutes: e.minutes,
            identityId: e.identityId,
            amount: e.amount,
          })),
        }
      : null,

    // Control rules
    controlRules: {
      manualMinting: serializeControlRule(controlRules.manualMinting),
      manualBurning: serializeControlRule(controlRules.manualBurning),
      freeze: serializeControlRule(controlRules.freeze),
      unfreeze: serializeControlRule(controlRules.unfreeze),
      destroyFrozenFunds: serializeControlRule(
        controlRules.destroyFrozenFunds,
      ),
      emergencyAction: serializeControlRule(controlRules.emergencyAction),
      maxSupplyChange: serializeControlRule(controlRules.maxSupplyChange),
      conventionsChange: serializeControlRule(
        controlRules.conventionsChange,
      ),
      marketplace: serializeControlRule(controlRules.marketplace),
      directPurchasePricing: serializeControlRule(
        controlRules.directPurchasePricing,
      ),
      mainControlGroupChange: {
        actionTaker: controlRules.mainControlGroupChange,
        identityId: controlRules.mainControlGroupChangeIdentityId || null,
        groupPosition: controlRules.mainControlGroupChangeGroupPosition || null,
      },
    },
    mintExtras: {
      destinationDefaultToContractOwner:
        controlRules.mintExtras.destinationDefaultToContractOwner,
      destinationOtherIdentityEnabled:
        controlRules.mintExtras.destinationOtherIdentityEnabled,
      destinationOtherIdentityId:
        controlRules.mintExtras.destinationOtherIdentityId || null,
      allowChoosingDestination:
        controlRules.mintExtras.allowChoosingDestination,
      destinationIdentityRules: serializeControlRule(
        controlRules.mintExtras.destinationIdentityRules,
      ),
      choosingDestinationRules: serializeControlRule(
        controlRules.mintExtras.choosingDestinationRules,
      ),
    },

    // Groups
    groups: groups.groups.map((g) => ({
      requiredPower: parseInt(g.requiredPower, 10) || 0,
      members: g.members.map((m) => ({
        identityId: m.identityId.trim(),
        power: parseInt(m.power, 10) || 0,
      })),
    })),

    // History
    keepHistory: {
      transfers: history.keepTransferHistory,
      freezing: history.keepFreezingHistory,
      minting: history.keepMintingHistory,
      burning: history.keepBurningHistory,
      directPricing: history.keepDirectPricingHistory,
      directPurchase: history.keepDirectPurchaseHistory,
    },

    // Keywords
    tokenKeywords: keywords.keywords,
  };
}

function serializeControlRule(
  rule: import("./ControlRulesStep").ChangeControlRulesState,
): Record<string, unknown> {
  return {
    authorizedActionTaker: rule.authorizedActionTaker,
    authorizedIdentityId: rule.authorizedIdentityId || null,
    authorizedGroupPosition: rule.authorizedGroupPosition || null,
    adminActionTaker: rule.adminActionTaker,
    adminIdentityId: rule.adminIdentityId || null,
    adminGroupPosition: rule.adminGroupPosition || null,
    changingAuthorizedToNoOneAllowed: rule.changingAuthorizedToNoOneAllowed,
    changingAdminToNoOneAllowed: rule.changingAdminToNoOneAllowed,
    selfChangingAdminAllowed: rule.selfChangingAdminAllowed,
  };
}
