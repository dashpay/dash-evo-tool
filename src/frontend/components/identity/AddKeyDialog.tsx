import { useCallback, useMemo, useState } from "react";
import {
  Plus,
  RefreshCw,
  ArrowLeft,
  Loader2,
  Check,
  X,
  Lock,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { formatAmount } from "@/components/shared/AmountInput";
import type { QualifiedIdentityDto } from "@/bindings";

// ─── Constants ─────────────────────────────────────────────────────

const KEY_TYPES = [
  { value: "ECDSA_SECP256K1", label: "ECDSA secp256k1" },
  { value: "BLS12_381", label: "BLS 12-381" },
  { value: "ECDSA_HASH160", label: "ECDSA Hash160" },
  { value: "EDDSA_25519_HASH160", label: "EdDSA 25519 Hash160" },
] as const;

const PURPOSES = [
  { value: "AUTHENTICATION", label: "Authentication" },
  { value: "TRANSFER", label: "Transfer" },
  { value: "ENCRYPTION", label: "Encryption" },
  { value: "DECRYPTION", label: "Decryption" },
] as const;


const CREDITS_DECIMAL_PLACES = 8;

/**
 * Which security levels are available for each purpose.
 * Matches egui add_key_screen.rs logic.
 */
function getAvailableSecurityLevels(purpose: string): string[] {
  switch (purpose) {
    case "AUTHENTICATION":
      return ["CRITICAL", "HIGH", "MEDIUM"];
    case "TRANSFER":
      return ["CRITICAL"];
    case "ENCRYPTION":
    case "DECRYPTION":
      return ["MEDIUM"];
    default:
      return ["MEDIUM"];
  }
}

function getDefaultSecurityLevel(purpose: string): string {
  const levels = getAvailableSecurityLevels(purpose);
  return levels[0] ?? "";
}

// ─── Status ────────────────────────────────────────────────────────

export type AddKeyStatus =
  | { type: "idle" }
  | { type: "submitting"; startedAt: number }
  | { type: "success" }
  | { type: "error"; message: string };

// ─── Props ─────────────────────────────────────────────────────────

export interface AddKeyDialogProps {
  /** The identity to add a key to. */
  identity: QualifiedIdentityDto;
  /** Current status of the add key operation. */
  status: AddKeyStatus;
  /** Estimated fee in credits (null if not yet calculated). */
  estimatedFee?: number | null;
  /** Called when user submits the form. */
  onSubmit?: (params: {
    purpose: string;
    securityLevel: string;
    keyType: string;
    privateKeyHex: string;
    contractBounds?: {
      contractId: string;
      documentTypeName?: string;
    };
  }) => void;
  /** Called when user dismisses an error to go back to form. */
  onDismissError?: () => void;
  /** Called when user clicks "Back to Keys" after success. */
  onBackToKeys?: () => void;
  /** Called when user clicks "Add Another Key" after success. */
  onAddAnother?: () => void;
  /** Called to navigate back. */
  onBack?: () => void;
  /** Pre-configured purpose (e.g., for DashPay encryption). */
  defaultPurpose?: string;
  /** Pre-configured contract bounds (e.g., for DashPay). */
  defaultContractBounds?: {
    contractId: string;
    documentTypeName?: string;
  };
  /** Additional CSS class. */
  className?: string;
  /** Whether the associated wallet is locked and needs unlock before proceeding. */
  walletLocked?: boolean;
  /** Called when user wants to unlock the wallet. */
  onRequestUnlock?: () => void;
}

// ─── Component ─────────────────────────────────────────────────────

/**
 * Add key form for adding new keys to an identity.
 *
 * Matches egui add_key_screen.rs: form with purpose, security level, key type,
 * private key input (with generate random), optional contract bounds, fee
 * estimate, and states: idle → submitting → success/error.
 */
export function AddKeyDialog({
  identity,
  status,
  estimatedFee,
  onSubmit,
  onDismissError,
  onBackToKeys,
  onAddAnother,
  onBack,
  defaultPurpose,
  defaultContractBounds,
  className,
  walletLocked = false,
  onRequestUnlock,
}: AddKeyDialogProps) {
  const [purpose, setPurpose] = useState(defaultPurpose || "AUTHENTICATION");
  const [securityLevel, setSecurityLevel] = useState(
    getDefaultSecurityLevel(defaultPurpose || "AUTHENTICATION"),
  );
  const [keyType, setKeyType] = useState("ECDSA_SECP256K1");
  const [privateKeyHex, setPrivateKeyHex] = useState("");
  const [enableContractBounds, setEnableContractBounds] = useState(
    !!defaultContractBounds,
  );
  const [contractIdInput, setContractIdInput] = useState(
    defaultContractBounds?.contractId || "",
  );
  const [documentTypeInput, setDocumentTypeInput] = useState(
    defaultContractBounds?.documentTypeName || "",
  );
  const [validationError, setValidationError] = useState<string | null>(null);

  const availablePurposes = useMemo(() => {
    if (enableContractBounds) {
      return PURPOSES.filter(
        (p) => p.value === "ENCRYPTION" || p.value === "DECRYPTION",
      );
    }
    return [...PURPOSES];
  }, [enableContractBounds]);

  const availableSecurityLevels = useMemo(
    () => getAvailableSecurityLevels(purpose),
    [purpose],
  );

  const isSecurityLevelFixed = availableSecurityLevels.length === 1;

  const displayName =
    identity.alias || `${identity.id.slice(0, 8)}…${identity.id.slice(-6)}`;

  // When purpose changes, auto-set security level
  const handlePurposeChange = useCallback(
    (newPurpose: string) => {
      setPurpose(newPurpose);
      const levels = getAvailableSecurityLevels(newPurpose);
      if (!levels.includes(securityLevel)) {
        setSecurityLevel(levels[0] ?? "");
      }
    },
    [securityLevel],
  );

  // When contract bounds toggled
  const handleContractBoundsToggle = useCallback(
    (checked: boolean) => {
      setEnableContractBounds(checked);
      if (checked) {
        // Auto-set to ENCRYPTION / MEDIUM
        setPurpose("ENCRYPTION");
        setSecurityLevel("MEDIUM");
      }
    },
    [],
  );

  const handleGenerateRandom = useCallback(() => {
    // Generate 32 random bytes as hex
    const bytes = new Uint8Array(32);
    crypto.getRandomValues(bytes);
    const hex = Array.from(bytes)
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("");
    setPrivateKeyHex(hex);
    setValidationError(null);
  }, []);

  const handleSubmit = useCallback(() => {
    // Validate
    const trimmedKey = privateKeyHex.trim();
    if (!trimmedKey) {
      setValidationError("Private key is required. Click 'Generate Random' or enter one manually.");
      return;
    }
    if (!/^[0-9a-fA-F]+$/.test(trimmedKey)) {
      setValidationError("Private key must be a hex string.");
      return;
    }
    if (trimmedKey.length !== 64 && trimmedKey.length !== 96) {
      setValidationError("Private key must be 32 bytes (64 hex) for ECDSA or 48 bytes (96 hex) for BLS.");
      return;
    }
    if (enableContractBounds && !contractIdInput.trim()) {
      setValidationError("Contract ID is required when contract bounds are enabled.");
      return;
    }

    setValidationError(null);

    const contractBounds =
      enableContractBounds && contractIdInput.trim()
        ? {
            contractId: contractIdInput.trim(),
            documentTypeName: documentTypeInput.trim() || undefined,
          }
        : undefined;

    onSubmit?.({
      purpose,
      securityLevel,
      keyType,
      privateKeyHex: trimmedKey,
      contractBounds,
    });
  }, [
    purpose,
    securityLevel,
    keyType,
    privateKeyHex,
    enableContractBounds,
    contractIdInput,
    documentTypeInput,
    onSubmit,
  ]);

  // ─── Success State ─────────────────────────────────────────

  if (status.type === "success") {
    return (
      <div
        className={cn(
          "flex flex-col items-center justify-center gap-4 py-12",
          className,
        )}
        role="status"
      >
        <div className="flex size-16 items-center justify-center rounded-full bg-green-100 dark:bg-green-900/30">
          <Check className="size-8 text-green-600 dark:text-green-400" />
        </div>
        <div className="text-center">
          <h3 className="text-lg font-semibold">Key Added Successfully</h3>
          <p className="text-sm text-muted-foreground">
            A new {purpose.toLowerCase()} key has been added to {displayName}.
          </p>
        </div>
        <div className="flex gap-3">
          {onBackToKeys && (
            <Button variant="outline" onClick={onBackToKeys}>
              Back to Keys
            </Button>
          )}
          {onAddAnother && (
            <Button onClick={onAddAnother}>Add Another Key</Button>
          )}
        </div>
      </div>
    );
  }

  // ─── Submitting State ──────────────────────────────────────

  if (status.type === "submitting") {
    return (
      <div
        className={cn(
          "flex flex-col items-center justify-center gap-4 py-12",
          className,
        )}
        role="status"
        aria-label="Adding key"
      >
        <Loader2 className="size-8 animate-spin text-primary" />
        <div className="text-center">
          <h3 className="text-lg font-semibold">Adding Key…</h3>
          <p className="text-sm text-muted-foreground">
            This may take a moment.
          </p>
        </div>
      </div>
    );
  }

  // ─── Form State (idle or error) ────────────────────────────

  return (
    <div className={cn("flex flex-col gap-5", className)} role="form" aria-label="Add key form">
      {/* Header */}
      <div className="flex items-center gap-2">
        {onBack && (
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={onBack}
            aria-label="Go back"
          >
            <ArrowLeft className="size-4" />
          </Button>
        )}
        <div>
          <h2 className="text-lg font-semibold">Add New Key</h2>
          <p className="text-sm text-muted-foreground">{displayName}</p>
        </div>
      </div>

      {/* Error message */}
      {status.type === "error" && (
        <div
          className="flex items-center justify-between rounded-md border border-destructive/50 bg-destructive/10 px-4 py-3"
          role="alert"
        >
          <p className="text-sm text-destructive">{status.message}</p>
          {onDismissError && (
            <Button
              variant="ghost"
              size="icon-xs"
              onClick={onDismissError}
              aria-label="Dismiss error"
            >
              <X className="size-3.5" />
            </Button>
          )}
        </div>
      )}

      {validationError && (
        <div
          className="rounded-md border border-destructive/50 bg-destructive/10 px-4 py-3"
          role="alert"
        >
          <p className="text-sm text-destructive">{validationError}</p>
        </div>
      )}

      {/* Wallet locked gate */}
      {walletLocked && (
        <div className="flex items-center gap-3 rounded-md border border-amber-500/30 bg-amber-50 dark:bg-amber-900/20 px-4 py-3" data-testid="wallet-locked-gate">
          <Lock className="size-4 text-amber-600 dark:text-amber-400 shrink-0" />
          <p className="text-sm text-amber-700 dark:text-amber-300 flex-1">
            Wallet is locked. Please unlock to continue.
          </p>
          {onRequestUnlock && (
            <Button variant="outline" size="sm" onClick={onRequestUnlock}>
              Unlock Wallet
            </Button>
          )}
        </div>
      )}

      <Separator />

      {/* Form Fields */}
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
        {/* Purpose */}
        <div className="space-y-1.5">
          <Label htmlFor="add-key-purpose">Purpose</Label>
          <Select value={purpose} onValueChange={handlePurposeChange}>
            <SelectTrigger id="add-key-purpose" aria-label="Key purpose">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {availablePurposes.map((p) => (
                <SelectItem key={p.value} value={p.value}>
                  {p.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        {/* Security Level */}
        <div className="space-y-1.5">
          <Label htmlFor="add-key-security">Security Level</Label>
          {isSecurityLevelFixed ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <Select value={securityLevel} disabled>
                  <SelectTrigger
                    id="add-key-security"
                    aria-label="Security level"
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {availableSecurityLevels.map((l) => (
                      <SelectItem key={l} value={l}>
                        {l.charAt(0) + l.slice(1).toLowerCase()}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </TooltipTrigger>
              <TooltipContent>
                Security level is fixed for {purpose.toLowerCase()} keys.
              </TooltipContent>
            </Tooltip>
          ) : (
            <Select value={securityLevel} onValueChange={setSecurityLevel}>
              <SelectTrigger
                id="add-key-security"
                aria-label="Security level"
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {availableSecurityLevels.map((l) => (
                  <SelectItem key={l} value={l}>
                    {l.charAt(0) + l.slice(1).toLowerCase()}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          )}
        </div>

        {/* Key Type */}
        <div className="space-y-1.5">
          <Label htmlFor="add-key-type">Key Type</Label>
          <Select value={keyType} onValueChange={setKeyType}>
            <SelectTrigger id="add-key-type" aria-label="Key type">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {KEY_TYPES.map((kt) => (
                <SelectItem key={kt.value} value={kt.value}>
                  {kt.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        {/* Private Key */}
        <div className="space-y-1.5">
          <Label htmlFor="add-key-private">Private Key</Label>
          <div className="flex gap-2">
            <Input
              id="add-key-private"
              placeholder="Hex (64 or 96 chars)"
              value={privateKeyHex}
              onChange={(e) => {
                setPrivateKeyHex(e.target.value);
                setValidationError(null);
              }}
              className="font-mono text-xs"
              aria-label="Private key hex"
            />
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={handleGenerateRandom}
                  aria-label="Generate random key"
                >
                  <RefreshCw className="size-3.5" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>Generate Random</TooltipContent>
            </Tooltip>
          </div>
        </div>
      </div>

      {/* Contract Bounds */}
      <Separator />
      <div className="space-y-3">
        <div className="flex items-center gap-2">
          <Checkbox
            id="add-key-contract-bounds"
            checked={enableContractBounds}
            onCheckedChange={(checked) =>
              handleContractBoundsToggle(checked === true)
            }
          />
          <Label htmlFor="add-key-contract-bounds" className="cursor-pointer">
            Enable Contract Bounds
          </Label>
        </div>
        {enableContractBounds && (
          <div className="grid grid-cols-1 gap-3 pl-6 sm:grid-cols-2">
            <div className="space-y-1.5">
              <Label htmlFor="add-key-contract-id">
                Contract ID <span className="text-destructive">*</span>
              </Label>
              <Input
                id="add-key-contract-id"
                placeholder="Base58 contract identifier"
                value={contractIdInput}
                onChange={(e) => setContractIdInput(e.target.value)}
                className="font-mono text-xs"
                aria-label="Contract ID"
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="add-key-doc-type">
                Document Type <span className="text-muted-foreground">(optional)</span>
              </Label>
              <Input
                id="add-key-doc-type"
                placeholder="e.g. profile"
                value={documentTypeInput}
                onChange={(e) => setDocumentTypeInput(e.target.value)}
                className="text-sm"
                aria-label="Document type name"
              />
            </div>
          </div>
        )}
      </div>

      {/* Fee Estimate */}
      {estimatedFee != null && (
        <>
          <Separator />
          <div className="flex items-center gap-2 text-sm">
            <span className="text-muted-foreground">Estimated fee:</span>
            <span className="font-medium">
              {formatAmount(estimatedFee, CREDITS_DECIMAL_PLACES)} DASH
            </span>
          </div>
        </>
      )}

      {/* Submit */}
      <div className="flex justify-end pt-2">
        <Button onClick={handleSubmit} disabled={walletLocked}>
          <Plus className="mr-1.5 size-4" />
          Add Key
        </Button>
      </div>
    </div>
  );
}
