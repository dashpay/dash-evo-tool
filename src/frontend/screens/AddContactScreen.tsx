import { useState, useMemo, useCallback } from "react";
import { useTaskListener } from "@/hooks/useTaskListener";
import type { TaskResultEvent, TaskErrorEvent } from "@/bindings";
import {
  UserPlus,
  ArrowLeft,
  Loader2,
  CheckCircle,
  Info,
  Key,
  ChevronDown,
} from "lucide-react";
import { useNavigate } from "@tanstack/react-router";
import { Island } from "@/components/layout/Island";
import { EmptyState } from "@/components/feedback/EmptyState";
import { WalletUnlockDialog } from "@/components/shared";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
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
import { Separator } from "@/components/ui/separator";
import { useDashPayStore } from "@/stores/dashpayStore";
import { useIdentityStore } from "@/stores/identityStore";
import { useWalletStore } from "@/stores/walletStore";
import { cn } from "@/lib/utils";
import type { IdentityKeyDto } from "@/bindings";

// ─── Constants ────────────────────────────────────────────────────────

const MAX_ACCOUNT_LABEL = 100;
const WARN_ACCOUNT_LABEL = 80;

const CONTACT_REQUEST_INFO = [
  "Contact requests are sent on the Dash Platform blockchain.",
  "The recipient must accept the request before you become contacts.",
  "You need an identity with an AUTHENTICATION key to send a request.",
  "A small fee is required to broadcast the contact request.",
  "Once accepted, you can exchange encrypted messages and payments.",
  "Contact requests cannot be cancelled once sent.",
];

// ─── Validation ───────────────────────────────────────────────────────

interface ValidationResult {
  valid: boolean;
  error?: string;
  tip?: string;
}

function validateUsernameOrId(value: string): ValidationResult {
  if (!value.trim()) {
    return { valid: false, error: "Username or Identity ID is required." };
  }
  if (value.includes(".") && !value.endsWith(".dash")) {
    return {
      valid: false,
      error: "Invalid username format.",
      tip: 'Usernames must end with ".dash" (e.g., alice.dash).',
    };
  }
  return { valid: true };
}

function validateAccountLabel(value: string): ValidationResult {
  if (value.length > MAX_ACCOUNT_LABEL) {
    return {
      valid: false,
      error: `Label too long (${value.length}/${MAX_ACCOUNT_LABEL} characters).`,
      tip: "Use a shorter relationship label.",
    };
  }
  return { valid: true };
}

// ─── Key selection helpers ────────────────────────────────────────────

/** Find the best signing key for contact requests: AUTHENTICATION + CRITICAL/HIGH */
function findBestSigningKey(
  keys: IdentityKeyDto[],
): IdentityKeyDto | undefined {
  return keys.find(
    (k) =>
      k.purpose === "AUTHENTICATION" &&
      (k.securityLevel === "CRITICAL" || k.securityLevel === "HIGH") &&
      !k.isDisabled,
  );
}

/** Filter keys suitable for signing contact requests */
function getSigningKeys(keys: IdentityKeyDto[]): IdentityKeyDto[] {
  return keys.filter(
    (k) => k.purpose === "AUTHENTICATION" && !k.isDisabled,
  );
}

/** Format a key for display in the selector */
function formatKeyLabel(key: IdentityKeyDto): string {
  return `Key #${key.keyId} — ${key.purpose} (${key.securityLevel}, ${key.keyType})`;
}

// ─── Error categorization ─────────────────────────────────────────────

type ErrorCategory =
  | "missing_encryption_key"
  | "missing_decryption_key"
  | "username_not_found"
  | "network"
  | "generic";

function categorizeError(error: string): ErrorCategory {
  const lower = error.toLowerCase();
  if (lower.includes("encryption key") || lower.includes("encryptionkey")) {
    return "missing_encryption_key";
  }
  if (lower.includes("decryption key") || lower.includes("decryptionkey")) {
    return "missing_decryption_key";
  }
  if (
    lower.includes("username") &&
    (lower.includes("not found") || lower.includes("resolution"))
  ) {
    return "username_not_found";
  }
  if (lower.includes("network") || lower.includes("connection")) {
    return "network";
  }
  return "generic";
}

function getErrorRecoveryAction(
  category: ErrorCategory,
): { label: string; action: "add_key" | "retry" } | null {
  switch (category) {
    case "missing_encryption_key":
      return { label: "Add Encryption Key", action: "add_key" };
    case "missing_decryption_key":
      return { label: "Add Decryption Key", action: "add_key" };
    case "username_not_found":
    case "network":
      return { label: "Retry", action: "retry" };
    default:
      return null;
  }
}

function getErrorTip(category: ErrorCategory): string | null {
  switch (category) {
    case "missing_encryption_key":
      return "Your identity needs an ENCRYPTION key to send contact requests. Add one in the Identity Keys screen.";
    case "missing_decryption_key":
      return "Your identity needs a DECRYPTION key to send contact requests. Add one in the Identity Keys screen.";
    case "username_not_found":
      return 'Check that the username is correct. DPNS usernames end with ".dash" (e.g., alice.dash).';
    case "network":
      return "Check your network connection and try again.";
    default:
      return null;
  }
}

// ─── Component ────────────────────────────────────────────────────────

type ScreenState = "form" | "sending" | "success";

export function AddContactScreen() {
  const navigate = useNavigate();

  // ── Store state ──
  const selectedIdentityId = useDashPayStore((s) => s.selectedIdentityId);
  const requestsError = useDashPayStore((s) => s.requestsError);
  const sendContactRequest = useDashPayStore((s) => s.sendContactRequest);
  const clearErrors = useDashPayStore((s) => s.clearErrors);
  const identities = useIdentityStore((s) => s.identities);
  const wallets = useWalletStore((s) => s.hdWallets);

  // ── Derived identity ──
  const selectedIdentity = useMemo(
    () => identities.find((i) => i.id === selectedIdentityId) ?? null,
    [identities, selectedIdentityId],
  );

  // ── Wallet alias for unlock dialog ──
  const walletAlias = useMemo(() => {
    if (!selectedIdentity) return "Wallet";
    const hash = selectedIdentity.associatedWalletHashes[0];
    if (!hash) return "Wallet";
    const wallet = wallets.find((w) => w.seedHash === hash);
    return wallet?.alias ?? "Wallet";
  }, [selectedIdentity, wallets]);

  // ── Available signing keys ──
  const signingKeys = useMemo(
    () => (selectedIdentity ? getSigningKeys(selectedIdentity.keys) : []),
    [selectedIdentity],
  );

  // ── Form state ──
  const [usernameOrId, setUsernameOrId] = useState("");
  const [accountLabel, setAccountLabel] = useState("");
  const [selectedKeyId, setSelectedKeyId] = useState<number | null>(null);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [screenState, setScreenState] = useState<ScreenState>("form");
  const [taskId, setTaskId] = useState<string | null>(null);
  const [localError, setLocalError] = useState<string | null>(null);
  const [trackedIdentityId, setTrackedIdentityId] =
    useState(selectedIdentityId);

  // ── Dialog state ──
  const [showWalletUnlock, setShowWalletUnlock] = useState(false);
  const [showInfoDialog, setShowInfoDialog] = useState(false);

  // ── Auto-select best key when identity changes ──
  if (trackedIdentityId !== selectedIdentityId) {
    setTrackedIdentityId(selectedIdentityId);
    setSelectedKeyId(null);
    setLocalError(null);
    setScreenState("form");
    setTaskId(null);
  }

  // Auto-select key if none selected yet
  if (
    selectedIdentity &&
    selectedKeyId === null &&
    signingKeys.length > 0
  ) {
    const best = findBestSigningKey(selectedIdentity.keys);
    if (best) {
      setSelectedKeyId(best.keyId);
    } else if (signingKeys.length > 0) {
      setSelectedKeyId(signingKeys[0]!.keyId);
    }
  }

  // ── Validation ──
  const usernameValidation = useMemo(
    () => (usernameOrId ? validateUsernameOrId(usernameOrId) : { valid: false }),
    [usernameOrId],
  );
  const labelValidation = useMemo(
    () => validateAccountLabel(accountLabel),
    [accountLabel],
  );

  const selectedKey = useMemo(
    () =>
      selectedIdentity?.keys.find((k) => k.keyId === selectedKeyId) ?? null,
    [selectedIdentity, selectedKeyId],
  );

  const canSend =
    usernameOrId.trim().length > 0 &&
    usernameValidation.valid &&
    labelValidation.valid &&
    selectedIdentity !== null &&
    selectedKeyId !== null;

  const showSummary =
    selectedIdentity !== null && usernameOrId.trim().length > 0;

  // ── Computed error display ──
  const displayError = localError ?? requestsError;
  const errorCategory = displayError ? categorizeError(displayError) : null;
  const recoveryAction = errorCategory
    ? getErrorRecoveryAction(errorCategory)
    : null;
  const errorTip = errorCategory ? getErrorTip(errorCategory) : null;

  // ── Handlers ──

  const handleSend = useCallback(async () => {
    // Client-side validation
    const uv = validateUsernameOrId(usernameOrId);
    if (!uv.valid) {
      setLocalError(uv.error ?? "Invalid input.");
      return;
    }
    const lv = validateAccountLabel(accountLabel);
    if (!lv.valid) {
      setLocalError(lv.error ?? "Invalid label.");
      return;
    }
    if (!selectedIdentity || selectedKeyId === null) {
      setLocalError("Please select an identity and signing key.");
      return;
    }

    setLocalError(null);
    setScreenState("sending");

    const id = await sendContactRequest({
      signingKeyId: selectedKeyId,
      toUsername: usernameOrId.trim(),
      accountLabel: accountLabel.trim() || null,
    });

    if (!id) {
      // IPC dispatch failed — store already has requestsError
      const storeError = useDashPayStore.getState().requestsError;
      setLocalError(storeError ?? "Failed to send contact request.");
      setScreenState("form");
      return;
    }

    // Task dispatched — stay in "sending", wait for task event
    setTaskId(id);
  }, [
    usernameOrId,
    accountLabel,
    selectedIdentity,
    selectedKeyId,
    sendContactRequest,
  ]);

  // Listen for task completion/error
  useTaskListener(
    taskId,
    useCallback((_event: TaskResultEvent) => {
      setScreenState("success");
      setTaskId(null);
    }, []),
    useCallback((event: TaskErrorEvent) => {
      setLocalError(event.message);
      setScreenState("form");
      setTaskId(null);
    }, []),
  );

  const handleRetry = useCallback(() => {
    setLocalError(null);
    clearErrors();
    setTaskId(null);
    setScreenState("form");
  }, [clearErrors]);

  const handleSendAnother = useCallback(() => {
    setUsernameOrId("");
    setAccountLabel("");
    setLocalError(null);
    setTaskId(null);
    setScreenState("form");
  }, []);

  const handleBack = useCallback(() => {
    navigate({ to: "/dashpay/contacts" });
  }, [navigate]);

  const handleAddKey = useCallback(() => {
    navigate({ to: "/identities" });
  }, [navigate]);

  const handleWalletUnlockResult = useCallback(
    (result: { status: "unlocked" | "cancelled" }) => {
      if (result.status === "unlocked") {
        setShowWalletUnlock(false);
        handleSend();
      }
    },
    [handleSend],
  );

  // ── No identity selected ──
  if (!selectedIdentityId || !selectedIdentity) {
    return (
      <Island className="flex-1">
        <EmptyState
          icon={UserPlus}
          title="No Identity Selected"
          description="Select an identity from the sidebar to add a contact."
        />
      </Island>
    );
  }

  // ── Success screen ──
  if (screenState === "success") {
    return (
      <Island className="flex-1">
        <div className="flex flex-col items-center justify-center gap-6 py-16">
          <div className="rounded-full bg-success/10 p-4">
            <CheckCircle className="h-12 w-12 text-success" />
          </div>
          <div className="text-center space-y-2">
            <h2 className="text-xl font-semibold">
              Contact Request Sent
            </h2>
            <p className="text-sm text-muted-foreground max-w-md">
              Your contact request has been sent to{" "}
              <span className="font-medium text-foreground">
                {usernameOrId}
              </span>
              . They will need to accept it before you become contacts.
            </p>
          </div>
          <div className="flex gap-3">
            <Button variant="outline" onClick={handleSendAnother}>
              Send Another Request
            </Button>
            <Button variant="outline" onClick={handleBack}>
              Back to Contacts
            </Button>
          </div>
        </div>
      </Island>
    );
  }

  // ── Form screen ──
  return (
    <Island className="flex-1 flex flex-col min-h-0">
      {/* Header */}
      <div className="flex items-center justify-between px-1 pb-4">
        <div className="flex items-center gap-3">
          <Button
            variant="ghost"
            size="icon"
            onClick={handleBack}
            aria-label="Back to contacts"
          >
            <ArrowLeft className="h-4 w-4" />
          </Button>
          <h2 className="text-xl font-semibold">Add Contact</h2>
        </div>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              onClick={() => setShowInfoDialog(true)}
              aria-label="Contact request info"
            >
              <Info className="h-4 w-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>About contact requests</TooltipContent>
        </Tooltip>
      </div>

      <Separator className="mb-6" />

      <div className="flex-1 overflow-y-auto px-1 space-y-6">
        {/* From (Sender) Section */}
        <div className="space-y-3">
          <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
            From
          </h3>
          <div className="rounded-lg border bg-card p-4 space-y-3">
            <div className="flex items-center gap-3">
              <div className="h-8 w-8 rounded-full bg-primary/10 flex items-center justify-center">
                <Key className="h-4 w-4 text-primary" />
              </div>
              <div className="min-w-0 flex-1">
                <p className="text-sm font-medium truncate">
                  {selectedIdentity.alias ??
                    `${selectedIdentity.id.slice(0, 12)}...`}
                </p>
                <p className="text-xs text-muted-foreground font-mono truncate">
                  {selectedIdentity.id}
                </p>
              </div>
              <Badge variant="outline" className="shrink-0">
                {selectedIdentity.identityType}
              </Badge>
            </div>

            {/* Advanced Options — Key Selector */}
            <div>
              <Button
                variant="ghost"
                size="sm"
                className="w-full justify-between text-xs text-muted-foreground"
                onClick={() => setShowAdvanced(!showAdvanced)}
                data-testid="advanced-options-toggle"
              >
                Advanced Options
                <ChevronDown
                  className={cn(
                    "h-3.5 w-3.5 transition-transform",
                    showAdvanced && "rotate-180",
                  )}
                />
              </Button>
              {showAdvanced && (
                <div className="pt-2 space-y-2">
                  <Label
                    htmlFor="signing-key"
                    className="text-xs text-muted-foreground"
                  >
                    Signing Key
                  </Label>
                  {signingKeys.length === 0 ? (
                    <p className="text-xs text-destructive">
                      No AUTHENTICATION keys available on this identity.
                    </p>
                  ) : (
                    <Select
                      value={
                        selectedKeyId !== null
                          ? String(selectedKeyId)
                          : undefined
                      }
                      onValueChange={(v) => setSelectedKeyId(Number(v))}
                    >
                      <SelectTrigger
                        id="signing-key"
                        data-testid="key-selector"
                      >
                        <SelectValue placeholder="Select signing key" />
                      </SelectTrigger>
                      <SelectContent>
                        {signingKeys.map((k) => (
                          <SelectItem
                            key={k.keyId}
                            value={String(k.keyId)}
                          >
                            {formatKeyLabel(k)}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  )}
                </div>
              )}
            </div>
          </div>
        </div>

        {/* To (Recipient) Section */}
        <div className="space-y-3">
          <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
            To
          </h3>
          <div className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="username-or-id">
                Username or Identity ID
              </Label>
              <Input
                id="username-or-id"
                value={usernameOrId}
                onChange={(e) => {
                  setUsernameOrId(e.target.value);
                  setLocalError(null);
                }}
                placeholder="e.g., alice.dash or identity ID"
                disabled={screenState === "sending"}
                data-testid="username-input"
                aria-invalid={
                  usernameOrId.length > 0 && !usernameValidation.valid
                }
              />
              {usernameOrId.length > 0 && !usernameValidation.valid && (
                <p className="text-xs text-destructive">
                  {usernameValidation.error}
                  {usernameValidation.tip && (
                    <span className="block text-muted-foreground mt-0.5">
                      {usernameValidation.tip}
                    </span>
                  )}
                </p>
              )}
            </div>

            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <Label htmlFor="account-label">
                  Relationship Label{" "}
                  <span className="text-muted-foreground font-normal">
                    (optional)
                  </span>
                </Label>
                <span
                  className={cn(
                    "text-xs",
                    accountLabel.length > MAX_ACCOUNT_LABEL
                      ? "text-destructive"
                      : accountLabel.length > WARN_ACCOUNT_LABEL
                        ? "text-warning"
                        : "text-muted-foreground",
                  )}
                >
                  {accountLabel.length}/{MAX_ACCOUNT_LABEL}
                </span>
              </div>
              <Input
                id="account-label"
                value={accountLabel}
                onChange={(e) => {
                  setAccountLabel(e.target.value);
                  setLocalError(null);
                }}
                placeholder="e.g., Friend, Family, Business Partner"
                disabled={screenState === "sending"}
                data-testid="label-input"
                maxLength={MAX_ACCOUNT_LABEL + 10}
                aria-invalid={!labelValidation.valid}
              />
              {!labelValidation.valid && (
                <p className="text-xs text-destructive">
                  {labelValidation.error}
                </p>
              )}
            </div>
          </div>
        </div>

        {/* Request Summary */}
        {showSummary && (
          <div className="space-y-3">
            <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
              Request Summary
            </h3>
            <div
              className="rounded-lg border bg-muted/30 p-4 space-y-2 text-sm"
              data-testid="request-summary"
            >
              <div className="flex gap-2">
                <span className="font-medium text-muted-foreground w-14 shrink-0">
                  From:
                </span>
                <span className="truncate">
                  {selectedIdentity.alias ??
                    selectedIdentity.id.slice(0, 20) + "..."}
                </span>
              </div>
              <div className="flex gap-2">
                <span className="font-medium text-muted-foreground w-14 shrink-0">
                  To:
                </span>
                <span className="truncate">{usernameOrId}</span>
              </div>
              {accountLabel.trim() && (
                <div className="flex gap-2">
                  <span className="font-medium text-muted-foreground w-14 shrink-0">
                    Label:
                  </span>
                  <span className="truncate">{accountLabel}</span>
                </div>
              )}
              {selectedKey && (
                <div className="flex gap-2">
                  <span className="font-medium text-muted-foreground w-14 shrink-0">
                    Key:
                  </span>
                  <span className="truncate text-muted-foreground">
                    #{selectedKey.keyId} ({selectedKey.securityLevel})
                  </span>
                </div>
              )}
            </div>
          </div>
        )}

        {/* Error display */}
        {displayError && screenState !== "sending" && (
          <div
            className="rounded-md border border-destructive/30 bg-destructive/5 p-4 space-y-2"
            role="alert"
            data-testid="error-banner"
          >
            <p className="text-sm text-destructive font-medium">
              {displayError}
            </p>
            {errorTip && (
              <p className="text-xs text-muted-foreground">{errorTip}</p>
            )}
            {recoveryAction && (
              <Button
                variant="outline"
                size="sm"
                className="mt-1"
                onClick={
                  recoveryAction.action === "add_key"
                    ? handleAddKey
                    : handleRetry
                }
                data-testid="recovery-action-button"
              >
                {recoveryAction.label}
              </Button>
            )}
          </div>
        )}

        {/* Sending state */}
        {screenState === "sending" && (
          <div className="flex items-center gap-3 rounded-lg border bg-muted/30 p-4">
            <Loader2 className="h-5 w-5 animate-spin text-primary" />
            <p className="text-sm text-muted-foreground">
              Sending contact request...
            </p>
          </div>
        )}
      </div>

      {/* Action bar */}
      <Separator className="mt-6 mb-4" />
      <div className="flex items-center justify-between px-1">
        <Button variant="ghost" onClick={handleBack}>
          Cancel
        </Button>
        <Button
          onClick={handleSend}
          disabled={!canSend || screenState === "sending"}
          data-testid="send-button"
        >
          {screenState === "sending" ? (
            <>
              <Loader2 className="mr-1.5 h-4 w-4 animate-spin" />
              Sending...
            </>
          ) : (
            <>
              <UserPlus className="mr-1.5 h-4 w-4" />
              Add Contact
            </>
          )}
        </Button>
      </div>

      {/* Info Dialog */}
      <Dialog open={showInfoDialog} onOpenChange={setShowInfoDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>About Contact Requests</DialogTitle>
            <DialogDescription>
              How contact requests work on the Dash Platform.
            </DialogDescription>
          </DialogHeader>
          <ul className="space-y-2 text-sm text-muted-foreground">
            {CONTACT_REQUEST_INFO.map((item, i) => (
              <li key={i} className="flex gap-2">
                <span className="text-muted-foreground/50 shrink-0">
                  •
                </span>
                <span>{item}</span>
              </li>
            ))}
          </ul>
        </DialogContent>
      </Dialog>

      {/* Wallet Unlock Dialog */}
      <WalletUnlockDialog
        open={showWalletUnlock}
        onOpenChange={setShowWalletUnlock}
        walletAlias={walletAlias}
        onResult={handleWalletUnlockResult}
      />
    </Island>
  );
}
