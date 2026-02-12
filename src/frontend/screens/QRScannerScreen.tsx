import { useState, useMemo, useCallback, useEffect } from "react";
import { useTaskListener } from "@/hooks/useTaskListener";
import type { TaskResultEvent, TaskErrorEvent } from "@/bindings";
import {
  ArrowLeft,
  Loader2,
  ScanLine,
  CheckCircle,
  AlertTriangle,
  Info,
  Zap,
  Lock,
} from "lucide-react";
import { useNavigate } from "@tanstack/react-router";
import { Island } from "@/components/layout/Island";
import { EmptyState } from "@/components/feedback/EmptyState";
import { WalletUnlockDialog, type WalletUnlockResult } from "@/components/shared";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Separator } from "@/components/ui/separator";
import {
  IdentitySelector,
  type IdentityOption,
} from "@/components/shared/IdentitySelector";
import { commands } from "@/bindings";
import { useIdentityStore } from "@/stores/identityStore";
import { useWalletStore } from "@/stores/walletStore";
import { useDashPayStore } from "@/stores/dashpayStore";
import type { IdentityKeyDto } from "@/bindings";

// ─── Types ────────────────────────────────────────────────────────────

interface ParsedQRData {
  identityId: string;
  proofKeyHex: string;
  accountReference: number;
  expiresAt: number;
}

type ScreenState = "form" | "sending" | "success" | "error";

// ─── Component ────────────────────────────────────────────────────────

export function QRScannerScreen() {
  const navigate = useNavigate();

  // Stores
  const identities = useIdentityStore((s) => s.identities);
  const selectedIdentityId = useDashPayStore((s) => s.selectedIdentityId);
  const wallets = useWalletStore((s) => s.hdWallets);

  // Local state
  const [identityId, setIdentityId] = useState(selectedIdentityId ?? "");
  const [qrDataInput, setQrDataInput] = useState("");
  const [parsedData, setParsedData] = useState<ParsedQRData | null>(null);
  const [parsing, setParsing] = useState(false);
  const [screenState, setScreenState] = useState<ScreenState>("form");
  const [error, setError] = useState<string | null>(null);
  const [successMsg, setSuccessMsg] = useState<string | null>(null);
  const [showWalletUnlock, setShowWalletUnlock] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);

  // Identity options
  const identityOptions: IdentityOption[] = useMemo(
    () =>
      identities.map((i) => ({
        id: i.id,
        displayName: i.alias?.trim() || i.id.slice(0, 12) + "...",
      })),
    [identities],
  );

  // Find signing key for selected identity (AUTHENTICATION + ECDSA)
  const signingKey = useMemo((): IdentityKeyDto | null => {
    const identity = identities.find((i) => i.id === identityId);
    if (!identity?.keys) return null;
    return (
      identity.keys.find(
        (k) =>
          k.purpose === "AUTHENTICATION" &&
          k.keyType === "ECDSA_SECP256K1" &&
          !k.disabledAt &&
          (k.securityLevel === "CRITICAL" ||
            k.securityLevel === "HIGH" ||
            k.securityLevel === "MEDIUM"),
      ) ?? null
    );
  }, [identityId, identities]);

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

  // Handlers
  const handleIdentityChange = useCallback((id: string) => {
    setIdentityId(id);
    setError(null);
    setSuccessMsg(null);
  }, []);

  const handleParse = useCallback(async () => {
    if (!qrDataInput.trim()) {
      setError("Please enter QR code data.");
      setParsedData(null);
      return;
    }

    setParsing(true);
    setError(null);
    setSuccessMsg(null);

    try {
      const result = await commands.dashpayParseAutoAcceptProof({
        qrData: qrDataInput.trim(),
      });
      if (result.status === "ok") {
        setParsedData(result.data);
        setSuccessMsg("QR code parsed successfully.");
      } else {
        setParsedData(null);
        setError(result.error);
      }
    } catch (e) {
      setParsedData(null);
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setParsing(false);
    }
  }, [qrDataInput]);

  // Listen for task completion/error
  useTaskListener(
    taskId,
    useCallback((_event: TaskResultEvent) => {
      setSuccessMsg("Contact request sent successfully! The contact will be automatically accepted.");
      setScreenState("success");
      setTaskId(null);
    }, []),
    useCallback((event: TaskErrorEvent) => {
      setError(event.message);
      setScreenState("error");
      setTaskId(null);
    }, []),
  );

  // Timeout for QR contact requests (120s) — this screen bypasses the store
  useEffect(() => {
    if (!taskId) return;
    const timer = setTimeout(() => {
      setError("Operation timed out. The task may still complete in the background.");
      setScreenState("error");
      setTaskId(null);
    }, 120_000);
    return () => clearTimeout(timer);
  }, [taskId]);

  const handleClear = useCallback(() => {
    setQrDataInput("");
    setParsedData(null);
    setError(null);
    setSuccessMsg(null);
    setTaskId(null);
    setScreenState("form");
  }, []);

  const handleAddContact = useCallback(async () => {
    if (!identityId) {
      setError("Please select an identity.");
      return;
    }
    if (!parsedData) {
      setError("Please parse a QR code first.");
      return;
    }
    if (!signingKey) {
      setError(
        "No suitable signing key found. This operation requires an ECDSA_SECP256K1 AUTHENTICATION key.",
      );
      return;
    }

    setScreenState("sending");
    setError(null);

    try {
      const result = await commands.dashpaySendContactRequestWithProof({
        identityId,
        signingKeyId: signingKey.keyId,
        toIdentityId: parsedData.identityId,
        accountLabel: `QR Contact (Account #${parsedData.accountReference})`,
        proofIdentityId: parsedData.identityId,
        proofKeyHex: parsedData.proofKeyHex,
        proofAccountReference: parsedData.accountReference,
        proofExpiresAt: parsedData.expiresAt,
      });
      if (result.status === "ok") {
        // Task dispatched — stay in "sending", wait for task event
        setTaskId(result.data.taskId);
      } else {
        setScreenState("error");
        setError(result.error);
      }
    } catch (e) {
      setScreenState("error");
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [identityId, parsedData, signingKey]);

  const handleBack = useCallback(() => {
    navigate({ to: "/dashpay/contacts" });
  }, [navigate]);

  const handleWalletUnlockResult = useCallback(
    (result: WalletUnlockResult) => {
      setShowWalletUnlock(false);
      if (result.status === "unlocked") {
        setError(null);
      }
    },
    [],
  );

  // Expiration text
  const expiryText = useMemo(() => {
    if (!parsedData) return "";
    const date = new Date(parsedData.expiresAt * 1000);
    const now = Date.now();
    if (date.getTime() < now) return "EXPIRED";
    return date.toLocaleString();
  }, [parsedData]);

  const isExpired = useMemo(() => {
    if (!parsedData) return false;
    return parsedData.expiresAt * 1000 < Date.now();
  }, [parsedData]);

  // ─── No identities state ───────────────────────────────────────────

  if (identities.length === 0) {
    return (
      <Island className="flex-1">
        <EmptyState
          icon={ScanLine}
          title="No Identities Loaded"
          description="You need at least one identity to scan a contact QR code."
          actionLabel="Load Identity"
          onAction={() => navigate({ to: "/identities" })}
        />
      </Island>
    );
  }

  // ─── Success state ──────────────────────────────────────────────────

  if (screenState === "success") {
    return (
      <Island className="flex-1">
        <div className="flex flex-col items-center justify-center py-12 gap-6">
          <div className="rounded-full bg-green-500/10 p-4">
            <CheckCircle className="h-12 w-12 text-green-500" />
          </div>
          <div className="text-center space-y-2">
            <h2 className="text-xl font-semibold">Contact Added!</h2>
            <p className="text-sm text-muted-foreground max-w-md">
              {successMsg}
            </p>
          </div>
          <div className="flex gap-3">
            <Button variant="outline" onClick={handleClear}>
              Scan Another
            </Button>
            <Button onClick={handleBack}>Back to Contacts</Button>
          </div>
        </div>
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
        <h2 className="text-xl font-semibold flex-1">Scan Contact QR Code</h2>
      </div>

      <Separator className="mb-6" />

      {/* Status messages (only show top-level error in form state; step 3 has its own error display) */}
      {error && screenState === "form" && (
        <div className="mb-4 rounded-lg border border-destructive/50 bg-destructive/10 p-3 text-sm text-destructive" role="alert">
          {error}
        </div>
      )}
      {successMsg && !error && screenState === "form" && (
        <div className="mb-4 rounded-lg border border-green-500/50 bg-green-500/10 p-3 text-sm text-green-700 dark:text-green-400" role="status">
          {successMsg}
        </div>
      )}

      <div className="space-y-6">
        {/* Step 1: Select identity */}
        <div className="rounded-lg border p-4 space-y-3">
          <h3 className="text-sm font-semibold">1. Select Your Identity</h3>
          <Separator />
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
          {identityId && !signingKey && (
            <p className="text-xs text-amber-600 dark:text-amber-400 flex items-center gap-1">
              <AlertTriangle className="h-3 w-3" />
              No suitable ECDSA AUTHENTICATION key found for this identity.
            </p>
          )}
        </div>

        {/* Step 2: Enter QR data */}
        <div className="rounded-lg border p-4 space-y-3">
          <h3 className="text-sm font-semibold">2. Enter QR Code Data</h3>
          <Separator />
          <p className="text-xs text-muted-foreground">
            Paste the QR code data below:
          </p>
          <Textarea
            value={qrDataInput}
            onChange={(e) => setQrDataInput(e.target.value)}
            placeholder="dash:?di=..."
            rows={3}
            className="font-mono text-xs"
            aria-label="QR code data"
          />
          <div className="flex gap-2">
            <Button
              onClick={handleParse}
              disabled={!qrDataInput.trim() || parsing}
              size="sm"
            >
              {parsing ? (
                <>
                  <Loader2 className="h-4 w-4 mr-1 animate-spin" />
                  Parsing...
                </>
              ) : (
                "Parse QR Code"
              )}
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={handleClear}
              disabled={!qrDataInput}
            >
              Clear
            </Button>
          </div>
        </div>

        {/* Step 3: Parsed details + Add Contact */}
        {parsedData && (
          <div className="rounded-lg border p-4 space-y-3">
            <h3 className="text-sm font-semibold">3. QR Code Details</h3>
            <Separator />

            <div className="grid grid-cols-[140px_1fr] gap-2 text-sm">
              <span className="text-muted-foreground">Contact Identity:</span>
              <span className="font-mono text-xs break-all">
                {parsedData.identityId}
              </span>

              <span className="text-muted-foreground">Account Reference:</span>
              <span>{parsedData.accountReference}</span>

              <span className="text-muted-foreground">Expires:</span>
              <span className={isExpired ? "text-destructive font-medium" : ""}>
                {expiryText}
              </span>
            </div>

            <Separator />

            {isExpired ? (
              <div className="rounded-lg border border-destructive/50 bg-destructive/10 p-3 text-sm text-destructive flex items-center gap-2">
                <AlertTriangle className="h-4 w-4 shrink-0" />
                This QR code has expired and cannot be used.
              </div>
            ) : walletNeedsPassword ? (
              <div className="flex items-center gap-2 rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2">
                <Lock className="h-3.5 w-3.5 text-amber-500" />
                <span className="text-sm text-amber-600 dark:text-amber-400">
                  Wallet is locked. Please unlock to add contact.
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
            ) : screenState === "sending" ? (
              <div className="flex items-center gap-2 text-sm text-muted-foreground py-2">
                <Loader2 className="h-4 w-4 animate-spin" />
                Sending contact request...
              </div>
            ) : screenState === "error" ? (
              <div className="space-y-3">
                <div className="rounded-lg border border-destructive/50 bg-destructive/10 p-3 text-sm text-destructive" role="alert">
                  {error}
                </div>
                <Button size="sm" onClick={handleAddContact}>
                  Retry
                </Button>
              </div>
            ) : (
              <Button
                onClick={handleAddContact}
                disabled={!identityId || !signingKey}
              >
                Add Contact
              </Button>
            )}

            <Separator />

            <div className="space-y-1">
              <p className="text-xs text-muted-foreground flex items-center gap-1">
                <Info className="h-3 w-3" />
                This will send a contact request that will be automatically accepted.
              </p>
              <p className="text-xs text-muted-foreground flex items-center gap-1">
                <Zap className="h-3 w-3" />
                Both you and the contact will become mutual contacts instantly.
              </p>
            </div>
          </div>
        )}

        {/* Information box */}
        <div className="rounded-lg border p-4 space-y-2">
          <h3 className="text-sm font-semibold flex items-center gap-1">
            <Info className="h-4 w-4" />
            About QR Code Scanning
          </h3>
          <Separator />
          <ul className="space-y-1 text-xs text-muted-foreground">
            <li>• QR codes enable instant mutual contact establishment</li>
            <li>• The contact request is automatically accepted by both parties</li>
            <li>• No manual approval is needed when using valid QR codes</li>
            <li>• QR codes expire after the specified time period</li>
            <li>• Each QR code can only be used once</li>
          </ul>
        </div>
      </div>

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
