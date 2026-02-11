import { useCallback, useMemo, useState } from "react";
import { useNavigate, useParams } from "@tanstack/react-router";
import { Island } from "@/components/layout";
import { CopyButton } from "@/components/shared/CopyButton";
import { WalletUnlockDialog } from "@/components/shared/WalletUnlockDialog";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";
import { useWalletStore } from "@/stores/walletStore";
import { commands } from "@/bindings";
import type { WalletDto, AssetLockDto, AssetLockProofDetailsDto } from "@/bindings";
import { formatAmount } from "@/components/shared/AmountInput";
import {
  ArrowLeft,
  Eye,
  EyeOff,
  ShieldAlert,
  Lock,
  CheckCircle2,
  Clock,
  ChevronDown,
  ChevronRight,
} from "lucide-react";

// ─── Constants ──────────────────────────────────────────────────────

const CREDITS_PER_DUFF = 1000;

// ─── Helpers ────────────────────────────────────────────────────────

function creditsToDisplayDash(credits: number): string {
  const duffs = credits / CREDITS_PER_DUFF;
  return formatAmount(Math.round(duffs), 8) + " DASH";
}

function creditsToDisplayDuffs(credits: number): string {
  const duffs = Math.round(credits / CREDITS_PER_DUFF);
  return duffs.toLocaleString() + " duffs";
}

// ─── ProofDetailsSection ────────────────────────────────────────────

function ProofDetailsSection({
  proofDetails,
  proofHex,
}: {
  proofDetails: AssetLockProofDetailsDto;
  proofHex: string | null;
}) {
  const [rawExpanded, setRawExpanded] = useState(false);

  return (
    <section className="space-y-4">
      <h2 className="text-lg font-semibold">Asset Lock Proof Details</h2>

      <div className="grid grid-cols-[140px_1fr] gap-y-3 gap-x-4 text-sm">
        <Label className="text-muted-foreground">Type</Label>
        <code className="text-xs">
          {proofDetails.type === "instantSend" ? "Instant Send" : "Chain Lock"}
        </code>

        {proofDetails.type === "instantSend" && (
          <>
            <Label className="text-muted-foreground">InstantLock TxID</Label>
            <div className="flex items-center gap-2">
              <code className="text-xs break-all">{proofDetails.instantLockTxid}</code>
              <CopyButton value={proofDetails.instantLockTxid} label="Copy InstantLock TxID" />
            </div>

            <Label className="text-muted-foreground">Output Index</Label>
            <code className="text-xs">{proofDetails.outputIndex}</code>
          </>
        )}

        {proofDetails.type === "chainLock" && (
          <>
            <Label className="text-muted-foreground">Core Chain Locked Height</Label>
            <code className="text-xs">{proofDetails.coreChainLockedHeight}</code>

            <Label className="text-muted-foreground">OutPoint</Label>
            <div className="flex items-center gap-2">
              <code className="text-xs break-all">
                {proofDetails.outPointTxid}:{proofDetails.outPointVout}
              </code>
              <CopyButton
                value={`${proofDetails.outPointTxid}:${proofDetails.outPointVout}`}
                label="Copy OutPoint"
              />
            </div>
          </>
        )}
      </div>

      {/* Proof Hex */}
      {proofHex && (
        <div className="space-y-2">
          <div className="flex items-center gap-2">
            <Label className="text-muted-foreground text-sm">Asset Lock Proof (hex)</Label>
            <CopyButton value={proofHex} label="Copy proof hex" />
          </div>
          <div className="overflow-x-auto rounded bg-muted px-3 py-2">
            <code className="text-xs text-muted-foreground break-all font-mono whitespace-pre-wrap">
              {proofHex}
            </code>
          </div>
        </div>
      )}

      {/* Raw Proof Details (collapsible) */}
      {proofHex && (
        <button
          type="button"
          className="flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground transition-colors"
          onClick={() => setRawExpanded((v) => !v)}
          aria-expanded={rawExpanded}
        >
          {rawExpanded ? (
            <ChevronDown className="h-4 w-4" />
          ) : (
            <ChevronRight className="h-4 w-4" />
          )}
          View Raw Proof Details
        </button>
      )}
      {rawExpanded && proofHex && (
        <div className="overflow-x-auto rounded bg-muted px-3 py-2">
          <pre className="text-xs text-muted-foreground font-mono whitespace-pre-wrap">
            {JSON.stringify(
              proofDetails,
              null,
              2,
            )}
          </pre>
        </div>
      )}
    </section>
  );
}

// ─── AssetLockDetailScreen ──────────────────────────────────────────

export function AssetLockDetailScreen() {
  const navigate = useNavigate();
  const { txid } = useParams({ strict: false }) as { txid?: string };

  const hdWallets = useWalletStore((s) => s.hdWallets);
  const selectedWallet = useWalletStore((s) => s.selectedWallet);

  // Find selected HD wallet
  const wallet: WalletDto | null =
    selectedWallet?.type === "hd"
      ? (hdWallets.find((w) => w.seedHash === selectedWallet.seedHash) ?? null)
      : null;

  // Find the asset lock by txid
  const assetLock = useMemo(() => {
    if (!wallet || !txid) return null;
    return wallet.unusedAssetLocks.find(
      (al: AssetLockDto) => al.txid === txid,
    ) ?? null;
  }, [wallet, txid]);

  // Private key state
  const [showPrivateKey, setShowPrivateKey] = useState(false);
  const [privateKeyWif, setPrivateKeyWif] = useState<string | null>(null);
  const [privateKeyError, setPrivateKeyError] = useState<string | null>(null);

  // Wallet unlock state
  const [unlockOpen, setUnlockOpen] = useState(false);
  const [unlockError, setUnlockError] = useState<string | null>(null);
  const [walletUnlocked, setWalletUnlocked] = useState(false);

  // Auto-unlock if no password
  useState(() => {
    if (wallet && !wallet.usesPassword) {
      setWalletUnlocked(true);
    }
  });

  // ─── Handlers ─────────────────────────────────────────────────────

  const handleBack = useCallback(() => {
    navigate({ to: "/wallets" });
  }, [navigate]);

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

  const handleViewPrivateKey = useCallback(async () => {
    if (!wallet || !assetLock) return;

    if (!walletUnlocked && wallet.usesPassword) {
      setUnlockOpen(true);
      return;
    }

    try {
      // Find the derivation path for this address
      const addr = wallet.addresses.find((a) => a.address === assetLock.address);
      if (!addr) {
        setPrivateKeyError("Address not found in wallet");
        return;
      }

      const result = await commands.walletGetPrivateKey({
        walletSeedHash: wallet.seedHash,
        address: assetLock.address,
        derivationPath: addr.derivationPath,
      });

      if (result.status === "ok") {
        setPrivateKeyWif(result.data);
        setShowPrivateKey(true);
      } else {
        setPrivateKeyError(result.error);
      }
    } catch (e) {
      setPrivateKeyError(e instanceof Error ? e.message : String(e));
    }
  }, [wallet, assetLock, walletUnlocked]);

  const handleHidePrivateKey = useCallback(() => {
    setShowPrivateKey(false);
    setPrivateKeyWif(null);
  }, []);

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

  // ─── Asset lock not found ─────────────────────────────────────────

  if (!assetLock) {
    return (
      <Island className="flex flex-1 items-center justify-center">
        <div className="text-center space-y-4">
          <p className="text-muted-foreground">Asset lock not found</p>
          <p className="text-xs text-muted-foreground">
            Transaction ID: <code>{txid}</code>
          </p>
          <Button variant="outline" onClick={handleBack}>
            Back to Wallets
          </Button>
        </div>
      </Island>
    );
  }

  const walletAlias = wallet.alias?.trim() || "Unnamed Wallet";

  // ─── Proof status ─────────────────────────────────────────────────

  const proofStatus = assetLock.hasAssetLockProof
    ? assetLock.hasInstantLock
      ? { label: "Instant Send Locked", color: "text-success", icon: CheckCircle2 }
      : { label: "Chain Locked", color: "text-success", icon: Lock }
    : { label: "Waiting for Lock", color: "text-warning", icon: Clock };

  const ProofIcon = proofStatus.icon;

  // ─── Render ───────────────────────────────────────────────────────

  return (
    <Island className="flex flex-col flex-1 overflow-auto">
      {/* Header */}
      <div className="flex items-center gap-3 mb-6">
        <Button
          variant="ghost"
          size="icon"
          onClick={handleBack}
          aria-label="Back to wallets"
        >
          <ArrowLeft className="h-5 w-5" />
        </Button>
        <h1 className="text-2xl font-semibold">Asset Lock Detail</h1>
      </div>

      {/* Wallet info */}
      <div className="text-sm text-muted-foreground mb-6">
        Wallet: <span className="font-medium text-foreground">{walletAlias}</span>
      </div>

      <div className="space-y-6 max-w-2xl">
        {/* Transaction Information */}
        <section className="space-y-4">
          <h2 className="text-lg font-semibold">Transaction Information</h2>

          <div className="grid grid-cols-[140px_1fr] gap-y-3 gap-x-4 text-sm">
            <Label className="text-muted-foreground">Transaction ID</Label>
            <div className="flex items-center gap-2">
              <code className="text-xs break-all">{assetLock.txid}</code>
              <CopyButton value={assetLock.txid} label="Copy TX ID" />
            </div>

            <Label className="text-muted-foreground">Address</Label>
            <div className="flex items-center gap-2">
              <code className="text-xs break-all">{assetLock.address}</code>
              <CopyButton value={assetLock.address} label="Copy address" />
            </div>

            <Label className="text-muted-foreground">Amount</Label>
            <div>
              <span className="font-medium">
                {creditsToDisplayDash(assetLock.amount)}
              </span>
              <span className="text-xs text-muted-foreground ml-2">
                ({creditsToDisplayDuffs(assetLock.amount)})
              </span>
            </div>

            <Label className="text-muted-foreground">Proof Status</Label>
            <div className="flex items-center gap-2">
              <ProofIcon className={`h-4 w-4 ${proofStatus.color}`} />
              <Badge
                variant="outline"
                className={proofStatus.color}
              >
                {proofStatus.label}
              </Badge>
            </div>

            <Label className="text-muted-foreground">InstantLock</Label>
            <span>
              {assetLock.hasInstantLock ? (
                <Badge variant="outline" className="text-success">Yes</Badge>
              ) : (
                <Badge variant="outline" className="text-muted-foreground">No</Badge>
              )}
            </span>

            <Label className="text-muted-foreground">Usable</Label>
            <span>
              {assetLock.hasAssetLockProof ? (
                <Badge variant="outline" className="text-success">Yes</Badge>
              ) : (
                <Badge variant="outline" className="text-warning">Not yet</Badge>
              )}
            </span>
          </div>
        </section>

        {/* Proof Details Section — only when proof exists */}
        {assetLock.proofDetails && (
          <>
            <Separator />
            <ProofDetailsSection
              proofDetails={assetLock.proofDetails}
              proofHex={assetLock.proofHex}
            />
          </>
        )}

        <Separator />

        {/* Private Key Section */}
        <section className="space-y-4">
          <h2 className="text-lg font-semibold">Private Key</h2>

          {/* Wallet unlock gate */}
          {!walletUnlocked && wallet.usesPassword ? (
            <div className="space-y-3">
              <p className="text-sm text-muted-foreground">
                Unlock your wallet to view the private key for this address.
              </p>
              <Button variant="outline" onClick={() => setUnlockOpen(true)}>
                Unlock Wallet
              </Button>
            </div>
          ) : (
            <div className="space-y-4">
              {/* Security warning */}
              <div
                className="flex items-start gap-3 rounded-md border border-destructive/30 bg-destructive/5 p-3"
                role="alert"
              >
                <ShieldAlert className="h-5 w-5 text-destructive shrink-0 mt-0.5" />
                <p className="text-sm text-destructive">
                  Keep your private key secure. Never share it with anyone.
                  Anyone with access to this key can spend funds at this address.
                </p>
              </div>

              {/* Key display */}
              {showPrivateKey && privateKeyWif ? (
                <div className="space-y-3">
                  <Label className="text-sm text-muted-foreground">
                    Private Key (WIF)
                  </Label>
                  <div className="flex items-center gap-2">
                    <code
                      className="text-xs bg-muted px-3 py-2 rounded break-all flex-1 font-mono"
                      aria-label="Private key WIF"
                    >
                      {privateKeyWif}
                    </code>
                    <CopyButton value={privateKeyWif} label="Copy private key" />
                  </div>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={handleHidePrivateKey}
                  >
                    <EyeOff className="h-4 w-4 mr-2" />
                    Hide Key
                  </Button>
                </div>
              ) : (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={handleViewPrivateKey}
                >
                  <Eye className="h-4 w-4 mr-2" />
                  View Private Key
                </Button>
              )}

              {privateKeyError && (
                <p className="text-sm text-destructive">{privateKeyError}</p>
              )}
            </div>
          )}
        </section>
      </div>

      {/* Wallet unlock dialog */}
      <WalletUnlockDialog
        open={unlockOpen}
        onOpenChange={setUnlockOpen}
        walletAlias={walletAlias}
        passwordHint={wallet.passwordHint ?? null}
        error={unlockError}
        onResult={handleUnlockResult}
      />
    </Island>
  );
}
