import { useState, useMemo, useCallback, useEffect, useLayoutEffect, useRef } from "react";
import {
  ArrowLeft,
  Loader2,
  CheckCircle,
  Info,
  Send,
  User,
  Wallet,
  AlertCircle,
  Lock,
} from "lucide-react";
import { useNavigate } from "@tanstack/react-router";
import { Island } from "@/components/layout/Island";
import { EmptyState } from "@/components/feedback/EmptyState";
import { AmountInput, useAmountInput, formatAmount } from "@/components/shared/AmountInput";
import { WalletUnlockDialog } from "@/components/shared";
import type { WalletUnlockResult } from "@/components/shared/WalletUnlockDialog";
import { ConfirmationDialog } from "@/components/shared/ConfirmationDialog";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Textarea } from "@/components/ui/textarea";
import { Separator } from "@/components/ui/separator";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useDashPayStore } from "@/stores/dashpayStore";
import { useIdentityStore } from "@/stores/identityStore";
import { useWalletStore } from "@/stores/walletStore";
import { useTaskListener } from "@/hooks/useTaskListener";
import type { TaskResultEvent, TaskErrorEvent } from "@/bindings";
import { cn } from "@/lib/utils";

// ─── Constants ────────────────────────────────────────────────────────

const MAX_MEMO_LENGTH = 100;
const WARN_MEMO_LENGTH = 80;

const PAYMENT_GUIDELINES = [
  "Payments are sent on the Dash Platform using DashPay.",
  "The amount is deducted from your wallet's confirmed balance.",
  "Memos are stored locally and are not sent on-chain.",
  "A small network fee is required to broadcast the payment.",
  "Payments cannot be reversed once sent.",
];

// ─── Types ────────────────────────────────────────────────────────────

type ScreenState = "form" | "sending" | "success" | "error";

interface SendPaymentScreenProps {
  contactId: string;
}

// ─── Component ────────────────────────────────────────────────────────

export function SendPaymentScreen({ contactId }: SendPaymentScreenProps) {
  const navigate = useNavigate();

  // ── Store access ──
  const {
    selectedIdentityId,
    contacts,
    sendPayment,
    loadContacts,
  } = useDashPayStore();
  const identities = useIdentityStore((s) => s.identities);
  const hdWallets = useWalletStore((s) => s.hdWallets);
  const unlockWallet = useWalletStore((s) => s.unlockWallet);

  // ── Local state ──
  const [screenState, setScreenState] = useState<ScreenState>("form");
  const [memo, setMemo] = useState("");
  const [showInfoPopup, setShowInfoPopup] = useState(false);
  const [showWalletUnlock, setShowWalletUnlock] = useState(false);
  const [walletUnlockError, setWalletUnlockError] = useState<string | null>(null);
  const [walletUnlockedHashes, setWalletUnlockedHashes] = useState<Set<string>>(new Set());
  const [sendError, setSendError] = useState<string | null>(null);
  const [sentAmount, setSentAmount] = useState<string>("");
  const [sentAddress, setSentAddress] = useState<string>("");
  const [showConfirmDialog, setShowConfirmDialog] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);

  const { value: amountValue, setValue: setAmountValue, parsedAmount, isValid: isAmountValid } =
    useAmountInput(8);

  // ── Derived state ──
  const selectedIdentity = useMemo(
    () => identities.find((i) => i.id === selectedIdentityId) ?? null,
    [identities, selectedIdentityId],
  );

  const contact = useMemo(
    () => contacts.find((c) => c.contactIdentityId === contactId) ?? null,
    [contacts, contactId],
  );

  const contactDisplayName = useMemo(() => {
    if (!contact) {
      // Truncate raw ID for display
      return contactId.length > 16
        ? `${contactId.slice(0, 8)}...${contactId.slice(-8)}`
        : contactId;
    }
    return contact.displayName || contact.username || contactId.slice(0, 16) + "...";
  }, [contact, contactId]);

  // Find associated wallet for the selected identity
  const associatedWallet = useMemo(() => {
    if (!selectedIdentity) return null;
    const hashes = selectedIdentity.associatedWalletHashes;
    if (!hashes || hashes.length === 0) return null;
    return hdWallets.find((w) => hashes.includes(w.seedHash)) ?? null;
  }, [selectedIdentity, hdWallets]);

  const walletBalance = associatedWallet?.confirmedBalance ?? 0;
  const walletAlias = associatedWallet?.alias ?? "Wallet";
  const walletLocked =
    !!associatedWallet &&
    associatedWallet.usesPassword &&
    !walletUnlockedHashes.has(associatedWallet.seedHash);

  const memoError = useMemo(() => {
    if (memo.length > MAX_MEMO_LENGTH) {
      return `Memo must be ${MAX_MEMO_LENGTH} characters or less`;
    }
    return null;
  }, [memo]);

  const memoCounterColor = useMemo(() => {
    if (memo.length > MAX_MEMO_LENGTH) return "text-destructive";
    if (memo.length >= WARN_MEMO_LENGTH) return "text-warning";
    return "text-muted-foreground";
  }, [memo.length]);

  const canSend = useMemo(() => {
    return (
      isAmountValid &&
      parsedAmount !== null &&
      parsedAmount > 0 &&
      memo.length <= MAX_MEMO_LENGTH &&
      screenState === "form" &&
      !walletLocked
    );
  }, [isAmountValid, parsedAmount, memo.length, screenState, walletLocked]);

  // ── Load contacts if empty ──
  useEffect(() => {
    if (selectedIdentityId && contacts.length === 0) {
      loadContacts();
    }
  }, [selectedIdentityId, contacts.length, loadContacts]);

  // ── Handlers ──
  const handleBack = useCallback(() => {
    navigate({ to: "/dashpay/contacts" });
  }, [navigate]);

  const handleMaxAmount = useCallback(() => {
    if (walletBalance > 0) {
      setAmountValue(formatAmount(walletBalance, 8));
    }
  }, [walletBalance, setAmountValue]);

  const handleSendClick = useCallback(() => {
    if (!selectedIdentityId || !canSend || parsedAmount === null) return;

    if (parsedAmount <= 0) {
      setSendError("Please enter an amount");
      return;
    }

    if (memo.length > MAX_MEMO_LENGTH) {
      setSendError(`Memo must be ${MAX_MEMO_LENGTH} characters or less`);
      return;
    }

    setSendError(null);
    setShowConfirmDialog(true);
  }, [selectedIdentityId, canSend, parsedAmount, memo]);

  const handleConfirmSend = useCallback(async () => {
    if (!selectedIdentityId || parsedAmount === null) return;

    // Convert from duffs to DASH
    const amountDash = parsedAmount / 100_000_000;

    setScreenState("sending");
    setSentAmount(formatAmount(parsedAmount, 8));
    setSendError(null);

    const id = await sendPayment({
      contactId,
      amountDash,
      memo: memo.trim() || null,
    });

    if (!id) {
      // IPC dispatch failed — store already has paymentsError
      const storeError = useDashPayStore.getState().paymentsError;
      setSendError(storeError ?? "Failed to send payment.");
      setScreenState("error");
      return;
    }

    // Task dispatched — stay in "sending", wait for task event
    setTaskId(id);
  }, [
    selectedIdentityId,
    parsedAmount,
    memo,
    contactId,
    sendPayment,
  ]);

  // Listen for task completion/error
  const contactDisplayNameRef = useRef(contactDisplayName);
  useLayoutEffect(() => {
    contactDisplayNameRef.current = contactDisplayName;
  });

  useTaskListener(
    taskId,
    useCallback((_event: TaskResultEvent) => {
      setSentAddress(`Sent to ${contactDisplayNameRef.current}`);
      setScreenState("success");
      setTaskId(null);
    }, []),
    useCallback((event: TaskErrorEvent) => {
      setSendError(event.message);
      setScreenState("error");
      setTaskId(null);
    }, []),
  );

  const handleSendAnother = useCallback(() => {
    setAmountValue("");
    setMemo("");
    setSendError(null);
    setSentAmount("");
    setSentAddress("");
    setTaskId(null);
    setScreenState("form");
  }, [setAmountValue]);

  const handleWalletUnlockResult = useCallback(
    async (result: WalletUnlockResult) => {
      if (result.status === "unlocked" && associatedWallet) {
        setWalletUnlockError(null);
        const error = await unlockWallet(
          { type: "hd", seedHash: associatedWallet.seedHash },
          result.password,
        );
        if (error) {
          setWalletUnlockError(error);
          return;
        }
        setWalletUnlockedHashes(
          (prev) => new Set([...prev, associatedWallet.seedHash]),
        );
        setShowWalletUnlock(false);
      }
    },
    [associatedWallet, unlockWallet],
  );

  // ── No identity state ──
  if (!selectedIdentityId || !selectedIdentity) {
    return (
      <Island>
        <EmptyState
          icon={User}
          title="No Identity Selected"
          description="Select an identity from the sidebar to send a payment."
        />
      </Island>
    );
  }

  // ── Success screen ──
  if (screenState === "success") {
    return (
      <Island className="flex-1">
        <div className="flex flex-col items-center justify-center gap-6 py-20">
          <CheckCircle className="h-16 w-16 text-green-500" />
          <div className="space-y-2 text-center">
            <h2 className="text-2xl font-bold">Payment Sent!</h2>
            <p className="text-muted-foreground">
              {sentAmount} DASH sent to {contactDisplayName}
            </p>
            {sentAddress && (
              <p className="text-xs text-muted-foreground font-mono">{sentAddress}</p>
            )}
          </div>
          <div className="flex gap-3">
            <Button variant="outline" onClick={handleBack}>
              Back to DashPay
            </Button>
            <Button onClick={handleSendAnother}>Send Another Payment</Button>
          </div>
        </div>
      </Island>
    );
  }

  // ── Main form ──
  return (
    <>
      <Island className="flex-1">
        <div className="space-y-6">
          {/* Header */}
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <Button variant="ghost" size="icon-sm" onClick={handleBack} aria-label="Go back">
                <ArrowLeft className="h-4 w-4" />
              </Button>
              <div className="flex items-center gap-2">
                <Send className="h-5 w-5 text-primary" />
                <h2 className="text-xl font-bold">Send Payment</h2>
              </div>
            </div>
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={() => setShowInfoPopup(true)}
              aria-label="Payment guidelines"
            >
              <Info className="h-4 w-4" />
            </Button>
          </div>

          <Separator />

          {/* Error banner */}
          {sendError && (
            <div
              className="flex items-start gap-2 rounded-md bg-destructive/10 px-4 py-3 text-sm text-destructive"
              role="alert"
            >
              <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
              <span>{sendError}</span>
            </div>
          )}

          {/* From Identity */}
          <div className="space-y-1.5">
            <Label className="text-sm font-medium text-muted-foreground">From</Label>
            <div className="flex items-center gap-2 rounded-md border bg-muted/50 px-3 py-2">
              <User className="h-4 w-4 text-muted-foreground" />
              <span className="text-sm font-medium">
                {selectedIdentity.alias ||
                  selectedIdentity.dpnsNames?.[0]?.name ||
                  `${selectedIdentity.id.slice(0, 8)}...${selectedIdentity.id.slice(-8)}`}
              </span>
            </div>
          </div>

          {/* Wallet Balance */}
          <div className="space-y-1.5">
            <Label className="text-sm font-medium text-muted-foreground">Wallet Balance</Label>
            {associatedWallet ? (
              <div className="flex items-center gap-2 rounded-md border bg-muted/50 px-3 py-2">
                <Wallet className="h-4 w-4 text-muted-foreground" />
                <span className="text-sm font-medium">{walletAlias}</span>
                <Badge variant="outline" className="ml-auto text-xs">
                  {formatAmount(walletBalance, 8)} DASH
                </Badge>
              </div>
            ) : (
              <div className="flex items-center gap-2 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2">
                <AlertCircle className="h-4 w-4 text-destructive" />
                <span className="text-sm text-destructive">
                  No wallet associated with this identity
                </span>
              </div>
            )}

            {/* Wallet locked warning */}
            {walletLocked && (
              <div className="flex items-center gap-2">
                <Lock className="h-3.5 w-3.5 text-amber-500" />
                <span className="text-xs text-amber-600 dark:text-amber-400">
                  Wallet is locked.
                </span>
                <Button
                  variant="link"
                  size="sm"
                  className="h-auto p-0 text-xs"
                  onClick={() => setShowWalletUnlock(true)}
                >
                  Unlock Wallet
                </Button>
              </div>
            )}
          </div>

          {/* To Contact */}
          <div className="space-y-1.5">
            <Label className="text-sm font-medium text-muted-foreground">To</Label>
            <div className="flex items-center gap-2 rounded-md border bg-muted/50 px-3 py-2">
              <User className="h-4 w-4 text-muted-foreground" />
              <span className="text-sm font-medium">{contactDisplayName}</span>
              {contact?.username && contact.displayName && (
                <span className="text-xs text-muted-foreground">@{contact.username}</span>
              )}
            </div>
          </div>

          {/* Amount Input */}
          <AmountInput
            value={amountValue}
            onChange={setAmountValue}
            label="Amount"
            placeholder="Enter amount in Dash"
            decimalPlaces={8}
            unitName="DASH"
            maxAmount={walletBalance > 0 ? walletBalance : null}
            showMaxButton={walletBalance > 0}
            onMaxClick={handleMaxAmount}
            disabled={screenState === "sending"}
          />

          {/* Memo */}
          <div className="space-y-1.5">
            <Label htmlFor="payment-memo" className="text-sm font-medium">
              Memo
            </Label>
            <Textarea
              id="payment-memo"
              value={memo}
              onChange={(e) => setMemo(e.target.value)}
              placeholder="Add a note to this payment (optional)"
              rows={3}
              disabled={screenState === "sending"}
              aria-describedby={memoError ? "memo-error" : undefined}
              aria-invalid={!!memoError}
            />
            <div className="flex items-center justify-between">
              {memoError ? (
                <p id="memo-error" className="text-sm text-destructive" role="alert">
                  {memoError}
                </p>
              ) : (
                <span />
              )}
              <span className={cn("text-xs", memoCounterColor)}>
                {memo.length}/{MAX_MEMO_LENGTH}
              </span>
            </div>
          </div>

          <Separator />

          {/* Action buttons */}
          <div className="flex items-center justify-end gap-3">
            <Button
              variant="outline"
              onClick={handleBack}
              disabled={screenState === "sending"}
            >
              Cancel
            </Button>
            <Button
              onClick={handleSendClick}
              disabled={!canSend || !associatedWallet}
            >
              {screenState === "sending" ? (
                <>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Sending...
                </>
              ) : (
                <>
                  <Send className="mr-2 h-4 w-4" />
                  Send Payment
                </>
              )}
            </Button>
          </div>
        </div>
      </Island>

      {/* Info popup */}
      <Dialog open={showInfoPopup} onOpenChange={setShowInfoPopup}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Payment Guidelines</DialogTitle>
            <DialogDescription>
              Important information about DashPay payments
            </DialogDescription>
          </DialogHeader>
          <ul className="space-y-2 text-sm text-muted-foreground" role="list">
            {PAYMENT_GUIDELINES.map((guideline, i) => (
              <li key={i} className="flex items-start gap-2">
                <span className="mt-0.5 text-primary">•</span>
                <span>{guideline}</span>
              </li>
            ))}
          </ul>
        </DialogContent>
      </Dialog>

      {/* Wallet unlock dialog */}
      {associatedWallet && (
        <WalletUnlockDialog
          open={showWalletUnlock}
          onOpenChange={setShowWalletUnlock}
          walletAlias={walletAlias}
          error={walletUnlockError}
          passwordHint={associatedWallet.passwordHint ?? null}
          onResult={handleWalletUnlockResult}
        />
      )}

      {/* Send confirmation dialog */}
      <ConfirmationDialog
        open={showConfirmDialog}
        onOpenChange={setShowConfirmDialog}
        title="Confirm Payment"
        message={`Send ${parsedAmount ? formatAmount(parsedAmount, 8) : "0"} DASH to ${contactDisplayName}?\n\nPayments cannot be reversed once sent.`}
        confirmText="Send Payment"
        cancelText="Cancel"
        onResult={(status) => {
          if (status === "confirmed") {
            handleConfirmSend();
          }
        }}
      />
    </>
  );
}
