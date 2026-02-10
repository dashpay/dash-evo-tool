import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  ArrowLeft,
  ChevronDown,
  ChevronUp,
  FileCode2,
  Loader2,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Island, PageHeader } from "@/components/layout";
import { IdentitySelector } from "@/components/shared/IdentitySelector";
import {
  WalletUnlockDialog,
  type WalletUnlockResult,
} from "@/components/shared/WalletUnlockDialog";
import { LoadingSpinner } from "@/components/feedback";
import { commands, events } from "@/bindings";
import type {
  TaskResultEvent,
  TaskErrorEvent,
  QualifiedIdentityDto,
  ContractSummaryDto,
} from "@/bindings";
import { useIdentityStore } from "@/stores/identityStore";
import { useContractStore } from "@/stores/contractStore";
import { useWalletStore } from "@/stores/walletStore";
import type { WalletDto, SingleKeyWalletDto } from "@/bindings";
import { formatAmount } from "@/components/shared/AmountInput";
import { toastError } from "@/lib/toastError";
import { toast } from "sonner";

// Credits per Dash (1 DASH = 100_000_000_000 credits on Platform)
const CREDITS_PER_DASH = 100_000_000_000;

/** System contracts that cannot be updated by users. */
const SYSTEM_CONTRACT_ALIASES = new Set([
  "dpns",
  "keyword_search",
  "token_history",
  "withdrawals",
  "dashpay",
]);

type ScreenStatus =
  | { type: "input" }
  | { type: "broadcasting"; startTime: number }
  | { type: "success" }
  | { type: "error"; message: string };

/**
 * Format credits as DASH (1 DASH = 100,000,000,000 credits).
 */
function formatCreditsAsDash(credits: number): string {
  return (credits / CREDITS_PER_DASH).toFixed(8);
}

/**
 * Auto-select the best key for contract update from an identity.
 * Contract updates require CRITICAL security level AUTHENTICATION keys.
 */
function autoSelectKey(
  identity: QualifiedIdentityDto | null,
): number | null {
  if (!identity) return null;

  const criticalAuthKeys = identity.keys.filter(
    (k) =>
      !k.isDisabled &&
      k.purpose === "AUTHENTICATION" &&
      k.securityLevel === "CRITICAL",
  );

  if (criticalAuthKeys.length > 0) return criticalAuthKeys[0].keyId;
  return null;
}

/**
 * Estimate the update fee for a contract JSON.
 * Rough client-side estimation: base fee + per-byte storage.
 */
function estimateUpdateFee(jsonStr: string): number | null {
  try {
    const bytes = new TextEncoder().encode(jsonStr).length;
    const baseFee = 20 * 5000; // 100,000 credits for tree ops
    const storageFee = bytes * 50;
    return baseFee + storageFee;
  } catch {
    return null;
  }
}

interface WalletInfo {
  seedHash: string;
  alias: string | null;
  usesPassword: boolean;
  passwordHint: string | null;
}

/**
 * Find the wallet associated with an identity by matching wallet hashes.
 */
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

/**
 * Format elapsed time as a human-readable string.
 */
function formatElapsed(ms: number): string {
  const seconds = Math.floor(ms / 1000);
  if (seconds < 60) {
    return `${seconds} ${seconds === 1 ? "second" : "seconds"}`;
  }
  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = seconds % 60;
  return `${minutes} ${minutes === 1 ? "minute" : "minutes"} and ${remainingSeconds} ${remainingSeconds === 1 ? "second" : "seconds"}`;
}

/**
 * Check if a contract is a system contract that should be excluded from updates.
 */
function isSystemContract(contract: ContractSummaryDto): boolean {
  return (
    contract.alias !== null && SYSTEM_CONTRACT_ALIASES.has(contract.alias)
  );
}

/**
 * UpdateContractScreen — Form for updating an existing data contract.
 *
 * Steps:
 * 1. Select identity (CRITICAL auth key required)
 * 2. Select contract to update (excludes system contracts)
 * 3. Edit contract JSON (auto-loaded when contract selected)
 * 4. Review fee estimation
 * 5. Update contract (broadcast)
 *
 * Advanced options: manual key selection
 * Wallet unlock: required before broadcast if wallet is password-protected
 */
export function UpdateContractScreen() {
  const navigate = useNavigate();

  // Stores
  const {
    identities,
    loading: identitiesLoading,
    loadIdentities,
    subscribeToUpdates: subscribeIdentityUpdates,
  } = useIdentityStore();
  const { contracts, loadContracts, getContractById } = useContractStore();
  const { hdWallets, singleKeyWallets, loadWallets } = useWalletStore();

  // Form state
  const [rawIdentityId, setRawIdentityId] = useState<string>("");
  const [manualKeyId, setManualKeyId] = useState<number | null>(null);
  const [selectedContractId, setSelectedContractId] = useState<string>("");
  const [contractJson, setContractJson] = useState("");
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [contractLoading, setContractLoading] = useState(false);

  // Broadcast state
  const [status, setStatus] = useState<ScreenStatus>({ type: "input" });
  const [elapsedMs, setElapsedMs] = useState(0);
  const activeTaskIdRef = useRef<string | null>(null);
  const elapsedTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Wallet unlock state
  const [walletUnlockOpen, setWalletUnlockOpen] = useState(false);
  const [walletUnlockError, setWalletUnlockError] = useState<string | null>(
    null,
  );
  const [walletUnlockedHashes, setWalletUnlockedHashes] = useState<
    Set<string>
  >(new Set());

  // Derived: user-updatable contracts (exclude system contracts)
  const updatableContracts = useMemo(
    () => contracts.filter((c) => !isSystemContract(c)),
    [contracts],
  );

  // Derived: effective identity ID (auto-selects first if none chosen)
  const selectedIdentityId = useMemo(() => {
    if (rawIdentityId && identities.some((i) => i.id === rawIdentityId)) {
      return rawIdentityId;
    }
    return identities.length > 0 ? identities[0].id : "";
  }, [rawIdentityId, identities]);

  // Derived: selected identity
  const selectedIdentity = useMemo(
    () => identities.find((i) => i.id === selectedIdentityId) ?? null,
    [identities, selectedIdentityId],
  );

  // Derived: identity options for selector
  const identityOptions = useMemo(
    () =>
      identities.map((i) => ({
        id: i.id,
        displayName: i.alias || i.id.slice(0, 16) + "...",
      })),
    [identities],
  );

  // Derived: eligible keys — CRITICAL AUTHENTICATION keys only for updates
  const eligibleKeys = useMemo(() => {
    if (!selectedIdentity) return [];
    return selectedIdentity.keys.filter(
      (k) =>
        !k.isDisabled &&
        k.purpose === "AUTHENTICATION" &&
        k.securityLevel === "CRITICAL",
    );
  }, [selectedIdentity]);

  // Derived: auto-selected key (use manual override if set, otherwise auto-select)
  const selectedKeyId = useMemo(() => {
    if (manualKeyId !== null) {
      if (eligibleKeys.some((k) => k.keyId === manualKeyId)) {
        return manualKeyId;
      }
    }
    return autoSelectKey(selectedIdentity);
  }, [selectedIdentity, manualKeyId, eligibleKeys]);

  // Derived: parse contract JSON
  const { parsedJson, parseError, estimatedFee } = useMemo(() => {
    if (!contractJson.trim()) {
      return { parsedJson: null, parseError: null, estimatedFee: null };
    }
    try {
      const parsed = JSON.parse(contractJson);
      if (
        typeof parsed !== "object" ||
        parsed === null ||
        Array.isArray(parsed)
      ) {
        return {
          parsedJson: null,
          parseError: "Contract JSON must be an object.",
          estimatedFee: null,
        };
      }
      // Override ownerId with the selected identity
      let contractObj = parsed as Record<string, unknown>;
      if (selectedIdentityId) {
        contractObj = { ...contractObj, ownerId: selectedIdentityId };
      }
      const jsonStr = JSON.stringify(contractObj);
      const fee = estimateUpdateFee(jsonStr);
      return { parsedJson: contractObj, parseError: null, estimatedFee: fee };
    } catch (e) {
      return {
        parsedJson: null,
        parseError: `Invalid JSON: ${e instanceof Error ? e.message : String(e)}`,
        estimatedFee: null,
      };
    }
  }, [contractJson, selectedIdentityId]);

  // Derived: associated wallet for the selected identity
  const associatedWallet = useMemo(
    () => findAssociatedWallet(selectedIdentity, hdWallets, singleKeyWallets),
    [selectedIdentity, hdWallets, singleKeyWallets],
  );

  // Derived: wallet locked state
  const walletLocked =
    !!associatedWallet &&
    associatedWallet.usesPassword &&
    !walletUnlockedHashes.has(associatedWallet.seedHash);

  // Load identities, contracts, and wallets on mount
  useEffect(() => {
    loadIdentities();
    loadContracts();
    loadWallets();
  }, [loadIdentities, loadContracts, loadWallets]);

  // Subscribe to identity updates
  useEffect(() => {
    let cleanup: (() => void) | undefined;
    subscribeIdentityUpdates().then((unsub) => {
      cleanup = unsub;
    });
    return () => {
      cleanup?.();
    };
  }, [subscribeIdentityUpdates]);

  // Elapsed time ticker
  useEffect(() => {
    if (status.type === "broadcasting") {
      elapsedTimerRef.current = setInterval(() => {
        setElapsedMs(Date.now() - status.startTime);
      }, 200);
    } else {
      if (elapsedTimerRef.current) {
        clearInterval(elapsedTimerRef.current);
        elapsedTimerRef.current = null;
      }
    }
    return () => {
      if (elapsedTimerRef.current) {
        clearInterval(elapsedTimerRef.current);
      }
    };
  }, [status]);

  // Subscribe to task events
  useEffect(() => {
    let cleanupResult: (() => void) | undefined;
    let cleanupError: (() => void) | undefined;

    const subscribe = async () => {
      cleanupResult = await events.taskResultEvent.listen(
        (event: { payload: TaskResultEvent }) => {
          const { taskId, resultType } = event.payload;
          if (resultType !== "Contract") return;
          if (activeTaskIdRef.current !== taskId) return;

          activeTaskIdRef.current = null;
          setStatus({ type: "success" });
          toast.success("Contract updated successfully!");
          loadContracts();
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
  }, [loadContracts]);

  // --- Handlers ---

  const handleIdentityChange = useCallback((id: string) => {
    setRawIdentityId(id);
    setManualKeyId(null);
  }, []);

  const handleContractSelect = useCallback(
    async (contractId: string) => {
      setSelectedContractId(contractId);
      if (!contractId) {
        setContractJson("");
        return;
      }
      setContractLoading(true);
      try {
        const detail = await getContractById(contractId);
        if (detail && detail.schemaJson) {
          setContractJson(JSON.stringify(detail.schemaJson, null, 2));
        } else {
          setContractJson("");
        }
      } catch {
        setContractJson("");
      } finally {
        setContractLoading(false);
      }
    },
    [getContractById],
  );

  const handleUpdate = useCallback(async () => {
    if (!parsedJson || !selectedIdentityId || selectedKeyId === null) return;

    setStatus({ type: "broadcasting", startTime: Date.now() });
    setElapsedMs(0);

    try {
      const result = await commands.contractUpdate({
        contractJson: parsedJson as unknown as import("@/bindings").JsonValue,
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
  }, [parsedJson, selectedIdentityId, selectedKeyId]);

  const handleBack = useCallback(() => {
    navigate({ to: "/contracts" });
  }, [navigate]);

  const handleDismissError = useCallback(() => {
    setStatus({ type: "input" });
  }, []);

  const handleUpdateAnother = useCallback(() => {
    setStatus({ type: "input" });
    setContractJson("");
    setSelectedContractId("");
    setManualKeyId(null);
  }, []);

  // Wallet unlock handlers
  const handleRequestUnlock = useCallback(() => {
    setWalletUnlockOpen(true);
    setWalletUnlockError(null);
  }, []);

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

  // Can update: valid parsed JSON, identity selected, key selected, contract selected, not broadcasting
  const canUpdate =
    status.type === "input" &&
    parsedJson !== null &&
    !!selectedIdentityId &&
    selectedKeyId !== null &&
    !!selectedContractId &&
    !walletLocked;

  // Show loading while identities load initially
  if (identitiesLoading && identities.length === 0) {
    return (
      <div className="flex flex-1 items-center justify-center">
        <LoadingSpinner label="Loading identities..." />
      </div>
    );
  }

  return (
    <div className="flex flex-1 flex-col gap-6 overflow-auto p-6">
      <PageHeader
        title="Update Data Contract"
        breadcrumbs={[
          { label: "Contracts", href: "/contracts" },
          { label: "Update Contract" },
        ]}
        actions={
          <Button variant="outline" size="sm" onClick={handleBack}>
            <ArrowLeft className="size-4 mr-2" />
            Back to Contracts
          </Button>
        }
      />

      <Island>
        <div className="flex flex-col gap-6 p-6 max-w-2xl">
          {/* --- INPUT PHASE --- */}
          {(status.type === "input" || status.type === "error") && (
            <>
              {/* Step 1: Identity selection */}
              <div className="space-y-3">
                <h3 className="text-sm font-semibold">1. Select Identity</h3>
                <IdentitySelector
                  value={selectedIdentityId}
                  onChange={handleIdentityChange}
                  identities={identityOptions}
                  showOther={false}
                  label="Contract Owner"
                  disabled={status.type === "error"}
                />
                {selectedIdentity && (
                  <p className="text-xs text-muted-foreground">
                    Balance:{" "}
                    {formatAmount(selectedIdentity.balance, 8)} DASH
                  </p>
                )}
                {identities.length === 0 && (
                  <p className="text-xs text-warning">
                    No identities loaded. Please load or create an identity
                    first.
                  </p>
                )}
                {selectedIdentity && eligibleKeys.length === 0 && (
                  <p className="text-xs text-destructive">
                    No critical authentication keys available. Contract updates
                    require CRITICAL security level AUTHENTICATION keys.
                  </p>
                )}
              </div>

              {/* Advanced Options toggle */}
              <button
                type="button"
                className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground transition-colors w-fit"
                onClick={() => setShowAdvanced(!showAdvanced)}
                aria-expanded={showAdvanced}
              >
                {showAdvanced ? (
                  <ChevronUp className="size-3.5" />
                ) : (
                  <ChevronDown className="size-3.5" />
                )}
                Advanced Options
              </button>

              {/* Advanced: Key selector */}
              {showAdvanced && selectedIdentity && (
                <div className="space-y-1.5 pl-4 border-l-2 border-muted">
                  <Label className="text-sm font-medium">Signing Key</Label>
                  <Select
                    value={
                      selectedKeyId !== null ? String(selectedKeyId) : ""
                    }
                    onValueChange={(val) =>
                      setManualKeyId(parseInt(val, 10))
                    }
                  >
                    <SelectTrigger
                      className="w-full"
                      aria-label="Signing key"
                    >
                      <SelectValue placeholder="Select key" />
                    </SelectTrigger>
                    <SelectContent>
                      {eligibleKeys.map((k) => (
                        <SelectItem
                          key={k.keyId}
                          value={String(k.keyId)}
                        >
                          Key {k.keyId} — {k.securityLevel} (
                          {k.keyType})
                        </SelectItem>
                      ))}
                      {eligibleKeys.length === 0 && (
                        <SelectItem value="__none__" disabled>
                          No eligible keys
                        </SelectItem>
                      )}
                    </SelectContent>
                  </Select>
                  <p className="text-xs text-muted-foreground">
                    Contract updates require CRITICAL security level
                    AUTHENTICATION keys.
                  </p>
                </div>
              )}

              {/* Step 2: Contract selection */}
              <div className="space-y-2">
                <h3 className="text-sm font-semibold">
                  2. Select Contract to Update
                </h3>
                <Select
                  value={selectedContractId}
                  onValueChange={handleContractSelect}
                >
                  <SelectTrigger
                    className="w-full"
                    aria-label="Contract to update"
                  >
                    <SelectValue placeholder="Select a contract..." />
                  </SelectTrigger>
                  <SelectContent>
                    {updatableContracts.map((c) => (
                      <SelectItem key={c.id} value={c.id}>
                        {c.alias || c.id.slice(0, 16) + "..."}
                      </SelectItem>
                    ))}
                    {updatableContracts.length === 0 && (
                      <SelectItem value="__none__" disabled>
                        No contracts available
                      </SelectItem>
                    )}
                  </SelectContent>
                </Select>
                {updatableContracts.length === 0 && (
                  <p className="text-xs text-muted-foreground">
                    No user contracts found. Add contracts first from the
                    contracts browser.
                  </p>
                )}
              </div>

              {/* Step 3: Contract JSON */}
              <div className="space-y-2">
                <h3 className="text-sm font-semibold">
                  3. Edit the contract JSON
                </h3>
                <p className="text-xs text-muted-foreground">
                  The current contract JSON is loaded automatically. Edit the
                  schema and submit the update.
                </p>
                {contractLoading ? (
                  <div className="flex items-center gap-2 py-4 text-sm text-muted-foreground">
                    <Loader2 className="size-4 animate-spin" />
                    Loading contract...
                  </div>
                ) : (
                  <Textarea
                    value={contractJson}
                    onChange={(e) => setContractJson(e.target.value)}
                    placeholder="Select a contract above to load its JSON schema..."
                    rows={12}
                    className="font-mono text-sm"
                    aria-label="Contract JSON"
                    disabled={!selectedContractId}
                  />
                )}
              </div>

              {/* Parse error */}
              {parseError && (
                <div className="flex items-start gap-3 rounded-lg border border-destructive/30 bg-destructive/5 p-4">
                  <p className="text-sm text-destructive">{parseError}</p>
                </div>
              )}

              {/* Fee estimation */}
              {estimatedFee !== null && !parseError && (
                <div className="rounded-lg border bg-muted/30 p-4">
                  <p className="text-sm text-muted-foreground">
                    Estimated Fee:{" "}
                    <span className="font-mono font-medium text-foreground">
                      {formatCreditsAsDash(estimatedFee)} DASH
                    </span>
                  </p>
                </div>
              )}

              {/* Wallet locked warning */}
              {walletLocked && (
                <div className="flex items-center gap-3 rounded-lg border border-warning/30 bg-warning/5 p-4">
                  <p className="text-sm text-warning flex-1">
                    Wallet is locked. Please unlock to continue.
                  </p>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={handleRequestUnlock}
                  >
                    Unlock Wallet
                  </Button>
                </div>
              )}

              {/* Broadcast error */}
              {status.type === "error" && (
                <div className="flex items-start gap-3 rounded-lg border border-destructive/30 bg-destructive/5 p-4">
                  <div className="flex-1 min-w-0">
                    <p className="text-sm font-medium text-destructive">
                      Update Failed
                    </p>
                    <p className="mt-1 text-sm text-destructive/80">
                      {status.message}
                    </p>
                  </div>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={handleDismissError}
                    className="shrink-0 text-destructive hover:text-destructive"
                  >
                    Dismiss
                  </Button>
                </div>
              )}

              {/* Update button */}
              <div>
                <Button onClick={handleUpdate} disabled={!canUpdate}>
                  <FileCode2 className="size-4 mr-2" />
                  Update Contract
                </Button>
              </div>
            </>
          )}

          {/* --- BROADCASTING PHASE --- */}
          {status.type === "broadcasting" && (
            <div className="flex flex-col items-center gap-4 py-12">
              <Loader2 className="size-8 animate-spin text-dash-blue" />
              <p className="text-sm text-muted-foreground">
                Broadcasting contract update... Time taken so far:{" "}
                {formatElapsed(elapsedMs)}
              </p>
            </div>
          )}

          {/* --- SUCCESS PHASE --- */}
          {status.type === "success" && (
            <div className="flex flex-col items-center gap-6 py-12">
              <div className="flex size-16 items-center justify-center rounded-full bg-success/10">
                <FileCode2 className="size-8 text-success" />
              </div>
              <div className="text-center space-y-1">
                <h3 className="text-lg font-semibold">
                  Contract Updated Successfully
                </h3>
                <p className="text-sm text-muted-foreground">
                  Your data contract has been updated on Platform.
                </p>
              </div>
              <div className="flex gap-3">
                <Button variant="outline" onClick={handleBack}>
                  <ArrowLeft className="size-4 mr-2" />
                  Back to Contracts
                </Button>
                <Button onClick={handleUpdateAnother}>
                  Update Another Contract
                </Button>
              </div>
            </div>
          )}
        </div>
      </Island>

      {/* Wallet unlock dialog */}
      {associatedWallet && (
        <WalletUnlockDialog
          open={walletUnlockOpen}
          onOpenChange={setWalletUnlockOpen}
          walletAlias={
            associatedWallet.alias ||
            associatedWallet.seedHash.slice(0, 10)
          }
          passwordHint={associatedWallet.passwordHint ?? null}
          error={walletUnlockError}
          onResult={handleWalletUnlockResult}
        />
      )}
    </div>
  );
}
