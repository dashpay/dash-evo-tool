import { useState, useMemo, useCallback } from "react";
import {
  ArrowLeft,
  Loader2,
  QrCode,
  Copy,
  Check,
  Info,
  ChevronDown,
  AlertTriangle,
  Lock,
} from "lucide-react";
import { useNavigate } from "@tanstack/react-router";
import { QRCodeSVG } from "qrcode.react";
import { Island } from "@/components/layout/Island";
import { EmptyState } from "@/components/feedback/EmptyState";
import { WalletUnlockDialog } from "@/components/shared";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Checkbox } from "@/components/ui/checkbox";
import { Separator } from "@/components/ui/separator";
import {
  IdentitySelector,
  type IdentityOption,
} from "@/components/shared/IdentitySelector";
import { commands } from "@/bindings";
import { useIdentityStore } from "@/stores/identityStore";
import { useWalletStore } from "@/stores/walletStore";
import { useDashPayStore } from "@/stores/dashpayStore";
import { cn } from "@/lib/utils";

// ─── Constants ────────────────────────────────────────────────────────

const QR_CODE_INFO = [
  "QR codes allow instant mutual contact establishment.",
  "The recipient can scan to automatically send and accept contact requests.",
  "QR codes expire after the specified validity period.",
  "Each QR code is unique and can only be used once.",
  "WARNING: Anyone with this QR code can automatically become your contact.",
];

const ACCOUNT_INDEX_INFO =
  "The account index determines which HD wallet account is used for this contact relationship. " +
  "Most users should leave this at 0 (the default). Advanced users may use different indices to " +
  "segregate contacts (e.g., separate personal and business contacts).";

// ─── Component ────────────────────────────────────────────────────────

export function QRCodeGeneratorScreen() {
  const navigate = useNavigate();

  // Stores
  const identities = useIdentityStore((s) => s.identities);
  const selectedIdentityId = useDashPayStore((s) => s.selectedIdentityId);
  const wallets = useWalletStore((s) => s.hdWallets);

  // Local state
  const [identityId, setIdentityId] = useState(selectedIdentityId ?? "");
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [accountIndex, setAccountIndex] = useState("0");
  const [validityHours, setValidityHours] = useState("24");
  const [generating, setGenerating] = useState(false);
  const [qrString, setQrString] = useState<string | null>(null);
  const [qrMeta, setQrMeta] = useState<{ accountReference: number; expiresAt: number } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [successMsg, setSuccessMsg] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [showInfoDialog, setShowInfoDialog] = useState(false);
  const [showWalletUnlock, setShowWalletUnlock] = useState(false);
  const [dataExpanded, setDataExpanded] = useState(false);

  // Identity options
  const identityOptions: IdentityOption[] = useMemo(
    () =>
      identities.map((i) => ({
        id: i.id,
        displayName: i.alias?.trim() || i.id.slice(0, 12) + "...",
      })),
    [identities],
  );

  // Associated wallet and lock status
  const associatedWallet = useMemo(() => {
    const identity = identities.find((i) => i.id === identityId);
    if (!identity?.associatedWalletHashes?.length) return null;
    return wallets.find(
      (w) => identity.associatedWalletHashes.includes(w.seedHash),
    ) ?? null;
  }, [identityId, identities, wallets]);

  const walletNeedsPassword = associatedWallet?.usesPassword ?? false;
  const walletAlias = associatedWallet?.alias ?? "Wallet";

  // Validation
  const validAccountIndex = useMemo(() => {
    const n = parseInt(accountIndex, 10);
    return !isNaN(n) && n >= 0;
  }, [accountIndex]);

  const validValidity = useMemo(() => {
    const n = parseInt(validityHours, 10);
    return !isNaN(n) && n >= 1 && n <= 720;
  }, [validityHours]);

  const canGenerate = identityId && validAccountIndex && validValidity;

  // Handlers
  const handleIdentityChange = useCallback(
    (id: string) => {
      setIdentityId(id);
      setQrString(null);
      setQrMeta(null);
      setError(null);
      setSuccessMsg(null);
    },
    [],
  );

  const handleGenerate = useCallback(async () => {
    if (!identityId) {
      setError("Please select an identity first.");
      return;
    }
    const acctIdx = parseInt(accountIndex, 10);
    if (isNaN(acctIdx) || acctIdx < 0) {
      setError("Invalid account index number.");
      return;
    }
    const validity = parseInt(validityHours, 10);
    if (isNaN(validity) || validity < 1 || validity > 720) {
      setError("Validity hours must be between 1 and 720.");
      return;
    }

    setGenerating(true);
    setError(null);
    setSuccessMsg(null);

    try {
      const result = await commands.dashpayGenerateAutoAcceptProof({
        identityId,
        accountIndex: acctIdx,
        validityHours: validity,
      });
      if (result.status === "ok") {
        setQrString(result.data.qrString);
        setQrMeta({
          accountReference: result.data.accountReference,
          expiresAt: result.data.expiresAt,
        });
        setSuccessMsg("QR code generated successfully.");
      } else {
        setError(result.error);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setGenerating(false);
    }
  }, [identityId, accountIndex, validityHours]);

  const handleClear = useCallback(() => {
    setQrString(null);
    setQrMeta(null);
    setError(null);
    setSuccessMsg(null);
    setDataExpanded(false);
  }, []);

  const handleCopy = useCallback(() => {
    if (qrString) {
      navigator.clipboard.writeText(qrString).then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
      });
    }
  }, [qrString]);

  const handleBack = useCallback(() => {
    navigate({ to: "/dashpay/contacts" });
  }, [navigate]);

  const handleWalletUnlockResult = useCallback(
    (result: { success: boolean }) => {
      setShowWalletUnlock(false);
      if (result.success) {
        setError(null);
      }
    },
    [],
  );

  // ─── No identities state ───────────────────────────────────────────

  if (identities.length === 0) {
    return (
      <Island className="flex-1">
        <EmptyState
          icon={QrCode}
          title="No Identities Loaded"
          description="You need at least one identity to generate a contact QR code."
          actionLabel="Load Identity"
          onAction={() => navigate({ to: "/identities" })}
        />
      </Island>
    );
  }

  // ─── Render ─────────────────────────────────────────────────────────

  return (
    <Island className="flex-1 overflow-y-auto">
      {/* Header */}
      <div className="flex items-center gap-3 mb-6">
        <Button variant="ghost" size="sm" onClick={handleBack} aria-label="Back">
          <ArrowLeft className="h-4 w-4 mr-1" />
          Back
        </Button>
        <h2 className="text-xl font-semibold flex-1">Generate Contact QR Code</h2>
        <Button
          variant="ghost"
          size="icon"
          onClick={() => setShowInfoDialog(true)}
          aria-label="QR code information"
        >
          <Info className="h-4 w-4" />
        </Button>
        <label className="flex items-center gap-2 text-sm cursor-pointer select-none">
          <Checkbox
            checked={showAdvanced}
            onCheckedChange={(checked) => setShowAdvanced(!!checked)}
            aria-label="Show advanced options"
          />
          Advanced Options
        </label>
      </div>

      <Separator className="mb-6" />

      {/* Status messages */}
      {error && (
        <div className="mb-4 rounded-lg border border-destructive/50 bg-destructive/10 p-3 text-sm text-destructive" role="alert">
          {error}
        </div>
      )}
      {successMsg && !error && (
        <div className="mb-4 rounded-lg border border-green-500/50 bg-green-500/10 p-3 text-sm text-green-700 dark:text-green-400" role="status">
          {successMsg}
        </div>
      )}

      {/* Configuration */}
      <div className="space-y-6">
        <div className="rounded-lg border p-4 space-y-4">
          <h3 className="text-sm font-semibold">Configuration</h3>
          <Separator />

          {/* Identity selector */}
          <div className="grid grid-cols-[120px_1fr] items-center gap-3">
            <Label>Identity:</Label>
            <IdentitySelector
              value={identityId}
              onChange={handleIdentityChange}
              identities={identityOptions}
              placeholder="Select identity"
              showOther={false}
            />
          </div>

          {/* Advanced options */}
          {showAdvanced && (
            <>
              <Separator />
              <div className="grid grid-cols-[120px_1fr] items-center gap-3">
                <div className="flex items-center gap-1">
                  <Label>Account Index:</Label>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-5 w-5"
                    onClick={() => setShowInfoDialog(true)}
                    aria-label="Account index info"
                  >
                    <Info className="h-3 w-3" />
                  </Button>
                </div>
                <Input
                  type="number"
                  min={0}
                  value={accountIndex}
                  onChange={(e) => setAccountIndex(e.target.value)}
                  placeholder="0"
                  className="w-32"
                  aria-label="Account index"
                />
              </div>

              <div className="grid grid-cols-[120px_1fr] items-center gap-3">
                <Label>Validity (hours):</Label>
                <div className="flex items-center gap-2">
                  <Input
                    type="number"
                    min={1}
                    max={720}
                    value={validityHours}
                    onChange={(e) => setValidityHours(e.target.value)}
                    placeholder="24"
                    className="w-32"
                    aria-label="Validity hours"
                  />
                  <span className="text-xs text-muted-foreground">
                    How long the QR code remains valid (1-720, default: 24)
                  </span>
                </div>
              </div>
            </>
          )}

          {/* Wallet locked warning */}
          {walletNeedsPassword && (
            <div className="flex items-center gap-2 rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2">
              <Lock className="h-3.5 w-3.5 text-amber-500" />
              <span className="text-sm text-amber-600 dark:text-amber-400">
                Wallet is locked. Please unlock to generate QR code.
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

          {/* Generate button */}
          <div className="flex gap-2 pt-2">
            <Button
              onClick={handleGenerate}
              disabled={!canGenerate || generating || walletNeedsPassword}
            >
              {generating ? (
                <>
                  <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                  Generating...
                </>
              ) : (
                <>
                  <QrCode className="h-4 w-4 mr-2" />
                  Generate QR Code
                </>
              )}
            </Button>
            {qrString && (
              <Button variant="outline" onClick={handleClear}>
                Clear
              </Button>
            )}
          </div>
        </div>

        {/* Generated QR code display */}
        {qrString && (
          <div className="rounded-lg border p-4 space-y-4">
            <h3 className="text-sm font-semibold">Generated QR Code</h3>
            <Separator />

            {/* QR code image */}
            <div className="flex justify-center py-4" data-testid="qr-code">
              <QRCodeSVG
                value={qrString}
                size={256}
                level="M"
                includeMargin
              />
            </div>

            {/* Expiration info */}
            {qrMeta && (
              <div className="text-sm text-center text-muted-foreground">
                Expires: {new Date(qrMeta.expiresAt * 1000).toLocaleString()}
              </div>
            )}

            {/* Expandable text data */}
            <Button
              variant="ghost"
              size="sm"
              className="w-full justify-between"
              onClick={() => setDataExpanded(!dataExpanded)}
            >
              QR Code Data (text)
              <ChevronDown
                className={cn("h-4 w-4 transition-transform", dataExpanded && "rotate-180")}
              />
            </Button>
            {dataExpanded && (
              <pre className="rounded-md bg-muted p-3 text-xs font-mono break-all whitespace-pre-wrap">
                {qrString}
              </pre>
            )}

            {/* Copy button */}
            <div className="flex justify-center">
              <Button variant="outline" onClick={handleCopy}>
                {copied ? (
                  <>
                    <Check className="h-4 w-4 mr-2" />
                    Copied!
                  </>
                ) : (
                  <>
                    <Copy className="h-4 w-4 mr-2" />
                    Copy Data to Clipboard
                  </>
                )}
              </Button>
            </div>

            <Separator />

            <p className="text-xs text-muted-foreground text-center">
              Share this QR code with someone to establish a mutual contact.
            </p>
            <p className="text-xs text-center flex items-center justify-center gap-1 text-amber-600 dark:text-amber-400">
              <AlertTriangle className="h-3 w-3" />
              Anyone with this QR code can automatically become your contact.
            </p>
          </div>
        )}
      </div>

      {/* Info dialog */}
      <Dialog open={showInfoDialog} onOpenChange={setShowInfoDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>About Contact QR Codes</DialogTitle>
            <DialogDescription>
              Learn about QR code contact establishment.
            </DialogDescription>
          </DialogHeader>
          <ul className="space-y-2 text-sm">
            {QR_CODE_INFO.map((text, i) => (
              <li key={i} className="flex gap-2">
                <span className="text-muted-foreground shrink-0">•</span>
                <span className={text.startsWith("WARNING") ? "text-amber-600 dark:text-amber-400 font-medium" : ""}>
                  {text}
                </span>
              </li>
            ))}
          </ul>
          {showAdvanced && (
            <>
              <Separator />
              <div>
                <h4 className="text-sm font-medium mb-1">Account Index</h4>
                <p className="text-sm text-muted-foreground">{ACCOUNT_INDEX_INFO}</p>
              </div>
            </>
          )}
        </DialogContent>
      </Dialog>

      {/* Wallet unlock dialog */}
      <WalletUnlockDialog
        open={showWalletUnlock}
        onOpenChange={setShowWalletUnlock}
        walletAlias={walletAlias}
        onResult={handleWalletUnlockResult}
      />
    </Island>
  );
}
