import { useCallback, useEffect, useState } from "react";
import {
  ArrowLeft,
  Shield,
  ShieldAlert,
  ShieldCheck,
  Lock,
  Unlock,
  Ban,
  RefreshCw,
  Pen,
  Info,
  Loader2,
  X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
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
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { CopyButton } from "@/components/shared/CopyButton";
import {
  ConfirmationDialog,
} from "@/components/shared/ConfirmationDialog";
import type { IdentityKeyDto, QualifiedIdentityDto } from "@/bindings";

// ─── Constants ─────────────────────────────────────────────────────

const KEY_TYPE_LABELS: Record<string, string> = {
  ECDSA_SECP256K1: "ECDSA secp256k1",
  BLS12_381: "BLS 12-381",
  ECDSA_HASH160: "ECDSA Hash160",
  EDDSA_25519_HASH160: "EdDSA 25519 Hash160",
};

const REPLACE_KEY_TYPES = [
  "ECDSA_SECP256K1",
  "BLS12_381",
  "ECDSA_HASH160",
  "EDDSA_25519_HASH160",
] as const;

/** Generate a random 32-byte hex private key using Web Crypto. */
function generateRandomPrivateKeyHex(): string {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

// ─── Props ─────────────────────────────────────────────────────────

export interface KeyInfoScreenProps {
  /** The identity that owns this key. */
  identity: QualifiedIdentityDto;
  /** The key to display information for. */
  keyData: IdentityKeyDto;
  /** Called to navigate back. */
  onBack?: () => void;
  /** Label for the back button (for accessibility). Defaults to "Back". */
  backLabel?: string;
  /** Called to disable this key on platform. */
  onDisableKey?: (keyId: number) => void;
  /** Called to replace the master key. */
  onReplaceKey?: (keyId: number, newKeyType: string, newPrivateKeyHex: string) => void;
  /** Called to add a private key to local storage. */
  onAddPrivateKey?: (keyId: number, privateKeyHex: string) => void;
  /** Called to remove the private key from local storage. */
  onRemovePrivateKey?: (keyId: number) => void;
  /** Called to sign a message with this key's private key. */
  onSignMessage?: (keyId: number, message: string) => Promise<string>;
  /** Whether a disable/replace operation is in progress. */
  isSubmitting?: boolean;
  /** Error message from the last operation. */
  error?: string | null;
  /** Success message from the last operation. */
  success?: string | null;
  /** Called to clear the error message. */
  onClearError?: () => void;
  /** Called to clear the success message. */
  onClearSuccess?: () => void;
  /** Additional CSS class. */
  className?: string;
  /** Whether the associated wallet is locked and needs unlock before proceeding. */
  walletLocked?: boolean;
  /** Called when user wants to unlock the wallet. */
  onRequestUnlock?: () => void;
}

// ─── Helpers ───────────────────────────────────────────────────────

function getSecurityLevelColor(level: string): string {
  switch (level) {
    case "MASTER":
      return "text-red-600 dark:text-red-400";
    case "CRITICAL":
      return "text-orange-600 dark:text-orange-400";
    case "HIGH":
      return "text-yellow-600 dark:text-yellow-400";
    case "MEDIUM":
      return "text-green-600 dark:text-green-400";
    default:
      return "text-muted-foreground";
  }
}

function SecurityIcon({ level, className }: { level: string; className?: string }) {
  switch (level) {
    case "MASTER":
      return <ShieldAlert className={className} />;
    case "CRITICAL":
      return <ShieldCheck className={className} />;
    default:
      return <Shield className={className} />;
  }
}

// ─── Component ─────────────────────────────────────────────────────

/**
 * Key info screen showing full details of a single identity key.
 *
 * Matches egui key_info_screen.rs functionality:
 * - Key metadata grid (ID, Purpose, Security Level, Type, Status)
 * - Public key display (hex, base64)
 * - Private key section (view, add, remove)
 * - Message signing
 * - Disable key action
 * - Replace master key action
 */
export function KeyInfoScreen({
  identity,
  keyData,
  onBack,
  backLabel = "Back",
  onDisableKey,
  onReplaceKey,
  onAddPrivateKey,
  onRemovePrivateKey,
  onSignMessage,
  isSubmitting = false,
  error,
  success,
  onClearError,
  onClearSuccess,
  className,
  walletLocked = false,
  onRequestUnlock,
}: KeyInfoScreenProps) {
  const [privateKeyInput, setPrivateKeyInput] = useState("");
  const [addKeyError, setAddKeyError] = useState<string | null>(null);
  const [messageInput, setMessageInput] = useState("");
  const [signedMessage, setSignedMessage] = useState<string | null>(null);
  const [signError, setSignError] = useState<string | null>(null);
  const [isSigning, setIsSigning] = useState(false);
  const [showConfirmDisable, setShowConfirmDisable] = useState(false);
  const [showConfirmReplace, setShowConfirmReplace] = useState(false);
  const [showConfirmRemovePrivKey, setShowConfirmRemovePrivKey] = useState(false);
  const [replaceKeyType, setReplaceKeyType] = useState("ECDSA_SECP256K1");
  const [replaceKeyPrivateHex, setReplaceKeyPrivateHex] = useState("");

  // Generate a random private key when the replace dialog opens or key type changes
  useEffect(() => {
    if (showConfirmReplace) {
      setReplaceKeyPrivateHex(generateRandomPrivateKeyHex());
    }
  }, [showConfirmReplace, replaceKeyType]);

  const [showInfoPopup, setShowInfoPopup] = useState(false);
  const isMasterKey = keyData.securityLevel === "MASTER";
  const canDisable = !isMasterKey && !keyData.isDisabled;
  const canReplace = isMasterKey;

  const displayName =
    identity.alias || `${identity.id.slice(0, 8)}…${identity.id.slice(-6)}`;

  const handleAddPrivateKey = useCallback(() => {
    const trimmed = privateKeyInput.trim();
    if (!trimmed) {
      setAddKeyError("Please enter a private key.");
      return;
    }
    // Pass directly to backend — it handles hex and WIF parsing
    setAddKeyError(null);
    onAddPrivateKey?.(keyData.keyId, trimmed);
    setPrivateKeyInput("");
  }, [privateKeyInput, keyData.keyId, onAddPrivateKey]);

  const handleSignMessage = useCallback(async () => {
    if (!messageInput.trim()) {
      setSignError("Please enter a message to sign.");
      return;
    }
    if (!onSignMessage) return;
    setIsSigning(true);
    setSignError(null);
    setSignedMessage(null);
    try {
      const sig = await onSignMessage(keyData.keyId, messageInput);
      setSignedMessage(sig);
    } catch (err) {
      setSignError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsSigning(false);
    }
  }, [messageInput, keyData.keyId, onSignMessage]);

  const handleConfirmDisable = useCallback(() => {
    setShowConfirmDisable(false);
    onDisableKey?.(keyData.keyId);
  }, [keyData.keyId, onDisableKey]);

  const handleConfirmReplace = useCallback(() => {
    const type = replaceKeyType;
    const hex = replaceKeyPrivateHex;
    setShowConfirmReplace(false);
    setReplaceKeyType("ECDSA_SECP256K1");
    setReplaceKeyPrivateHex("");
    onReplaceKey?.(keyData.keyId, type, hex);
  }, [keyData.keyId, replaceKeyType, replaceKeyPrivateHex, onReplaceKey]);

  const handleCancelReplace = useCallback(() => {
    setShowConfirmReplace(false);
    setReplaceKeyType("ECDSA_SECP256K1");
    setReplaceKeyPrivateHex("");
  }, []);

  const handleRegenerateKey = useCallback(() => {
    setReplaceKeyPrivateHex(generateRandomPrivateKeyHex());
  }, []);

  const handleConfirmRemovePrivKey = useCallback(() => {
    setShowConfirmRemovePrivKey(false);
    onRemovePrivateKey?.(keyData.keyId);
  }, [keyData.keyId, onRemovePrivateKey]);

  return (
    <div className={cn("flex flex-col gap-5", className)} role="region" aria-label="Key information">
      {/* Header */}
      <div className="flex items-center gap-2">
        {onBack && (
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={onBack}
            aria-label={backLabel}
          >
            <ArrowLeft className="size-4" />
          </Button>
        )}
        <div>
          <h2 className="text-lg font-semibold">Key Info</h2>
          <p className="text-sm text-muted-foreground">
            {displayName} — Key #{keyData.keyId}
          </p>
        </div>
      </div>

      {/* Status Messages */}
      {error && (
        <div
          className="flex items-center justify-between rounded-md border border-destructive/50 bg-destructive/10 px-4 py-3"
          role="alert"
        >
          <p className="text-sm text-destructive">{error}</p>
          {onClearError && (
            <Button
              variant="ghost"
              size="icon-xs"
              onClick={onClearError}
              aria-label="Dismiss error"
            >
              <X className="size-3.5" />
            </Button>
          )}
        </div>
      )}

      {success && (
        <div
          className="flex items-center justify-between rounded-md border border-green-500/50 bg-green-500/10 px-4 py-3"
          role="status"
        >
          <p className="text-sm text-green-700 dark:text-green-400">{success}</p>
          {onClearSuccess && (
            <Button
              variant="ghost"
              size="icon-xs"
              onClick={onClearSuccess}
              aria-label="Dismiss success"
            >
              <X className="size-3.5" />
            </Button>
          )}
        </div>
      )}

      <Separator />

      {/* Key Metadata */}
      <section aria-label="Key metadata">
        <h3 className="mb-3 text-sm font-medium text-muted-foreground">
          Key Metadata
        </h3>
        <div className="grid grid-cols-2 gap-x-6 gap-y-3 text-sm">
          <MetadataRow label="Key ID" value={String(keyData.keyId)} />
          <MetadataRow label="Purpose" value={keyData.purpose.charAt(0) + keyData.purpose.slice(1).toLowerCase()} />
          <MetadataRow label="Security Level">
            <div className="flex items-center gap-1.5">
              <SecurityIcon
                level={keyData.securityLevel}
                className={cn("size-3.5", getSecurityLevelColor(keyData.securityLevel))}
              />
              <span className={cn("font-medium", getSecurityLevelColor(keyData.securityLevel))}>
                {keyData.securityLevel.charAt(0) + keyData.securityLevel.slice(1).toLowerCase()}
              </span>
            </div>
          </MetadataRow>
          <MetadataRow
            label="Key Type"
            value={KEY_TYPE_LABELS[keyData.keyType] || keyData.keyType}
          />
          <MetadataRow label="Status">
            {keyData.isDisabled ? (
              <Badge variant="destructive" className="text-xs">Disabled</Badge>
            ) : (
              <Badge
                variant="secondary"
                className="bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-400 text-xs"
              >
                Active
              </Badge>
            )}
          </MetadataRow>
          {keyData.disabledAt && (
            <MetadataRow label="Disabled At" value={new Date(keyData.disabledAt * 1000).toLocaleString()} />
          )}
          {keyData.contractBounds && (
            <>
              <MetadataRow label="Contract Bound">
                <div className="flex items-center gap-2">
                  <code className="rounded bg-muted px-1.5 py-0.5 text-xs font-mono break-all">
                    {keyData.contractBounds.contractId}
                  </code>
                  <CopyButton value={keyData.contractBounds.contractId} />
                </div>
              </MetadataRow>
              {keyData.contractBounds.type === "singleContractDocumentType" && (
                <MetadataRow
                  label="Document Type"
                  value={keyData.contractBounds.documentTypeName}
                />
              )}
            </>
          )}
        </div>
      </section>

      <Separator />

      {/* Public Key */}
      <section aria-label="Public key">
        <h3 className="mb-3 text-sm font-medium text-muted-foreground">
          Public Key
        </h3>
        <div className="space-y-2">
          <div className="flex items-center gap-2">
            <Label className="w-20 text-xs text-muted-foreground">Hex</Label>
            <code className="flex-1 rounded bg-muted px-2 py-1 text-xs font-mono break-all">
              {keyData.data}
            </code>
            <CopyButton value={keyData.data} />
          </div>
          <div className="flex items-center gap-2">
            <Label className="w-20 text-xs text-muted-foreground">Base64</Label>
            <code className="flex-1 rounded bg-muted px-2 py-1 text-xs font-mono break-all">
              {hexToBase64(keyData.data)}
            </code>
            <CopyButton value={hexToBase64(keyData.data)} />
          </div>
        </div>
      </section>

      <Separator />

      {/* Private Key Section */}
      <section aria-label="Private key">
        <h3 className="mb-3 text-sm font-medium text-muted-foreground">
          Private Key
        </h3>
        {keyData.hasPrivateKey ? (
          <div className="space-y-3">
            <div className="flex items-center gap-2 rounded-md bg-green-50 dark:bg-green-900/20 px-3 py-2">
              <Unlock className="size-4 text-green-600 dark:text-green-400" />
              <span className="text-sm text-green-700 dark:text-green-400">
                Private key is available in local storage
              </span>
            </div>
            {onRemovePrivateKey && (
              <Button
                variant="outline"
                size="sm"
                onClick={() => setShowConfirmRemovePrivKey(true)}
                className="text-destructive hover:text-destructive"
              >
                <Ban className="mr-1.5 size-3.5" />
                Remove Private Key from DET
              </Button>
            )}
          </div>
        ) : (
          <div className="space-y-3">
            <div className="flex items-center gap-2 rounded-md bg-muted px-3 py-2">
              <Lock className="size-4 text-muted-foreground" />
              <span className="text-sm text-muted-foreground">
                No private key in local storage
              </span>
            </div>
            {onAddPrivateKey && (
              <div className="space-y-2">
                <div className="flex gap-2">
                  <Input
                    placeholder="Enter private key (hex or WIF)"
                    value={privateKeyInput}
                    onChange={(e) => {
                      setPrivateKeyInput(e.target.value);
                      setAddKeyError(null);
                    }}
                    className="font-mono text-xs"
                    aria-label="Private key hex input"
                  />
                  <Button
                    size="sm"
                    onClick={handleAddPrivateKey}
                    disabled={!privateKeyInput.trim()}
                  >
                    Add
                  </Button>
                </div>
                {addKeyError && (
                  <p className="text-xs text-destructive" role="alert">
                    {addKeyError}
                  </p>
                )}
              </div>
            )}
          </div>
        )}
      </section>

      {/* Sign Message Section — only when we have private key */}
      {keyData.hasPrivateKey && onSignMessage && (
        <>
          <Separator />
          <section aria-label="Sign message">
            <div className="mb-3 flex items-center gap-2">
              <h3 className="text-sm font-medium text-muted-foreground">
                Sign Message
              </h3>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon-xs"
                    onClick={() => setShowInfoPopup(!showInfoPopup)}
                    aria-label="Signing info"
                  >
                    <Info className="size-3.5 text-muted-foreground" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent className="max-w-xs">
                  Signs the message using the private key associated with this
                  identity key. The output is a Base64-encoded signature that can
                  be verified by anyone with the public key.
                </TooltipContent>
              </Tooltip>
            </div>
            <div className="space-y-2">
              <textarea
                className="w-full rounded-md border bg-background px-3 py-2 text-sm resize-y min-h-[60px] focus:outline-none focus:ring-2 focus:ring-ring"
                placeholder="Enter message to sign"
                value={messageInput}
                onChange={(e) => setMessageInput(e.target.value)}
                rows={3}
                aria-label="Message to sign"
              />
              <Button
                size="sm"
                onClick={handleSignMessage}
                disabled={!messageInput.trim() || isSigning}
              >
                {isSigning ? (
                  <>
                    <Loader2 className="mr-1.5 size-3.5 animate-spin" />
                    Signing…
                  </>
                ) : (
                  <>
                    <Pen className="mr-1.5 size-3.5" />
                    Sign Message
                  </>
                )}
              </Button>
              {signError && (
                <p className="text-xs text-destructive" role="alert">
                  {signError}
                </p>
              )}
              {signedMessage && (
                <div className="space-y-1">
                  <Label className="text-xs text-muted-foreground">
                    Signature (Base64)
                  </Label>
                  <div className="flex items-start gap-2">
                    <code className="flex-1 rounded bg-muted px-2 py-1 text-xs font-mono break-all">
                      {signedMessage}
                    </code>
                    <CopyButton value={signedMessage} />
                  </div>
                </div>
              )}
            </div>
          </section>
        </>
      )}

      {/* Key Actions */}
      {(canDisable || canReplace) && (
        <>
          <Separator />
          <section aria-label="Key actions">
            <h3 className="mb-3 text-sm font-medium text-muted-foreground">
              Key Actions
            </h3>
            {walletLocked && (
              <div className="flex items-center gap-3 rounded-md border border-amber-500/30 bg-amber-50 dark:bg-amber-900/20 px-4 py-3 mb-3" data-testid="wallet-locked-gate">
                <Lock className="size-4 text-amber-600 dark:text-amber-400 shrink-0" />
                <p className="text-sm text-amber-700 dark:text-amber-300 flex-1">
                  Wallet is locked. Unlock to perform key actions.
                </p>
                {onRequestUnlock && (
                  <Button variant="outline" size="sm" onClick={onRequestUnlock}>
                    Unlock Wallet
                  </Button>
                )}
              </div>
            )}
            <div className="space-y-3">
              {canDisable && onDisableKey && (
                <div>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => setShowConfirmDisable(true)}
                    disabled={isSubmitting || walletLocked}
                    className="text-destructive hover:text-destructive"
                  >
                    {isSubmitting ? (
                      <>
                        <Loader2 className="mr-1.5 size-3.5 animate-spin" />
                        Disabling…
                      </>
                    ) : (
                      <>
                        <Ban className="mr-1.5 size-3.5" />
                        Disable Key on Platform
                      </>
                    )}
                  </Button>
                  <p className="mt-1 text-xs text-muted-foreground">
                    Permanently disables this key on the Dash Platform. This action cannot be undone.
                  </p>
                </div>
              )}

              {canReplace && onReplaceKey && (
                <div className="space-y-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => setShowConfirmReplace(true)}
                    disabled={isSubmitting || walletLocked}
                    className="text-amber-600 hover:text-amber-600 dark:text-amber-400 dark:hover:text-amber-400"
                  >
                    {isSubmitting ? (
                      <>
                        <Loader2 className="mr-1.5 size-3.5 animate-spin" />
                        Replacing…
                      </>
                    ) : (
                      <>
                        <RefreshCw className="mr-1.5 size-3.5" />
                        Replace Master Key
                      </>
                    )}
                  </Button>
                  <p className="mt-1 text-xs text-muted-foreground">
                    Replaces the master key with a new one. The old master key will be disabled.
                  </p>
                </div>
              )}
            </div>
          </section>
        </>
      )}

      {/* Confirmation Dialogs */}
      <ConfirmationDialog
        open={showConfirmDisable}
        onOpenChange={setShowConfirmDisable}
        title="Disable Key"
        message={`Are you sure you want to disable Key #${keyData.keyId} (${keyData.purpose})? This action is permanent and cannot be undone.`}
        confirmText="Disable"
        danger
        onResult={(status) => {
          if (status === "confirmed") handleConfirmDisable();
        }}
      />

      <Dialog open={showConfirmReplace} onOpenChange={(open) => {
        if (!open) handleCancelReplace();
      }}>
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>Replace Master Key</DialogTitle>
            <DialogDescription>
              This will generate a new master key and disable the current one
              in a single atomic transition. Make sure to save the new private
              key! You will need it to sign future identity updates.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-4 py-2">
            {/* Old key info */}
            <div className="text-sm text-muted-foreground">
              <span>Old Key ID: {keyData.keyId}</span>
              <span className="mx-2">·</span>
              <span>Key Type: {KEY_TYPE_LABELS[keyData.keyType] || keyData.keyType}</span>
            </div>

            {/* New key type selector */}
            <div className="space-y-1.5">
              <Label htmlFor="replace-key-type">New Key Type</Label>
              <Select value={replaceKeyType} onValueChange={setReplaceKeyType}>
                <SelectTrigger id="replace-key-type" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {REPLACE_KEY_TYPES.map((type) => (
                    <SelectItem key={type} value={type}>
                      {KEY_TYPE_LABELS[type] || type}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            {/* Generated private key */}
            <div className="space-y-1.5">
              <Label>New Private Key</Label>
              <div className="flex items-center gap-2">
                <Input
                  readOnly
                  value={replaceKeyPrivateHex}
                  className="font-mono text-xs"
                  aria-label="Generated private key"
                />
                <CopyButton value={replaceKeyPrivateHex} />
              </div>
              <Button
                variant="outline"
                size="sm"
                onClick={handleRegenerateKey}
              >
                <RefreshCw className="mr-1.5 size-3.5" />
                Regenerate
              </Button>
            </div>
          </div>

          <DialogFooter>
            <Button variant="outline" onClick={handleCancelReplace}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={handleConfirmReplace}
            >
              Replace Master Key
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ConfirmationDialog
        open={showConfirmRemovePrivKey}
        onOpenChange={setShowConfirmRemovePrivKey}
        title="Remove Private Key"
        message="Are you sure you want to remove the private key from DET? You will no longer be able to sign messages or perform operations with this key unless you re-add it."
        confirmText="Remove"
        danger
        onResult={(status) => {
          if (status === "confirmed") handleConfirmRemovePrivKey();
        }}
      />
    </div>
  );
}

// ─── Metadata Row ──────────────────────────────────────────────────

function MetadataRow({
  label,
  value,
  children,
}: {
  label: string;
  value?: string;
  children?: React.ReactNode;
}) {
  return (
    <>
      <span className="text-muted-foreground">{label}</span>
      <span className="font-medium">
        {children || value}
      </span>
    </>
  );
}

// ─── Utility ───────────────────────────────────────────────────────

function hexToBase64(hex: string): string {
  try {
    const bytes = hex.match(/.{1,2}/g)?.map((b) => parseInt(b, 16)) ?? [];
    return btoa(String.fromCharCode(...bytes));
  } catch {
    return "(invalid hex)";
  }
}
