import { useCallback, useEffect, useMemo, useState } from "react";
import { useElapsedTimer } from "@/hooks/useElapsedTimer";
import {
  ArrowLeft,
  CheckCircle2,
  AlertCircle,
  Loader2,
  ChevronDown,
  ChevronUp,
  Info,
  AlertTriangle,
  Lock,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { formatCreditsAsDash } from "@/components/shared/AmountInput";
import { InlineError } from "@/components/feedback/InlineError";
import type { QualifiedIdentityDto, IdentityKeyDto } from "@/bindings";

// ─── Constants ─────────────────────────────────────────────────────

/** Approximate cost for a contested name registration (~0.2006 DASH). */
const CONTESTED_NAME_COST_DASH = "≈ 0.2006 Dash";

/** Estimated fee for a non-contested registration (2 document batch). */
const ESTIMATED_FEE_CREDITS = 200_000_000;

// ─── Types ─────────────────────────────────────────────────────────

export type RegisterDpnsNameStatus =
  | { type: "form" }
  | { type: "registering"; startedAt: number }
  | { type: "success"; contested: boolean; feeEstimated: number | null; feeActual: number | null }
  | { type: "error"; message: string };

/** Where the user navigated from — affects breadcrumb trail. */
export type RegisterDpnsNameSource = "identities" | "dpns";

export interface RegisterDpnsNameScreenProps {
  /** Available identities for selection. */
  identities: QualifiedIdentityDto[];
  /** Pre-selected identity ID (e.g. from the identity detail panel). */
  preselectedIdentityId?: string | null;
  /** Current status. */
  status: RegisterDpnsNameStatus;
  /** Where the user came from. */
  source?: RegisterDpnsNameSource;
  /** Whether the selected identity's wallet is locked. */
  walletLocked?: boolean;
  /** Called when the user requests to unlock the wallet. */
  onRequestUnlock?: () => void;
  /** Called when the selected identity changes (for parent wallet lookup). */
  onIdentityChange?: (identityId: string) => void;
  /** Called to submit the registration. */
  onSubmit?: (params: { identityId: string; name: string }) => void;
  /** Called to dismiss an error. */
  onDismissError?: () => void;
  /** Called to go back. */
  onBack?: () => void;
  /** Called to register another name (resets form). */
  onRegisterAnother?: () => void;
}

// ─── Validation ────────────────────────────────────────────────────

export type DpnsNameValidation =
  | { valid: true }
  | { valid: false; error: string };

export function validateDpnsName(name: string): DpnsNameValidation {
  const trimmed = name.trim();
  if (trimmed.length === 0) {
    return { valid: false, error: "" }; // empty = no error shown
  }
  if (trimmed.length < 3) {
    return { valid: false, error: "Name must be at least 3 characters long" };
  }
  if (trimmed.length > 63) {
    return { valid: false, error: "Name must be no more than 63 characters long" };
  }
  if (trimmed.startsWith("-")) {
    return { valid: false, error: "Name cannot start with a hyphen" };
  }
  if (trimmed.endsWith("-")) {
    return { valid: false, error: "Name cannot end with a hyphen" };
  }
  for (const c of trimmed) {
    if (!/[a-zA-Z0-9-]/.test(c)) {
      return {
        valid: false,
        error: `Invalid character '${c}'. Only letters, numbers, and hyphens are allowed`,
      };
    }
  }
  return { valid: true };
}

/**
 * A name is contested if:
 * 1. Length < 20 characters
 * 2. Contains no digits except 0 and 1
 */
export function isContestedName(name: string): boolean {
  const trimmed = name.trim().toLowerCase();
  if (trimmed.length === 0) return false;
  if (trimmed.length >= 20) return false;
  for (const c of trimmed) {
    if (c >= "2" && c <= "9") return false;
  }
  return true;
}

// ─── Helpers ───────────────────────────────────────────────────────

/** Get suitable signing keys for DPNS: AUTHENTICATION purpose, CRITICAL/HIGH/MEDIUM level, not MASTER. */
function getDpnsSigningKeys(identity: QualifiedIdentityDto): IdentityKeyDto[] {
  return identity.keys.filter(
    (k) =>
      k.purpose.toUpperCase() === "AUTHENTICATION" &&
      ["CRITICAL", "HIGH", "MEDIUM"].includes(k.securityLevel.toUpperCase()) &&
      k.securityLevel.toUpperCase() !== "MASTER" &&
      k.hasPrivateKey &&
      !k.isDisabled,
  );
}

function getKeyLabel(key: IdentityKeyDto): string {
  return `Key #${key.keyId} (${key.keyType}, ${key.securityLevel})`;
}

// ─── Component ─────────────────────────────────────────────────────

export function RegisterDpnsNameScreen({
  identities,
  preselectedIdentityId,
  status,
  source = "identities",
  walletLocked = false,
  onRequestUnlock,
  onIdentityChange,
  onSubmit,
  onDismissError,
  onBack,
  onRegisterAnother,
}: RegisterDpnsNameScreenProps) {
  // ─── Elapsed timer ────────────────────────────────────────────────
  const registeringStartedAt = status.type === "registering" ? status.startedAt : null;
  const elapsed = useElapsedTimer(registeringStartedAt);

  // ─── State ──────────────────────────────────────────────────────

  const [selectedIdentityId, setSelectedIdentityId] = useState<string>(
    preselectedIdentityId ?? identities[0]?.id ?? "",
  );
  const [nameInput, setNameInput] = useState("");
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [manualKeyId, setManualKeyId] = useState<number | null>(null);

  // ─── Derived state ──────────────────────────────────────────────

  const selectedIdentity = useMemo(
    () => identities.find((i) => i.id === selectedIdentityId) ?? null,
    [identities, selectedIdentityId],
  );

  const signingKeys = useMemo(
    () => (selectedIdentity ? getDpnsSigningKeys(selectedIdentity) : []),
    [selectedIdentity],
  );

  // Auto-select first available key (derived, not effect-based)
  const selectedKeyId = useMemo(() => {
    if (signingKeys.length === 0) return null;
    if (manualKeyId !== null && signingKeys.some((k) => k.keyId === manualKeyId)) {
      return manualKeyId;
    }
    return signingKeys[0]?.keyId ?? null;
  }, [signingKeys, manualKeyId]);

  const validation = useMemo(
    () => validateDpnsName(nameInput),
    [nameInput],
  );

  const contested = useMemo(
    () => validation.valid && isContestedName(nameInput),
    [validation, nameInput],
  );

  const estimatedFeeDisplay = useMemo(
    () => formatCreditsAsDash(ESTIMATED_FEE_CREDITS),
    [],
  );

  const hasInsufficientBalance = useMemo(() => {
    if (!selectedIdentity) return false;
    return selectedIdentity.balance < ESTIMATED_FEE_CREDITS;
  }, [selectedIdentity]);

  const canSubmit = useMemo(() => {
    if (!selectedIdentity) return false;
    if (!validation.valid) return false;
    if (nameInput.trim().length === 0) return false;
    if (signingKeys.length === 0) return false;
    if (hasInsufficientBalance) return false;
    if (walletLocked) return false;
    if (status.type !== "form") return false;
    return true;
  }, [selectedIdentity, validation, nameInput, signingKeys, hasInsufficientBalance, walletLocked, status]);

  // ─── Callbacks ──────────────────────────────────────────────────

  const handleSubmit = useCallback(() => {
    if (!canSubmit || !selectedIdentityId) return;
    onSubmit?.({ identityId: selectedIdentityId, name: nameInput.trim() });
  }, [canSubmit, selectedIdentityId, nameInput, onSubmit]);

  const handleRegisterAnother = useCallback(() => {
    setNameInput("");
    onRegisterAnother?.();
  }, [onRegisterAnother]);

  const handleIdentityChange = useCallback(
    (id: string) => {
      setSelectedIdentityId(id);
      setManualKeyId(null); // reset key selection
      onIdentityChange?.(id);
    },
    [onIdentityChange],
  );

  // Notify parent of initial identity selection
  useEffect(() => {
    if (selectedIdentityId) {
      onIdentityChange?.(selectedIdentityId);
    }
  }, []); // eslint-disable-line react-hooks/exhaustive-deps -- only on mount

  // ─── Breadcrumbs ────────────────────────────────────────────────

  const breadcrumbLabel = source === "dpns" ? "DPNS" : "Identities";

  // ─── Render ─────────────────────────────────────────────────────

  // Success screen
  if (status.type === "success") {
    return (
      <div className="flex flex-col gap-6" data-testid="register-dpns-success">
        <div className="flex items-center gap-2">
          <CheckCircle2 className="h-6 w-6 text-green-500" />
          <h2 className="text-xl font-semibold">
            {status.contested
              ? "DPNS Name Submitted (Contested)"
              : "DPNS Name Registered!"}
          </h2>
        </div>

        {status.contested && (
          <div className="rounded-lg border border-amber-200 bg-amber-50 dark:bg-amber-950/20 dark:border-amber-800 p-4">
            <div className="flex items-start gap-2">
              <AlertTriangle className="h-5 w-5 text-amber-500 mt-0.5 shrink-0" />
              <div className="text-sm text-amber-800 dark:text-amber-200">
                <p className="font-medium">Contested Name</p>
                <p className="mt-1">
                  This name is contested and will go through a two-week voting period.
                  The name will be awarded to the contestant with the most votes at the end of the period.
                </p>
              </div>
            </div>
          </div>
        )}

        {(status.feeEstimated !== null || status.feeActual !== null) && (
          <div className="rounded-lg border p-4 space-y-2">
            <h3 className="text-sm font-medium">Fee Breakdown</h3>
            {status.feeEstimated !== null && (
              <div className="flex justify-between text-sm">
                <span className="text-muted-foreground">Estimated fee:</span>
                <span>{formatCreditsAsDash(status.feeEstimated)} DASH</span>
              </div>
            )}
            {status.feeActual !== null && (
              <div className="flex justify-between text-sm">
                <span className="text-muted-foreground">Actual fee:</span>
                <span className="font-medium">{formatCreditsAsDash(status.feeActual)} DASH</span>
              </div>
            )}
          </div>
        )}

        <div className="flex gap-3">
          <Button variant="outline" onClick={onBack} data-testid="back-btn">
            <ArrowLeft className="h-4 w-4 mr-1" />
            Back
          </Button>
          <Button onClick={handleRegisterAnother} data-testid="register-another-btn">
            Register another name
          </Button>
        </div>
      </div>
    );
  }

  // Error screen
  if (status.type === "error") {
    return (
      <div data-testid="register-dpns-error">
        <InlineError
          message={status.message}
          heading="Registration Failed"
          onDismiss={onDismissError}
          fullScreen
        />
      </div>
    );
  }

  // Registering screen
  if (status.type === "registering") {
    return (
      <div className="flex flex-col items-center justify-center gap-4 py-12" data-testid="register-dpns-registering">
        <Loader2 className="h-8 w-8 animate-spin text-primary" />
        <h2 className="text-lg font-semibold">Registering...</h2>
        <p className="text-sm text-muted-foreground" data-testid="elapsed-time">
          Time taken so far: {elapsed}
        </p>
      </div>
    );
  }

  // ─── Form ─────────────────────────────────────────────────────

  return (
    <div className="flex flex-col gap-6">
      {/* Header / Breadcrumbs */}
      <div className="flex items-center gap-3">
        <Button
          variant="ghost"
          size="icon"
          onClick={onBack}
          data-testid="back-btn"
          aria-label="Go back"
        >
          <ArrowLeft className="h-4 w-4" />
        </Button>
        <div className="flex items-center gap-1.5 text-sm text-muted-foreground">
          <span>{breadcrumbLabel}</span>
          <span>/</span>
          <span className="text-foreground font-medium">Register Name</span>
        </div>
      </div>

      <h2 className="text-xl font-semibold">Register DPNS Name</h2>

      {/* No identities warning */}
      {identities.length === 0 && (
        <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-4">
          <p className="text-sm text-destructive">
            No identities loaded. Please create or load an identity first.
          </p>
        </div>
      )}

      {/* Identity selector */}
      {identities.length > 0 && (
        <div className="space-y-1.5">
          <Label htmlFor="identity-select">Identity</Label>
          {identities.length === 1 ? (
            <div className="text-sm" data-testid="identity-display">
              <Badge variant="outline" className="font-mono">
                {selectedIdentity?.alias ?? selectedIdentity?.id.slice(0, 12)}
              </Badge>
            </div>
          ) : (
            <Select
              value={selectedIdentityId}
              onValueChange={handleIdentityChange}
              data-testid="identity-select"
            >
              <SelectTrigger className="w-full" id="identity-select" aria-label="Identity">
                <SelectValue placeholder="Select identity" />
              </SelectTrigger>
              <SelectContent>
                {identities.map((identity) => (
                  <SelectItem key={identity.id} value={identity.id}>
                    {identity.alias?.trim() || identity.id.slice(0, 16)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          )}

          {/* Balance display */}
          {selectedIdentity && (
            <p className="text-xs text-muted-foreground" data-testid="identity-balance">
              Balance: {formatCreditsAsDash(selectedIdentity.balance)} DASH
              ({selectedIdentity.balance.toLocaleString()} credits)
            </p>
          )}
        </div>
      )}

      {/* Wallet locked warning */}
      {walletLocked && selectedIdentity && (
        <div
          className="rounded-lg border border-amber-300 bg-amber-50 dark:bg-amber-950/20 dark:border-amber-700 p-4"
          data-testid="wallet-locked-warning"
        >
          <div className="flex items-start gap-3">
            <Lock className="h-5 w-5 text-amber-600 dark:text-amber-400 mt-0.5 shrink-0" />
            <div className="flex-1 space-y-2">
              <p className="text-sm font-medium text-amber-800 dark:text-amber-200">
                Wallet is locked
              </p>
              <p className="text-sm text-amber-700 dark:text-amber-300">
                The wallet associated with this identity is locked. Please unlock it to continue.
              </p>
              {onRequestUnlock && (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={onRequestUnlock}
                  data-testid="unlock-wallet-btn"
                >
                  <Lock className="h-3.5 w-3.5 mr-1.5" />
                  Unlock Wallet
                </Button>
              )}
            </div>
          </div>
        </div>
      )}

      {/* Advanced options toggle */}
      <button
        className="flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground transition-colors"
        onClick={() => setShowAdvanced(!showAdvanced)}
        data-testid="advanced-toggle"
        type="button"
      >
        {showAdvanced ? (
          <ChevronUp className="h-4 w-4" />
        ) : (
          <ChevronDown className="h-4 w-4" />
        )}
        Advanced Options
      </button>

      {/* Key selector (advanced) */}
      {showAdvanced && selectedIdentity && (
        <div className="space-y-1.5 pl-4 border-l-2 border-muted">
          <Label htmlFor="key-select">Signing Key</Label>
          {signingKeys.length === 0 ? (
            <p className="text-sm text-destructive" data-testid="no-keys-warning">
              No suitable signing keys found. An AUTHENTICATION key at CRITICAL, HIGH,
              or MEDIUM security level is required.
            </p>
          ) : (
            <Select
              value={selectedKeyId?.toString() ?? ""}
              onValueChange={(v) => setManualKeyId(Number(v))}
            >
              <SelectTrigger className="w-full" id="key-select" aria-label="Signing key">
                <SelectValue placeholder="Select key" />
              </SelectTrigger>
              <SelectContent>
                {signingKeys.map((key) => (
                  <SelectItem key={key.keyId} value={key.keyId.toString()}>
                    {getKeyLabel(key)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          )}
        </div>
      )}

      <Separator />

      {/* Name input */}
      <div className="space-y-1.5">
        <Label htmlFor="name-input">Name (without &quot;.dash&quot;)</Label>
        <div className="flex items-center gap-2">
          <Input
            id="name-input"
            data-testid="name-input"
            type="text"
            placeholder="e.g. alice"
            value={nameInput}
            onChange={(e) => setNameInput(e.target.value)}
            className="flex-1"
            aria-describedby="name-validation"
          />
          <span className="text-sm text-muted-foreground">.dash</span>
        </div>

        {/* Validation feedback */}
        <div id="name-validation" data-testid="name-validation">
          {nameInput.trim().length > 0 && (
            <>
              {validation.valid ? (
                <div className="flex items-center gap-1.5 text-sm text-green-600 dark:text-green-400">
                  <CheckCircle2 className="h-3.5 w-3.5" />
                  <span>Valid name format</span>
                </div>
              ) : validation.error ? (
                <div className="flex items-center gap-1.5 text-sm text-destructive">
                  <AlertCircle className="h-3.5 w-3.5" />
                  <span>{validation.error}</span>
                </div>
              ) : null}

              {/* Contested name detection */}
              {validation.valid && (
                <div className="mt-1">
                  {contested ? (
                    <div className="flex items-center gap-1.5 text-sm text-amber-600 dark:text-amber-400" data-testid="contested-warning">
                      <AlertTriangle className="h-3.5 w-3.5" />
                      <span>
                        This is a contested name. Cost {CONTESTED_NAME_COST_DASH}
                      </span>
                    </div>
                  ) : (
                    <div className="flex items-center gap-1.5 text-sm text-green-600 dark:text-green-400" data-testid="not-contested">
                      <CheckCircle2 className="h-3.5 w-3.5" />
                      <span>This is not a contested name.</span>
                    </div>
                  )}
                </div>
              )}
            </>
          )}
        </div>
      </div>

      {/* Fee estimate */}
      {validation.valid && nameInput.trim().length > 0 && (
        <div className="rounded-lg border p-3" data-testid="fee-estimate">
          <div className="flex justify-between text-sm">
            <span className="text-muted-foreground">Estimated fee:</span>
            <span className="font-medium">{estimatedFeeDisplay} DASH</span>
          </div>
        </div>
      )}

      {/* Insufficient balance warning */}
      {hasInsufficientBalance && selectedIdentity && (
        <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-3" data-testid="insufficient-balance">
          <p className="text-sm text-destructive">
            Insufficient balance. The identity needs at least {estimatedFeeDisplay} DASH
            to cover the registration fee.
          </p>
        </div>
      )}

      {/* No signing keys warning (non-advanced) */}
      {!showAdvanced && selectedIdentity && signingKeys.length === 0 && (
        <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-3" data-testid="no-keys-warning">
          <p className="text-sm text-destructive">
            No suitable signing keys found. An AUTHENTICATION key at CRITICAL, HIGH,
            or MEDIUM security level is required.
          </p>
        </div>
      )}

      {/* Register button */}
      <Button
        onClick={handleSubmit}
        disabled={!canSubmit}
        className="w-full sm:w-auto"
        data-testid="register-btn"
      >
        Register Name
      </Button>

      <Separator />

      {/* Info sections */}
      <div className="space-y-4">
        <details className="group">
          <summary className="flex items-center gap-1.5 cursor-pointer text-sm font-medium text-muted-foreground hover:text-foreground">
            <Info className="h-4 w-4" />
            DPNS Name Constraints
          </summary>
          <ul className="mt-2 pl-6 space-y-1 text-sm text-muted-foreground list-disc" data-testid="constraints-list">
            <li>Must be between 3 and 63 characters long</li>
            <li>Can only contain letters (a-z, A-Z), numbers (0-9), and hyphens (-)</li>
            <li>Cannot start with a hyphen</li>
            <li>Cannot end with a hyphen</li>
            <li>Names are case-insensitive (alice = Alice = ALICE)</li>
            <li>The &quot;.dash&quot; suffix is added automatically</li>
          </ul>
        </details>

        <details className="group">
          <summary className="flex items-center gap-1.5 cursor-pointer text-sm font-medium text-muted-foreground hover:text-foreground">
            <Info className="h-4 w-4" />
            Contested Names Info
          </summary>
          <ul className="mt-2 pl-6 space-y-1 text-sm text-muted-foreground list-disc" data-testid="contested-info-list">
            <li>Names shorter than 20 characters with only letters and the digits 0/1 are considered contested</li>
            <li>Contested names go through a two-week voting period</li>
            <li>Masternodes and evonodes vote on which contestant should receive the name</li>
            <li>The contestant with the most votes at the end of the period wins the name</li>
          </ul>
        </details>
      </div>
    </div>
  );
}
