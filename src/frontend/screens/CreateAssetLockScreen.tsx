import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Island } from "@/components/layout";
import { AmountInput, formatAmount } from "@/components/shared/AmountInput";
import { WalletUnlockDialog } from "@/components/shared/WalletUnlockDialog";
import { CopyButton } from "@/components/shared/CopyButton";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useWalletStore } from "@/stores/walletStore";
import { useUtxoMonitor } from "@/hooks/useUtxoMonitor";
import { commands, events } from "@/bindings";
import type { WalletDto, QualifiedIdentityDto } from "@/bindings";
import { QRCodeSVG } from "qrcode.react";
import {
  ArrowLeft,
  Shield,
  TrendingUp,
  Loader2,
  CheckCircle2,
  QrCode,
  RotateCw,
} from "lucide-react";

// ─── Constants ──────────────────────────────────────────────────────

const DUFFS_PER_DASH = 100_000_000;
const CREDITS_PER_DUFF = 1000;
const MAX_IDENTITY_INDEX = 30;
const DEFAULT_AMOUNT_DASH = "0.5";

// ─── Types ──────────────────────────────────────────────────────────

type AssetLockPurpose = "registration" | "topUp";

type Step =
  | "purpose"
  | "configure"
  | "funding"
  | "creating"
  | "success";

// ─── Helpers ────────────────────────────────────────────────────────

function dashToCredits(dashStr: string): number | null {
  const num = parseFloat(dashStr);
  if (isNaN(num) || num <= 0) return null;
  return Math.round(num * DUFFS_PER_DASH * CREDITS_PER_DUFF);
}

function formatDash(duffs: number): string {
  return formatAmount(duffs, 8) + " DASH";
}

// ─── CreateAssetLockScreen ──────────────────────────────────────────

export function CreateAssetLockScreen() {
  const navigate = useNavigate();
  const hdWallets = useWalletStore((s) => s.hdWallets);
  const selectedWallet = useWalletStore((s) => s.selectedWallet);

  // Find selected HD wallet
  const wallet: WalletDto | null =
    selectedWallet?.type === "hd"
      ? (hdWallets.find((w) => w.seedHash === selectedWallet.seedHash) ?? null)
      : null;

  // Step state
  const [step, setStep] = useState<Step>("purpose");
  const [purpose, setPurpose] = useState<AssetLockPurpose | null>(null);

  // Identity state (for top-up)
  const [identities, setIdentities] = useState<QualifiedIdentityDto[]>([]);
  const [selectedIdentityId, setSelectedIdentityId] = useState<string | null>(null);

  // Configuration
  const [amountValue, setAmountValue] = useState(DEFAULT_AMOUNT_DASH);
  const [identityIndex, setIdentityIndex] = useState(0);
  const [topUpIndex, setTopUpIndex] = useState(0);
  const [showAdvanced, setShowAdvanced] = useState(false);

  // Funding
  const [fundingAddress, setFundingAddress] = useState<string | null>(null);
  const [generatingAddress, setGeneratingAddress] = useState(false);

  // Asset lock creation tracking
  const [taskId, setTaskId] = useState<string | null>(null);
  const [assetLockTxId, setAssetLockTxId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Wallet unlock state
  const [unlockOpen, setUnlockOpen] = useState(false);
  const [unlockError, setUnlockError] = useState<string | null>(null);
  const [walletUnlocked, setWalletUnlocked] = useState(false);

  // UTXO polling during funding step
  const { fundsReceived } = useUtxoMonitor({
    seedHash: wallet?.seedHash ?? null,
    address: fundingAddress,
    enabled: step === "funding",
  });

  // Load identities for top-up
  useEffect(() => {
    commands
      .identityListLocal()
      .then((result) => {
        if (result.status === "ok") {
          setIdentities(result.data);
        }
      })
      .catch(() => {});
  }, []);

  // Auto-unlock if no password
  useEffect(() => {
    if (!wallet) return;
    if (!wallet.usesPassword) {
      setWalletUnlocked(true);
    }
  }, [wallet]);

  // Calculate next unused identity index
  useEffect(() => {
    if (!wallet) return;
    const used = wallet.identityIndexes ?? [];
    for (let i = 0; i <= MAX_IDENTITY_INDEX; i++) {
      if (!used.includes(i)) {
        setIdentityIndex(i);
        return;
      }
    }
    setIdentityIndex(0);
  }, [wallet]);

  // Selected identity object
  const selectedIdentity = useMemo(
    () => identities.find((id) => id.id === selectedIdentityId) ?? null,
    [identities, selectedIdentityId],
  );

  // Calculate next unused top-up index when identity changes
  useEffect(() => {
    if (!selectedIdentity) return;
    const usedTopUpIndexes = new Set(selectedIdentity.topUps.map((t) => t.index));
    for (let i = 0; i <= MAX_IDENTITY_INDEX; i++) {
      if (!usedTopUpIndexes.has(i)) {
        setTopUpIndex(i);
        return;
      }
    }
    setTopUpIndex(0);
  }, [selectedIdentity]);

  // Used index sets for dropdown labels
  const usedIdentityIndexes = useMemo(
    () => new Set(wallet?.identityIndexes ?? []),
    [wallet],
  );
  const usedTopUpIndexes = useMemo(
    () => new Set(selectedIdentity?.topUps.map((t) => t.index) ?? []),
    [selectedIdentity],
  );

  // Listen for task events
  const taskIdRef = useRef<string | null>(null);
  const stepRef = useRef(step);
  useEffect(() => { taskIdRef.current = taskId; }, [taskId]);
  useEffect(() => { stepRef.current = step; }, [step]);

  useEffect(() => {
    let cancelled = false;
    let cleanupResult: (() => void) | undefined;
    let cleanupError: (() => void) | undefined;

    const subscribe = async () => {
      cleanupResult = await events.taskResultEvent.listen((event) => {
        if (cancelled) return;
        const tid = taskIdRef.current;
        if (tid && event.payload.taskId === tid) {
          const msg = event.payload.result.type === "message" ? event.payload.result.text : "";
          const txIdMatch = msg.match(/TX ID:\s*(\S+)/);
          const extractedTxId = txIdMatch ? txIdMatch[1] : null;
          setAssetLockTxId(extractedTxId ?? "created");
          setStep("success");
        } else if (stepRef.current === "funding") {
          const msg = event.payload.result.type === "message" ? event.payload.result.text : "";
          if (msg.includes("UTXO") || event.payload.result.type === "coreCompleted") {
            handleCreateAssetLock();
          }
        }
      });
      cleanupError = await events.taskErrorEvent.listen((event) => {
        if (cancelled) return;
        const tid = taskIdRef.current;
        if (tid && event.payload.taskId === tid) {
          setError(event.payload.message);
          setStep("configure");
        }
      });
    };
    subscribe().catch(console.error);

    return () => {
      cancelled = true;
      cleanupResult?.();
      cleanupError?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // ─── Handlers ─────────────────────────────────────────────────────

  const handleBack = useCallback(() => {
    navigate({ to: "/wallets" });
  }, [navigate]);

  const handleSelectPurpose = useCallback(
    (p: AssetLockPurpose) => {
      setPurpose(p);
      setStep("configure");
    },
    [],
  );

  const handleChangePurpose = useCallback(() => {
    setPurpose(null);
    setStep("purpose");
    setSelectedIdentityId(null);
    setAmountValue(DEFAULT_AMOUNT_DASH);
    setFundingAddress(null);
    setError(null);
  }, []);

  const handleGenerateFundingAddress = useCallback(async () => {
    if (!wallet) return;
    setGeneratingAddress(true);
    setError(null);
    try {
      const result = await commands.walletGenerateReceiveAddress({
        walletSeedHash: wallet.seedHash,
      });
      if (result.status === "ok") {
        setFundingAddress(result.data.address);
        setStep("funding");
        setGeneratingAddress(false);
      } else {
        setError(result.error);
        setGeneratingAddress(false);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setGeneratingAddress(false);
    }
  }, [wallet]);

  const handleCreateAssetLock = useCallback(async () => {
    if (!wallet) return;

    const credits = dashToCredits(amountValue);
    if (!credits) {
      setError("Invalid amount");
      return;
    }

    setStep("creating");
    setError(null);

    try {
      let result;
      if (purpose === "registration") {
        result = await commands.coreCreateRegistrationAssetLock({
          walletSeedHash: wallet.seedHash,
          amountCredits: credits,
          identityIndex,
        });
      } else {
        // Top-up
        result = await commands.coreCreateTopUpAssetLock({
          walletSeedHash: wallet.seedHash,
          amountCredits: credits,
          identityIndex: selectedIdentity
            ? identityIndex
            : identityIndex,
          topUpIndex,
        });
      }

      if (result.status === "error") {
        setError(result.error);
        setStep("configure");
        return;
      }

      setTaskId(result.data.taskId);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setStep("configure");
    }
  }, [wallet, amountValue, purpose, identityIndex, topUpIndex, selectedIdentity]);

  // Auto-trigger asset lock creation when UTXO polling detects funds
  const handleCreateRef = useRef(handleCreateAssetLock);
  handleCreateRef.current = handleCreateAssetLock;
  useEffect(() => {
    if (fundsReceived && step === "funding") {
      handleCreateRef.current();
    }
    // Only trigger on fundsReceived/step changes, not callback identity
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fundsReceived, step]);

  const handleUnlockResult = useCallback(
    async (result: { status: "unlocked"; password: string } | { status: "cancelled" }) => {
      if (result.status === "unlocked" && wallet) {
        setUnlockError(null);
        try {
          await commands.walletNotifyUnlocked(wallet.seedHash);
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

  const handleCreateAnother = useCallback(() => {
    setPurpose(null);
    setStep("purpose");
    setSelectedIdentityId(null);
    setAmountValue(DEFAULT_AMOUNT_DASH);
    setFundingAddress(null);
    setTaskId(null);
    setAssetLockTxId(null);
    setError(null);
    setShowAdvanced(false);
  }, []);

  // ─── Validation ───────────────────────────────────────────────────

  const canProceedToFunding = useMemo(() => {
    if (!walletUnlocked) return false;
    const credits = dashToCredits(amountValue);
    if (!credits || credits <= 0) return false;
    if (purpose === "topUp" && !selectedIdentityId) return false;
    return true;
  }, [walletUnlocked, amountValue, purpose, selectedIdentityId]);

  // ─── No wallet ────────────────────────────────────────────────────

  if (!wallet) {
    return (
      <div className="flex flex-1 items-center justify-center">
        <div className="text-center">
          <p className="text-muted-foreground">No HD wallet selected</p>
          <Button variant="outline" className="mt-4" onClick={handleBack}>
            Back to Wallets
          </Button>
        </div>
      </div>
    );
  }

  const walletAlias = wallet.alias?.trim() || "Unnamed Wallet";

  // ─── Success step ─────────────────────────────────────────────────

  if (step === "success") {
    return (
      <Island className="flex flex-1 items-center justify-center">
        <div className="text-center space-y-6">
          <CheckCircle2 className="h-16 w-16 text-success mx-auto" />
          <h2 className="text-2xl font-semibold">Asset Lock Created</h2>
          <p className="text-muted-foreground">
            Your asset lock has been created successfully.
          </p>
          {assetLockTxId && assetLockTxId !== "created" && (
            <div className="flex items-center justify-center gap-2">
              <code className="text-xs bg-muted px-2 py-1 rounded break-all max-w-md">
                {assetLockTxId}
              </code>
              <CopyButton value={assetLockTxId} label="Copy TX ID" />
            </div>
          )}
          <div className="flex gap-3 justify-center">
            <Button variant="outline" onClick={handleCreateAnother}>
              Create Another
            </Button>
            <Button onClick={handleBack}>Back to Wallets</Button>
          </div>
        </div>
      </Island>
    );
  }

  // ─── Creating step ────────────────────────────────────────────────

  if (step === "creating") {
    return (
      <Island className="flex flex-1 items-center justify-center">
        <div className="text-center space-y-6">
          <Loader2 className="h-12 w-12 animate-spin text-primary mx-auto" />
          <h2 className="text-2xl font-semibold">Creating Asset Lock</h2>
          <p className="text-muted-foreground">
            Waiting for Core Chain to produce proof of asset lock...
          </p>
        </div>
      </Island>
    );
  }

  // ─── Main form ────────────────────────────────────────────────────

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
          <h1 className="text-2xl font-semibold">Create Asset Lock</h1>
        </div>
        {step === "configure" && (
          <label className="flex items-center gap-2 text-sm text-muted-foreground cursor-pointer">
            <Checkbox
              checked={showAdvanced}
              onCheckedChange={(checked) => setShowAdvanced(checked === true)}
            />
            Advanced Options
          </label>
        )}
      </div>

      {/* Error banner */}
      {error && (
        <div
          className="flex items-start gap-3 rounded-md border border-destructive/50 bg-destructive/10 p-3 mb-4"
          role="alert"
        >
          <p className="text-sm text-destructive flex-1">{error}</p>
          <Button
            variant="ghost"
            size="icon"
            className="h-6 w-6 shrink-0"
            onClick={() => setError(null)}
            aria-label="Dismiss error"
          >
            ×
          </Button>
        </div>
      )}

      {/* Wallet info */}
      <div className="text-sm text-muted-foreground mb-4">
        Wallet: <span className="font-medium text-foreground">{walletAlias}</span>
        <span className="ml-4">
          Balance: <span className="font-medium text-success">{formatDash(wallet.confirmedBalance)}</span>
        </span>
      </div>

      {/* Wallet unlock gate */}
      {!walletUnlocked && wallet.usesPassword && (
        <div className="space-y-3 mb-6">
          <p className="text-sm text-warning">
            Wallet is locked. Please unlock to continue.
          </p>
          <Button variant="outline" onClick={() => setUnlockOpen(true)}>
            Unlock Wallet
          </Button>
        </div>
      )}

      {walletUnlocked && (
        <div className="space-y-6">
          {/* Purpose selection */}
          {step === "purpose" && (
            <div className="space-y-4">
              <Label className="text-base font-semibold">
                What is this asset lock for?
              </Label>
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 max-w-lg">
                <button
                  type="button"
                  className="flex flex-col items-center gap-3 rounded-lg border p-6 hover:bg-muted/50 hover:border-primary transition-colors text-center"
                  onClick={() => handleSelectPurpose("registration")}
                >
                  <Shield className="h-10 w-10 text-primary" />
                  <span className="font-semibold">Registration</span>
                  <span className="text-xs text-muted-foreground">
                    Create an asset lock for registering a new identity on the
                    Dash Platform.
                  </span>
                </button>
                <button
                  type="button"
                  className="flex flex-col items-center gap-3 rounded-lg border p-6 hover:bg-muted/50 hover:border-primary transition-colors text-center"
                  onClick={() => handleSelectPurpose("topUp")}
                >
                  <TrendingUp className="h-10 w-10 text-primary" />
                  <span className="font-semibold">Top Up</span>
                  <span className="text-xs text-muted-foreground">
                    Add credits to an existing identity by creating an asset lock
                    top-up transaction.
                  </span>
                </button>
              </div>
            </div>
          )}

          {/* Configure step */}
          {step === "configure" && purpose && (
            <div className="space-y-6 max-w-lg">
              {/* Purpose badge + change */}
              <div className="flex items-center gap-3">
                <Badge variant="outline" className="text-sm">
                  {purpose === "registration" ? "Registration" : "Top Up"}
                </Badge>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={handleChangePurpose}
                >
                  Change
                </Button>
              </div>

              {/* Identity selector (top-up only) */}
              {purpose === "topUp" && (
                <div className="space-y-2">
                  <Label className="text-sm font-semibold">
                    Select Identity to Top Up
                  </Label>
                  {identities.length === 0 ? (
                    <p className="text-sm text-muted-foreground">
                      No identities found. Load or register an identity first.
                    </p>
                  ) : (
                    <Select
                      value={selectedIdentityId ?? ""}
                      onValueChange={setSelectedIdentityId}
                    >
                      <SelectTrigger className="w-full">
                        <SelectValue placeholder="Select an identity..." />
                      </SelectTrigger>
                      <SelectContent>
                        {identities.map((id) => (
                          <SelectItem key={id.id} value={id.id}>
                            {id.alias || id.id.slice(0, 16) + "..."}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  )}
                </div>
              )}

              {/* Advanced index selection */}
              {showAdvanced && (
                <div className="space-y-4 rounded-md border bg-muted/20 p-4">
                  <Label className="text-xs font-semibold text-muted-foreground">
                    Advanced Options
                  </Label>
                  {purpose === "registration" && (
                    <div className="space-y-2">
                      <Label className="text-sm">Identity Index</Label>
                      <Select
                        value={identityIndex.toString()}
                        onValueChange={(v) => setIdentityIndex(parseInt(v))}
                      >
                        <SelectTrigger className="w-32">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {Array.from({ length: MAX_IDENTITY_INDEX + 1 }, (_, i) => (
                            <SelectItem key={i} value={i.toString()} disabled={usedIdentityIndexes.has(i)}>
                              {i}{usedIdentityIndexes.has(i) ? " (used)" : ""}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </div>
                  )}
                  {purpose === "topUp" && (
                    <div className="space-y-2">
                      <Label className="text-sm">Top-Up Index</Label>
                      <Select
                        value={topUpIndex.toString()}
                        onValueChange={(v) => setTopUpIndex(parseInt(v))}
                      >
                        <SelectTrigger className="w-32">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {Array.from({ length: MAX_IDENTITY_INDEX + 1 }, (_, i) => (
                            <SelectItem key={i} value={i.toString()} disabled={usedTopUpIndexes.has(i)}>
                              {i}{usedTopUpIndexes.has(i) ? " (used)" : ""}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </div>
                  )}
                </div>
              )}

              {/* Amount input */}
              <AmountInput
                value={amountValue}
                onChange={setAmountValue}
                label="Amount (DASH)"
                placeholder="0.5"
                decimalPlaces={8}
                unitName="DASH"
                minAmount={1000}
                showValidationErrors
              />
              {amountValue && dashToCredits(amountValue) && (
                <p className="text-xs text-muted-foreground">
                  ≈ {dashToCredits(amountValue)!.toLocaleString()} credits
                </p>
              )}

              {/* Proceed button */}
              <div className="flex gap-3">
                <Button variant="outline" onClick={handleBack}>
                  Cancel
                </Button>
                <Button
                  onClick={handleGenerateFundingAddress}
                  disabled={!canProceedToFunding || generatingAddress}
                >
                  {generatingAddress ? (
                    <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                  ) : (
                    <QrCode className="h-4 w-4 mr-2" />
                  )}
                  Generate Funding Address
                </Button>
              </div>
            </div>
          )}

          {/* Funding step */}
          {step === "funding" && fundingAddress && (
            <div className="space-y-6 max-w-lg">
              <div className="flex items-center gap-3">
                <Badge variant="outline" className="text-sm">
                  {purpose === "registration" ? "Registration" : "Top Up"}
                </Badge>
                <Badge variant="secondary" className="text-sm">
                  {amountValue} DASH
                </Badge>
              </div>

              <div className="text-center space-y-4 rounded-lg border p-6">
                <h3 className="text-lg font-semibold">
                  Fund This Address
                </h3>
                <p className="text-sm text-muted-foreground">
                  Send exactly <span className="font-semibold text-foreground">{amountValue} DASH</span>{" "}
                  to the address below. The asset lock will be created automatically
                  when funds are received.
                </p>

                {/* QR code */}
                <div
                  className="mx-auto p-3 rounded-lg border bg-white w-fit"
                  aria-label={`QR code for ${fundingAddress}`}
                >
                  <QRCodeSVG
                    value={`dash:${fundingAddress}?amount=${amountValue}`}
                    size={220}
                    level="M"
                  />
                </div>

                {/* Address display */}
                <div className="flex items-center justify-center gap-2">
                  <code className="text-xs bg-muted px-3 py-2 rounded break-all max-w-sm">
                    {fundingAddress}
                  </code>
                  <CopyButton
                    value={`dash:${fundingAddress}?amount=${amountValue}`}
                    label="Copy URI"
                  />
                </div>

                <div className="flex items-center justify-center gap-2 text-sm text-muted-foreground">
                  <Loader2 className="h-4 w-4 animate-spin" />
                  {fundsReceived
                    ? "Funds received! Creating asset lock..."
                    : "Waiting for funds..."}
                </div>
              </div>

              {/* Manual create button (if funds already sent) */}
              <div className="flex gap-3 justify-center">
                <Button variant="outline" onClick={handleBack}>
                  Cancel
                </Button>
                <Button
                  variant="outline"
                  onClick={handleCreateAssetLock}
                >
                  <RotateCw className="h-4 w-4 mr-2" />
                  Create Asset Lock Now
                </Button>
              </div>
            </div>
          )}
        </div>
      )}

      {/* Wallet unlock dialog */}
      {wallet && (
        <WalletUnlockDialog
          open={unlockOpen}
          onOpenChange={setUnlockOpen}
          walletAlias={walletAlias}
          passwordHint={wallet.passwordHint ?? null}
          error={unlockError}
          onResult={handleUnlockResult}
        />
      )}
    </Island>
  );
}
