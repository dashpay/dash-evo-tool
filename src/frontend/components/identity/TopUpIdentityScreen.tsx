import { useCallback, useMemo, useState } from "react";
import { useUtxoMonitor } from "@/hooks/useUtxoMonitor";
import {
  ArrowLeft,
  AlertCircle,
  CheckCircle2,
  Loader2,
  Copy,
  Check,
} from "lucide-react";
import { QRCodeSVG } from "qrcode.react";
import { Button } from "@/components/ui/button";
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
} from "@/components/shared/AmountInput";
import type {
  QualifiedIdentityDto,
  WalletDto,
  AssetLockDto,
  PlatformAddressDto,
  TopUpIdentityFundingMethodDto,
} from "@/bindings";

// ─── Constants ─────────────────────────────────────────────────────

const DASH_DECIMAL_PLACES = 8;
/** Reserve 0.005 DASH (500,000 duffs) for on-chain fees. */
const FEE_RESERVE_DUFFS = 500_000;

type FundingMethod =
  | "assetLock"
  | "walletBalance"
  | "qrCode"
  | "platformAddress";

const FUNDING_METHODS: { value: FundingMethod; label: string }[] = [
  { value: "assetLock", label: "Unused Evo Funding Locks" },
  { value: "walletBalance", label: "Wallet Balance" },
  { value: "qrCode", label: "Address with QR Code" },
  { value: "platformAddress", label: "Platform Address" },
];

// ─── Types ─────────────────────────────────────────────────────────

export type TopUpStatus =
  | { type: "form" }
  | { type: "waitingForAssetLock"; startedAt: number }
  | { type: "waitingForPlatform"; startedAt: number }
  | { type: "error"; message: string }
  | { type: "success" };

export interface TopUpIdentityScreenProps {
  /** The identity being topped up. */
  identity: QualifiedIdentityDto;
  /** Available HD wallets for funding. */
  wallets: WalletDto[];
  /** Current status of the top-up operation. */
  status: TopUpStatus;
  /** Called to submit the top-up request. */
  onSubmit?: (params: {
    identityId: string;
    walletSeedHash: string;
    identityIndex: number;
    fundingMethod: TopUpIdentityFundingMethodDto;
  }) => void;
  /** Called for platform address top-up. */
  onSubmitPlatformAddress?: (params: {
    identityId: string;
    walletSeedHash: string;
    outputs: { address: string; amount: number }[];
  }) => void;
  /** Called to dismiss an error and return to form. */
  onDismissError?: () => void;
  /** Called to navigate back. */
  onBack?: () => void;
  /** Estimated fee in duffs (from backend). */
  estimatedFee?: number | null;
}

// ─── Helpers ───────────────────────────────────────────────────────

function formatElapsedTime(startedAt: number): string {
  const elapsed = Math.floor((Date.now() - startedAt) / 1000);
  if (elapsed < 60) return `${elapsed} second${elapsed !== 1 ? "s" : ""}`;
  const minutes = Math.floor(elapsed / 60);
  const seconds = elapsed % 60;
  return `${minutes}m ${seconds}s`;
}

function truncateAddress(address: string): string {
  if (address.length <= 22) return address;
  return address.slice(0, 12) + "…" + address.slice(-8);
}

// ─── Component ─────────────────────────────────────────────────────

export function TopUpIdentityScreen({
  identity,
  wallets,
  status,
  onSubmit,
  onSubmitPlatformAddress,
  onDismissError,
  onBack,
  estimatedFee,
}: TopUpIdentityScreenProps) {
  // ─── State ────────────────────────────────────────────────────

  const [selectedWalletSeedHash, setSelectedWalletSeedHash] = useState<string>(
    () => {
      // Default to wallet associated with identity
      if (
        identity.associatedWalletHashes.length > 0 &&
        wallets.some(
          (w) => w.seedHash === identity.associatedWalletHashes[0],
        )
      ) {
        return identity.associatedWalletHashes[0] ?? "";
      }
      return wallets.length > 0 ? wallets[0]?.seedHash ?? "" : "";
    },
  );

  const selectedWallet = useMemo(
    () => wallets.find((w) => w.seedHash === selectedWalletSeedHash) ?? null,
    [wallets, selectedWalletSeedHash],
  );

  const [fundingMethod, setFundingMethod] = useState<FundingMethod>(() => {
    const wallet = wallets.find((w) => w.seedHash === selectedWalletSeedHash);
    if (wallet?.unusedAssetLocks.some((l) => l.hasAssetLockProof))
      return "assetLock";
    if (wallet && wallet.totalBalance > 0) return "walletBalance";
    return "walletBalance";
  });

  const [selectedAssetLockTxid, setSelectedAssetLockTxid] = useState<
    string | null
  >(null);

  const [selectedPlatformAddress, setSelectedPlatformAddress] = useState<
    string | null
  >(null);

  const [copiedAddress, setCopiedAddress] = useState(false);

  const walletBalanceAmount = useAmountInput(DASH_DECIMAL_PLACES);
  const qrAmount = useAmountInput(DASH_DECIMAL_PLACES);
  const platformAmount = useAmountInput(DASH_DECIMAL_PLACES);

  // ─── Derived ──────────────────────────────────────────────────

  const unusedAssetLocks: AssetLockDto[] = useMemo(
    () =>
      (selectedWallet?.unusedAssetLocks ?? []).filter(
        (lock) => lock.hasAssetLockProof,
      ),
    [selectedWallet],
  );

  const platformAddresses: PlatformAddressDto[] = useMemo(
    () =>
      (selectedWallet?.platformAddresses ?? []).filter(
        (addr) => addr.balance > 0,
      ),
    [selectedWallet],
  );

  const selectedAssetLock = useMemo(
    () => unusedAssetLocks.find((l) => l.txid === selectedAssetLockTxid) ?? null,
    [unusedAssetLocks, selectedAssetLockTxid],
  );

  const selectedPlatAddr = useMemo(
    () =>
      platformAddresses.find((a) => a.address === selectedPlatformAddress) ??
      null,
    [platformAddresses, selectedPlatformAddress],
  );

  const walletBalance = selectedWallet?.totalBalance ?? 0;
  const maxWalletAmount = Math.max(0, walletBalance - FEE_RESERVE_DUFFS);

  const maxPlatformAmount = selectedPlatAddr
    ? Math.max(0, selectedPlatAddr.balance - FEE_RESERVE_DUFFS)
    : 0;

  const identityName =
    identity.alias?.trim() || identity.id.slice(0, 12) + "…";

  const identityIndex = identity.walletIndex ?? 0;

  const isFormActive = status.type === "form";

  // ─── QR code funding: address & UTXO monitoring ─────────────────

  const qrReceiveAddress = useMemo(
    () => selectedWallet?.addresses[0]?.address ?? null,
    [selectedWallet],
  );

  const { fundsReceived: qrFundsReceived, balance: qrFundsBalance } =
    useUtxoMonitor({
      seedHash: selectedWallet?.seedHash ?? null,
      address: qrReceiveAddress,
      enabled: fundingMethod === "qrCode" && isFormActive,
    });

  // Available funding methods: only show methods with available resources
  const availableFundingMethods = useMemo(() => {
    return FUNDING_METHODS.filter((m) => {
      switch (m.value) {
        case "assetLock":
          return unusedAssetLocks.length > 0;
        case "walletBalance":
          return walletBalance > 0;
        case "platformAddress":
          return platformAddresses.length > 0;
        case "qrCode":
          return true; // Always available
        default:
          return true;
      }
    });
  }, [unusedAssetLocks, walletBalance, platformAddresses]);

  // ─── Validation ───────────────────────────────────────────────

  const canSubmit = useMemo(() => {
    if (status.type !== "form") return false;
    if (!selectedWallet) return false;

    switch (fundingMethod) {
      case "assetLock":
        return selectedAssetLock !== null;
      case "walletBalance":
        return (
          walletBalanceAmount.isValid &&
          walletBalanceAmount.parsedAmount !== null &&
          walletBalanceAmount.parsedAmount > 0 &&
          walletBalanceAmount.parsedAmount <= maxWalletAmount
        );
      case "qrCode":
        return qrFundsReceived;
      case "platformAddress":
        return (
          selectedPlatAddr !== null &&
          platformAmount.isValid &&
          platformAmount.parsedAmount !== null &&
          platformAmount.parsedAmount > 0 &&
          platformAmount.parsedAmount <= maxPlatformAmount
        );
      default:
        return false;
    }
  }, [
    status,
    selectedWallet,
    fundingMethod,
    selectedAssetLock,
    walletBalanceAmount,
    maxWalletAmount,
    qrFundsReceived,
    selectedPlatAddr,
    platformAmount,
    maxPlatformAmount,
  ]);

  // ─── Handlers ─────────────────────────────────────────────────

  const handleSubmit = useCallback(() => {
    if (!selectedWallet || !canSubmit) return;

    if (fundingMethod === "platformAddress" && selectedPlatAddr && platformAmount.parsedAmount) {
      onSubmitPlatformAddress?.({
        identityId: identity.id,
        walletSeedHash: selectedWallet.seedHash,
        outputs: [
          {
            address: selectedPlatAddr.address,
            amount: platformAmount.parsedAmount,
          },
        ],
      });
      return;
    }

    let method: TopUpIdentityFundingMethodDto;

    switch (fundingMethod) {
      case "assetLock":
        if (!selectedAssetLock) return;
        method = {
          method: "useAssetLock",
          assetLockProofHex: "", // Proof is resolved backend-side from txid
          transactionHex: "",
          address: selectedAssetLock.address,
        };
        break;
      case "walletBalance":
        if (!walletBalanceAmount.parsedAmount) return;
        method = {
          method: "fundWithWallet",
          amountDuffs: walletBalanceAmount.parsedAmount,
          topUpIndex: 0,
        };
        break;
      case "qrCode":
        if (!qrFundsReceived || qrFundsBalance <= 0) return;
        method = {
          method: "fundWithWallet",
          amountDuffs: qrFundsBalance,
          topUpIndex: 0,
        };
        break;
      default:
        return;
    }

    onSubmit?.({
      identityId: identity.id,
      walletSeedHash: selectedWallet.seedHash,
      identityIndex,
      fundingMethod: method,
    });
  }, [
    canSubmit,
    selectedWallet,
    fundingMethod,
    selectedAssetLock,
    walletBalanceAmount,
    qrFundsReceived,
    qrFundsBalance,
    platformAmount,
    selectedPlatAddr,
    identity,
    identityIndex,
    onSubmit,
    onSubmitPlatformAddress,
  ]);

  const handleCopyAddress = useCallback(
    (address: string) => {
      navigator.clipboard?.writeText(address).then(() => {
        setCopiedAddress(true);
        setTimeout(() => setCopiedAddress(false), 2000);
      });
    },
    [],
  );

  const handleWalletMaxClick = useCallback(() => {
    walletBalanceAmount.setValue(
      formatAmount(maxWalletAmount, DASH_DECIMAL_PLACES),
    );
  }, [walletBalanceAmount, maxWalletAmount]);

  const handlePlatformMaxClick = useCallback(() => {
    platformAmount.setValue(
      formatAmount(maxPlatformAmount, DASH_DECIMAL_PLACES),
    );
  }, [platformAmount, maxPlatformAmount]);

  // ─── Status screens ──────────────────────────────────────────

  if (status.type === "success") {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-6 p-8">
        <CheckCircle2 className="w-16 h-16 text-green-500" aria-hidden />
        <h2 className="text-2xl font-semibold">Top-Up Successful!</h2>
        <p className="text-muted-foreground text-center max-w-md">
          Credits have been added to identity {identityName}.
        </p>
        <Button onClick={onBack}>Back to Identity</Button>
      </div>
    );
  }

  if (status.type === "error") {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-6 p-8">
        <AlertCircle className="w-16 h-16 text-destructive" aria-hidden />
        <h2 className="text-xl font-semibold">Top-Up Failed</h2>
        <p className="text-sm text-destructive bg-destructive/10 rounded-md p-4 max-w-md text-center">
          {status.message}
        </p>
        <Button variant="outline" onClick={onDismissError}>
          Try Again
        </Button>
      </div>
    );
  }

  if (
    status.type === "waitingForAssetLock" ||
    status.type === "waitingForPlatform"
  ) {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-6 p-8">
        <Loader2 className="w-12 h-12 animate-spin text-primary" aria-hidden />
        <h2 className="text-xl font-semibold">
          {status.type === "waitingForAssetLock"
            ? "Waiting for Core Chain proof…"
            : "Waiting for Platform acknowledgement…"}
        </h2>
        <p className="text-sm text-muted-foreground">
          Time elapsed: {formatElapsedTime(status.startedAt)}
        </p>
      </div>
    );
  }

  // ─── Form ─────────────────────────────────────────────────────

  return (
    <div className="flex flex-col gap-6 p-1">
      {/* Header */}
      <div className="flex items-center gap-3">
        <Button
          variant="ghost"
          size="icon"
          onClick={onBack}
          aria-label="Back"
        >
          <ArrowLeft className="w-4 h-4" />
        </Button>
        <div>
          <h2 className="text-lg font-semibold">Top Up Identity</h2>
          <p className="text-sm text-muted-foreground">
            Add credits to <span className="font-medium">{identityName}</span>
          </p>
        </div>
        <Badge variant="secondary" className="ml-auto">
          Balance: {formatCreditsAsDash(identity.balance)} DASH
        </Badge>
      </div>

      <Separator />

      {/* 1. Wallet selection */}
      {wallets.length === 0 ? (
        <div className="flex items-center gap-2 p-4 bg-destructive/10 rounded-md">
          <AlertCircle className="w-4 h-4 text-destructive" />
          <p className="text-sm text-destructive">
            No wallets available. Create a wallet first.
          </p>
        </div>
      ) : (
        <div className="space-y-2">
          <label className="text-sm font-medium">1. Select Wallet</label>
          {wallets.length === 1 ? (
            <p className="text-sm text-muted-foreground">
              {selectedWallet?.alias?.trim() ||
                selectedWallet?.seedHash.slice(0, 10)}{" "}
              — {formatAmount(walletBalance, DASH_DECIMAL_PLACES)} DASH
            </p>
          ) : (
            <Select
              value={selectedWalletSeedHash}
              onValueChange={setSelectedWalletSeedHash}
            >
              <SelectTrigger>
                <SelectValue placeholder="Select wallet" />
              </SelectTrigger>
              <SelectContent>
                {wallets.map((w) => (
                  <SelectItem key={w.seedHash} value={w.seedHash}>
                    {w.alias?.trim() || w.seedHash.slice(0, 10)} —{" "}
                    {formatAmount(w.totalBalance, DASH_DECIMAL_PLACES)} DASH
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          )}
        </div>
      )}

      {/* 2. Funding method */}
      {selectedWallet && (
        <div className="space-y-2">
          <label className="text-sm font-medium">2. Funding Method</label>
          <Select
            value={fundingMethod}
            onValueChange={(v) => setFundingMethod(v as FundingMethod)}
          >
            <SelectTrigger aria-label="Funding method">
              <SelectValue placeholder="Select funding method" />
            </SelectTrigger>
            <SelectContent>
              {availableFundingMethods.map((m) => (
                <SelectItem key={m.value} value={m.value}>
                  {m.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      )}

      {/* 3. Funding method details */}
      {selectedWallet && (
        <>
          <Separator />
          {fundingMethod === "assetLock" && renderAssetLockMethod()}
          {fundingMethod === "walletBalance" && renderWalletBalanceMethod()}
          {fundingMethod === "qrCode" && renderQrCodeMethod()}
          {fundingMethod === "platformAddress" && renderPlatformAddressMethod()}
        </>
      )}

      {/* Fee estimation */}
      {estimatedFee != null && (
        <div className="p-3 bg-muted rounded-md text-sm">
          Estimated fee:{" "}
          <span className="font-medium">
            {formatAmount(estimatedFee, DASH_DECIMAL_PLACES)} DASH
          </span>
        </div>
      )}

      {/* Submit button */}
      {selectedWallet && (
        <Button
          onClick={handleSubmit}
          disabled={!canSubmit}
          className="w-full"
          size="lg"
        >
          Top Up Identity
        </Button>
      )}
    </div>
  );

  // ─── Funding method renderers ──────────────────────────────────

  function renderAssetLockMethod() {
    if (unusedAssetLocks.length === 0) {
      return (
        <p className="text-sm text-muted-foreground">
          No unused asset locks available in this wallet.
        </p>
      );
    }

    return (
      <div className="space-y-3">
        <label className="text-sm font-medium">
          3. Select Asset Lock
        </label>
        <div className="space-y-2 max-h-64 overflow-y-auto">
          {unusedAssetLocks.map((lock) => {
            const isSelected = lock.txid === selectedAssetLockTxid;
            return (
              <button
                key={lock.txid}
                type="button"
                onClick={() => setSelectedAssetLockTxid(lock.txid)}
                className={`w-full flex items-center justify-between p-3 rounded-md border text-left transition-colors ${
                  isSelected
                    ? "border-primary bg-primary/5"
                    : "border-border hover:bg-muted/50"
                }`}
                aria-pressed={isSelected}
              >
                <div className="flex flex-col gap-0.5">
                  <span className="text-xs font-mono text-muted-foreground">
                    {truncateAddress(lock.txid)}
                  </span>
                  <span className="text-sm font-medium">
                    {formatAmount(lock.amount, DASH_DECIMAL_PLACES)} DASH
                  </span>
                </div>
                <div className="flex items-center gap-2">
                  {lock.hasAssetLockProof && (
                    <Badge variant="outline" className="text-xs">
                      Confirmed
                    </Badge>
                  )}
                  {isSelected && (
                    <Badge variant="default" className="text-xs">
                      Selected
                    </Badge>
                  )}
                </div>
              </button>
            );
          })}
        </div>
      </div>
    );
  }

  function renderWalletBalanceMethod() {
    return (
      <div className="space-y-3">
        <div className="flex items-center justify-between text-sm">
          <span className="font-medium">
            3. Amount to top up
          </span>
          <span className="text-muted-foreground">
            Wallet balance:{" "}
            {formatAmount(walletBalance, DASH_DECIMAL_PLACES)} DASH
          </span>
        </div>
        <AmountInput
          value={walletBalanceAmount.value}
          onChange={walletBalanceAmount.setValue}
          placeholder="Enter amount (e.g., 0.01)"
          maxAmount={maxWalletAmount}
          maxExceededHint={`~${formatAmount(FEE_RESERVE_DUFFS, DASH_DECIMAL_PLACES)} DASH reserved for fees`}
          showMaxButton
          onMaxClick={handleWalletMaxClick}
        />
      </div>
    );
  }

  function renderQrCodeMethod() {
    const addr = qrReceiveAddress ?? "";
    // QR code mode: user enters amount, gets an address to share
    return (
      <div className="space-y-4">
        <div className="space-y-2">
          <label className="text-sm font-medium">
            3. Amount to receive
          </label>
          <AmountInput
            value={qrAmount.value}
            onChange={qrAmount.setValue}
            placeholder="Enter amount (e.g., 0.5)"
            disabled={qrFundsReceived}
          />
        </div>

        {qrAmount.isValid && qrAmount.parsedAmount && qrAmount.parsedAmount > 0 && addr && (() => {
          const dashAmount = formatAmount(qrAmount.parsedAmount, DASH_DECIMAL_PLACES);
          const paymentUri = `dash:${addr}?amount=${dashAmount}`;
          return (
            <div className="p-4 bg-muted rounded-md space-y-3">
              <p className="text-sm font-medium">
                Send funds to this address:
              </p>
              <div
                className="flex items-center justify-center p-3 bg-white rounded-lg border mx-auto w-fit"
                role="img"
                aria-label={`QR code for address ${addr}`}
                data-testid="qr-code"
              >
                <QRCodeSVG
                  value={paymentUri}
                  size={192}
                  level="M"
                  includeMargin={false}
                />
              </div>
              <div className="flex items-center gap-2">
                <code className="text-xs font-mono bg-background p-2 rounded flex-1 break-all">
                  {paymentUri}
                </code>
                <Button
                  variant="outline"
                  size="icon"
                  onClick={() => handleCopyAddress(paymentUri)}
                  aria-label="Copy payment URI"
                >
                  {copiedAddress ? (
                    <Check className="w-3.5 h-3.5" />
                  ) : (
                    <Copy className="w-3.5 h-3.5" />
                  )}
                </Button>
              </div>
            </div>
          );
        })()}

        {qrFundsReceived && (
          <div className="flex items-center gap-2 text-sm text-green-600">
            <CheckCircle2 className="h-4 w-4" />
            <span>Funds received! Ready to top up identity.</span>
          </div>
        )}

        {!qrFundsReceived && qrAmount.isValid && qrAmount.parsedAmount && qrAmount.parsedAmount > 0 && addr && (
          <p className="text-sm text-muted-foreground">
            Scan the QR code or copy the payment URI to send funds from an
            external wallet. Funds will be detected automatically.
          </p>
        )}
      </div>
    );
  }

  function renderPlatformAddressMethod() {
    if (platformAddresses.length === 0) {
      return (
        <p className="text-sm text-muted-foreground">
          No Platform addresses with balance found in this wallet.
        </p>
      );
    }

    return (
      <div className="space-y-4">
        <div className="space-y-2">
          <label className="text-sm font-medium">
            3. Select Platform Address
          </label>
          <div className="space-y-2 max-h-48 overflow-y-auto">
            {platformAddresses.map((addr) => {
              const isSelected = addr.address === selectedPlatformAddress;
              return (
                <button
                  key={addr.address}
                  type="button"
                  onClick={() => setSelectedPlatformAddress(addr.address)}
                  className={`w-full flex items-center justify-between p-3 rounded-md border text-left transition-colors ${
                    isSelected
                      ? "border-primary bg-primary/5"
                      : "border-border hover:bg-muted/50"
                  }`}
                  aria-pressed={isSelected}
                >
                  <span className="text-xs font-mono">
                    {truncateAddress(addr.address)}
                  </span>
                  <div className="flex items-center gap-2">
                    <span className="text-sm">
                      {formatAmount(addr.balance, DASH_DECIMAL_PLACES)} DASH
                    </span>
                    {isSelected && (
                      <Badge variant="default" className="text-xs">
                        Selected
                      </Badge>
                    )}
                  </div>
                </button>
              );
            })}
          </div>
        </div>

        {selectedPlatAddr && (
          <div className="space-y-2">
            <div className="flex items-center justify-between text-sm">
              <span className="font-medium">4. Amount to top up</span>
              <span className="text-muted-foreground">
                Available:{" "}
                {formatAmount(selectedPlatAddr.balance, DASH_DECIMAL_PLACES)}{" "}
                DASH
              </span>
            </div>
            <AmountInput
              value={platformAmount.value}
              onChange={platformAmount.setValue}
              placeholder="Enter amount (e.g., 0.01)"
              maxAmount={maxPlatformAmount}
              maxExceededHint={`~${formatAmount(FEE_RESERVE_DUFFS, DASH_DECIMAL_PLACES)} DASH reserved for fees`}
              showMaxButton
              onMaxClick={handlePlatformMaxClick}
            />
          </div>
        )}
      </div>
    );
  }
}
