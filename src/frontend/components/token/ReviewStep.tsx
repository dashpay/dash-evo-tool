import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  AlertTriangle,
  Calculator,
  Check,
  ChevronDown,
  ChevronUp,
  Copy,
  FileJson2,
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
  /** When true, hides the configuration summary sections that aren't relevant in simple mode */
  simpleMode?: boolean;
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
        passwordHint: null,
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
  if (authKeys.length > 0) return authKeys[0]?.keyId ?? null;
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
  simpleMode = false,
}: ReviewStepProps) {
  // ── Store state ──────────────────────────────────────────────────────
  const identities = useIdentityStore((s) => s.identities);
  const loadIdentities = useIdentityStore((s) => s.loadIdentities);
  const hdWallets = useWalletStore((s) => s.hdWallets);
  const singleKeyWallets = useWalletStore((s) => s.singleKeyWallets);
  const loadWallets = useWalletStore((s) => s.loadWallets);
  const { loadMyTokenBalances } = useTokenStore();

  // ── Identity & key selection ─────────────────────────────────────────
  const [rawIdentityId, setRawIdentityId] = useState<string>("");
  const [rawKeyId, setRawKeyId] = useState<string>("");

  const selectedIdentity = useMemo(() => {
    if (!rawIdentityId && identities.length > 0) return identities[0] ?? null;
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

  // ── Contract JSON preview dialog ────────────────────────────────────
  const [jsonPreviewOpen, setJsonPreviewOpen] = useState(false);
  const [jsonPreviewText, setJsonPreviewText] = useState("");
  const [jsonCopied, setJsonCopied] = useState(false);

  const handleViewContractJson = useCallback(() => {
    const configJson = buildConfigJson(
      basicInfo,
      distribution,
      controlRules,
      groups,
      history,
      keywords,
    );
    setJsonPreviewText(JSON.stringify(configJson, null, 2));
    setJsonCopied(false);
    setJsonPreviewOpen(true);
  }, [basicInfo, distribution, controlRules, groups, history, keywords]);

  const handleCopyJson = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(jsonPreviewText);
      setJsonCopied(true);
      toast.success("JSON copied to clipboard");
      setTimeout(() => setJsonCopied(false), 2000);
    } catch {
      toast.error("Failed to copy to clipboard");
    }
  }, [jsonPreviewText]);

  // ── Calculate fee dialog ──────────────────────────────────────────
  const [feeDialogOpen, setFeeDialogOpen] = useState(false);

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
          const { taskId, result } = event.payload;
          if (result.type !== "tokenCompleted" && result.type !== "contractCompleted") return;
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
  const basicInfoValid = simpleMode
    ? ((basicInfo.names[0]?.singular?.trim().length ?? 0) >= 3 && !!basicInfo.baseSupply.trim() && !!basicInfo.preset)
    : true; // In advanced mode, basicInfo was already validated at step 0
  const canCreate =
    !!selectedIdentityId && selectedKeyId !== null && !walletLocked && basicInfoValid;

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

        {!simpleMode && (
          <>
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
          </>
        )}
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

      {/* Action buttons */}
      <div className="flex flex-wrap justify-between gap-3">
        <div className="flex gap-2">
          <Button
            variant="outline"
            onClick={handleViewContractJson}
            data-testid="review-view-json-button"
          >
            <FileJson2 className="h-4 w-4 mr-2" />
            View Contract JSON
          </Button>
          <Button
            variant="outline"
            onClick={() => setFeeDialogOpen(true)}
            data-testid="review-calculate-fee-button"
          >
            <Calculator className="h-4 w-4 mr-2" />
            Calculate Fee
          </Button>
        </div>
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

      {/* Contract JSON preview dialog */}
      <Dialog open={jsonPreviewOpen} onOpenChange={setJsonPreviewOpen}>
        <DialogContent
          className="max-w-2xl max-h-[80vh] flex flex-col"
          data-testid="json-preview-dialog"
        >
          <DialogHeader>
            <DialogTitle>Data Contract JSON</DialogTitle>
            <DialogDescription>
              Preview of the token contract configuration that will be submitted
              to the Dash Platform.
            </DialogDescription>
          </DialogHeader>
          <div className="flex-1 overflow-auto rounded-md border bg-muted/30 p-4 min-h-0">
            <pre className="text-xs font-mono whitespace-pre-wrap break-all">
              {jsonPreviewText}
            </pre>
          </div>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={handleCopyJson}
              data-testid="json-preview-copy"
            >
              {jsonCopied ? (
                <>
                  <Check className="h-4 w-4 mr-2" />
                  Copied
                </>
              ) : (
                <>
                  <Copy className="h-4 w-4 mr-2" />
                  Copy JSON
                </>
              )}
            </Button>
            <Button onClick={() => setJsonPreviewOpen(false)} data-testid="json-preview-close">Close</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Fee calculation dialog */}
      <Dialog open={feeDialogOpen} onOpenChange={setFeeDialogOpen}>
        <DialogContent data-testid="fee-dialog">
          <DialogHeader>
            <DialogTitle>Fee Estimation Breakdown</DialogTitle>
            <DialogDescription>
              Detailed breakdown of the estimated registration cost for this
              token contract.
            </DialogDescription>
          </DialogHeader>
          <FeeBreakdown
            basicInfo={basicInfo}
            distribution={distribution}
            keywords={keywords}
          />
          <DialogFooter>
            <Button onClick={() => setFeeDialogOpen(false)} data-testid="fee-dialog-close">Close</Button>
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

// ─── FeeBreakdown ───────────────────────────────────────────────────────────

function FeeBreakdown({
  basicInfo,
  distribution,
  keywords,
}: {
  basicInfo: BasicInfoState;
  distribution: DistributionState;
  keywords: KeywordsState;
}) {
  const rows: Array<{ label: string; credits: number }> = [];

  // Base contract registration fee
  rows.push({ label: "Base contract registration", credits: 50_000_000 });

  // Token registration fee
  rows.push({ label: "Token registration", credits: 100_000_000 });

  // Distribution fees
  if (distribution.perpetual.enabled) {
    rows.push({ label: "Perpetual distribution", credits: 50_000_000 });
  }
  if (distribution.preProgrammed.enabled) {
    rows.push({ label: "Pre-programmed distribution", credits: 50_000_000 });
  }

  // Contract keywords
  const contractKeywords = basicInfo.contractKeywords
    .split(",")
    .map((k) => k.trim())
    .filter((k) => k.length >= 3 && k.length <= 50);
  if (contractKeywords.length > 0) {
    rows.push({
      label: `Contract keywords (${contractKeywords.length})`,
      credits: contractKeywords.length * 10_000_000,
    });
  }

  // Searchable token names
  const searchableNames = basicInfo.names.filter(
    (n) => n.singular.trim().length >= 3,
  );
  if (searchableNames.length > 0) {
    rows.push({
      label: `Searchable token names (${searchableNames.length})`,
      credits: searchableNames.length * 10_000_000,
    });
  }

  // Token keywords
  if (keywords.keywords.length > 0) {
    rows.push({
      label: `Token keywords (${keywords.keywords.length})`,
      credits: keywords.keywords.length * 10_000_000,
    });
  }

  // Buffer
  rows.push({ label: "Estimation buffer", credits: 200_000_000 });

  const total = rows.reduce((sum, r) => sum + r.credits, 0);

  return (
    <div className="space-y-3" data-testid="fee-breakdown">
      <div className="space-y-1">
        {rows.map((row, i) => (
          <div key={i} className="flex justify-between text-sm">
            <span className="text-muted-foreground">{row.label}</span>
            <span className="font-mono tabular-nums">
              {formatCreditsAsDash(row.credits)}
            </span>
          </div>
        ))}
      </div>
      <div className="border-t pt-2 flex justify-between text-sm font-semibold">
        <span>Total estimated cost</span>
        <span className="font-mono tabular-nums" data-testid="fee-breakdown-total">
          {formatCreditsAsDash(total)}
        </span>
      </div>
    </div>
  );
}

// ─── buildConfigJson ────────────────────────────────────────────────────────

/**
 * Serialize all wizard state into the JSON structure expected by
 * the Tauri `tokenRegisterContract` IPC command.
 *
 * Field names and structures must match the Rust backend's
 * `serde_json::from_value` calls in `token_register_contract`.
 */
export function buildConfigJson(
  basicInfo: BasicInfoState,
  distribution: DistributionState,
  controlRules: ControlRulesState,
  groups: GroupsState,
  history: HistoryState,
  _keywords: KeywordsState,
): Record<string, unknown> {
  // Build groups as position-keyed object: { "0": { "$format_version": "0", members: {...}, required_power: N } }
  const groupsObj: Record<string, unknown> = {};
  groups.groups.forEach((g, i) => {
    const members: Record<string, number> = {};
    g.members.forEach((m) => {
      if (m.identityId.trim()) {
        members[m.identityId.trim()] = parseInt(m.power, 10) || 0;
      }
    });
    groupsObj[String(i)] = {
      $format_version: "0",
      members,
      required_power: parseInt(g.requiredPower, 10) || 0,
    };
  });

  return {
    // Basic info — backend reads "tokenNames" as Vec<(String, String, String)>
    tokenNames: basicInfo.names.map((n) => [
      n.singular.trim(),
      n.plural.trim(),
      n.languageCode,
    ]),
    tokenDescription: basicInfo.description.trim() || null,
    baseSupply: basicInfo.baseSupply.trim(),
    maxSupply: basicInfo.maxSupply.trim() || null,
    decimals: parseInt(basicInfo.decimals, 10),
    shouldCapitalize: basicInfo.shouldCapitalize,
    startPaused: basicInfo.startPaused,
    allowTransfersToFrozenIdentities: basicInfo.allowTransfersToFrozen,
    contractKeywords: basicInfo.contractKeywords
      .split(",")
      .map((k) => k.trim())
      .filter(Boolean),

    // History — backend reads "keepsHistory" as TokenKeepsHistoryRules (internally tagged)
    keepsHistory: {
      $format_version: "0",
      keepsTransferHistory: history.keepTransferHistory,
      keepsFreezingHistory: history.keepFreezingHistory,
      keepsMintingHistory: history.keepMintingHistory,
      keepsBurningHistory: history.keepBurningHistory,
      keepsDirectPricingHistory: history.keepDirectPricingHistory,
      keepsDirectPurchaseHistory: history.keepDirectPurchaseHistory,
    },

    // Control rules — backend reads each as top-level keys
    manualMintingRules: serializeControlRule(controlRules.manualMinting),
    manualBurningRules: serializeControlRule(controlRules.manualBurning),
    freezeRules: serializeControlRule(controlRules.freeze),
    unfreezeRules: serializeControlRule(controlRules.unfreeze),
    destroyFrozenFundsRules: serializeControlRule(
      controlRules.destroyFrozenFunds,
    ),
    emergencyActionRules: serializeControlRule(controlRules.emergencyAction),
    maxSupplyChangeRules: serializeControlRule(controlRules.maxSupplyChange),
    conventionsChangeRules: serializeControlRule(
      controlRules.conventionsChange,
    ),

    // Main control group change — backend reads as AuthorizedActionTakers
    mainControlGroupChangeAuthorized: serializeActionTaker(
      controlRules.mainControlGroupChange,
      controlRules.mainControlGroupChangeIdentityId,
      controlRules.mainControlGroupChangeGroupPosition,
    ),

    // Distribution — backend reads "distributionRules" as TokenDistributionRules (internally tagged)
    distributionRules: buildDistributionRules(distribution, controlRules),

    // Groups — backend reads as BTreeMap<u16, Group>
    groups: groupsObj,

    // Marketplace — backend reads as top-level keys
    marketplaceTradeMode: 0,
    marketplaceRules: serializeControlRule(controlRules.marketplace),
  };
}

/** Convert an ActionTaker UI value to the Platform SDK AuthorizedActionTakers serde format */
function serializeActionTaker(
  actionTaker: import("./ControlRulesStep").ActionTaker,
  identityId?: string,
  groupPosition?: string,
): unknown {
  switch (actionTaker) {
    case "noOne":
      return "NoOne";
    case "contractOwner":
      return "ContractOwner";
    case "mainGroup":
      return "MainGroup";
    case "identity":
      return { Identity: (identityId || "").trim() || null };
    case "group":
      return { Group: parseInt(groupPosition || "0", 10) };
    default:
      return "NoOne";
  }
}

/** Serialize a ChangeControlRulesState to ChangeControlRules serde format (externally tagged V0) */
function serializeControlRule(
  rule: import("./ControlRulesStep").ChangeControlRulesState,
): Record<string, unknown> {
  return {
    V0: {
      authorized_to_make_change: serializeActionTaker(
        rule.authorizedActionTaker,
        rule.authorizedIdentityId,
        rule.authorizedGroupPosition,
      ),
      admin_action_takers: serializeActionTaker(
        rule.adminActionTaker,
        rule.adminIdentityId,
        rule.adminGroupPosition,
      ),
      changing_authorized_action_takers_to_no_one_allowed:
        rule.changingAuthorizedToNoOneAllowed,
      changing_admin_action_takers_to_no_one_allowed:
        rule.changingAdminToNoOneAllowed,
      self_changing_admin_action_takers_allowed:
        rule.selfChangingAdminAllowed,
    },
  };
}

/** Default ChangeControlRules for fields not exposed in the UI */
function defaultControlRule(): Record<string, unknown> {
  return {
    V0: {
      authorized_to_make_change: "NoOne",
      admin_action_takers: "NoOne",
      changing_authorized_action_takers_to_no_one_allowed: false,
      changing_admin_action_takers_to_no_one_allowed: false,
      self_changing_admin_action_takers_allowed: false,
    },
  };
}

/** Build the distributionRules object matching TokenDistributionRules serde format */
function buildDistributionRules(
  distribution: DistributionState,
  controlRules: ControlRulesState,
): Record<string, unknown> {
  const perp = distribution.perpetual;

  // Build perpetual distribution if enabled
  let perpetualDistribution: unknown = null;
  if (perp.enabled) {
    const fn = buildDistributionFunction(perp);
    let distributionType: unknown;
    if (perp.intervalType === "blockBased") {
      distributionType = {
        BlockBasedDistribution: {
          interval: parseInt(perp.intervalAmount, 10) || 0,
          function: fn,
        },
      };
    } else if (perp.intervalType === "epochBased") {
      distributionType = {
        EpochBasedDistribution: {
          interval: parseInt(perp.intervalAmount, 10) || 0,
          function: fn,
        },
      };
    } else {
      // timeBased — convert to milliseconds based on time unit
      const amount = parseInt(perp.intervalAmount, 10) || 0;
      const msMultiplier: Record<string, number> = {
        second: 1000,
        minute: 60_000,
        hour: 3_600_000,
        day: 86_400_000,
      };
      const intervalMs = amount * (msMultiplier[perp.intervalTimeUnit] || 86_400_000);
      distributionType = {
        TimeBasedDistribution: {
          interval: intervalMs,
          function: fn,
        },
      };
    }

    let distributionRecipient: unknown = "ContractOwner";
    if (perp.recipient === "identity" && perp.recipientIdentityId) {
      distributionRecipient = { Identity: perp.recipientIdentityId.trim() };
    } else if (perp.recipient === "evonodesByParticipation") {
      distributionRecipient = "EvonodesByParticipation";
    }

    perpetualDistribution = {
      $format_version: "0",
      distributionType,
      distributionRecipient,
    };
  }

  // Build pre-programmed distribution if enabled
  let preProgrammedDistribution: unknown = null;
  if (distribution.preProgrammed.enabled && distribution.preProgrammed.entries.length > 0) {
    const distributions: Record<string, Record<string, number>> = {};
    for (const entry of distribution.preProgrammed.entries) {
      const days = parseInt(entry.days || "0", 10);
      const hours = parseInt(entry.hours || "0", 10);
      const minutes = parseInt(entry.minutes || "0", 10);
      const timestampMs = ((days * 24 + hours) * 60 + minutes) * 60_000;
      const amount = parseInt(entry.amount || "0", 10);
      const id = entry.identityId.trim();
      if (id) {
        if (!distributions[String(timestampMs)]) {
          distributions[String(timestampMs)] = {};
        }
        distributions[String(timestampMs)][id] = amount;
      }
    }
    preProgrammedDistribution = {
      $format_version: "0",
      distributions,
    };
  }

  // Determine newTokensDestinationIdentity from mintExtras
  const mintExtras = controlRules.mintExtras;
  let newTokensDestinationIdentity: string | null = null;
  if (mintExtras.destinationDefaultToContractOwner) {
    // Will be resolved server-side to the contract owner
    // For now, we don't send a specific ID — the backend will use the identity
    newTokensDestinationIdentity = null;
  } else if (
    mintExtras.destinationOtherIdentityEnabled &&
    mintExtras.destinationOtherIdentityId.trim()
  ) {
    newTokensDestinationIdentity = mintExtras.destinationOtherIdentityId.trim();
  }

  return {
    $format_version: "0",
    perpetualDistribution,
    perpetualDistributionRules: defaultControlRule(),
    preProgrammedDistribution,
    newTokensDestinationIdentity,
    newTokensDestinationIdentityRules: serializeControlRule(
      mintExtras.destinationIdentityRules,
    ),
    mintingAllowChoosingDestination: mintExtras.allowChoosingDestination,
    mintingAllowChoosingDestinationRules: serializeControlRule(
      mintExtras.choosingDestinationRules,
    ),
    changeDirectPurchasePricingRules: serializeControlRule(
      controlRules.directPurchasePricing,
    ),
  };
}

/** Build a DistributionFunction serde value from the perpetual distribution state */
function buildDistributionFunction(
  perp: DistributionState["perpetual"],
): unknown {
  const parseU64 = (v: string) => parseInt(v, 10) || 0;
  const parseI64 = (v: string) => parseInt(v, 10) || 0;
  const parseOptU64 = (v: string): number | null => {
    if (!v || !v.trim()) return null;
    const n = parseInt(v, 10);
    return isNaN(n) || n === 0 ? null : n;
  };

  switch (perp.distributionFunction) {
    case "fixedAmount":
      return { FixedAmount: { amount: parseU64(perp.fixedAmount) } };
    case "stepDecreasing":
      return {
        StepDecreasingAmount: {
          step_count: parseU64(perp.stepCount),
          decrease_per_interval_numerator: parseU64(perp.decreaseNumerator),
          decrease_per_interval_denominator: parseU64(perp.decreaseDenominator),
          start_decreasing_offset: parseOptU64(perp.startPeriodOffset),
          max_interval_count: parseOptU64(perp.stepDecreasingMaxIntervalCount),
          distribution_start_amount: parseU64(perp.initialEmission),
          trailing_distribution_interval_amount: parseU64(perp.trailingAmount),
          min_value: parseOptU64(perp.stepDecreasingMinValue),
        },
      };
    case "stepwise": {
      const entries: Record<string, number> = {};
      (perp.stepwiseEntries || []).forEach(
        (e: { step: string; amount: string }) => {
          entries[String(parseU64(e.step))] = parseU64(e.amount);
        },
      );
      return { Stepwise: entries };
    }
    case "linear":
      return {
        Linear: {
          a: parseI64(perp.linearA),
          d: parseU64(perp.linearD) || 1,
          start_step: parseOptU64(perp.linearStartStep),
          starting_amount: parseU64(perp.linearStartingAmount),
          min_value: parseOptU64(perp.linearMinValue),
          max_value: parseOptU64(perp.linearMaxValue),
        },
      };
    case "polynomial":
      return {
        Polynomial: {
          a: parseI64(perp.polyA ?? ""),
          d: parseU64(perp.polyD ?? "") || 1,
          m: parseI64(perp.polyM ?? ""),
          n: parseU64(perp.polyN ?? "") || 1,
          o: parseI64(perp.polyO ?? ""),
          start_moment: parseOptU64(perp.polyS ?? ""),
          b: parseU64(perp.polyB ?? ""),
          min_value: parseOptU64(perp.polyMinValue ?? ""),
          max_value: parseOptU64(perp.polyMaxValue ?? ""),
        },
      };
    case "exponential":
      return {
        Exponential: {
          a: parseU64(perp.expA ?? ""),
          d: parseU64(perp.expD ?? "") || 1,
          m: parseI64(perp.expM ?? ""),
          n: parseU64(perp.expN ?? "") || 1,
          o: parseI64(perp.expO ?? ""),
          start_moment: parseOptU64(perp.expS ?? ""),
          b: parseU64(perp.expB ?? ""),
          min_value: parseOptU64(perp.expMinValue ?? ""),
          max_value: parseOptU64(perp.expMaxValue ?? ""),
        },
      };
    case "logarithmic":
      return {
        Logarithmic: {
          a: parseI64(perp.logA ?? ""),
          d: parseU64(perp.logD ?? "") || 1,
          m: parseU64(perp.logM ?? ""),
          n: parseU64(perp.logN ?? "") || 1,
          o: parseI64(perp.logO ?? ""),
          start_moment: parseOptU64(perp.logS ?? ""),
          b: parseU64(perp.logB ?? ""),
          min_value: parseOptU64(perp.logMinValue ?? ""),
          max_value: parseOptU64(perp.logMaxValue ?? ""),
        },
      };
    case "invertedLogarithmic":
      return {
        InvertedLogarithmic: {
          a: parseI64(perp.invLogA ?? ""),
          d: parseU64(perp.invLogD ?? "") || 1,
          m: parseU64(perp.invLogM ?? ""),
          n: parseU64(perp.invLogN ?? "") || 1,
          o: parseI64(perp.invLogO ?? ""),
          start_moment: parseOptU64(perp.invLogS ?? ""),
          b: parseU64(perp.invLogB ?? ""),
          min_value: parseOptU64(perp.invLogMinValue ?? ""),
          max_value: parseOptU64(perp.invLogMaxValue ?? ""),
        },
      };
    default:
      return { FixedAmount: { amount: 0 } };
  }
}
