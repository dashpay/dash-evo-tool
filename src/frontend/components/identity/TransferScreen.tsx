import { useCallback, useMemo, useState } from "react";
import { useElapsedTimer } from "@/hooks/useElapsedTimer";
import {
  ArrowLeft,
  ArrowLeftRight,
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
import {
  IdentitySelector,
  type IdentityOption,
} from "@/components/shared/IdentitySelector";
import { InlineError } from "@/components/feedback/InlineError";
import type { QualifiedIdentityDto, IdentityKeyDto } from "@/bindings";

// ─── Constants ─────────────────────────────────────────────────────

const DASH_DECIMAL_PLACES = 8;
/** Reserve 0.0002 DASH (20M credits) for transfer fees. */
const TRANSFER_FEE_RESERVE = 20_000_000;

// ─── Types ─────────────────────────────────────────────────────────

export type TransferDestinationType = "identity" | "platformAddress";

export type TransferStatus =
  | { type: "form" }
  | { type: "sending"; startedAt: number }
  | { type: "error"; message: string }
  | { type: "success" };

export interface TransferScreenProps {
  /** The source identity to transfer from. */
  identity: QualifiedIdentityDto;
  /** Current status of the transfer operation. */
  status: TransferStatus;
  /** Known identities for the receiver selector. */
  knownIdentities?: IdentityOption[];
  /** Called to submit a transfer to another identity. */
  onSubmitToIdentity?: (params: {
    fromIdentityId: string;
    toIdentityId: string;
    credits: number;
    keyId: number | null;
  }) => void;
  /** Called to submit a transfer to a platform address. */
  onSubmitToAddress?: (params: {
    identityId: string;
    address: string;
    credits: number;
    keyId: number | null;
  }) => void;
  /** Called to dismiss an error and return to form. */
  onDismissError?: () => void;
  /** Called to navigate back after success or cancel. */
  onBack?: () => void;
  /** Called to navigate to key info screen. */
  onViewKey?: (keyId: number) => void;
  /** Called to navigate to add key screen. */
  onAddKey?: () => void;
  /** Estimated fee in credits for identity transfer. */
  estimatedFeeIdentity?: number | null;
  /** Estimated fee in credits for platform address transfer. */
  estimatedFeeAddress?: number | null;
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

function getKeyLabel(key: IdentityKeyDto): string {
  return `#${key.keyId} — Transfer (${key.securityLevel})`;
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

export function TransferScreen({
  identity,
  status,
  knownIdentities = [],
  onSubmitToIdentity,
  onSubmitToAddress,
  onDismissError,
  onBack,
  onViewKey,
  onAddKey,
  estimatedFeeIdentity,
  estimatedFeeAddress,
  walletLocked = false,
  onRequestUnlock,
}: TransferScreenProps) {
  // ─── Elapsed timer ──────────────────────────────────────────────
  const sendingStartedAt = status.type === "sending" ? status.startedAt : null;
  const elapsed = useElapsedTimer(sendingStartedAt);

  const [showAdvanced, setShowAdvanced] = useState(false);
  const [destinationType, setDestinationType] =
    useState<TransferDestinationType>("identity");
  const [receiverIdentityId, setReceiverIdentityId] = useState("");
  const [platformAddress, setPlatformAddress] = useState("");
  const [selectedKeyId, setSelectedKeyId] = useState<number | null>(() => {
    const keys = getTransferKeys(identity);
    return keys.length > 0 ? keys[0]?.keyId ?? null : null;
  });
  const [showConfirmation, setShowConfirmation] = useState(false);

  const amount = useAmountInput(DASH_DECIMAL_PLACES);

  const transferKeys = useMemo(() => getTransferKeys(identity), [identity]);
  const selectedKey = useMemo(
    () => transferKeys.find((k) => k.keyId === selectedKeyId) ?? null,
    [transferKeys, selectedKeyId],
  );

  // maxAmount in duffs (AmountInput works in duffs with 8 decimal places)
  const maxAmount = useMemo(() => {
    const availableCredits = identity.balance - TRANSFER_FEE_RESERVE;
    return availableCredits > 0 ? Math.floor(availableCredits / CREDITS_PER_DUFF) : 0;
  }, [identity.balance]);

  const balanceFormatted = formatCreditsAsDash(identity.balance);

  const isDisabled = status.type === "sending" || status.type === "success" || walletLocked;

  const estimatedFee =
    destinationType === "identity"
      ? estimatedFeeIdentity
      : estimatedFeeAddress;

  const handleMaxClick = useCallback(() => {
    if (maxAmount > 0) {
      amount.setValue(formatAmount(maxAmount, DASH_DECIMAL_PLACES));
    }
  }, [maxAmount, amount]);

  const isReady = useMemo(() => {
    if (!amount.parsedAmount || amount.parsedAmount <= 0) return false;
    if (!selectedKey) return false;
    if (amount.parsedAmount > maxAmount) return false;
    if (destinationType === "identity" && !receiverIdentityId.trim())
      return false;
    if (destinationType === "platformAddress" && !platformAddress.trim())
      return false;
    return true;
  }, [
    amount.parsedAmount,
    selectedKey,
    maxAmount,
    destinationType,
    receiverIdentityId,
    platformAddress,
  ]);

  const handleTransfer = useCallback(() => {
    if (!isReady) return;
    setShowConfirmation(true);
  }, [isReady]);

  const handleConfirm = useCallback(() => {
    if (!amount.parsedAmount || !selectedKey) return;

    if (destinationType === "identity") {
      onSubmitToIdentity?.({
        fromIdentityId: identity.id,
        toIdentityId: receiverIdentityId.trim(),
        credits: amount.parsedAmount * CREDITS_PER_DUFF,
        keyId: selectedKey.keyId,
      });
    } else {
      onSubmitToAddress?.({
        identityId: identity.id,
        address: platformAddress.trim(),
        credits: amount.parsedAmount * CREDITS_PER_DUFF,
        keyId: selectedKey.keyId,
      });
    }
  }, [
    amount.parsedAmount,
    selectedKey,
    destinationType,
    identity.id,
    receiverIdentityId,
    platformAddress,
    onSubmitToIdentity,
    onSubmitToAddress,
  ]);

  const confirmTitle =
    destinationType === "identity"
      ? "Confirm Transfer"
      : "Confirm Transfer to Platform Address";

  const confirmMessage = useMemo(() => {
    if (!amount.parsedAmount) return "";
    const amountStr = formatAmount(amount.parsedAmount, DASH_DECIMAL_PLACES);
    if (destinationType === "identity") {
      return `Are you sure you want to transfer ${amountStr} DASH to ${receiverIdentityId.trim()}?`;
    }
    return `Are you sure you want to transfer ${amountStr} DASH to Platform address ${platformAddress.trim()}?`;
  }, [amount.parsedAmount, destinationType, receiverIdentityId, platformAddress]);

  // ─── Success screen ────────────────────────────────────────────

  if (status.type === "success") {
    return (
      <div className="flex flex-col h-full" data-testid="transfer-screen">
        <Header onBack={onBack} />
        <Separator />
        <div className="flex-1 flex flex-col items-center justify-center gap-4 p-8">
          <CheckCircle2 className="h-12 w-12 text-green-500" />
          <div className="text-center space-y-2">
            <h2 className="text-xl font-semibold">Transfer Successful!</h2>
          </div>
          <Button onClick={onBack} className="mt-4">
            Back to Identities
          </Button>
        </div>
      </div>
    );
  }

  // ─── No keys available ─────────────────────────────────────────

  if (transferKeys.length === 0) {
    const typeLabel = getIdentityTypeLabel(identity.identityType);

    return (
      <div className="flex flex-col h-full" data-testid="transfer-screen">
        <Header onBack={onBack} />
        <Separator />
        <div className="flex-1 flex flex-col items-center justify-center gap-4 p-8">
          <AlertCircle className="h-10 w-10 text-muted-foreground" />
          <p className="text-sm text-muted-foreground text-center max-w-md">
            You do not have any transfer keys loaded for this {typeLabel}{" "}
            identity.
          </p>
          <div className="flex items-center gap-2">
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
                Check Transfer Key
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
    <div className="flex flex-col h-full" data-testid="transfer-screen">
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
            1. Input the amount to transfer
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
              Available balance:{" "}
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
              maxExceededHint="0.0002 DASH reserved for fees"
              showMaxButton
              onMaxClick={handleMaxClick}
              disabled={isDisabled}
            />
          </div>
        </section>

        {/* ── Step 2: Destination type ────────────────────── */}
        <section>
          <h3 className="text-sm font-semibold mb-3" data-testid="step-2-heading">
            2. Select transfer destination type
          </h3>
          <div className="flex items-center gap-2 pl-4">
            <Button
              variant={destinationType === "identity" ? "default" : "outline"}
              size="sm"
              className="min-w-[120px]"
              onClick={() => setDestinationType("identity")}
              disabled={isDisabled}
              aria-pressed={destinationType === "identity"}
            >
              Identity
            </Button>
            <Button
              variant={
                destinationType === "platformAddress" ? "default" : "outline"
              }
              size="sm"
              className="min-w-[140px]"
              onClick={() => setDestinationType("platformAddress")}
              disabled={isDisabled}
              aria-pressed={destinationType === "platformAddress"}
            >
              Platform Address
            </Button>
          </div>
        </section>

        {/* ── Step 3: Destination input ───────────────────── */}
        <section>
          <h3 className="text-sm font-semibold mb-3" data-testid="step-3-heading">
            {destinationType === "identity"
              ? "3. ID of the identity to transfer to"
              : "3. Platform address to transfer to"}
          </h3>
          <div className="pl-4">
            {destinationType === "identity" ? (
              <IdentitySelector
                value={receiverIdentityId}
                onChange={setReceiverIdentityId}
                identities={knownIdentities}
                exclude={[identity.id]}
                label="Receiver Identity ID:"
                placeholder="Select identity"
                disabled={isDisabled}
              />
            ) : (
              <div className="space-y-1.5">
                <Label
                  htmlFor="platform-address"
                  className="text-sm font-medium"
                >
                  Platform Address:
                </Label>
                <Input
                  id="platform-address"
                  type="text"
                  placeholder="Enter Platform address (evo1... / tevo1...)"
                  value={platformAddress}
                  onChange={(e) => setPlatformAddress(e.target.value)}
                  disabled={isDisabled}
                  className="max-w-md"
                />
              </div>
            )}
          </div>
        </section>

        {/* ── Step 4: Key selector (advanced) ─────────────── */}
        {showAdvanced && (
          <section>
            <h3
              className="text-sm font-semibold mb-3"
              data-testid="step-4-heading"
            >
              4. Select the key to sign the transaction with
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
                  {transferKeys.map((k) => (
                    <SelectItem key={k.keyId} value={k.keyId.toString()}>
                      {getKeyLabel(k)}
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
              Transferring… ({elapsed})
            </span>
          </div>
        )}

        {/* ── Transfer button ─────────────────────────────── */}
        <div className="pt-2">
          <Button
            onClick={handleTransfer}
            disabled={!isReady || isDisabled}
            className="w-full sm:w-auto"
          >
            <ArrowLeftRight className="h-4 w-4 mr-1.5" />
            Transfer
          </Button>
        </div>
      </div>

      {/* ── Confirmation dialog ────────────────────────── */}
      <ConfirmationDialog
        open={showConfirmation}
        onOpenChange={setShowConfirmation}
        title={confirmTitle}
        message={confirmMessage}
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
        <h2 className="text-lg font-semibold">Transfer Funds</h2>
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
