import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  ArrowLeft,
  ChevronDown,
  ChevronUp,
  ExternalLink,
  FileCode2,
  Info,
  Loader2,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
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
import type { TaskResultEvent, TaskErrorEvent, QualifiedIdentityDto } from "@/bindings";
import { useIdentityStore } from "@/stores/identityStore";
import { useContractStore } from "@/stores/contractStore";
import { useWalletStore } from "@/stores/walletStore";
import type { WalletDto, SingleKeyWalletDto } from "@/bindings";
import { formatAmount } from "@/components/shared/AmountInput";
import { toastError } from "@/lib/toastError";
import { toast } from "sonner";

// Credits per Dash (1 DASH = 100_000_000_000 credits on Platform)
const CREDITS_PER_DASH = 100_000_000_000;

type ScreenStatus =
  | { type: "input" }
  | { type: "broadcasting"; startTime: number }
  | { type: "success" }
  | { type: "error"; message: string };

interface ParseResult {
  parsedJson: Record<string, unknown> | null;
  parseError: string | null;
  wasWrapped: boolean;
  estimatedFee: number | null;
}

/**
 * Check if a JSON object looks like raw document schemas (without contract wrapper).
 *
 * Detection: top-level object has no contract metadata fields ($format_version, id,
 * version, documentSchemas) AND at least one value looks like a document schema
 * (has type, properties, or indices fields).
 */
function looksLikeRawDocumentSchemas(json: Record<string, unknown>): boolean {
  // If it has contract metadata keys, it's already a full contract
  if (
    "$format_version" in json ||
    "id" in json ||
    "version" in json ||
    "documentSchemas" in json
  ) {
    return false;
  }

  // Check if at least one value looks like a document schema
  for (const value of Object.values(json)) {
    if (value && typeof value === "object" && !Array.isArray(value)) {
      const v = value as Record<string, unknown>;
      if ("type" in v || "properties" in v || "indices" in v) {
        return true;
      }
    }
  }
  return false;
}

/**
 * Wrap raw document schemas in a full contract JSON structure.
 */
function wrapDocumentSchemas(
  schemas: Record<string, unknown>,
  ownerId: string,
): Record<string, unknown> {
  // Generate a random 32-byte hex identifier
  const idBytes = new Uint8Array(32);
  crypto.getRandomValues(idBytes);
  const id = Array.from(idBytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");

  return {
    $format_version: "0",
    id,
    ownerId,
    version: 1,
    documentSchemas: schemas,
    config: {
      $format_version: "0",
      canBeDeleted: false,
      readonly: false,
      keepsHistory: false,
      documentsKeepHistoryContractDefault: false,
      documentsMutableContractDefault: true,
      documentsCanBeDeletedContractDefault: false,
      requiresIdentityEncryptionBoundedKey: null,
      requiresIdentityDecryptionBoundedKey: null,
    },
  };
}

/**
 * Format credits as DASH (1 DASH = 100,000,000,000 credits).
 */
function formatCreditsAsDash(credits: number): string {
  return (credits / CREDITS_PER_DASH).toFixed(8);
}

/**
 * Auto-select the best key for contract registration from an identity.
 * Prefers HIGH or CRITICAL security level AUTHENTICATION keys.
 */
function autoSelectKey(
  identity: QualifiedIdentityDto | null,
): number | null {
  if (!identity) return null;

  const authKeys = identity.keys.filter(
    (k) => !k.isDisabled && k.purpose === "AUTHENTICATION",
  );

  // Prefer HIGH, then CRITICAL
  const highKey = authKeys.find((k) => k.securityLevel === "HIGH");
  if (highKey) return highKey.keyId;

  const criticalKey = authKeys.find((k) => k.securityLevel === "CRITICAL");
  if (criticalKey) return criticalKey.keyId;

  // Fall back to any auth key
  if (authKeys.length > 0) return authKeys[0]!.keyId;

  return null;
}

/**
 * Estimate the registration fee for a contract JSON.
 * This is a rough client-side estimation: base fee + per-byte storage.
 *
 * The actual fee is calculated server-side, but this gives users a ballpark.
 */
function estimateRegistrationFee(jsonStr: string): number | null {
  try {
    const bytes = new TextEncoder().encode(jsonStr).length;
    // Approximate: ~40 credits per byte for storage + base overhead of ~20 tree operations
    // Each tree operation is ~5000 credits, plus per-byte cost ~50 credits
    const baseFee = 20 * 5000; // 100,000 credits for tree ops
    const storageFee = bytes * 50;
    return baseFee + storageFee;
  } catch {
    return null;
  }
}

/**
 * Parse contract JSON input into a validated contract object.
 */
function parseContractInput(
  contractJson: string,
  selectedIdentityId: string,
): ParseResult {
  const empty: ParseResult = {
    parsedJson: null,
    parseError: null,
    wasWrapped: false,
    estimatedFee: null,
  };

  if (!contractJson.trim()) return empty;

  try {
    const parsed = JSON.parse(contractJson);
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
      return { ...empty, parseError: "Contract JSON must be an object." };
    }

    let contractObj = parsed as Record<string, unknown>;
    let wrapped = false;

    // Check if it looks like raw document schemas
    if (looksLikeRawDocumentSchemas(contractObj)) {
      if (!selectedIdentityId) {
        return {
          ...empty,
          parseError:
            "Please select an identity before pasting raw document schemas.",
        };
      }
      contractObj = wrapDocumentSchemas(contractObj, selectedIdentityId);
      wrapped = true;
    } else {
      // Override ownerId with the selected identity
      if (selectedIdentityId) {
        contractObj = { ...contractObj, ownerId: selectedIdentityId };
      }
    }

    const jsonStr = JSON.stringify(contractObj);
    const fee = estimateRegistrationFee(jsonStr);

    return {
      parsedJson: contractObj,
      parseError: null,
      wasWrapped: wrapped,
      estimatedFee: fee,
    };
  } catch (e) {
    return {
      ...empty,
      parseError: `Invalid JSON: ${e instanceof Error ? e.message : String(e)}`,
    };
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
 * RegisterContractScreen — Step-by-step form for registering a new data contract.
 *
 * Steps:
 * 1. Select identity (auto-selects HIGH/CRITICAL auth key)
 * 2. (Optional) Enter contract alias
 * 3. Paste contract JSON (auto-detects raw schemas and wraps)
 * 4. Review fee estimation
 * 5. Register contract (broadcast)
 *
 * Advanced options: manual key selection
 * Wallet unlock: required before broadcast if wallet is password-protected
 */
export function RegisterContractScreen() {
  const navigate = useNavigate();

  // Stores
  const identities = useIdentityStore((s) => s.identities);
  const identitiesLoading = useIdentityStore((s) => s.loading);
  const loadIdentities = useIdentityStore((s) => s.loadIdentities);
  const subscribeIdentityUpdates = useIdentityStore((s) => s.subscribeToUpdates);
  const { loadContracts } = useContractStore();
  const hdWallets = useWalletStore((s) => s.hdWallets);
  const singleKeyWallets = useWalletStore((s) => s.singleKeyWallets);
  const loadWallets = useWalletStore((s) => s.loadWallets);

  // Form state — raw selection (empty string = "auto-select first")
  const [rawIdentityId, setRawIdentityId] = useState<string>("");
  const [manualKeyId, setManualKeyId] = useState<number | null>(null);
  const [alias, setAlias] = useState("");
  const [contractJson, setContractJson] = useState("");
  const [showAdvanced, setShowAdvanced] = useState(false);

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
  const [walletUnlockedHashes, setWalletUnlockedHashes] = useState<Set<string>>(
    new Set(),
  );

  // Derived: effective identity ID (auto-selects first if none chosen)
  const selectedIdentityId = useMemo(() => {
    if (rawIdentityId && identities.some((i) => i.id === rawIdentityId)) {
      return rawIdentityId;
    }
    return identities.length > 0 ? identities[0]!.id : "";
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

  // Derived: eligible keys for advanced selection
  const eligibleKeys = useMemo(() => {
    if (!selectedIdentity) return [];
    return selectedIdentity.keys.filter(
      (k) => !k.isDisabled && k.purpose === "AUTHENTICATION",
    );
  }, [selectedIdentity]);

  // Derived: auto-selected key (use manual override if set, otherwise auto-select)
  const selectedKeyId = useMemo(() => {
    if (manualKeyId !== null) {
      // Validate that the manual key is still eligible
      if (eligibleKeys.some((k) => k.keyId === manualKeyId)) {
        return manualKeyId;
      }
    }
    return autoSelectKey(selectedIdentity);
  }, [selectedIdentity, manualKeyId, eligibleKeys]);

  // Derived: parse contract JSON
  const { parsedJson, parseError, wasWrapped, estimatedFee } = useMemo(
    () => parseContractInput(contractJson, selectedIdentityId),
    [contractJson, selectedIdentityId],
  );

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

  // Load identities and wallets on mount
  useEffect(() => {
    loadIdentities();
    loadWallets();
  }, [loadIdentities, loadWallets]);

  // Subscribe to identity updates
  useEffect(() => {
    let cleanup: (() => void) | undefined;
    subscribeIdentityUpdates()
      .then((unsub) => {
        cleanup = unsub;
      })
      .catch((e) => console.error("Failed to subscribe to identity events:", e));
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
          const { taskId, result } = event.payload;
          if (result.type !== "contractCompleted") return;
          if (activeTaskIdRef.current !== taskId) return;

          activeTaskIdRef.current = null;
          setStatus({ type: "success" });
          toast.success("Contract registered successfully!");
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

  const handleIdentityChange = useCallback(
    (id: string) => {
      setRawIdentityId(id);
      setManualKeyId(null); // Reset manual key when identity changes
    },
    [],
  );

  const handleRegister = useCallback(async () => {
    if (!parsedJson || !selectedIdentityId || selectedKeyId === null) return;

    setStatus({ type: "broadcasting", startTime: Date.now() });
    setElapsedMs(0);

    try {
      const result = await commands.contractRegister({
        contractJson: parsedJson as unknown as import("@/bindings").JsonValue,
        alias: alias.trim(),
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
  }, [parsedJson, selectedIdentityId, selectedKeyId, alias]);

  const handleBack = useCallback(() => {
    navigate({ to: "/contracts" });
  }, [navigate]);

  const handleDismissError = useCallback(() => {
    setStatus({ type: "input" });
  }, []);

  const handleRegisterAnother = useCallback(() => {
    setStatus({ type: "input" });
    setContractJson("");
    setAlias("");
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

  // Can register: valid parsed JSON, identity selected, key selected, not broadcasting
  const canRegister =
    status.type === "input" &&
    parsedJson !== null &&
    !!selectedIdentityId &&
    selectedKeyId !== null &&
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
        title="Register Data Contract"
        breadcrumbs={[
          { label: "Contracts" },
          { label: "Register Contract" },
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
                    Contract registration requires HIGH or CRITICAL security
                    level AUTHENTICATION keys.
                  </p>
                </div>
              )}

              {/* Step 2: Contract alias */}
              <div className="space-y-1.5">
                <h3 className="text-sm font-semibold">
                  2. Contract Alias (optional)
                </h3>
                <Input
                  type="text"
                  value={alias}
                  onChange={(e) => setAlias(e.target.value)}
                  placeholder="e.g., My DApp Contract"
                  className="max-w-sm"
                  aria-label="Contract alias"
                />
              </div>

              {/* Step 3: Contract JSON */}
              <div className="space-y-2">
                <h3 className="text-sm font-semibold">
                  3. Paste the contract JSON below
                </h3>
                <p className="text-xs text-muted-foreground">
                  You can paste either a full contract JSON or just the document
                  schemas (they will be auto-wrapped).{" "}
                  <a
                    href="https://dashpay.io"
                    target="_blank"
                    rel="noopener noreferrer"
                    className="inline-flex items-center gap-1 text-dash-blue hover:underline"
                  >
                    Create a contract on dashpay.io
                    <ExternalLink className="size-3" />
                  </a>
                </p>
                <Textarea
                  value={contractJson}
                  onChange={(e) => setContractJson(e.target.value)}
                  placeholder='{"note": {"type": "object", "properties": {"message": {"type": "string"}}, "additionalProperties": false}}'
                  rows={12}
                  className="font-mono text-sm"
                  aria-label="Contract JSON"
                />
              </div>

              {/* Auto-wrap notification */}
              {wasWrapped && (
                <div className="flex items-start gap-3 rounded-lg border border-success/30 bg-success/5 p-4">
                  <Info className="size-4 text-success shrink-0 mt-0.5" />
                  <p className="text-sm text-success">
                    Raw document schemas detected. Contract metadata (format
                    version, ID, owner, config) was auto-populated.
                  </p>
                </div>
              )}

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
                      Registration Failed
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

              {/* Register button */}
              <div>
                <Button
                  onClick={handleRegister}
                  disabled={!canRegister}
                >
                  <FileCode2 className="size-4 mr-2" />
                  Register Contract
                </Button>
              </div>
            </>
          )}

          {/* --- BROADCASTING PHASE --- */}
          {status.type === "broadcasting" && (
            <div className="flex flex-col items-center gap-4 py-12">
              <Loader2 className="size-8 animate-spin text-dash-blue" />
              <p className="text-sm text-muted-foreground">
                Broadcasting contract registration... Time taken so far:{" "}
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
                  Contract Registered Successfully
                </h3>
                <p className="text-sm text-muted-foreground">
                  Your data contract has been registered on Platform.
                </p>
              </div>
              <div className="flex gap-3">
                <Button variant="outline" onClick={handleBack}>
                  <ArrowLeft className="size-4 mr-2" />
                  Back to Contracts
                </Button>
                <Button onClick={handleRegisterAnother}>
                  Register Another Contract
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
