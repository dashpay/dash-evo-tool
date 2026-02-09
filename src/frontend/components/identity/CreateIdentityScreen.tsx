import { useCallback, useEffect, useMemo, useState } from "react";
import {
  ArrowLeft,
  ChevronDown,
  ChevronUp,
  AlertCircle,
  CheckCircle2,
  Loader2,
  Plus,
  Trash2,
  Copy,
  Info,
  UserPlus,
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
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  AmountInput,
  useAmountInput,
  formatAmount,
} from "@/components/shared/AmountInput";
import type {
  WalletDto,
  AssetLockDto,
  PlatformAddressDto,
  RegisterIdentityFundingMethodDto,
  KeySpecDto,
} from "@/bindings";

// ─── Constants ─────────────────────────────────────────────────────

const DASH_DECIMAL_PLACES = 8;
const MAX_IDENTITY_INDEX = 30;

const MASTER_KEY_TYPES = [
  { value: "ECDSA_HASH160", label: "ECDSA Hash160 (Recommended)" },
  { value: "ECDSA_SECP256K1", label: "ECDSA secp256k1" },
] as const;

const KEY_TYPES = [
  { value: "ECDSA_HASH160", label: "ECDSA Hash160" },
  { value: "ECDSA_SECP256K1", label: "ECDSA secp256k1" },
] as const;

const KEY_PURPOSES = [
  { value: "AUTHENTICATION", label: "Authentication" },
  { value: "TRANSFER", label: "Transfer" },
  { value: "ENCRYPTION", label: "Encryption" },
  { value: "DECRYPTION", label: "Decryption" },
] as const;

const SECURITY_LEVELS = [
  { value: "CRITICAL", label: "Critical" },
  { value: "HIGH", label: "High" },
  { value: "MEDIUM", label: "Medium" },
] as const;

type FundingMethod =
  | "assetLock"
  | "walletBalance"
  | "qrCode"
  | "platformAddress";

const FUNDING_METHODS: { value: FundingMethod; label: string }[] = [
  { value: "assetLock", label: "Unused Evo Funding Locks (Recommended)" },
  { value: "walletBalance", label: "Wallet Balance" },
  { value: "qrCode", label: "Address with QR Code" },
  { value: "platformAddress", label: "Platform Address" },
];

// ─── Types ─────────────────────────────────────────────────────────

export type CreateIdentityStatus =
  | { type: "form" }
  | { type: "waitingForFunds" }
  | { type: "waitingForAssetLock"; startedAt: number }
  | { type: "waitingForPlatform"; startedAt: number }
  | { type: "error"; message: string }
  | { type: "success" };

/** A single editable key spec row for the advanced key editor. */
interface EditableKeySpec {
  id: number;
  keyType: string;
  purpose: string;
  securityLevel: string;
}

export interface CreateIdentityScreenProps {
  /** Available HD wallets for funding. */
  wallets: WalletDto[];
  /** Current status of the registration operation. */
  status: CreateIdentityStatus;
  /** Called to submit the identity creation request. */
  onSubmit?: (params: {
    walletSeedHash: string;
    identityIndex: number;
    alias: string;
    masterKeyType: string;
    keySpecs: KeySpecDto[];
    useDefaultKeys: boolean;
    fundingMethod: RegisterIdentityFundingMethodDto;
  }) => void;
  /** Called to dismiss an error and return to form. */
  onDismissError?: () => void;
  /** Called to navigate back. */
  onBack?: () => void;
  /** Called after success to go back to identity list. */
  onBackToIdentities?: () => void;
  /** Called after success to go to DPNS registration. */
  onRegisterDpns?: () => void;
  /** Called to copy text to clipboard. */
  onCopy?: (text: string) => void;
  /** QR code receive address (generated for QR funding method). */
  qrReceiveAddress?: string | null;
  /** Whether a QR funding UTXO has been detected. */
  qrFundsReceived?: boolean;
}

// ─── Helpers ───────────────────────────────────────────────────────

function getNextAvailableIndex(usedIndexes: number[]): number {
  for (let i = 0; i <= MAX_IDENTITY_INDEX; i++) {
    if (!usedIndexes.includes(i)) return i;
  }
  return 0;
}

function getSecurityLevelForPurpose(purpose: string): string {
  switch (purpose) {
    case "TRANSFER":
      return "CRITICAL";
    case "ENCRYPTION":
    case "DECRYPTION":
      return "MEDIUM";
    default:
      return "CRITICAL";
  }
}

function isSecurityLevelLocked(purpose: string): boolean {
  return (
    purpose === "TRANSFER" ||
    purpose === "ENCRYPTION" ||
    purpose === "DECRYPTION"
  );
}

let nextKeyId = 1;

function makeDefaultKeys(): EditableKeySpec[] {
  return [
    {
      id: nextKeyId++,
      keyType: "ECDSA_HASH160",
      purpose: "AUTHENTICATION",
      securityLevel: "CRITICAL",
    },
    {
      id: nextKeyId++,
      keyType: "ECDSA_HASH160",
      purpose: "AUTHENTICATION",
      securityLevel: "HIGH",
    },
    {
      id: nextKeyId++,
      keyType: "ECDSA_HASH160",
      purpose: "TRANSFER",
      securityLevel: "CRITICAL",
    },
    {
      id: nextKeyId++,
      keyType: "ECDSA_SECP256K1",
      purpose: "ENCRYPTION",
      securityLevel: "MEDIUM",
    },
    {
      id: nextKeyId++,
      keyType: "ECDSA_SECP256K1",
      purpose: "DECRYPTION",
      securityLevel: "MEDIUM",
    },
  ];
}

function formatElapsedTime(startedAt: number): string {
  const elapsed = Math.floor((Date.now() - startedAt) / 1000);
  if (elapsed < 60) return `${elapsed} second${elapsed !== 1 ? "s" : ""}`;
  const minutes = Math.floor(elapsed / 60);
  const seconds = elapsed % 60;
  return `${minutes} minute${minutes !== 1 ? "s" : ""} and ${seconds} second${seconds !== 1 ? "s" : ""}`;
}

function truncateAddress(address: string): string {
  if (address.length <= 22) return address;
  return address.slice(0, 12) + "…" + address.slice(-8);
}

// ─── Component ─────────────────────────────────────────────────────

export function CreateIdentityScreen({
  wallets,
  status,
  onSubmit,
  onDismissError,
  onBack,
  onBackToIdentities,
  onRegisterDpns,
  onCopy,
  qrReceiveAddress,
  qrFundsReceived,
}: CreateIdentityScreenProps) {
  // ─── Form state ────────────────────────────────────────────────

  const [selectedWalletSeedHash, setSelectedWalletSeedHash] = useState<
    string
  >(() => (wallets.length > 0 ? wallets[0].seedHash : ""));

  const selectedWallet = useMemo(
    () => wallets.find((w) => w.seedHash === selectedWalletSeedHash) ?? null,
    [wallets, selectedWalletSeedHash],
  );

  const [showAdvanced, setShowAdvanced] = useState(false);

  const [identityIndex, setIdentityIndex] = useState<number>(() =>
    getNextAvailableIndex(wallets[0]?.identityIndexes ?? []),
  );

  const [masterKeyType, setMasterKeyType] = useState("ECDSA_HASH160");

  const [keyMode, setKeyMode] = useState<"default" | "advanced">("default");
  const [customKeys, setCustomKeys] = useState<EditableKeySpec[]>(
    makeDefaultKeys,
  );

  const [alias, setAlias] = useState("");

  const [fundingMethod, setFundingMethod] = useState<FundingMethod>(() => {
    const wallet = wallets.length > 0 ? wallets[0] : null;
    const hasAssetLocks =
      wallet?.unusedAssetLocks.some((l) => l.hasAssetLockProof) ?? false;
    if (hasAssetLocks) return "assetLock";
    return "walletBalance";
  });

  const [selectedAssetLockTxid, setSelectedAssetLockTxid] = useState<
    string | null
  >(null);

  const [selectedPlatformAddress, setSelectedPlatformAddress] = useState<
    string | null
  >(null);

  const walletBalanceAmount = useAmountInput(DASH_DECIMAL_PLACES);
  const qrAmount = useAmountInput(DASH_DECIMAL_PLACES);
  const platformAmount = useAmountInput(DASH_DECIMAL_PLACES);

  // ─── Derived data ──────────────────────────────────────────────

  const usedIndexes = useMemo(
    () => selectedWallet?.identityIndexes ?? [],
    [selectedWallet],
  );

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

  const totalPlatformBalance = useMemo(
    () => platformAddresses.reduce((sum, a) => sum + a.balance, 0),
    [platformAddresses],
  );

  const selectedAssetLock = useMemo(
    () =>
      unusedAssetLocks.find((l) => l.txid === selectedAssetLockTxid) ?? null,
    [unusedAssetLocks, selectedAssetLockTxid],
  );

  const selectedPlatformAddr = useMemo(
    () =>
      platformAddresses.find((a) => a.address === selectedPlatformAddress) ??
      null,
    [platformAddresses, selectedPlatformAddress],
  );

  const isDisabled =
    status.type !== "form" && status.type !== "error";

  // ─── Funding method availability ───────────────────────────────

  const availableFundingMethods = useMemo(() => {
    const methods: { value: FundingMethod; label: string }[] = [];
    for (const m of FUNDING_METHODS) {
      if (m.value === "assetLock" && unusedAssetLocks.length === 0) continue;
      if (m.value === "platformAddress" && platformAddresses.length === 0)
        continue;
      methods.push(m);
    }
    return methods;
  }, [unusedAssetLocks, platformAddresses]);

  // Auto-correct funding method if current selection is not available
  useEffect(() => {
    if (
      availableFundingMethods.length > 0 &&
      !availableFundingMethods.some((m) => m.value === fundingMethod)
    ) {
      setFundingMethod(availableFundingMethods[0].value);
    }
  }, [availableFundingMethods, fundingMethod]);

  // ─── Readiness check ──────────────────────────────────────────

  const isReady = useMemo(() => {
    if (!selectedWallet) return false;
    if (usedIndexes.includes(identityIndex)) return false;

    switch (fundingMethod) {
      case "assetLock":
        return selectedAssetLock !== null;
      case "walletBalance":
        return (
          walletBalanceAmount.parsedAmount !== null &&
          walletBalanceAmount.parsedAmount > 0 &&
          walletBalanceAmount.parsedAmount <= selectedWallet.totalBalance
        );
      case "qrCode":
        return qrFundsReceived === true;
      case "platformAddress":
        return (
          selectedPlatformAddr !== null &&
          platformAmount.parsedAmount !== null &&
          platformAmount.parsedAmount > 0 &&
          platformAmount.parsedAmount <= selectedPlatformAddr.balance
        );
    }
  }, [
    selectedWallet,
    usedIndexes,
    identityIndex,
    fundingMethod,
    selectedAssetLock,
    walletBalanceAmount.parsedAmount,
    qrFundsReceived,
    selectedPlatformAddr,
    platformAmount.parsedAmount,
  ]);

  // ─── Handlers ──────────────────────────────────────────────────

  const handleWalletChange = useCallback(
    (seedHash: string) => {
      setSelectedWalletSeedHash(seedHash);
      const wallet = wallets.find((w) => w.seedHash === seedHash);
      if (wallet) {
        setIdentityIndex(getNextAvailableIndex(wallet.identityIndexes));
      }
      setSelectedAssetLockTxid(null);
      setSelectedPlatformAddress(null);
    },
    [wallets],
  );

  const handleAddKey = useCallback(() => {
    setCustomKeys((prev) => [
      ...prev,
      {
        id: nextKeyId++,
        keyType: "ECDSA_HASH160",
        purpose: "AUTHENTICATION",
        securityLevel: "HIGH",
      },
    ]);
  }, []);

  const handleRemoveKey = useCallback((keyId: number) => {
    setCustomKeys((prev) => prev.filter((k) => k.id !== keyId));
  }, []);

  const handleKeyPurposeChange = useCallback(
    (keyId: number, purpose: string) => {
      setCustomKeys((prev) =>
        prev.map((k) =>
          k.id === keyId
            ? {
                ...k,
                purpose,
                securityLevel: getSecurityLevelForPurpose(purpose),
              }
            : k,
        ),
      );
    },
    [],
  );

  const handleKeyTypeChange = useCallback(
    (keyId: number, keyType: string) => {
      setCustomKeys((prev) =>
        prev.map((k) => (k.id === keyId ? { ...k, keyType } : k)),
      );
    },
    [],
  );

  const handleKeySecurityLevelChange = useCallback(
    (keyId: number, securityLevel: string) => {
      setCustomKeys((prev) =>
        prev.map((k) => (k.id === keyId ? { ...k, securityLevel } : k)),
      );
    },
    [],
  );

  const handleSubmit = useCallback(() => {
    if (!selectedWallet || !isReady) return;

    const useDefaultKeys = keyMode === "default";
    const keySpecs: KeySpecDto[] = useDefaultKeys
      ? []
      : customKeys.map((k) => ({
          keyType: k.keyType,
          purpose: k.purpose,
          securityLevel: k.securityLevel,
          contractBounds: null,
        }));

    let fm: RegisterIdentityFundingMethodDto;
    switch (fundingMethod) {
      case "assetLock":
        if (!selectedAssetLock || !selectedAssetLock.proofHex) return;
        fm = {
          method: "useAssetLock",
          assetLockProofHex: selectedAssetLock.proofHex,
          transactionHex: selectedAssetLock.txid,
          address: selectedAssetLock.address,
        };
        break;
      case "walletBalance":
        if (!walletBalanceAmount.parsedAmount) return;
        fm = {
          method: "fundWithWallet",
          amountDuffs: walletBalanceAmount.parsedAmount,
        };
        break;
      case "qrCode":
        // QR code funding is handled differently — the UTXO info comes
        // from the backend after detection. This path should not be
        // directly called from here; the parent orchestrator handles it.
        return;
      case "platformAddress":
        if (!selectedPlatformAddr || !platformAmount.parsedAmount) return;
        fm = {
          method: "fundWithPlatformAddresses",
          inputs: [
            {
              address: selectedPlatformAddr.address,
              amount: platformAmount.parsedAmount,
            },
          ],
        };
        break;
    }

    onSubmit?.({
      walletSeedHash: selectedWallet.seedHash,
      identityIndex,
      alias: alias.trim(),
      masterKeyType,
      keySpecs,
      useDefaultKeys,
      fundingMethod: fm,
    });
  }, [
    selectedWallet,
    isReady,
    keyMode,
    customKeys,
    fundingMethod,
    selectedAssetLock,
    walletBalanceAmount.parsedAmount,
    selectedPlatformAddr,
    platformAmount.parsedAmount,
    identityIndex,
    alias,
    masterKeyType,
    onSubmit,
  ]);

  // ─── Success screen ────────────────────────────────────────────

  if (status.type === "success") {
    return (
      <div
        className="flex flex-col h-full"
        data-testid="create-identity-screen"
      >
        <Header onBack={onBackToIdentities} />
        <Separator />
        <div className="flex-1 flex flex-col items-center justify-center gap-4 p-8">
          <CheckCircle2 className="h-12 w-12 text-green-500" />
          <div className="text-center space-y-2">
            <h2 className="text-xl font-semibold">
              Identity Registered Successfully!
            </h2>
            <p className="text-sm text-muted-foreground max-w-md">
              Your new identity has been created on the Dash Platform. You can
              now register a DPNS name or return to the identities list.
            </p>
          </div>
          <div className="flex items-center gap-3 mt-4">
            <Button variant="outline" onClick={onBackToIdentities}>
              Back to Identities
            </Button>
            <Button onClick={onRegisterDpns}>Register DPNS Name</Button>
          </div>
        </div>
      </div>
    );
  }

  // ─── No wallets ────────────────────────────────────────────────

  if (wallets.length === 0) {
    return (
      <div
        className="flex flex-col h-full"
        data-testid="create-identity-screen"
      >
        <Header onBack={onBack} />
        <Separator />
        <div className="flex-1 flex flex-col items-center justify-center gap-4 p-8">
          <AlertCircle className="h-10 w-10 text-muted-foreground" />
          <p className="text-sm text-muted-foreground text-center max-w-md">
            You need at least one HD wallet to create an identity. Please create
            or import a wallet first.
          </p>
          <Button variant="outline" onClick={onBack}>
            Go Back
          </Button>
        </div>
      </div>
    );
  }

  // ─── Main form ─────────────────────────────────────────────────

  return (
    <div className="flex flex-col h-full" data-testid="create-identity-screen">
      <Header
        onBack={onBack}
        showAdvanced={showAdvanced}
        onToggleAdvanced={() => setShowAdvanced((v) => !v)}
      />
      <Separator />

      <div className="flex-1 overflow-y-auto p-6 space-y-6">
        {/* ── Wallet selection ──────────────────────────────── */}
        {wallets.length > 1 && (
          <section>
            <h3 className="text-sm font-semibold mb-3">1. Select Wallet</h3>
            <div className="pl-4">
              <Select
                value={selectedWalletSeedHash}
                onValueChange={handleWalletChange}
                disabled={isDisabled}
              >
                <SelectTrigger
                  className="w-[360px]"
                  aria-label="Select wallet"
                >
                  <SelectValue placeholder="Select a wallet" />
                </SelectTrigger>
                <SelectContent>
                  {wallets.map((w) => (
                    <SelectItem key={w.seedHash} value={w.seedHash}>
                      {w.alias?.trim() || w.seedHash.slice(0, 12) + "…"} —{" "}
                      {formatAmount(w.totalBalance, DASH_DECIMAL_PLACES)} DASH
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </section>
        )}

        {/* ── Advanced: Identity index ──────────────────────── */}
        {showAdvanced && (
          <section>
            <div className="flex items-center gap-2 mb-3">
              <h3 className="text-sm font-semibold">Identity Index</h3>
              <InfoTooltip text="The identity index determines which key derivation path is used. Each index creates a unique identity. Leave at default unless you know what you're doing." />
            </div>
            <div className="pl-4">
              <Select
                value={identityIndex.toString()}
                onValueChange={(v) => setIdentityIndex(Number(v))}
                disabled={isDisabled}
              >
                <SelectTrigger
                  className="w-[240px]"
                  aria-label="Identity index"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {Array.from({ length: MAX_IDENTITY_INDEX + 1 }, (_, i) => (
                    <SelectItem key={i} value={i.toString()}>
                      {i}
                      {usedIndexes.includes(i) ? " (used)" : ""}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {usedIndexes.includes(identityIndex) && (
                <p className="text-sm text-destructive mt-1" role="alert">
                  This index is already in use. Please select a different one.
                </p>
              )}
            </div>
          </section>
        )}

        {/* ── Advanced: Master key type ─────────────────────── */}
        {showAdvanced && (
          <section>
            <div className="flex items-center gap-2 mb-3">
              <h3 className="text-sm font-semibold">Master Key Type</h3>
              <InfoTooltip text="The master key type determines the cryptographic algorithm used for your identity's master key." />
            </div>
            <div className="pl-4">
              <Select
                value={masterKeyType}
                onValueChange={setMasterKeyType}
                disabled={isDisabled}
              >
                <SelectTrigger
                  className="w-[320px]"
                  aria-label="Master key type"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {MASTER_KEY_TYPES.map((t) => (
                    <SelectItem key={t.value} value={t.value}>
                      {t.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </section>
        )}

        {/* ── Advanced: Key configuration ───────────────────── */}
        {showAdvanced && (
          <section>
            <div className="flex items-center gap-2 mb-3">
              <h3 className="text-sm font-semibold">Key Configuration</h3>
              <InfoTooltip text="Keys define what your identity can do. Default keys include authentication, transfer, and DashPay encryption keys." />
            </div>
            <div className="pl-4 space-y-3">
              <div className="flex items-center gap-4">
                <Label className="text-sm">Mode:</Label>
                <Select
                  value={keyMode}
                  onValueChange={(v) =>
                    setKeyMode(v as "default" | "advanced")
                  }
                  disabled={isDisabled}
                >
                  <SelectTrigger className="w-[240px]" aria-label="Key mode">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="default">
                      Default (Recommended)
                    </SelectItem>
                    <SelectItem value="advanced">Advanced</SelectItem>
                  </SelectContent>
                </Select>
              </div>

              {keyMode === "default" ? (
                <div className="rounded-md border bg-muted/30 p-3">
                  <p className="text-sm text-muted-foreground mb-2">
                    Default keys:
                  </p>
                  <ul className="text-sm space-y-1 text-muted-foreground">
                    <li>
                      Master Key (ECDSA Hash160)
                    </li>
                    <li>
                      Authentication — Critical
                    </li>
                    <li>
                      Authentication — High
                    </li>
                    <li>
                      Transfer — Critical
                    </li>
                    <li>
                      Encryption — Medium (DashPay)
                    </li>
                    <li>
                      Decryption — Medium (DashPay)
                    </li>
                  </ul>
                </div>
              ) : (
                <KeyEditor
                  keys={customKeys}
                  disabled={isDisabled}
                  onAddKey={handleAddKey}
                  onRemoveKey={handleRemoveKey}
                  onKeyTypeChange={handleKeyTypeChange}
                  onPurposeChange={handleKeyPurposeChange}
                  onSecurityLevelChange={handleKeySecurityLevelChange}
                />
              )}
            </div>
          </section>
        )}

        {/* ── Alias ─────────────────────────────────────────── */}
        <section>
          <div className="flex items-center gap-2 mb-3">
            <h3 className="text-sm font-semibold">
              {wallets.length > 1 ? "2" : "1"}. Local Alias (optional)
            </h3>
            <InfoTooltip text="A local nickname for this identity. This is stored only on your device and is not visible on-chain. You can change it later." />
          </div>
          <div className="pl-4">
            <Input
              type="text"
              placeholder="e.g. My Main Identity"
              value={alias}
              onChange={(e) => setAlias(e.target.value)}
              disabled={isDisabled}
              className="max-w-sm"
              aria-label="Identity alias"
            />
          </div>
        </section>

        {/* ── Funding method ────────────────────────────────── */}
        <section>
          <h3 className="text-sm font-semibold mb-3">
            {wallets.length > 1 ? "3" : "2"}. Funding Method
          </h3>
          <div className="pl-4 space-y-4">
            <Select
              value={fundingMethod}
              onValueChange={(v) => setFundingMethod(v as FundingMethod)}
              disabled={isDisabled}
            >
              <SelectTrigger
                className="w-[360px]"
                aria-label="Funding method"
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {availableFundingMethods.map((m) => (
                  <SelectItem key={m.value} value={m.value}>
                    {m.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>

            {/* Funding method detail panels */}
            {fundingMethod === "assetLock" && (
              <AssetLockPanel
                assetLocks={unusedAssetLocks}
                selectedTxid={selectedAssetLockTxid}
                onSelect={setSelectedAssetLockTxid}
                disabled={isDisabled}
              />
            )}

            {fundingMethod === "walletBalance" && selectedWallet && (
              <WalletBalancePanel
                wallet={selectedWallet}
                amount={walletBalanceAmount}
                disabled={isDisabled}
              />
            )}

            {fundingMethod === "qrCode" && (
              <QrCodePanel
                address={qrReceiveAddress ?? null}
                amount={qrAmount}
                fundsReceived={qrFundsReceived ?? false}
                onCopy={onCopy}
                disabled={isDisabled}
              />
            )}

            {fundingMethod === "platformAddress" && (
              <PlatformAddressPanel
                platformAddresses={platformAddresses}
                totalBalance={totalPlatformBalance}
                selectedAddress={selectedPlatformAddress}
                onSelectAddress={setSelectedPlatformAddress}
                amount={platformAmount}
                disabled={isDisabled}
              />
            )}
          </div>
        </section>

        {/* ── Error display ─────────────────────────────────── */}
        {status.type === "error" && (
          <div
            className="rounded-md border border-destructive/30 bg-destructive/5 px-4 py-3 space-y-2"
            role="alert"
          >
            <div className="flex items-start gap-2">
              <AlertCircle className="h-4 w-4 text-destructive shrink-0 mt-0.5" />
              <p className="text-sm text-destructive">{status.message}</p>
            </div>
            <Button variant="outline" size="sm" onClick={onDismissError}>
              Dismiss
            </Button>
          </div>
        )}

        {/* ── Waiting states ────────────────────────────────── */}
        {status.type === "waitingForFunds" && (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            <span>Waiting for funds…</span>
          </div>
        )}
        {status.type === "waitingForAssetLock" && (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            <span>
              Waiting for Core Chain to produce proof of transfer… (
              {formatElapsedTime(status.startedAt)})
            </span>
          </div>
        )}
        {status.type === "waitingForPlatform" && (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            <span>
              Waiting for Platform acknowledgement… (
              {formatElapsedTime(status.startedAt)})
            </span>
          </div>
        )}

        {/* ── Create button ─────────────────────────────────── */}
        {status.type === "form" && (
          <div className="pt-2">
            <Button
              onClick={handleSubmit}
              disabled={!isReady || isDisabled}
              className="w-full sm:w-auto"
            >
              <UserPlus className="h-4 w-4 mr-1.5" />
              Create Identity
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}

// ─── Header ──────────────────────────────────────────────────────────

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
        <h2 className="text-lg font-semibold">Create New Identity</h2>
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

// ─── InfoTooltip ─────────────────────────────────────────────────────

function InfoTooltip({ text }: { text: string }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Info className="h-3.5 w-3.5 text-muted-foreground cursor-help" />
      </TooltipTrigger>
      <TooltipContent className="max-w-xs">
        <p className="text-sm">{text}</p>
      </TooltipContent>
    </Tooltip>
  );
}

// ─── Key Editor ──────────────────────────────────────────────────────

interface KeyEditorProps {
  keys: EditableKeySpec[];
  disabled: boolean;
  onAddKey: () => void;
  onRemoveKey: (keyId: number) => void;
  onKeyTypeChange: (keyId: number, keyType: string) => void;
  onPurposeChange: (keyId: number, purpose: string) => void;
  onSecurityLevelChange: (keyId: number, securityLevel: string) => void;
}

function KeyEditor({
  keys,
  disabled,
  onAddKey,
  onRemoveKey,
  onKeyTypeChange,
  onPurposeChange,
  onSecurityLevelChange,
}: KeyEditorProps) {
  return (
    <div className="space-y-2">
      <div className="rounded-md border">
        <div className="grid grid-cols-[1fr_1fr_1fr_auto] gap-2 p-2 border-b bg-muted/50 text-xs font-medium text-muted-foreground">
          <div>Purpose</div>
          <div>Key Type</div>
          <div>Security Level</div>
          <div className="w-8" />
        </div>
        {keys.map((key, index) => (
          <div
            key={key.id}
            className="grid grid-cols-[1fr_1fr_1fr_auto] gap-2 p-2 border-b last:border-b-0 items-center"
            data-testid={`key-row-${index}`}
          >
            <Select
              value={key.purpose}
              onValueChange={(v) => onPurposeChange(key.id, v)}
              disabled={disabled}
            >
              <SelectTrigger className="h-8 text-xs" aria-label="Key purpose">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {KEY_PURPOSES.map((p) => (
                  <SelectItem key={p.value} value={p.value}>
                    {p.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>

            <Select
              value={key.keyType}
              onValueChange={(v) => onKeyTypeChange(key.id, v)}
              disabled={disabled}
            >
              <SelectTrigger className="h-8 text-xs" aria-label="Key type">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {KEY_TYPES.map((t) => (
                  <SelectItem key={t.value} value={t.value}>
                    {t.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>

            <Select
              value={key.securityLevel}
              onValueChange={(v) => onSecurityLevelChange(key.id, v)}
              disabled={disabled || isSecurityLevelLocked(key.purpose)}
            >
              <SelectTrigger
                className="h-8 text-xs"
                aria-label="Security level"
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {SECURITY_LEVELS.map((s) => (
                  <SelectItem key={s.value} value={s.value}>
                    {s.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>

            <Button
              variant="ghost"
              size="icon-sm"
              onClick={() => onRemoveKey(key.id)}
              disabled={disabled}
              aria-label="Remove key"
            >
              <Trash2 className="h-3.5 w-3.5 text-muted-foreground" />
            </Button>
          </div>
        ))}
      </div>
      <Button
        variant="outline"
        size="sm"
        onClick={onAddKey}
        disabled={disabled}
      >
        <Plus className="h-3.5 w-3.5 mr-1" />
        Add Key
      </Button>
    </div>
  );
}

// ─── Asset Lock Panel ────────────────────────────────────────────────

interface AssetLockPanelProps {
  assetLocks: AssetLockDto[];
  selectedTxid: string | null;
  onSelect: (txid: string) => void;
  disabled: boolean;
}

function AssetLockPanel({
  assetLocks,
  selectedTxid,
  onSelect,
  disabled,
}: AssetLockPanelProps) {
  if (assetLocks.length === 0) {
    return (
      <div className="rounded-md border border-border bg-muted/30 px-4 py-3">
        <p className="text-sm text-muted-foreground">
          No unused asset locks available. Create an asset lock from the Wallets
          screen first, or use a different funding method.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      <p className="text-sm text-muted-foreground">
        Select an asset lock to fund identity creation:
      </p>
      <div className="rounded-md border max-h-[200px] overflow-y-auto">
        {assetLocks.map((lock) => {
          const isSelected = lock.txid === selectedTxid;
          return (
            <div
              key={lock.txid}
              className={`flex items-center justify-between p-3 border-b last:border-b-0 cursor-pointer hover:bg-muted/50 transition-colors ${
                isSelected ? "bg-primary/5 border-l-2 border-l-primary" : ""
              }`}
              onClick={() => !disabled && onSelect(lock.txid)}
              data-testid={`asset-lock-${lock.txid.slice(0, 8)}`}
              role="button"
              tabIndex={0}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  if (!disabled) onSelect(lock.txid);
                }
              }}
            >
              <div className="space-y-0.5">
                <p className="text-sm font-mono">
                  {lock.txid.slice(0, 16)}…{lock.txid.slice(-8)}
                </p>
                <p className="text-xs text-muted-foreground">
                  {truncateAddress(lock.address)}
                </p>
              </div>
              <div className="flex items-center gap-2">
                <span className="text-sm font-medium tabular-nums">
                  {formatAmount(lock.amount, DASH_DECIMAL_PLACES)} DASH
                </span>
                {lock.hasInstantLock && (
                  <Badge variant="outline" className="text-xs">
                    InstantLock
                  </Badge>
                )}
                {isSelected && (
                  <Badge className="text-xs bg-primary">Selected</Badge>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ─── Wallet Balance Panel ────────────────────────────────────────────

interface WalletBalancePanelProps {
  wallet: WalletDto;
  amount: ReturnType<typeof useAmountInput>;
  disabled: boolean;
}

function WalletBalancePanel({
  wallet,
  amount,
  disabled,
}: WalletBalancePanelProps) {
  const handleMaxClick = useCallback(() => {
    if (wallet.totalBalance > 0) {
      amount.setValue(formatAmount(wallet.totalBalance, DASH_DECIMAL_PLACES));
    }
  }, [wallet.totalBalance, amount]);

  return (
    <div className="space-y-3">
      <p className="text-sm text-muted-foreground">
        Wallet Balance:{" "}
        <span className="font-medium text-foreground tabular-nums">
          {formatAmount(wallet.totalBalance, DASH_DECIMAL_PLACES)} DASH
        </span>
      </p>
      <AmountInput
        value={amount.value}
        onChange={amount.setValue}
        label="Amount (DASH):"
        placeholder="Enter amount (e.g., 0.1234)"
        decimalPlaces={DASH_DECIMAL_PLACES}
        maxAmount={wallet.totalBalance}
        showMaxButton
        onMaxClick={handleMaxClick}
        disabled={disabled}
      />
    </div>
  );
}

// ─── QR Code Panel ───────────────────────────────────────────────────

interface QrCodePanelProps {
  address: string | null;
  amount: ReturnType<typeof useAmountInput>;
  fundsReceived: boolean;
  onCopy?: (text: string) => void;
  disabled: boolean;
}

function QrCodePanel({
  address,
  amount,
  fundsReceived,
  onCopy,
  disabled,
}: QrCodePanelProps) {
  const paymentUri = useMemo(() => {
    if (!address) return null;
    if (!amount.parsedAmount || amount.parsedAmount <= 0) return null;
    const dashAmount = formatAmount(amount.parsedAmount, DASH_DECIMAL_PLACES);
    return `${address}?amount=${dashAmount}`;
  }, [address, amount.parsedAmount]);

  return (
    <div className="space-y-3">
      <AmountInput
        value={amount.value}
        onChange={amount.setValue}
        label="Amount (DASH):"
        placeholder="Enter amount (e.g., 0.5)"
        decimalPlaces={DASH_DECIMAL_PLACES}
        disabled={disabled || fundsReceived}
      />

      {paymentUri && (
        <div className="rounded-md border bg-muted/30 p-4 space-y-3">
          {/* QR code would be rendered here by the parent with a library */}
          <div
            className="flex items-center justify-center p-4 bg-white rounded-md mx-auto w-fit"
            data-testid="qr-code-placeholder"
          >
            <div className="w-48 h-48 flex items-center justify-center border-2 border-dashed border-gray-300 rounded-md">
              <p className="text-xs text-gray-400 text-center px-2">
                QR Code
                <br />
                {truncateAddress(address ?? "")}
              </p>
            </div>
          </div>

          <div className="flex items-center gap-2">
            <p className="text-xs font-mono text-muted-foreground flex-1 break-all">
              {paymentUri}
            </p>
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={() => onCopy?.(paymentUri)}
              aria-label="Copy payment URI"
            >
              <Copy className="h-3.5 w-3.5" />
            </Button>
          </div>
        </div>
      )}

      {fundsReceived && (
        <div className="flex items-center gap-2 text-sm text-green-600">
          <CheckCircle2 className="h-4 w-4" />
          <span>Funds received! Ready to create identity.</span>
        </div>
      )}

      {!fundsReceived && paymentUri && (
        <p className="text-sm text-muted-foreground">
          Scan the QR code or copy the payment URI to send funds from an
          external wallet. Funds will be detected automatically.
        </p>
      )}
    </div>
  );
}

// ─── Platform Address Panel ──────────────────────────────────────────

interface PlatformAddressPanelProps {
  platformAddresses: PlatformAddressDto[];
  totalBalance: number;
  selectedAddress: string | null;
  onSelectAddress: (address: string) => void;
  amount: ReturnType<typeof useAmountInput>;
  disabled: boolean;
}

function PlatformAddressPanel({
  platformAddresses,
  totalBalance,
  selectedAddress,
  onSelectAddress,
  amount,
  disabled,
}: PlatformAddressPanelProps) {
  const selectedAddr = platformAddresses.find(
    (a) => a.address === selectedAddress,
  );

  const handleMaxClick = useCallback(() => {
    if (selectedAddr && selectedAddr.balance > 0) {
      amount.setValue(formatAmount(selectedAddr.balance, DASH_DECIMAL_PLACES));
    }
  }, [selectedAddr, amount]);

  if (platformAddresses.length === 0) {
    return (
      <div className="rounded-md border border-border bg-muted/30 px-4 py-3">
        <p className="text-sm text-muted-foreground">
          No platform addresses with balance available.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <p className="text-sm text-muted-foreground">
        Total Platform Balance:{" "}
        <span className="font-medium text-foreground tabular-nums">
          {formatAmount(totalBalance, DASH_DECIMAL_PLACES)} DASH
        </span>
      </p>

      <div className="space-y-1.5">
        <Label className="text-sm font-medium">Platform Address:</Label>
        <Select
          value={selectedAddress ?? ""}
          onValueChange={onSelectAddress}
          disabled={disabled}
        >
          <SelectTrigger
            className="w-[360px]"
            aria-label="Platform address"
          >
            <SelectValue placeholder="Select a platform address" />
          </SelectTrigger>
          <SelectContent>
            {platformAddresses.map((addr) => (
              <SelectItem key={addr.address} value={addr.address}>
                {truncateAddress(addr.address)} —{" "}
                {formatAmount(addr.balance, DASH_DECIMAL_PLACES)} DASH
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {selectedAddr && (
        <AmountInput
          value={amount.value}
          onChange={amount.setValue}
          label="Amount (DASH):"
          placeholder="Enter amount (e.g., 0.5)"
          decimalPlaces={DASH_DECIMAL_PLACES}
          maxAmount={selectedAddr.balance}
          showMaxButton
          onMaxClick={handleMaxClick}
          disabled={disabled}
        />
      )}
    </div>
  );
}
