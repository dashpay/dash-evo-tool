import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  AlertTriangle,
  ArrowLeft,
  Check,
  ChevronDown,
  ChevronUp,
  Clipboard,
  Eye,
  Loader2,
  Lock,
  Plus,
  Unlock,
  Users,
} from "lucide-react";
import { toast } from "sonner";
import { useNavigate } from "@tanstack/react-router";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn, displayId, hexToBase58 } from "@/lib/utils";
import { events } from "@/bindings";
import type {
  QualifiedIdentityDto,
  WalletDto,
  SingleKeyWalletDto,
  TaskResultEvent,
  TaskErrorEvent,
  DispatchTaskResponse,
} from "@/bindings";
import { useIdentityStore } from "@/stores/identityStore";
import { useWalletStore } from "@/stores/walletStore";
import { useTokenStore } from "@/stores/tokenStore";
import { WalletUnlockDialog } from "@/components/shared/WalletUnlockDialog";
import type { WalletUnlockResult } from "@/components/shared/WalletUnlockDialog";
import { ConfirmationDialog } from "@/components/shared/ConfirmationDialog";
import { toastError } from "@/lib/toastError";
import { formatTokenBalance } from "./MyTokensTable";

// ─── Types ──────────────────────────────────────────────────────────────────

export type OperationStatus =
  | { type: "idle" }
  | { type: "broadcasting"; startTime: number }
  | { type: "success" }
  | { type: "error"; message: string };

/**
 * Group action context passed to the form. Determines button text and
 * success messaging.
 */
export interface GroupActionContext {
  /** If set, user is signing an existing group action. */
  groupActionId?: string;
  /** Whether a group is required for this operation. */
  hasGroup: boolean;
  /** Whether the user has unilateral power in the group. */
  isUnilateral: boolean;
  /** Group member identities (for display). */
  groupMembers?: Array<{ identityId: string; power: number }>;
  /** Required group power. */
  requiredPower?: number;
}

/**
 * Configuration for the confirmation dialog.
 */
export interface ConfirmationConfig {
  /** Dialog title. */
  title: string;
  /** Dialog description / body. */
  description: string;
  /** Confirm button label (defaults to the action button text). */
  confirmLabel?: string;
  /** Whether to use destructive styling. */
  destructive?: boolean;
}

/**
 * Props for TokenOperationForm — a shared layout for all token action screens.
 */
export interface TokenOperationFormProps {
  /** The action name (e.g. "Transfer", "Mint", "Burn"). Used in titles and messages. */
  actionName: string;
  /** Token context: name, balance, IDs. */
  tokenContext: {
    tokenId: string;
    contractId: string;
    tokenPosition: number;
    name: string | null;
    balance: string;
    decimals: number;
    identityId: string;
  };
  /** Whether to show the amount input field. */
  showAmountInput?: boolean;
  /** Amount state (controlled). */
  amount?: string;
  /** Amount change handler. */
  onAmountChange?: (value: string) => void;
  /** Max amount for the "Max" button. */
  maxAmount?: string;
  /** Amount input label. */
  amountLabel?: string;
  /** Amount input placeholder. */
  amountPlaceholder?: string;
  /** Whether to show the recipient identity input. */
  showRecipientInput?: boolean;
  /** Recipient identity ID (controlled). */
  recipientId?: string;
  /** Recipient change handler. */
  onRecipientChange?: (value: string) => void;
  /** Recipient input label. */
  recipientLabel?: string;
  /** Recipient input placeholder. */
  recipientPlaceholder?: string;
  /** Whether recipient is optional. */
  recipientOptional?: boolean;
  /** Custom form content rendered between the standard inputs and the action bar. */
  children?: ReactNode;
  /** Group action context. */
  groupAction?: GroupActionContext;
  /** Confirmation dialog config. If omitted, action fires directly. */
  confirmation?: ConfirmationConfig;
  /** Whether the form inputs are valid (enables the action button). */
  isValid?: boolean;
  /** Custom validation message shown when isValid is false. */
  validationMessage?: string;
  /**
   * Called when the user confirms the action. Must return the IPC result
   * (with a taskId) or throw. Receives the selected identity ID, key ID,
   * and public note.
   */
  onSubmit: (params: {
    identityId: string;
    keyId: number;
    publicNote: string | null;
  }) => Promise<{ status: "ok"; data: DispatchTaskResponse } | { status: "error"; error: string }>;
  /** Result event type to listen for in task events (e.g. "tokenCompleted"). */
  resultEventType?: string;
  /** Custom success message. */
  successMessage?: string;
  /** Called after a successful operation. */
  onSuccess?: () => void;
  /** Label for the "do another" button on success screen. */
  doAnotherLabel?: string;
  /** Called when user clicks "do another". */
  onDoAnother?: () => void;
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

export function autoSelectKey(
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

/**
 * Determine the action button label based on group context.
 */
export function getActionButtonLabel(
  actionName: string,
  groupAction?: GroupActionContext,
): string {
  if (!groupAction) return actionName;
  if (groupAction.groupActionId) return `Sign ${actionName}`;
  if (groupAction.hasGroup && !groupAction.isUnilateral)
    return `Initiate Group ${actionName}`;
  return actionName;
}

/**
 * Determine the success title based on group context.
 */
export function getSuccessTitle(
  actionName: string,
  groupAction?: GroupActionContext,
): string {
  if (!groupAction) return `${actionName} Successful`;
  if (groupAction.groupActionId)
    return `Group ${actionName} Signing Successful`;
  if (groupAction.hasGroup && !groupAction.isUnilateral)
    return `Group ${actionName} Initiated`;
  return `${actionName} Successful`;
}

// ─── Component ──────────────────────────────────────────────────────────────

export function TokenOperationForm({
  actionName,
  tokenContext,
  showAmountInput = false,
  amount = "",
  onAmountChange,
  maxAmount,
  amountLabel = "Amount",
  amountPlaceholder = "0",
  showRecipientInput = false,
  recipientId = "",
  onRecipientChange,
  recipientLabel = "Recipient Identity ID",
  recipientPlaceholder = "Enter identity ID (hex or Base58)...",
  recipientOptional = false,
  children,
  groupAction,
  confirmation,
  isValid = true,
  validationMessage,
  onSubmit,
  resultEventType = "tokenCompleted",
  successMessage,
  onSuccess,
  doAnotherLabel,
  onDoAnother,
}: TokenOperationFormProps) {
  const navigate = useNavigate();

  // ── Store state ──────────────────────────────────────────────────────
  const identities = useIdentityStore((s) => s.identities);
  const loadIdentities = useIdentityStore((s) => s.loadIdentities);
  const hdWallets = useWalletStore((s) => s.hdWallets);
  const singleKeyWallets = useWalletStore((s) => s.singleKeyWallets);
  const loadWallets = useWalletStore((s) => s.loadWallets);
  const { loadMyTokenBalances } = useTokenStore();

  // ── Identity & key selection ─────────────────────────────────────────
  const [rawIdentityId, setRawIdentityId] = useState<string>(
    tokenContext.identityId || "",
  );
  const [rawKeyId, setRawKeyId] = useState<string>("");

  const selectedIdentity = useMemo(() => {
    const id = rawIdentityId || tokenContext.identityId;
    if (!id) return identities.length > 0 ? (identities[0] ?? null) : null;
    return identities.find((i) => i.id === id) ?? null;
  }, [rawIdentityId, tokenContext.identityId, identities]);

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

  // ── Wallet unlock ────────────────────────────────────────────────────
  const associatedWallet = useMemo(
    () =>
      findAssociatedWallet(selectedIdentity, hdWallets, singleKeyWallets),
    [selectedIdentity, hdWallets, singleKeyWallets],
  );

  const [walletUnlockOpen, setWalletUnlockOpen] = useState(false);
  const [walletUnlockError, setWalletUnlockError] = useState<string | null>(
    null,
  );
  const [walletUnlockedHashes, setWalletUnlockedHashes] = useState<
    Set<string>
  >(new Set());

  const walletLocked = useMemo(
    () =>
      !!associatedWallet &&
      associatedWallet.usesPassword &&
      !walletUnlockedHashes.has(associatedWallet.seedHash),
    [associatedWallet, walletUnlockedHashes],
  );

  // ── Advanced options ─────────────────────────────────────────────────
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [publicNote, setPublicNote] = useState("");

  // ── Confirmation dialog ──────────────────────────────────────────────
  const [confirmOpen, setConfirmOpen] = useState(false);

  // ── Broadcast status ─────────────────────────────────────────────────
  const [status, setStatus] = useState<OperationStatus>({ type: "idle" });
  const [elapsedMs, setElapsedMs] = useState(0);
  const activeTaskIdRef = useRef<string | null>(null);

  // ── Derived values ───────────────────────────────────────────────────
  const buttonLabel = useMemo(
    () => getActionButtonLabel(actionName, groupAction),
    [actionName, groupAction],
  );

  const successTitle = useMemo(
    () => getSuccessTitle(actionName, groupAction),
    [actionName, groupAction],
  );

  const formattedBalance = useMemo(
    () => formatTokenBalance(tokenContext.balance, tokenContext.decimals),
    [tokenContext.balance, tokenContext.decimals],
  );

  const isSubmitting = status.type === "broadcasting";

  const canSubmit =
    !!selectedIdentity &&
    selectedKeyId !== null &&
    !walletLocked &&
    isValid &&
    !isSubmitting;

  // ── Load data on mount ───────────────────────────────────────────────
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
          if (activeTaskIdRef.current !== taskId) return;
          if (resultEventType && result.type !== resultEventType) return;

          activeTaskIdRef.current = null;
          setStatus({ type: "success" });
          toast.success(
            successMessage || `${actionName} completed successfully!`,
          );
          loadMyTokenBalances();
          onSuccess?.();
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
  }, [actionName, resultEventType, successMessage, loadMyTokenBalances, onSuccess]);

  // ── Wallet unlock handler ────────────────────────────────────────────
  const handleWalletUnlockResult = useCallback(
    async (result: WalletUnlockResult) => {
      if (result.status === "unlocked" && associatedWallet) {
        setWalletUnlockError(null);
        try {
          const { commands } = await import("@/bindings");
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

  // ── Submit handler ───────────────────────────────────────────────────
  const handleSubmit = useCallback(async () => {
    if (!selectedIdentity || selectedKeyId === null) return;

    setConfirmOpen(false);
    setStatus({ type: "broadcasting", startTime: Date.now() });
    setElapsedMs(0);

    try {
      const result = await onSubmit({
        identityId: selectedIdentity.id,
        keyId: selectedKeyId,
        publicNote: publicNote.trim() || null,
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
  }, [selectedIdentity, selectedKeyId, publicNote, onSubmit]);

  // ── Action button click ──────────────────────────────────────────────
  const handleActionClick = useCallback(() => {
    if (confirmation) {
      setConfirmOpen(true);
    } else {
      handleSubmit();
    }
  }, [confirmation, handleSubmit]);

  // ── Reset to idle ────────────────────────────────────────────────────
  const handleTryAgain = useCallback(() => {
    setStatus({ type: "idle" });
  }, []);

  const handleBackToTokens = useCallback(() => {
    navigate({ to: "/tokens" });
  }, [navigate]);

  const handleGoToGroupActions = useCallback(() => {
    navigate({ to: "/contracts/group-actions" });
  }, [navigate]);

  // ════════════════════════════════════════════════════════════════════
  // Render: Success screen
  // ════════════════════════════════════════════════════════════════════
  if (status.type === "success") {
    return (
      <div
        className="flex flex-col items-center justify-center gap-6 py-12"
        data-testid="operation-success"
      >
        <div className="rounded-full bg-green-100 dark:bg-green-900/30 p-4">
          <Check className="h-8 w-8 text-green-600 dark:text-green-400" />
        </div>
        <h2 className="text-xl font-semibold">{successTitle}</h2>
        <p className="text-muted-foreground text-center max-w-md">
          {successMessage ||
            `${actionName} completed successfully on the Dash Platform.`}
        </p>
        <div className="flex gap-3">
          <Button
            variant="outline"
            onClick={handleBackToTokens}
            data-testid="operation-back-to-tokens"
          >
            <ArrowLeft className="h-4 w-4 mr-1" />
            Back to Tokens
          </Button>
          {groupAction?.hasGroup && !groupAction.isUnilateral && !groupAction.groupActionId && (
            <Button
              variant="outline"
              onClick={handleGoToGroupActions}
              data-testid="operation-go-to-group-actions"
            >
              <Users className="h-4 w-4 mr-1" />
              Go to Group Actions
            </Button>
          )}
          {onDoAnother && (
            <Button onClick={onDoAnother} data-testid="operation-do-another">
              {doAnotherLabel || `${actionName} Again`}
            </Button>
          )}
        </div>
      </div>
    );
  }

  // ════════════════════════════════════════════════════════════════════
  // Render: Error screen
  // ════════════════════════════════════════════════════════════════════
  if (status.type === "error") {
    return (
      <div
        className="flex flex-col items-center justify-center gap-6 py-12"
        data-testid="operation-error"
      >
        <div className="rounded-full bg-destructive/10 p-4">
          <AlertTriangle className="h-8 w-8 text-destructive" />
        </div>
        <h2 className="text-xl font-semibold">{actionName} Failed</h2>
        <p className="text-muted-foreground text-center max-w-md break-words">
          {status.message}
        </p>
        <div className="flex gap-3">
          <Button
            variant="outline"
            onClick={handleTryAgain}
            data-testid="operation-try-again"
          >
            Try Again
          </Button>
          <Button
            variant="outline"
            onClick={handleBackToTokens}
          >
            Back to Tokens
          </Button>
        </div>
      </div>
    );
  }

  // ════════════════════════════════════════════════════════════════════
  // Render: Main form (also handles broadcasting state inline)
  // ════════════════════════════════════════════════════════════════════
  return (
    <div className="space-y-6" data-testid="operation-form">
      {/* ── Broadcasting banner ────────────────────────────────────────── */}
      {isSubmitting && (
        <div
          className="flex items-center gap-3 rounded-lg border border-primary/30 bg-primary/5 p-4"
          data-testid="operation-broadcasting"
        >
          <Loader2 className="h-5 w-5 animate-spin text-primary shrink-0" />
          <div className="flex-1">
            <p className="font-medium text-sm">
              {groupAction?.groupActionId
                ? `Signing ${actionName}...`
                : `${buttonLabel}...`}
            </p>
            <p className="text-xs text-muted-foreground">
              Broadcasting to Dash Platform. This may take a moment.
            </p>
          </div>
          <span className="text-sm text-muted-foreground tabular-nums shrink-0">
            {(elapsedMs / 1000).toFixed(1)}s
          </span>
        </div>
      )}

      {/* ── Token context header ──────────────────────────────────────── */}
      <div
        className="flex items-start justify-between rounded-lg border bg-muted/30 p-4"
        data-testid="token-context-header"
      >
        <div className="space-y-1">
          <h3 className="text-lg font-semibold">
            {tokenContext.name || "Unnamed Token"}
          </h3>
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <span className="font-mono">{displayId(tokenContext.tokenId)}</span>
            <button
              className="text-muted-foreground hover:text-foreground transition-colors"
              onClick={() => {
                navigator.clipboard.writeText(hexToBase58(tokenContext.tokenId));
                toast.success("Token ID copied");
              }}
              aria-label="Copy token ID"
            >
              <Clipboard className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>
        <div className="text-right">
          <p className="text-sm text-muted-foreground">Balance</p>
          <p className="text-lg font-semibold tabular-nums">
            {formattedBalance}
          </p>
        </div>
      </div>

      {/* ── Group action info ─────────────────────────────────────────── */}
      {groupAction && groupAction.hasGroup && (
        <div
          className={cn(
            "flex items-start gap-3 rounded-lg border p-4",
            groupAction.groupActionId
              ? "border-amber-500/50 bg-amber-500/5"
              : "border-blue-500/50 bg-blue-500/5",
          )}
          data-testid="group-action-info"
        >
          <Users className="h-5 w-5 mt-0.5 shrink-0 text-muted-foreground" />
          <div className="space-y-1">
            {groupAction.groupActionId ? (
              <>
                <p className="font-medium">Group Action Signing</p>
                <p className="text-sm text-muted-foreground">
                  You are signing an existing group {actionName.toLowerCase()} action
                  (ID: {displayId(groupAction.groupActionId)}).
                </p>
              </>
            ) : groupAction.isUnilateral ? (
              <>
                <p className="font-medium">Unilateral Group Member</p>
                <p className="text-sm text-muted-foreground">
                  You have sufficient power to complete this {actionName.toLowerCase()} action
                  without needing other group members.
                </p>
              </>
            ) : (
              <>
                <p className="font-medium">Group Action Required</p>
                <p className="text-sm text-muted-foreground">
                  This action requires group approval. You will initiate the
                  {" "}{actionName.toLowerCase()} action and other group members will need to sign it.
                </p>
              </>
            )}
          </div>
        </div>
      )}

      {/* ── Identity & key selectors ──────────────────────────────────── */}
      <div className="space-y-3">
        <h3 className="text-sm font-semibold uppercase tracking-wider text-muted-foreground">
          Signing Identity
        </h3>
        <div className="grid gap-3 sm:grid-cols-2">
          <div className="space-y-1">
            <label className="text-sm font-medium" htmlFor="op-identity">
              Identity
            </label>
            <Select
              value={rawIdentityId || selectedIdentity?.id || ""}
              onValueChange={setRawIdentityId}
              disabled={isSubmitting}
            >
              <SelectTrigger data-testid="operation-identity-select">
                <SelectValue placeholder="Select identity..." />
              </SelectTrigger>
              <SelectContent>
                {identities.map((id) => (
                  <SelectItem key={id.id} value={id.id}>
                    {id.alias || displayId(id.id)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="space-y-1">
            <label className="text-sm font-medium" htmlFor="op-key">
              Signing Key
            </label>
            <Select
              value={
                rawKeyId ||
                (selectedKeyId !== null ? String(selectedKeyId) : "")
              }
              onValueChange={setRawKeyId}
              disabled={isSubmitting}
            >
              <SelectTrigger data-testid="operation-key-select">
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
                  data-testid="operation-unlock-wallet"
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
          <p className="text-sm text-destructive" data-testid="no-identities-warning">
            No identities loaded. Please load an identity before performing
            token operations.
          </p>
        )}
      </div>

      {/* ── Amount input ──────────────────────────────────────────────── */}
      {showAmountInput && (
        <div className="space-y-2" data-testid="amount-section">
          <label className="text-sm font-medium">{amountLabel}</label>
          <div className="flex gap-2">
            <Input
              type="text"
              inputMode="decimal"
              value={amount}
              onChange={(e) => onAmountChange?.(e.target.value)}
              placeholder={amountPlaceholder}
              disabled={isSubmitting}
              data-testid="operation-amount-input"
              className="font-mono"
            />
            {maxAmount && (
              <Button
                variant="outline"
                size="sm"
                onClick={() => onAmountChange?.(maxAmount)}
                disabled={isSubmitting}
                data-testid="operation-max-button"
              >
                Max
              </Button>
            )}
          </div>
          {maxAmount && (
            <p className="text-xs text-muted-foreground">
              Available: {formatTokenBalance(maxAmount, tokenContext.decimals)}
            </p>
          )}
        </div>
      )}

      {/* ── Recipient input ───────────────────────────────────────────── */}
      {showRecipientInput && (
        <div className="space-y-2" data-testid="recipient-section">
          <label className="text-sm font-medium">
            {recipientLabel}
            {recipientOptional && (
              <Badge variant="outline" className="ml-2 text-xs">
                Optional
              </Badge>
            )}
          </label>
          <Input
            type="text"
            value={recipientId}
            onChange={(e) => onRecipientChange?.(e.target.value)}
            placeholder={recipientPlaceholder}
            disabled={isSubmitting}
            data-testid="operation-recipient-input"
            className="font-mono"
          />
        </div>
      )}

      {/* ── Custom form content ───────────────────────────────────────── */}
      {children}

      {/* ── Advanced options ──────────────────────────────────────────── */}
      <div className="space-y-3">
        <button
          className="flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground transition-colors"
          onClick={() => setShowAdvanced(!showAdvanced)}
          data-testid="operation-advanced-toggle"
        >
          {showAdvanced ? (
            <ChevronUp className="h-4 w-4" />
          ) : (
            <ChevronDown className="h-4 w-4" />
          )}
          Advanced Options
        </button>
        {showAdvanced && (
          <div className="space-y-3 rounded-lg border bg-muted/20 p-4">
            <div className="space-y-1">
              <label className="text-sm font-medium">Public Note</label>
              <Input
                type="text"
                value={publicNote}
                onChange={(e) => setPublicNote(e.target.value)}
                placeholder="A note about the transaction visible to the public..."
                disabled={isSubmitting || !!groupAction?.groupActionId}
                data-testid="operation-public-note"
              />
              {groupAction?.groupActionId && (
                <p className="text-xs text-muted-foreground">
                  Cannot modify the note when signing an existing group action.
                </p>
              )}
            </div>

            {/* ── Key management shortcuts ──────────────────────────── */}
            {selectedIdentity && (
              <div className="space-y-1">
                <label className="text-sm font-medium">Key Management</label>
                <div className="flex gap-2">
                  {selectedKeyId !== null && (
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => navigate({ to: "/identities" })}
                      data-testid="operation-view-key-info"
                    >
                      <Eye className="h-3.5 w-3.5 mr-1" />
                      View Key Info
                    </Button>
                  )}
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => navigate({ to: "/identities" })}
                    data-testid="operation-add-key"
                  >
                    <Plus className="h-3.5 w-3.5 mr-1" />
                    Add Key
                  </Button>
                </div>
                <p className="text-xs text-muted-foreground">
                  Navigate to Identities to manage keys for this identity.
                </p>
              </div>
            )}
          </div>
        )}
      </div>

      {/* ── Validation message ────────────────────────────────────────── */}
      {validationMessage && !isValid && (
        <p
          className="text-sm text-destructive"
          data-testid="operation-validation-message"
        >
          {validationMessage}
        </p>
      )}

      {/* ── Action bar ────────────────────────────────────────────────── */}
      <div className="flex items-center justify-between border-t pt-4">
        <Button
          variant="ghost"
          onClick={handleBackToTokens}
          disabled={isSubmitting}
          data-testid="operation-cancel"
        >
          <ArrowLeft className="h-4 w-4 mr-1" />
          Back to Tokens
        </Button>
        <div className="flex items-center gap-3">
          {isSubmitting && (
            <span className="text-sm text-muted-foreground tabular-nums">
              {(elapsedMs / 1000).toFixed(1)}s
            </span>
          )}
          <Button
            onClick={handleActionClick}
            disabled={!canSubmit}
            data-testid="operation-submit"
            variant={confirmation?.destructive ? "destructive" : "default"}
          >
            {isSubmitting && (
              <Loader2 className="h-4 w-4 mr-2 animate-spin" />
            )}
            {isSubmitting ? `${buttonLabel}...` : buttonLabel}
          </Button>
        </div>
      </div>

      {/* ── Confirmation dialog ───────────────────────────────────────── */}
      {confirmation && (
        <ConfirmationDialog
          open={confirmOpen}
          onOpenChange={setConfirmOpen}
          title={confirmation.title}
          message={confirmation.description}
          confirmText={confirmation.confirmLabel || buttonLabel}
          danger={confirmation.destructive}
          onResult={(result) => {
            if (result === "confirmed") handleSubmit();
          }}
        />
      )}

      {/* ── Wallet unlock dialog ──────────────────────────────────────── */}
      {associatedWallet && (
        <WalletUnlockDialog
          open={walletUnlockOpen}
          onOpenChange={setWalletUnlockOpen}
          walletAlias={associatedWallet.alias ?? ""}
          passwordHint={associatedWallet.passwordHint}
          onResult={handleWalletUnlockResult}
          error={walletUnlockError}
        />
      )}
    </div>
  );
}
