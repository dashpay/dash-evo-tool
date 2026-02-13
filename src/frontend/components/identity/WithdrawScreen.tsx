import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useElapsedTimer } from "@/hooks/useElapsedTimer";
import {
  ArrowLeft,
  ArrowUpFromLine,
  ChevronDown,
  ChevronUp,
  AlertCircle,
  CheckCircle2,
  Loader2,
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
import {
  AmountInput,
  useAmountInput,
  formatAmount,
  formatCreditsAsDash,
  CREDITS_PER_DUFF,
} from "@/components/shared/AmountInput";
import {
  ConfirmationDialog,
} from "@/components/shared/ConfirmationDialog";
import { InlineError } from "@/components/feedback/InlineError";
import type { QualifiedIdentityDto, IdentityKeyDto } from "@/bindings";
import { validateCoreAddress } from "@/lib/validateAddress";

// ─── Constants ─────────────────────────────────────────────────────

const DASH_DECIMAL_PLACES = 8;
/** Reserve 0.005 DASH (500M credits) for withdrawal fees. */
const WITHDRAWAL_FEE_RESERVE = 500_000_000;

// ─── Types ─────────────────────────────────────────────────────────

export type WithdrawStatus =
  | { type: "form" }
  | { type: "sending"; startedAt: number }
  | { type: "error"; message: string }
  | { type: "success" };

export interface WithdrawScreenProps {
  /** The identity to withdraw from. */
  identity: QualifiedIdentityDto;
  /** Current status of the withdrawal operation. */
  status: WithdrawStatus;
  /** Called to submit the withdrawal request. */
  onSubmit?: (params: {
    identityId: string;
    toAddress: string | null;
    credits: number;
    keyId: number | null;
  }) => void;
  /** Called to dismiss an error and return to form. */
  onDismissError?: () => void;
  /** Called to navigate back after success. */
  onBack?: () => void;
  /** Called to navigate to key info screen. */
  onViewKey?: (keyId: number) => void;
  /** Called to navigate to add key screen. */
  onAddKey?: () => void;
  /** Estimated fee in credits (from backend). */
  estimatedFee?: number | null;
  /** Whether the associated wallet is locked and needs unlock before proceeding. */
  walletLocked?: boolean;
  /** Called when user wants to unlock the wallet. */
  onRequestUnlock?: () => void;
}

// ─── Helpers ───────────────────────────────────────────────────────

function getTransferKeys(identity: QualifiedIdentityDto): IdentityKeyDto[] {
  return identity.keys.filter(
    (k) =>
      k.purpose.toUpperCase() === "TRANSFER" &&
      k.hasPrivateKey &&
      !k.isDisabled,
  );
}

function getOwnerKeys(identity: QualifiedIdentityDto): IdentityKeyDto[] {
  return identity.keys.filter(
    (k) =>
      k.purpose.toUpperCase() === "OWNER" &&
      k.hasPrivateKey &&
      !k.isDisabled,
  );
}

function getWithdrawalKeys(identity: QualifiedIdentityDto): IdentityKeyDto[] {
  return [...getTransferKeys(identity), ...getOwnerKeys(identity)];
}

function getKeyLabel(key: IdentityKeyDto, identityType: string): string {
  const purposeLabel =
    key.purpose.toUpperCase() === "OWNER"
      ? "Owner"
      : identityType === "user"
        ? "Transfer"
        : "Payout";
  return `#${key.keyId} — ${purposeLabel} (${key.securityLevel})`;
}

function getIdentityTypeLabel(type: string): string {
  switch (type) {
    case "user":
      return "User";
    case "masternode":
      return "Masternode";
    case "evonode":
      return "Evonode";
    default:
      return type;
  }
}

// ─── Component ─────────────────────────────────────────────────────

export function WithdrawScreen({
  identity,
  status,
  onSubmit,
  onDismissError,
  onBack,
  onViewKey,
  onAddKey,
  estimatedFee,
  walletLocked = false,
  onRequestUnlock,
}: WithdrawScreenProps) {
  // ─── Elapsed timer ──────────────────────────────────────────────
  const sendingStartedAt = status.type === "sending" ? status.startedAt : null;
  const elapsed = useElapsedTimer(sendingStartedAt);

  const [showAdvanced, setShowAdvanced] = useState(false);
  const [address, setAddress] = useState("");
  const [addressError, setAddressError] = useState<string | null>(null);
  const [selectedKeyId, setSelectedKeyId] = useState<number | null>(() => {
    const keys = getWithdrawalKeys(identity);
    return keys.length > 0 ? keys[0]?.keyId ?? null : null;
  });
  const [showConfirmation, setShowConfirmation] = useState(false);

  const amount = useAmountInput(DASH_DECIMAL_PLACES);

  const withdrawalKeys = useMemo(() => getWithdrawalKeys(identity), [identity]);
  const selectedKey = useMemo(
    () => withdrawalKeys.find((k) => k.keyId === selectedKeyId) ?? null,
    [withdrawalKeys, selectedKeyId],
  );
  const isOwnerKey = selectedKey?.purpose.toUpperCase() === "OWNER";

  // maxAmount in duffs (AmountInput works in duffs with 8 decimal places)
  const maxAmount = useMemo(() => {
    const availableCredits = identity.balance - WITHDRAWAL_FEE_RESERVE;
    return availableCredits > 0 ? Math.floor(availableCredits / CREDITS_PER_DUFF) : 0;
  }, [identity.balance]);

  const balanceFormatted = formatCreditsAsDash(identity.balance);

  const isDisabled = status.type === "sending" || status.type === "success" || walletLocked;

  const handleMaxClick = useCallback(() => {
    if (maxAmount > 0) {
      amount.setValue(formatAmount(maxAmount, DASH_DECIMAL_PLACES));
    }
  }, [maxAmount, amount]);

  // Debounced address validation via Tauri backend
  const addressValidationTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const handleAddressChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const value = e.target.value;
      setAddress(value);
      setAddressError(null);

      if (addressValidationTimer.current) {
        clearTimeout(addressValidationTimer.current);
      }

      const trimmed = value.trim();
      if (!trimmed) return;

      addressValidationTimer.current = setTimeout(async () => {
        const error = await validateCoreAddress(trimmed);
        // Only set error if the address hasn't changed since we started validating
        setAddress((current) => {
          if (current.trim() === trimmed && error) {
            setAddressError(error);
          }
          return current;
        });
      }, 300);
    },
    [],
  );

  // Cleanup timer on unmount
  useEffect(() => {
    return () => {
      if (addressValidationTimer.current) {
        clearTimeout(addressValidationTimer.current);
      }
    };
  }, []);

  const isReady = useMemo(() => {
    if (!amount.parsedAmount || amount.parsedAmount <= 0) return false;
    if (!selectedKey) return false;
    if (amount.parsedAmount > maxAmount) return false;
    // Owner key can have empty address (uses payout address)
    if (!isOwnerKey && !address.trim()) return false;
    if (addressError) return false;
    return true;
  }, [amount.parsedAmount, selectedKey, maxAmount, isOwnerKey, address, addressError]);

  const handleWithdraw = useCallback(() => {
    if (!isReady) return;
    setShowConfirmation(true);
  }, [isReady]);

  const handleConfirm = useCallback(() => {
    if (!amount.parsedAmount || !selectedKey) return;
    const toAddress = address.trim() || null;
    onSubmit?.({
      identityId: identity.id,
      toAddress,
      credits: amount.parsedAmount * CREDITS_PER_DUFF,
      keyId: selectedKey.keyId,
    });
  }, [amount.parsedAmount, selectedKey, address, identity.id, onSubmit]);

  const confirmMessage = useMemo(() => {
    if (!amount.parsedAmount) return "";
    const amountStr = formatAmount(amount.parsedAmount, DASH_DECIMAL_PLACES);
    const dest = address.trim()
      ? address.trim()
      : identity.masternodePayoutAddress ?? "masternode payout address";
    return `Are you sure you want to withdraw ${amountStr} DASH to ${dest}?`;
  }, [amount.parsedAmount, address, identity.masternodePayoutAddress]);

  // ─── Success screen ────────────────────────────────────────────

  if (status.type === "success") {
    return (
      <div className="flex flex-col h-full" data-testid="withdraw-screen">
        <Header onBack={onBack} />
        <Separator />
        <div className="flex-1 flex flex-col items-center justify-center gap-4 p-8">
          <CheckCircle2 className="h-12 w-12 text-green-500" />
          <div className="text-center space-y-2">
            <h2 className="text-xl font-semibold">Withdrawal Successful!</h2>
            <p className="text-sm text-muted-foreground max-w-md">
              Note: It may take a few minutes for funds to appear on the Core
              chain.
            </p>
          </div>
          <Button onClick={onBack} className="mt-4">
            Back to Identities
          </Button>
        </div>
      </div>
    );
  }

  // ─── No keys available ─────────────────────────────────────────

  if (withdrawalKeys.length === 0) {
    const typeLabel = getIdentityTypeLabel(identity.identityType);
    const keyTypeLabel =
      identity.identityType === "user" ? "Transfer" : "Payout Address";

    return (
      <div className="flex flex-col h-full" data-testid="withdraw-screen">
        <Header onBack={onBack} />
        <Separator />
        <div className="flex-1 flex flex-col items-center justify-center gap-4 p-8">
          <AlertCircle className="h-10 w-10 text-muted-foreground" />
          <p className="text-sm text-muted-foreground text-center max-w-md">
            You do not have any withdrawal keys loaded for this {typeLabel}{" "}
            identity.
          </p>
          <div className="flex items-center gap-2">
            {identity.keys.find(
              (k) => k.purpose.toUpperCase() === "OWNER",
            ) && (
              <Button
                variant="outline"
                size="sm"
                onClick={() => {
                  const ownerKey = identity.keys.find(
                    (k) => k.purpose.toUpperCase() === "OWNER",
                  );
                  if (ownerKey) onViewKey?.(ownerKey.keyId);
                }}
              >
                Check Owner Key
              </Button>
            )}
            {identity.keys.find(
              (k) => k.purpose.toUpperCase() === "TRANSFER",
            ) && (
              <Button
                variant="outline"
                size="sm"
                onClick={() => {
                  const transferKey = identity.keys.find(
                    (k) => k.purpose.toUpperCase() === "TRANSFER",
                  );
                  if (transferKey) onViewKey?.(transferKey.keyId);
                }}
              >
                Check {keyTypeLabel} Key
              </Button>
            )}
            <Button variant="outline" size="sm" onClick={onAddKey}>
              Add key
            </Button>
          </div>
        </div>
      </div>
    );
  }

  // ─── Main form ─────────────────────────────────────────────────

  return (
    <div className="flex flex-col h-full" data-testid="withdraw-screen">
      <Header
        onBack={onBack}
        showAdvanced={showAdvanced}
        onToggleAdvanced={() => setShowAdvanced((v) => !v)}
      />
      <Separator />

      <div className="flex-1 overflow-y-auto p-6 space-y-6">
        {/* ── Wallet locked gate ────────────────────────────── */}
        {walletLocked && (
          <div className="flex items-center gap-3 rounded-md border border-amber-500/30 bg-amber-50 dark:bg-amber-900/20 px-4 py-3" data-testid="wallet-locked-gate">
            <Lock className="h-4 w-4 text-amber-600 dark:text-amber-400 shrink-0" />
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

        {/* ── Step 1: Amount ──────────────────────────────── */}
        <section>
          <h3 className="text-sm font-semibold mb-3" data-testid="step-1-heading">
            1. Amount to withdraw (DASH)
          </h3>
          <div className="space-y-2 pl-4">
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <span>
                From:{" "}
                <span className="font-medium text-foreground">
                  {identity.alias || identity.id.slice(0, 8) + "…"}
                </span>
              </span>
              <Badge variant="outline" className="text-xs">
                {getIdentityTypeLabel(identity.identityType)}
              </Badge>
            </div>
            <p className="text-sm text-muted-foreground">
              Available Balance:{" "}
              <span className="font-medium text-foreground tabular-nums">
                {balanceFormatted} DASH
              </span>
            </p>

            <AmountInput
              value={amount.value}
              onChange={amount.setValue}
              label="Amount:"
              placeholder="0.00000000"
              decimalPlaces={DASH_DECIMAL_PLACES}
              maxAmount={maxAmount}
              maxExceededHint="0.005 DASH reserved for fees"
              showMaxButton
              onMaxClick={handleMaxClick}
              disabled={isDisabled}
            />
          </div>
        </section>

        {/* ── Step 2: Address ─────────────────────────────── */}
        <section>
          <h3 className="text-sm font-semibold mb-3" data-testid="step-2-heading">
            2. Dash address to withdraw to
          </h3>
          <div className="space-y-2 pl-4">
            {isOwnerKey && !address.trim() ? (
              <p className="text-sm text-muted-foreground">
                {identity.masternodePayoutAddress
                  ? `Using masternode payout address: ${identity.masternodePayoutAddress}`
                  : "No masternode payout address available"}{" "}
                (leave address empty or enter a custom address).
              </p>
            ) : null}
            <div className="space-y-1.5">
              <Label htmlFor="withdraw-address" className="text-sm font-medium">
                Address:
              </Label>
              <Input
                id="withdraw-address"
                type="text"
                placeholder="Enter Dash address"
                value={address}
                onChange={handleAddressChange}
                disabled={isDisabled}
                aria-invalid={!!addressError}
                aria-describedby={addressError ? "address-error" : undefined}
              />
              {addressError && (
                <p
                  id="address-error"
                  className="text-sm text-destructive"
                  role="alert"
                >
                  {addressError}
                </p>
              )}
            </div>
          </div>
        </section>

        {/* ── Step 3: Key selector (advanced) ─────────────── */}
        {showAdvanced && (
          <section>
            <h3
              className="text-sm font-semibold mb-3"
              data-testid="step-3-heading"
            >
              3. Select the key to sign with
            </h3>
            <div className="pl-4">
              <Select
                value={selectedKeyId?.toString() ?? ""}
                onValueChange={(v) => setSelectedKeyId(Number(v))}
                disabled={isDisabled}
              >
                <SelectTrigger
                  className="w-[320px]"
                  aria-label="Signing key"
                >
                  <SelectValue placeholder="Select signing key" />
                </SelectTrigger>
                <SelectContent>
                  {withdrawalKeys.map((k) => (
                    <SelectItem key={k.keyId} value={k.keyId.toString()}>
                      {getKeyLabel(k, identity.identityType)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </section>
        )}

        {/* ── Fee estimation ──────────────────────────────── */}
        {estimatedFee != null && (
          <div className="rounded-md border border-border bg-muted/30 px-4 py-3">
            <p className="text-sm text-muted-foreground">
              Estimated fee:{" "}
              <span className="font-medium tabular-nums">
                {formatCreditsAsDash(estimatedFee)} DASH
              </span>
            </p>
          </div>
        )}

        {/* ── Error display ───────────────────────────────── */}
        {status.type === "error" && (
          <InlineError message={status.message} onDismiss={onDismissError} />
        )}

        {/* ── Sending state ───────────────────────────────── */}
        {status.type === "sending" && (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            <span>
              Withdrawing… ({elapsed})
            </span>
          </div>
        )}

        {/* ── Withdraw button ─────────────────────────────── */}
        <div className="pt-2">
          <Button
            onClick={handleWithdraw}
            disabled={!isReady || isDisabled}
            className="w-full sm:w-auto"
          >
            <ArrowUpFromLine className="h-4 w-4 mr-1.5" />
            Withdraw
          </Button>
        </div>
      </div>

      {/* ── Confirmation dialog ────────────────────────── */}
      <ConfirmationDialog
        open={showConfirmation}
        onOpenChange={setShowConfirmation}
        title="Confirm Withdrawal"
        message={confirmMessage}
        danger
        onResult={(result) => {
          if (result === "confirmed") {
            handleConfirm();
          }
        }}
      />
    </div>
  );
}

// ─── Header Sub-component ──────────────────────────────────────────

interface HeaderProps {
  onBack?: () => void;
  showAdvanced?: boolean;
  onToggleAdvanced?: () => void;
}

function Header({ onBack, showAdvanced, onToggleAdvanced }: HeaderProps) {
  return (
    <div className="flex items-center justify-between px-5 py-3">
      <div className="flex items-center gap-2">
        <Button
          variant="ghost"
          size="icon-sm"
          onClick={onBack}
          aria-label="Go back"
        >
          <ArrowLeft className="h-4 w-4" />
        </Button>
        <h2 className="text-lg font-semibold">Withdraw Funds</h2>
      </div>
      {onToggleAdvanced !== undefined && (
        <Button
          variant="ghost"
          size="sm"
          onClick={onToggleAdvanced}
          className="text-xs text-muted-foreground"
        >
          {showAdvanced ? "Hide" : "Show"} Advanced Options
          {showAdvanced ? (
            <ChevronUp className="h-3 w-3 ml-1" />
          ) : (
            <ChevronDown className="h-3 w-3 ml-1" />
          )}
        </Button>
      )}
    </div>
  );
}
