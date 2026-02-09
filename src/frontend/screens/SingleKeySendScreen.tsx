import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Island } from "@/components/layout";
import { AmountInput, formatAmount } from "@/components/shared/AmountInput";
import { FeeConfirmationDialog } from "@/components/shared/FeeConfirmationDialog";
import type { FeeConfirmationResult } from "@/components/shared/FeeConfirmationDialog";
import { WalletUnlockDialog } from "@/components/shared/WalletUnlockDialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import { useWalletStore } from "@/stores/walletStore";
import { commands, events } from "@/bindings";
import type { SingleKeyWalletDto } from "@/bindings";
import {
  ArrowLeft,
  Send,
  Loader2,
  CheckCircle2,
  AlertCircle,
  AlertTriangle,
  X,
  Plus,
  Trash2,
} from "lucide-react";

// ─── Constants ──────────────────────────────────────────────────────

const DUFFS_PER_DASH = 100_000_000;

// ─── Types ──────────────────────────────────────────────────────────

interface Recipient {
  id: number;
  address: string;
  amount: string;
  error: string | null;
}

type SendStatus =
  | { state: "idle" }
  | { state: "sending"; startTime: number; taskId?: string }
  | { state: "complete"; message: string }
  | { state: "error"; message: string };

// ─── Address validation ─────────────────────────────────────────────

function validateRecipientAddress(address: string): string | null {
  const trimmed = address.trim();
  if (!trimmed) return null;
  // Reject platform addresses
  if (trimmed.startsWith("evo1") || trimmed.startsWith("tevo1")) {
    return "Platform addresses are not supported for single-key sends. Use a Core address.";
  }
  // Basic Dash address format check
  if (!/^[Xy789][a-km-zA-HJ-NP-Z1-9]{24,}$/.test(trimmed)) {
    return "Invalid Dash address format";
  }
  return null;
}

// ─── Helpers ────────────────────────────────────────────────────────

function formatDash(duffs: number): string {
  return formatAmount(duffs, 8) + " DASH";
}

function duffsFromDashString(value: string): number | null {
  const num = parseFloat(value);
  if (isNaN(num) || num < 0) return null;
  return Math.round(num * DUFFS_PER_DASH);
}

// ─── Transaction size estimation ────────────────────────────────────

/**
 * Estimate the byte size of a P2PKH transaction given input/output counts.
 * Matches the Rust `estimate_p2pkh_tx_size` in model/fee_estimation.rs.
 */
export function estimateP2pkhTxSize(inputs: number, outputs: number): number {
  function varintSize(value: number): number {
    if (value <= 0xfc) return 1;
    if (value <= 0xffff) return 3;
    if (value <= 0xffffffff) return 5;
    return 9;
  }
  let size = 8; // version/type/lock_time
  size += varintSize(inputs);
  size += varintSize(outputs);
  size += inputs * 148; // P2PKH input size
  size += outputs * 34; // P2PKH output size
  return size;
}

/** Fee rate in duffs per byte (Normal fee level). */
const FEE_RATE_DUFFS_PER_BYTE = 1;

/**
 * Estimate the fee, input count, and tx size for a single-key send.
 * Returns [estimatedFee, inputCount, txSize] or null if estimation isn't possible.
 * Mirrors the Rust estimate_fee() logic in single_key_send_screen.rs.
 */
export function estimateFee(
  utxos: { amount: number }[],
  totalSendDuffs: number,
  recipientCount: number,
): { fee: number; inputCount: number; txSize: number } | null {
  if (utxos.length === 0) return null;

  const outputCount = recipientCount + 1; // +1 for change

  if (totalSendDuffs <= 0) {
    // No valid amounts yet — show estimate for minimum tx (1 input)
    const txSize = estimateP2pkhTxSize(1, outputCount);
    const fee = txSize * FEE_RATE_DUFFS_PER_BYTE;
    return { fee, inputCount: 1, txSize };
  }

  // Sort UTXOs by value descending to minimize input count
  const sortedValues = utxos.map((u) => u.amount).sort((a, b) => b - a);

  let selectedCount = 0;
  let selectedTotal = 0;

  for (const value of sortedValues) {
    selectedCount += 1;
    selectedTotal += value;

    const currentSize = estimateP2pkhTxSize(selectedCount, outputCount);
    const currentFee = currentSize * FEE_RATE_DUFFS_PER_BYTE;

    if (selectedTotal >= totalSendDuffs + currentFee) {
      return { fee: currentFee, inputCount: selectedCount, txSize: currentSize };
    }
  }

  // Not enough funds — show estimate with all UTXOs
  const txSize = estimateP2pkhTxSize(selectedCount, outputCount);
  const fee = txSize * FEE_RATE_DUFFS_PER_BYTE;
  return { fee, inputCount: selectedCount, txSize };
}

// ─── Fee error parsing ─────────────────────────────────────────────

/**
 * Parse a "min relay fee not met" error to extract the required fee.
 * Matches error format: "min relay fee not met, 226 < 1000"
 * Returns the required fee (the number after '<') or null if not a fee error.
 */
export function parseMinRelayFeeError(error: string): number | null {
  if (!error.includes("min relay fee")) return null;
  const ltPos = error.indexOf("<");
  if (ltPos === -1) return null;
  const afterLt = error.substring(ltPos + 1).trim();
  const digits = afterLt.match(/^(\d+)/);
  if (!digits) return null;
  const requiredFee = parseInt(digits[1], 10);
  return isNaN(requiredFee) ? null : requiredFee;
}

// ─── SingleKeySendScreen ────────────────────────────────────────────

export function SingleKeySendScreen() {
  const navigate = useNavigate();

  // Get wallet data from store
  const { singleKeyWallets, selectedWallet } = useWalletStore();

  const wallet: SingleKeyWalletDto | null =
    selectedWallet?.type === "singleKey"
      ? (singleKeyWallets.find(
          (w) => w.keyHash === selectedWallet.keyHash,
        ) ?? null)
      : null;

  // Recipients state
  const [recipients, setRecipients] = useState<Recipient[]>([
    { id: 0, address: "", amount: "", error: null },
  ]);
  const [nextId, setNextId] = useState(1);

  // Options state
  const [subtractFee, setSubtractFee] = useState(false);
  const [memo, setMemo] = useState("");
  const [showAdvanced, setShowAdvanced] = useState(false);

  // Send state
  const [sendStatus, setSendStatus] = useState<SendStatus>({ state: "idle" });

  // Wallet unlock state
  const [unlockOpen, setUnlockOpen] = useState(false);
  const [unlockError, setUnlockError] = useState<string | null>(null);
  const [walletUnlocked, setWalletUnlocked] = useState(false);

  // Fee confirmation dialog state
  const [feeDialogOpen, setFeeDialogOpen] = useState(false);
  const [feeDialogEstimated, setFeeDialogEstimated] = useState(0);
  const [feeDialogRequired, setFeeDialogRequired] = useState(0);
  // Stores the pending payment params for retry with override fee
  const pendingRequestRef = useRef<{
    keyHash: string;
    recipients: { address: string; amount: number }[];
    subtractFeeFromAmount: boolean;
    memo: string | null;
  } | null>(null);

  // Elapsed time
  const [elapsed, setElapsed] = useState(0);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Auto-unlock if wallet has no password
  useEffect(() => {
    if (!wallet) return;
    if (!wallet.usesPassword) {
      setWalletUnlocked(true);
    }
  }, [wallet]);

  // Elapsed time timer
  const sendingStartTime =
    sendStatus.state === "sending" ? sendStatus.startTime : 0;
  useEffect(() => {
    if (sendStatus.state === "sending") {
      setElapsed(0);
      timerRef.current = setInterval(() => {
        setElapsed(Math.floor((Date.now() - sendingStartTime) / 1000));
      }, 1000);
    } else {
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
    }
    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, [sendStatus.state, sendingStartTime]);

  // Listen for task result/error events
  useEffect(() => {
    let resultCleanup: (() => void) | undefined;
    let errorCleanup: (() => void) | undefined;

    events.taskResultEvent
      .listen((event) => {
        if (
          sendStatus.state === "sending" &&
          sendStatus.taskId &&
          event.payload.taskId === sendStatus.taskId
        ) {
          pendingRequestRef.current = null;
          const message = event.payload.message || "Transaction completed successfully!";
          setSendStatus({ state: "complete", message });
        }
      })
      .then((unsub) => {
        resultCleanup = unsub;
      })
      .catch(() => {});

    events.taskErrorEvent
      .listen((event) => {
        if (
          sendStatus.state === "sending" &&
          sendStatus.taskId &&
          event.payload.taskId === sendStatus.taskId
        ) {
          // Check for min relay fee error — show fee confirmation dialog instead of error
          const requiredFee = parseMinRelayFeeError(event.payload.message);
          if (requiredFee !== null && pendingRequestRef.current) {
            // Extract estimated fee from the error (the number before '<')
            const match = event.payload.message.match(/(\d+)\s*</);
            const estimatedFee = match ? parseInt(match[1], 10) : 0;
            setFeeDialogEstimated(estimatedFee);
            setFeeDialogRequired(requiredFee);
            setFeeDialogOpen(true);
            // Keep sending state — user must confirm or cancel
            return;
          }
          setSendStatus({
            state: "error",
            message: event.payload.message,
          });
        }
      })
      .then((unsub) => {
        errorCleanup = unsub;
      })
      .catch(() => {});

    return () => {
      resultCleanup?.();
      errorCleanup?.();
    };
  }, [sendStatus]);

  // ─── Derived values ─────────────────────────────────────────────

  const balance = wallet ? (wallet.totalBalance ?? 0) : 0;

  // Check if any recipient has validation errors
  const hasAddressErrors = recipients.some((r) => r.error !== null);

  // Can send?
  const canSend = useMemo(() => {
    if (sendStatus.state === "sending") return false;
    if (!walletUnlocked) return false;
    if (hasAddressErrors) return false;

    // At least one recipient with address and amount
    const hasValidRecipient = recipients.some(
      (r) => r.address.trim() && r.amount.trim() && parseFloat(r.amount) > 0,
    );
    return hasValidRecipient;
  }, [sendStatus.state, walletUnlocked, hasAddressErrors, recipients]);

  // Transaction size estimation (updates reactively as recipients change)
  const txEstimation = useMemo(() => {
    if (!wallet) return null;
    const utxos = (wallet.utxos ?? []).map((u) => ({ amount: u.amount }));
    const totalDuffs = recipients.reduce((sum, r) => {
      const d = duffsFromDashString(r.amount);
      return sum + (d && d > 0 ? d : 0);
    }, 0);
    const recipientCount = Math.max(recipients.length, 1);
    return estimateFee(utxos, totalDuffs, recipientCount);
  }, [wallet, recipients]);

  // ─── Handlers ─────────────────────────────────────────────────────

  const handleBack = useCallback(() => {
    navigate({ to: "/wallets" });
  }, [navigate]);

  const updateRecipientAddress = useCallback(
    (id: number, address: string) => {
      setRecipients((prev) =>
        prev.map((r) =>
          r.id === id
            ? { ...r, address, error: validateRecipientAddress(address) }
            : r,
        ),
      );
    },
    [],
  );

  const updateRecipientAmount = useCallback(
    (id: number, amount: string) => {
      // Only allow digits and decimal point
      if (amount && !/^\d*\.?\d*$/.test(amount)) return;
      setRecipients((prev) =>
        prev.map((r) => (r.id === id ? { ...r, amount } : r)),
      );
    },
    [],
  );

  const addRecipient = useCallback(() => {
    setRecipients((prev) => [
      ...prev,
      { id: nextId, address: "", amount: "", error: null },
    ]);
    setNextId((n) => n + 1);
  }, [nextId]);

  const removeRecipient = useCallback((id: number) => {
    setRecipients((prev) => {
      if (prev.length <= 1) return prev;
      return prev.filter((r) => r.id !== id);
    });
  }, []);

  const handleMaxClick = useCallback(() => {
    if (balance <= 0 || recipients.length === 0) return;
    const duffs = balance;
    const dash = duffs / DUFFS_PER_DASH;
    // Set max to first recipient
    setRecipients((prev) =>
      prev.map((r, i) =>
        i === 0 ? { ...r, amount: dash.toFixed(8) } : r,
      ),
    );
    setSubtractFee(true);
  }, [balance, recipients.length]);

  const resetForm = useCallback(() => {
    setRecipients([{ id: 0, address: "", amount: "", error: null }]);
    setNextId(1);
    setSubtractFee(false);
    setMemo("");
    setSendStatus({ state: "idle" });
  }, []);

  const handleUnlockResult = useCallback(
    async (result: { status: "unlocked"; password: string } | { status: "cancelled" }) => {
      if (result.status === "unlocked" && wallet) {
        setUnlockError(null);
        try {
          await commands.walletNotifyUnlocked(wallet.keyHash);
          setWalletUnlocked(true);
        } catch (e) {
          setUnlockError(e instanceof Error ? e.message : String(e));
          return;
        }
      }
      setUnlockOpen(false);
    },
    [wallet],
  );

  // ─── Send ─────────────────────────────────────────────────────────

  const handleSend = useCallback(async () => {
    if (!wallet || !canSend) return;

    // Validate all recipients
    const errors: string[] = [];
    const paymentRecipients: { address: string; amount: number }[] = [];
    let totalDuffs = 0;

    for (let i = 0; i < recipients.length; i++) {
      const r = recipients[i];
      if (!r.address.trim()) {
        errors.push(`Recipient ${i + 1} has empty address`);
        continue;
      }
      const addrError = validateRecipientAddress(r.address);
      if (addrError) {
        errors.push(`Recipient ${i + 1}: ${addrError}`);
        continue;
      }
      const duffs = duffsFromDashString(r.amount);
      if (!duffs || duffs <= 0) {
        errors.push(`Recipient ${i + 1} has invalid or zero amount`);
        continue;
      }
      paymentRecipients.push({ address: r.address.trim(), amount: duffs });
      totalDuffs += duffs;
    }

    if (errors.length > 0) {
      setSendStatus({ state: "error", message: errors.join("\n") });
      return;
    }

    if (paymentRecipients.length === 0) {
      setSendStatus({ state: "error", message: "No valid recipients" });
      return;
    }

    // Balance check
    if (totalDuffs > balance && !subtractFee) {
      setSendStatus({
        state: "error",
        message: `Insufficient balance. Need ${formatDash(totalDuffs)} but have ${formatDash(balance)}`,
      });
      return;
    }

    // Store the request for potential fee override retry
    const requestParams = {
      keyHash: wallet.keyHash,
      recipients: paymentRecipients,
      subtractFeeFromAmount: subtractFee,
      memo: memo.trim() || null,
    };
    pendingRequestRef.current = requestParams;

    try {
      const result = await commands.coreSendSingleKeyWalletPayment({
        ...requestParams,
        overrideFee: null,
      });

      if (result.status === "error") throw new Error(result.error);

      setSendStatus({
        state: "sending",
        startTime: Date.now(),
        taskId: result.data.taskId,
      });
    } catch (e) {
      setSendStatus({
        state: "error",
        message: e instanceof Error ? e.message : String(e),
      });
    }
  }, [wallet, canSend, recipients, balance, subtractFee, memo]);

  // ─── Fee confirmation ──────────────────────────────────────────

  const handleFeeConfirmResult = useCallback(
    async (result: FeeConfirmationResult) => {
      if (result.status === "confirmed" && pendingRequestRef.current) {
        // Retry send with the override fee
        try {
          const sendResult = await commands.coreSendSingleKeyWalletPayment({
            ...pendingRequestRef.current,
            overrideFee: result.overrideFee,
          });

          if (sendResult.status === "error") throw new Error(sendResult.error);

          setSendStatus({
            state: "sending",
            startTime: Date.now(),
            taskId: sendResult.data.taskId,
          });
        } catch (e) {
          setSendStatus({
            state: "error",
            message: e instanceof Error ? e.message : String(e),
          });
        }
      } else {
        // User canceled — abort the transaction
        pendingRequestRef.current = null;
        setSendStatus({ state: "idle" });
      }
    },
    [],
  );

  // ─── No wallet → redirect ─────────────────────────────────────

  if (!wallet) {
    return (
      <div className="flex flex-1 items-center justify-center">
        <div className="text-center">
          <p className="text-muted-foreground">No single-key wallet selected</p>
          <Button variant="outline" className="mt-4" onClick={handleBack}>
            Back to Wallets
          </Button>
        </div>
      </div>
    );
  }

  const walletAlias = wallet.alias?.trim() || "Unnamed Key";

  // ─── Sending state ─────────────────────────────────────────────

  if (sendStatus.state === "sending") {
    const formatElapsed = (secs: number) => {
      if (secs < 60) return `${secs} second${secs === 1 ? "" : "s"}`;
      const min = Math.floor(secs / 60);
      const sec = secs % 60;
      return `${min} minute${min === 1 ? "" : "s"} ${sec} second${sec === 1 ? "" : "s"}`;
    };

    return (
      <Island className="flex flex-1 items-center justify-center">
        <div className="text-center space-y-6">
          <Loader2 className="h-12 w-12 animate-spin text-primary mx-auto" />
          <h2 className="text-2xl font-semibold">Sending...</h2>
          <p className="text-muted-foreground">
            Time elapsed: {formatElapsed(elapsed)}
          </p>
        </div>

        {/* Fee confirmation dialog — may appear during sending state */}
        <FeeConfirmationDialog
          open={feeDialogOpen}
          onOpenChange={setFeeDialogOpen}
          estimatedFee={feeDialogEstimated}
          requiredFee={feeDialogRequired}
          unit="duffs"
          onResult={handleFeeConfirmResult}
        />
      </Island>
    );
  }

  // ─── Complete state ────────────────────────────────────────────

  if (sendStatus.state === "complete") {
    return (
      <Island className="flex flex-1 items-center justify-center">
        <div className="text-center space-y-6">
          <CheckCircle2 className="h-16 w-16 text-success mx-auto" />
          <h2 className="text-2xl font-semibold">Transaction Sent</h2>
          <p className="text-muted-foreground max-w-md whitespace-pre-line">
            {sendStatus.message}
          </p>
          <div className="flex gap-3 justify-center">
            <Button variant="outline" onClick={resetForm}>
              Send Another
            </Button>
            <Button onClick={handleBack}>Back to Wallet</Button>
          </div>
        </div>
      </Island>
    );
  }

  // ─── Main form ─────────────────────────────────────────────────

  return (
    <Island className="flex flex-col flex-1 overflow-auto">
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center gap-3">
          <Button
            variant="ghost"
            size="icon"
            onClick={handleBack}
            aria-label="Back to wallets"
          >
            <ArrowLeft className="h-5 w-5" />
          </Button>
          <h1 className="text-2xl font-semibold">Send Dash</h1>
        </div>
        <label className="flex items-center gap-2 text-sm text-muted-foreground cursor-pointer">
          <Checkbox
            checked={showAdvanced}
            onCheckedChange={(checked) => setShowAdvanced(checked === true)}
          />
          Advanced Options
        </label>
      </div>

      {/* Error banner */}
      {sendStatus.state === "error" && (
        <div
          className="flex items-start gap-3 rounded-md border border-destructive/50 bg-destructive/10 p-3 mb-4"
          role="alert"
        >
          <AlertCircle className="h-5 w-5 text-destructive shrink-0 mt-0.5" />
          <p className="text-sm text-destructive flex-1 whitespace-pre-line">
            {sendStatus.message}
          </p>
          <Button
            variant="ghost"
            size="icon"
            className="h-6 w-6 shrink-0"
            onClick={() => setSendStatus({ state: "idle" })}
            aria-label="Dismiss error"
          >
            <X className="h-4 w-4" />
          </Button>
        </div>
      )}

      <div className="space-y-6">
        {/* Wallet info */}
        <div className="rounded-md border bg-muted/30 p-4 space-y-1">
          <div className="text-sm text-muted-foreground">
            Wallet: <span className="font-medium text-foreground">{walletAlias}</span>
          </div>
          <div className="text-sm text-muted-foreground">
            Address: <code className="text-xs text-foreground">{wallet.address}</code>
          </div>
          <div className="text-sm">
            Available: <span className="font-medium text-success">{formatDash(balance)}</span>
          </div>
        </div>

        {/* Wallet unlock gate */}
        {!walletUnlocked && wallet.usesPassword && (
          <div className="space-y-3">
            <p className="text-sm text-warning">
              Wallet is locked. Please unlock to continue.
            </p>
            <Button variant="outline" onClick={() => setUnlockOpen(true)}>
              Unlock Wallet
            </Button>
          </div>
        )}

        {walletUnlocked && (
          <>
            {/* Recipients */}
            {showAdvanced ? (
              <div className="space-y-4">
                <Label className="text-sm font-semibold">Recipients</Label>
                {recipients.map((r, idx) => (
                  <div key={r.id} className="rounded-md border p-3 space-y-2">
                    <div className="flex items-center gap-2">
                      <Label className="text-xs shrink-0 w-16">
                        Address:
                      </Label>
                      <Input
                        value={r.address}
                        onChange={(e) =>
                          updateRecipientAddress(r.id, e.target.value)
                        }
                        placeholder="Enter Dash address (e.g., y...)"
                        className="flex-1 h-8 text-sm"
                        aria-label={`Recipient ${idx + 1} address`}
                      />
                      {recipients.length > 1 && (
                        <Button
                          variant="ghost"
                          size="icon"
                          className="h-6 w-6 shrink-0"
                          onClick={() => removeRecipient(r.id)}
                          aria-label={`Remove recipient ${idx + 1}`}
                        >
                          <Trash2 className="h-3 w-3" />
                        </Button>
                      )}
                    </div>
                    {r.error && (
                      <p className="text-xs text-destructive ml-[4.5rem]">{r.error}</p>
                    )}
                    <div className="flex items-center gap-2">
                      <Label className="text-xs shrink-0 w-16">Amount:</Label>
                      <Input
                        type="text"
                        inputMode="decimal"
                        value={r.amount}
                        onChange={(e) =>
                          updateRecipientAmount(r.id, e.target.value)
                        }
                        placeholder="0.01"
                        className="w-32 h-8 text-sm"
                        aria-label={`Recipient ${idx + 1} amount`}
                      />
                      <span className="text-xs text-muted-foreground">
                        DASH
                      </span>
                    </div>
                  </div>
                ))}

                <Button
                  variant="outline"
                  size="sm"
                  onClick={addRecipient}
                >
                  <Plus className="h-3 w-3 mr-1" /> Add Recipient
                </Button>

                {/* Memo */}
                <div className="space-y-2">
                  <Label className="text-xs">Memo (optional)</Label>
                  <Input
                    value={memo}
                    onChange={(e) => setMemo(e.target.value)}
                    placeholder="Add a note..."
                    className="max-w-[300px] h-8 text-sm"
                    aria-label="Transaction memo"
                  />
                </div>

                {/* Subtract fee */}
                <label className="flex items-center gap-2 text-sm cursor-pointer">
                  <Checkbox
                    checked={subtractFee}
                    onCheckedChange={(checked) =>
                      setSubtractFee(checked === true)
                    }
                  />
                  Subtract fee from amount
                </label>
              </div>
            ) : (
              // Simple mode — single recipient
              <div className="space-y-4">
                <div className="space-y-2">
                  <Label className="text-sm font-semibold">Send to</Label>
                  <Input
                    value={recipients[0]?.address ?? ""}
                    onChange={(e) =>
                      updateRecipientAddress(
                        recipients[0]?.id ?? 0,
                        e.target.value,
                      )
                    }
                    placeholder="Enter Dash address (e.g., y...)"
                    aria-label="Destination address"
                  />
                  {recipients[0]?.error && (
                    <p className="text-xs text-destructive">{recipients[0].error}</p>
                  )}
                </div>

                <AmountInput
                  value={recipients[0]?.amount ?? ""}
                  onChange={(v) =>
                    updateRecipientAmount(recipients[0]?.id ?? 0, v)
                  }
                  label="Amount"
                  placeholder="Enter amount"
                  decimalPlaces={8}
                  unitName="DASH"
                  maxAmount={balance > 0 ? balance : null}
                  showMaxButton
                  onMaxClick={handleMaxClick}
                  showValidationErrors
                />

                {/* Subtract fee */}
                <label className="flex items-center gap-2 text-sm cursor-pointer">
                  <Checkbox
                    checked={subtractFee}
                    onCheckedChange={(checked) =>
                      setSubtractFee(checked === true)
                    }
                  />
                  Subtract fee from amount
                  {subtractFee && (
                    <span className="text-xs text-muted-foreground italic">
                      (recipient receives amount minus fee)
                    </span>
                  )}
                </label>
              </div>
            )}

            {/* Transaction estimation */}
            {txEstimation && (
              <div className="rounded-md border bg-muted/30 p-3 space-y-1">
                <div className="text-sm text-muted-foreground">
                  Estimated fee:{" "}
                  <span className="font-medium text-foreground">
                    {txEstimation.fee} ({(txEstimation.fee / DUFFS_PER_DASH).toFixed(8)} DASH)
                  </span>
                </div>
                <div className="text-xs text-muted-foreground">
                  Transaction details: {txEstimation.inputCount} input{txEstimation.inputCount !== 1 ? "s" : ""}, ~{txEstimation.txSize} bytes
                </div>
                {txEstimation.inputCount > 100 && (
                  <div className="flex items-center gap-1.5 mt-1">
                    <AlertTriangle className="h-3.5 w-3.5 text-warning shrink-0" />
                    <span className="text-xs text-warning">
                      Large number of inputs may require higher network fee
                    </span>
                  </div>
                )}
              </div>
            )}

            {/* Send/Cancel buttons */}
            <div className="flex gap-3 pt-2">
              <Button variant="outline" onClick={handleBack}>
                Cancel
              </Button>
              <Button
                onClick={handleSend}
                disabled={!canSend}
                className="min-w-[160px]"
              >
                <Send className="h-4 w-4 mr-2" />
                Send
              </Button>
            </div>
          </>
        )}
      </div>

      {/* Wallet unlock dialog */}
      <WalletUnlockDialog
        open={unlockOpen}
        onOpenChange={setUnlockOpen}
        walletAlias={walletAlias}
        passwordHint={null}
        error={unlockError}
        onResult={handleUnlockResult}
      />

      {/* Fee confirmation dialog */}
      <FeeConfirmationDialog
        open={feeDialogOpen}
        onOpenChange={setFeeDialogOpen}
        estimatedFee={feeDialogEstimated}
        requiredFee={feeDialogRequired}
        unit="duffs"
        onResult={handleFeeConfirmResult}
      />
    </Island>
  );
}
