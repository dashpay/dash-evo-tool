import { useCallback, useEffect, useMemo, useState, useRef } from "react";
import { validateCoreAddress } from "@/lib/validateAddress";
import { useNavigate } from "@tanstack/react-router";
import { Island } from "@/components/layout";
import { AmountInput, formatAmount } from "@/components/shared/AmountInput";
import { WalletUnlockDialog } from "@/components/shared/WalletUnlockDialog";
import { ConfirmationDialog } from "@/components/shared/ConfirmationDialog";
import { FeeConfirmationDialog } from "@/components/shared/FeeConfirmationDialog";
import type { FeeConfirmationResult } from "@/components/shared/FeeConfirmationDialog";
import { parseMinRelayFeeError } from "@/lib/feeUtils";
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
import { Checkbox } from "@/components/ui/checkbox";
import { useWalletStore } from "@/stores/walletStore";
import { commands, events } from "@/bindings";
import type { WalletUnlockResult } from "@/components/shared/WalletUnlockDialog";
import type {
  WalletDto,
  SingleKeyWalletDto,
  QualifiedIdentityDto,
  PlatformAddressDto,
  WalletAddressDto,
} from "@/bindings";
import {
  ArrowLeft,
  Send,
  Loader2,
  CheckCircle2,
  AlertCircle,
  X,
  Plus,
  Trash2,
} from "lucide-react";

// ─── Constants ──────────────────────────────────────────────────────

const CREDITS_PER_DUFF = 1000;
const DUFFS_PER_DASH = 100_000_000;

// ─── Address type detection ─────────────────────────────────────────

type AddressType = "core" | "platform" | "unknown";

function detectAddressType(address: string): AddressType {
  const trimmed = address.trim();
  if (!trimmed) return "unknown";
  if (trimmed.startsWith("evo1") || trimmed.startsWith("tevo1"))
    return "platform";
  // Dash addresses start with X (mainnet P2PKH), y (mainnet P2SH),
  // or 8/9 (testnet P2PKH/P2SH)
  if (/^[Xy789][a-km-zA-HJ-NP-Z1-9]{24,}$/.test(trimmed)) return "core";
  return "unknown";
}

// ─── Source selection ───────────────────────────────────────────────

type SourceSelection =
  | { type: "core" }
  | { type: "platform"; addresses: PlatformAddressDto[] }
  | { type: "identity"; identity: QualifiedIdentityDto };

// ─── Send status ────────────────────────────────────────────────────

type SendStatus =
  | { state: "idle" }
  | { state: "sending"; startTime: number; taskId?: string }
  | { state: "complete"; message: string }
  | { state: "error"; message: string };

// ─── Advanced mode types ────────────────────────────────────────────

type AdvancedSourceType = "core" | "platform";

interface AdvancedInput {
  address: string;
  balance: number;
  amount: string;
}

interface AdvancedOutput {
  address: string;
  amount: string;
}

type FeeStrategy =
  | "deductFromFirstInput"
  | "deductFromLastInput"
  | "reduceFirstOutput"
  | "reduceLastOutput";

// ─── Formatting helpers ─────────────────────────────────────────────

function formatDash(duffs: number): string {
  return formatAmount(duffs, 8) + " DASH";
}

function formatCredits(credits: number): string {
  // credits → duffs → DASH
  const duffs = credits / CREDITS_PER_DUFF;
  return formatAmount(Math.round(duffs), 8) + " DASH";
}

function duffsFromDashString(value: string): number | null {
  const num = parseFloat(value);
  if (isNaN(num) || num < 0) return null;
  return Math.round(num * DUFFS_PER_DASH);
}

function creditsFromDashString(value: string): number | null {
  const duffs = duffsFromDashString(value);
  if (duffs === null) return null;
  return duffs * CREDITS_PER_DUFF;
}

// ─── Transaction type description ───────────────────────────────────

function getTransactionTypeDescription(
  source: SourceSelection | null,
  destType: AddressType,
): string {
  if (!source) return "Send";
  switch (source.type) {
    case "core":
      return destType === "platform"
        ? "Fund Platform Address"
        : destType === "core"
          ? "Core Transaction"
          : "Send";
    case "platform":
      return destType === "platform"
        ? "Platform Transfer"
        : destType === "core"
          ? "Withdraw to Core"
          : "Send";
    case "identity":
      return destType === "core" ? "Identity Withdrawal" : "Send";
  }
}

// ─── SendScreen ─────────────────────────────────────────────────────

export function SendScreen() {
  const navigate = useNavigate();

  // Get wallet data from store
  const hdWallets = useWalletStore((s) => s.hdWallets);
  const singleKeyWallets = useWalletStore((s) => s.singleKeyWallets);
  const selectedWallet = useWalletStore((s) => s.selectedWallet);

  // Find the selected HD wallet
  const wallet: WalletDto | null =
    selectedWallet?.type === "hd"
      ? (hdWallets.find((w) => w.seedHash === selectedWallet.seedHash) ?? null)
      : null;

  // Find selected single-key wallet (for task 3.5)
  const singleKeyWallet: SingleKeyWalletDto | null =
    selectedWallet?.type === "singleKey"
      ? (singleKeyWallets.find(
          (w) => w.keyHash === selectedWallet.keyHash,
        ) ?? null)
      : null;

  // State
  const [source, setSource] = useState<SourceSelection | null>({
    type: "core",
  });
  const [destinationAddress, setDestinationAddress] = useState("");
  const [addressError, setAddressError] = useState<string | null>(null);
  const [amountValue, setAmountValue] = useState("");
  const [subtractFee, setSubtractFee] = useState(false);
  const [sendStatus, setSendStatus] = useState<SendStatus>({ state: "idle" });
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [identities, setIdentities] = useState<QualifiedIdentityDto[]>([]);
  const [selectedKeyId, setSelectedKeyId] = useState<number | null>(null);

  // Wallet unlock state
  const [unlockOpen, setUnlockOpen] = useState(false);
  const [unlockError, setUnlockError] = useState<string | null>(null);
  const [walletUnlocked, setWalletUnlocked] = useState(false);

  // Advanced mode state
  const [advSourceType, setAdvSourceType] =
    useState<AdvancedSourceType>("core");
  const [advInputs, setAdvInputs] = useState<AdvancedInput[]>([]);
  const [advOutputs, setAdvOutputs] = useState<AdvancedOutput[]>([
    { address: "", amount: "" },
  ]);
  const [advFeeStrategy, setAdvFeeStrategy] =
    useState<FeeStrategy>("deductFromFirstInput");

  // Confirmation dialog state
  const [showConfirmDialog, setShowConfirmDialog] = useState(false);
  const [pendingSendMode, setPendingSendMode] = useState<"simple" | "advanced" | null>(null);

  // Fee confirmation dialog state
  const [feeDialogOpen, setFeeDialogOpen] = useState(false);
  const [feeDialogEstimated, setFeeDialogEstimated] = useState(0);
  const [feeDialogRequired, setFeeDialogRequired] = useState(0);
  // Stores the pending Core→Core payment params for retry with override fee
  const pendingCorePaymentRef = useRef<{
    walletSeedHash: string;
    recipients: { address: string; amount: number }[];
    subtractFeeFromAmount: boolean;
    memo: string | null;
  } | null>(null);

  // Elapsed time for sending state
  const [elapsed, setElapsed] = useState(0);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Load identities with positive balance on mount
  useEffect(() => {
    commands
      .identityListLocal()
      .then((result) => {
        if (result.status === "ok") {
          const filtered = result.data.filter(
            (id) => id.balance > 0 && id.keys.length > 0,
          );
          setIdentities(filtered);
        }
      })
      .catch(() => {});
  }, []);

  // Check if wallet is already open (no password needed)
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

  // Track the active task ID in a ref so the listener effect doesn't re-subscribe
  const activeTaskIdRef = useRef<string | null>(null);
  useEffect(() => {
    activeTaskIdRef.current =
      sendStatus.state === "sending" ? (sendStatus.taskId ?? null) : null;
  }, [sendStatus]);

  // Listen for task result/error events (subscribe once)
  useEffect(() => {
    let cancelled = false;
    let cleanupResult: (() => void) | undefined;
    let cleanupError: (() => void) | undefined;

    const subscribe = async () => {
      cleanupResult = await events.taskResultEvent.listen((event) => {
        if (cancelled) return;
        const tid = activeTaskIdRef.current;
        if (!tid || event.payload.taskId !== tid) return;
        pendingCorePaymentRef.current = null;
        let message = "Transaction sent successfully!";
        if (event.payload.result.type === "walletCompleted") {
          message = "Transaction completed successfully!";
        } else if (event.payload.result.type === "identityCompleted") {
          message = "Identity operation completed successfully!";
        }
        setSendStatus({ state: "complete", message });
      });
      cleanupError = await events.taskErrorEvent.listen((event) => {
        if (cancelled) return;
        const tid = activeTaskIdRef.current;
        if (!tid || event.payload.taskId !== tid) return;
        // Check for min relay fee error — show fee confirmation dialog instead of error
        const requiredFee = parseMinRelayFeeError(event.payload.message);
        if (requiredFee !== null && pendingCorePaymentRef.current) {
          const match = event.payload.message.match(/(\d+)\s*</);
          const estimatedFee = match ? parseInt(match[1] ?? "", 10) : 0;
          setFeeDialogEstimated(estimatedFee);
          setFeeDialogRequired(requiredFee);
          setFeeDialogOpen(true);
          return;
        }
        setSendStatus({ state: "error", message: event.payload.message });
      });
    };
    subscribe().catch(console.error);

    return () => {
      cancelled = true;
      cleanupResult?.();
      cleanupError?.();
    };
  }, []); // Subscribe once on mount

  // ─── Derived values ─────────────────────────────────────────────

  const destType = detectAddressType(destinationAddress);
  const txTypeDesc = getTransactionTypeDescription(source, destType);
  const isIdentitySource = source?.type === "identity";

  // Core balance
  const coreBalance = wallet?.confirmedBalance ?? 0;

  // Platform addresses with positive balance
  const platformAddresses = useMemo(() => {
    if (!wallet) return [];
    return wallet.platformAddresses.filter((a) => a.balance > 0);
  }, [wallet]);

  const totalPlatformBalance = useMemo(
    () => platformAddresses.reduce((sum, a) => sum + a.balance, 0),
    [platformAddresses],
  );

  // Core addresses with balances (for advanced mode)
  const coreAddressesWithBalance = useMemo(() => {
    if (!wallet) return [];
    return wallet.addresses
      .filter((a) => a.balance > 0)
      .sort((a, b) => b.balance - a.balance);
  }, [wallet]);

  // Max amount based on source
  const maxAmountCredits = useMemo(() => {
    if (!source) return null;
    switch (source.type) {
      case "core":
        return wallet
          ? (wallet.confirmedBalance + wallet.unconfirmedBalance) *
              CREDITS_PER_DUFF
          : null;
      case "platform":
        return totalPlatformBalance;
      case "identity":
        return source.identity.balance;
    }
  }, [source, wallet, totalPlatformBalance]);

  // Debounced address validation for Core addresses
  const addrValidationTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    setAddressError(null);

    if (addrValidationTimer.current) {
      clearTimeout(addrValidationTimer.current);
    }

    const trimmed = destinationAddress.trim();
    // Only validate core-type addresses (platform addresses are handled separately)
    if (!trimmed || destType !== "core") return;

    addrValidationTimer.current = setTimeout(async () => {
      const error = await validateCoreAddress(trimmed);
      // Only set error if address hasn't changed
      setDestinationAddress((current) => {
        if (current.trim() === trimmed && error) {
          setAddressError(error);
        }
        return current;
      });
    }, 300);

    return () => {
      if (addrValidationTimer.current) {
        clearTimeout(addrValidationTimer.current);
      }
    };
  }, [destinationAddress, destType]);

  // Can we send?
  const canSend = useMemo(() => {
    if (sendStatus.state === "sending") return false;
    if (!source) return false;
    if (destType === "unknown") return false;
    if (addressError) return false;
    if (!amountValue || parseFloat(amountValue) <= 0) return false;
    if (isIdentitySource && destType !== "core") return false;
    if (!isIdentitySource && !walletUnlocked) return false;
    return true;
  }, [
    sendStatus.state,
    source,
    destType,
    addressError,
    amountValue,
    isIdentitySource,
    walletUnlocked,
  ]);

  // ─── Handlers ───────────────────────────────────────────────────

  const handleBack = useCallback(() => {
    navigate({ to: "/wallets" });
  }, [navigate]);

  const handleMaxClick = useCallback(() => {
    if (maxAmountCredits === null || maxAmountCredits <= 0) return;
    // Convert credits to DASH
    const duffs = maxAmountCredits / CREDITS_PER_DUFF;
    const dash = duffs / DUFFS_PER_DASH;
    setAmountValue(dash.toFixed(8));
    // Auto-enable subtract fee for core→core max
    if (source?.type === "core" && destType === "core") {
      setSubtractFee(true);
    }
  }, [maxAmountCredits, source, destType]);

  const resetForm = useCallback(() => {
    setSource({ type: "core" });
    setDestinationAddress("");
    setAddressError(null);
    setAmountValue("");
    setSubtractFee(false);
    setSendStatus({ state: "idle" });
    setSelectedKeyId(null);
    setAdvInputs([]);
    setAdvOutputs([{ address: "", amount: "" }]);
    setAdvFeeStrategy("deductFromFirstInput");
  }, []);

  const [unlockLoading, setUnlockLoading] = useState(false);
  const storeUnlockWallet = useWalletStore((s) => s.unlockWallet);

  const handleUnlockResult = useCallback(
    async (result: WalletUnlockResult) => {
      if (result.status !== "unlocked" || !wallet) {
        setUnlockOpen(false);
        return;
      }
      setUnlockError(null);
      setUnlockLoading(true);
      try {
        const err = await storeUnlockWallet(
          { type: "hd", seedHash: wallet.seedHash },
          result.password,
        );
        if (err) {
          setUnlockError(err);
          return; // Keep dialog open on error
        }
        setWalletUnlocked(true);
        setUnlockOpen(false);
      } catch (e) {
        setUnlockError(e instanceof Error ? e.message : String(e));
      } finally {
        setUnlockLoading(false);
      }
    },
    [wallet, storeUnlockWallet],
  );

  // ─── Simple mode send ──────────────────────────────────────────

  const handleSendClick = useCallback(() => {
    if (!source || !canSend) return;
    setPendingSendMode("simple");
    setShowConfirmDialog(true);
  }, [source, canSend]);

  const handleSend = useCallback(async () => {
    if (!source || !canSend) return;
    if (!wallet && !singleKeyWallet && source.type !== "identity") return;

    const seedHash = wallet?.seedHash ?? "";

    try {
      let taskId: string | undefined;

      if (source.type === "identity") {
        // Identity withdrawal to Core
        const credits = creditsFromDashString(amountValue);
        if (!credits || credits <= 0)
          throw new Error("Invalid amount");

        const result = await commands.identityWithdraw({
          identityId: source.identity.id,
          toAddress: destinationAddress.trim(),
          credits,
          keyId: selectedKeyId,
        });
        if (result.status === "error") throw new Error(result.error);
        taskId = result.data.taskId;
      } else if (source.type === "core" && destType === "core") {
        // Core → Core
        const duffs = duffsFromDashString(amountValue);
        if (!duffs || duffs <= 0) throw new Error("Invalid amount");
        if (duffs > coreBalance)
          throw new Error(
            `Insufficient balance. Need ${formatDash(duffs)} but have ${formatDash(coreBalance)}`,
          );

        const requestParams = {
          walletSeedHash: seedHash,
          recipients: [
            {
              address: destinationAddress.trim(),
              amount: duffs,
            },
          ],
          subtractFeeFromAmount: subtractFee,
          memo: null as string | null,
        };
        pendingCorePaymentRef.current = requestParams;

        const result = await commands.coreSendWalletPayment({
          ...requestParams,
          overrideFee: null,
        });
        if (result.status === "error") throw new Error(result.error);
        taskId = result.data.taskId;
      } else if (source.type === "core" && destType === "platform") {
        // Core → Platform (fund platform address)
        const duffs = duffsFromDashString(amountValue);
        if (!duffs || duffs <= 0) throw new Error("Invalid amount");
        if (duffs > coreBalance)
          throw new Error(
            `Insufficient balance. Need ${formatDash(duffs)} but have ${formatDash(coreBalance)}`,
          );

        const result = await commands.walletFundPlatformAddressFromUtxos({
          walletSeedHash: seedHash,
          amount: duffs,
          destination: destinationAddress.trim(),
          feeDeductFromOutput: true,
        });
        if (result.status === "error") throw new Error(result.error);
        taskId = result.data.taskId;
      } else if (source.type === "platform" && destType === "platform") {
        // Platform → Platform
        const credits = creditsFromDashString(amountValue);
        if (!credits || credits <= 0) throw new Error("Invalid amount");
        if (credits > totalPlatformBalance)
          throw new Error(
            `Insufficient balance. Need ${formatCredits(credits)} but have ${formatCredits(totalPlatformBalance)}`,
          );

        // Use all platform addresses as inputs, sorted by balance desc
        const sorted = [...platformAddresses].sort(
          (a, b) => b.balance - a.balance,
        );
        // Simple allocation: use highest-balance addresses until we have enough
        const inputs: { address: string; amount: number }[] = [];
        let remaining = credits;
        for (const addr of sorted) {
          if (remaining <= 0) break;
          if (addr.address === destinationAddress.trim()) continue;
          const use = Math.min(addr.balance, remaining);
          inputs.push({ address: addr.address, amount: use });
          remaining -= use;
        }

        if (inputs.length === 0)
          throw new Error("Cannot send to your own address");

        const result = await commands.walletTransferPlatformCredits({
          walletSeedHash: seedHash,
          inputs,
          outputs: [
            {
              address: destinationAddress.trim(),
              amount: credits,
            },
          ],
          feePayerIndex: 0,
        });
        if (result.status === "error") throw new Error(result.error);
        taskId = result.data.taskId;
      } else if (source.type === "platform" && destType === "core") {
        // Platform → Core (withdrawal)
        const credits = creditsFromDashString(amountValue);
        if (!credits || credits <= 0) throw new Error("Invalid amount");
        if (credits > totalPlatformBalance)
          throw new Error(
            `Insufficient balance. Need ${formatCredits(credits)} but have ${formatCredits(totalPlatformBalance)}`,
          );

        const sorted = [...platformAddresses].sort(
          (a, b) => b.balance - a.balance,
        );
        const inputs: { address: string; amount: number }[] = [];
        let remaining = credits;
        for (const addr of sorted) {
          if (remaining <= 0) break;
          const use = Math.min(addr.balance, remaining);
          inputs.push({ address: addr.address, amount: use });
          remaining -= use;
        }

        const result = await commands.walletWithdrawFromPlatformAddress({
          walletSeedHash: seedHash,
          inputs,
          coreAddress: destinationAddress.trim(),
          coreFeePerByte: 1,
          feePayerIndex: 0,
        });
        if (result.status === "error") throw new Error(result.error);
        taskId = result.data.taskId;
      } else {
        throw new Error("Invalid source/destination combination");
      }

      setSendStatus({
        state: "sending",
        startTime: Date.now(),
        taskId,
      });
    } catch (e) {
      setSendStatus({
        state: "error",
        message: e instanceof Error ? e.message : String(e),
      });
    }
  }, [
    source,
    canSend,
    wallet,
    singleKeyWallet,
    destType,
    amountValue,
    subtractFee,
    destinationAddress,
    coreBalance,
    totalPlatformBalance,
    platformAddresses,
    selectedKeyId,
  ]);

  // ─── Advanced mode send ────────────────────────────────────────

  const handleAdvancedSendClick = useCallback(() => {
    setPendingSendMode("advanced");
    setShowConfirmDialog(true);
  }, []);

  const handleAdvancedSend = useCallback(async () => {
    if (!wallet) return;
    const seedHash = wallet.seedHash;

    try {
      let taskId: string | undefined;

      // Determine output types
      const outputTypes = advOutputs.map((o) => detectAddressType(o.address));
      const hasCoreOutput = outputTypes.includes("core");
      const hasPlatformOutput = outputTypes.includes("platform");

      if (hasCoreOutput && hasPlatformOutput) {
        throw new Error(
          "Cannot mix Core and Platform address outputs in the same transaction",
        );
      }

      if (advSourceType === "core") {
        if (advInputs.length === 0)
          throw new Error("Please add at least one Core address input");

        if (hasCoreOutput) {
          // Advanced Core → Core
          const recipients = advOutputs
            .filter((o) => o.address.trim() && o.amount.trim())
            .map((o) => {
              const duffs = duffsFromDashString(o.amount);
              if (!duffs || duffs <= 0)
                throw new Error(`Invalid amount: ${o.amount}`);
              return { address: o.address.trim(), amount: duffs };
            });

          if (recipients.length === 0)
            throw new Error("No valid outputs specified");

          const advRequestParams = {
            walletSeedHash: seedHash,
            recipients,
            subtractFeeFromAmount: false,
            memo: null as string | null,
          };
          pendingCorePaymentRef.current = advRequestParams;

          const result = await commands.coreSendWalletPayment({
            ...advRequestParams,
            overrideFee: null,
          });
          if (result.status === "error") throw new Error(result.error);
          taskId = result.data.taskId;
        } else if (hasPlatformOutput) {
          // Advanced Core → Platform
          if (advOutputs.length !== 1)
            throw new Error(
              "Core to Platform currently only supports a single destination",
            );
          const firstOutput = advOutputs[0];
          if (!firstOutput) throw new Error("No output specified");
          const duffs = duffsFromDashString(firstOutput.amount);
          if (!duffs || duffs <= 0) throw new Error("Invalid amount");

          const feeDeductFromOutput =
            advFeeStrategy === "reduceFirstOutput" ||
            advFeeStrategy === "reduceLastOutput";

          const result = await commands.walletFundPlatformAddressFromUtxos({
            walletSeedHash: seedHash,
            amount: duffs,
            destination: firstOutput.address.trim(),
            feeDeductFromOutput,
          });
          if (result.status === "error") throw new Error(result.error);
          taskId = result.data.taskId;
        } else {
          throw new Error("Invalid output address");
        }
      } else {
        // Platform source
        if (advInputs.length === 0)
          throw new Error("Please add at least one Platform address input");

        const inputs = advInputs
          .filter((i) => i.amount.trim())
          .map((i) => {
            const credits = creditsFromDashString(i.amount);
            if (!credits || credits <= 0)
              throw new Error(`Invalid input amount: ${i.amount}`);
            return { address: i.address, amount: credits };
          });

        if (inputs.length === 0)
          throw new Error("No valid Platform inputs specified");

        if (hasPlatformOutput) {
          // Advanced Platform → Platform
          const outputs = advOutputs
            .filter((o) => o.address.trim() && o.amount.trim())
            .map((o) => {
              const credits = creditsFromDashString(o.amount);
              if (!credits || credits <= 0)
                throw new Error(`Invalid output amount: ${o.amount}`);
              return { address: o.address.trim(), amount: credits };
            });

          if (outputs.length === 0)
            throw new Error("No valid Platform outputs specified");

          const result = await commands.walletTransferPlatformCredits({
            walletSeedHash: seedHash,
            inputs,
            outputs,
            feePayerIndex: 0,
          });
          if (result.status === "error") throw new Error(result.error);
          taskId = result.data.taskId;
        } else if (hasCoreOutput) {
          // Advanced Platform → Core (withdrawal)
          if (advOutputs.length !== 1)
            throw new Error(
              "Withdrawal currently only supports a single Core destination",
            );
          const coreOutput = advOutputs[0];
          if (!coreOutput) throw new Error("No output specified");

          const result = await commands.walletWithdrawFromPlatformAddress({
            walletSeedHash: seedHash,
            inputs,
            coreAddress: coreOutput.address.trim(),
            coreFeePerByte: 1,
            feePayerIndex: 0,
          });
          if (result.status === "error") throw new Error(result.error);
          taskId = result.data.taskId;
        } else {
          throw new Error("Invalid output address");
        }
      }

      setSendStatus({
        state: "sending",
        startTime: Date.now(),
        taskId,
      });
    } catch (e) {
      setSendStatus({
        state: "error",
        message: e instanceof Error ? e.message : String(e),
      });
    }
  }, [wallet, advSourceType, advInputs, advOutputs, advFeeStrategy]);

  // ─── Confirmation dialog ────────────────────────────────────────

  const confirmMessage = useMemo(() => {
    if (pendingSendMode === "advanced") {
      const totalDash = advOutputs.reduce((sum, o) => {
        const v = parseFloat(o.amount);
        return sum + (isNaN(v) ? 0 : v);
      }, 0);
      const destCount = advOutputs.filter((o) => o.address.trim()).length;
      return `Send ${totalDash.toFixed(8)} DASH to ${destCount} recipient${destCount !== 1 ? "s" : ""}?\n\nThis transaction cannot be reversed.`;
    }
    // Simple mode
    const addrShort = destinationAddress.trim().length > 16
      ? `${destinationAddress.trim().slice(0, 8)}...${destinationAddress.trim().slice(-8)}`
      : destinationAddress.trim();
    return `Send ${amountValue || "0"} DASH to ${addrShort}?\n\nTransaction type: ${txTypeDesc}. This cannot be reversed.`;
  }, [pendingSendMode, advOutputs, destinationAddress, amountValue, txTypeDesc]);

  const handleConfirmResult = useCallback((status: "confirmed" | "canceled") => {
    if (status === "confirmed") {
      if (pendingSendMode === "advanced") {
        handleAdvancedSend();
      } else {
        handleSend();
      }
    }
    setPendingSendMode(null);
  }, [pendingSendMode, handleAdvancedSend, handleSend]);

  // ─── Fee confirmation ──────────────────────────────────────────

  const handleFeeConfirmResult = useCallback(
    async (result: FeeConfirmationResult) => {
      if (result.status === "confirmed" && pendingCorePaymentRef.current) {
        // Retry send with the override fee
        try {
          const sendResult = await commands.coreSendWalletPayment({
            ...pendingCorePaymentRef.current,
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
        pendingCorePaymentRef.current = null;
        setSendStatus({ state: "idle" });
      }
    },
    [],
  );

  // ─── Advanced input management ─────────────────────────────────

  const addAdvancedCoreInput = useCallback(
    (addr: WalletAddressDto) => {
      if (advInputs.some((i) => i.address === addr.address)) return;
      setAdvInputs((prev) => [
        ...prev,
        { address: addr.address, balance: addr.balance, amount: "" },
      ]);
    },
    [advInputs],
  );

  const addAdvancedPlatformInput = useCallback(
    (addr: PlatformAddressDto) => {
      if (advInputs.some((i) => i.address === addr.address)) return;
      setAdvInputs((prev) => [
        ...prev,
        { address: addr.address, balance: addr.balance, amount: "" },
      ]);
    },
    [advInputs],
  );

  const removeAdvancedInput = useCallback((index: number) => {
    setAdvInputs((prev) => prev.filter((_, i) => i !== index));
  }, []);

  const updateAdvancedInputAmount = useCallback(
    (index: number, amount: string) => {
      setAdvInputs((prev) =>
        prev.map((input, i) => (i === index ? { ...input, amount } : input)),
      );
    },
    [],
  );

  const addAdvancedOutput = useCallback(() => {
    setAdvOutputs((prev) => [...prev, { address: "", amount: "" }]);
  }, []);

  const removeAdvancedOutput = useCallback((index: number) => {
    setAdvOutputs((prev) => prev.filter((_, i) => i !== index));
  }, []);

  const updateAdvancedOutput = useCallback(
    (index: number, field: "address" | "amount", value: string) => {
      setAdvOutputs((prev) =>
        prev.map((output, i) =>
          i === index ? { ...output, [field]: value } : output,
        ),
      );
    },
    [],
  );

  // ─── No wallet → redirect ─────────────────────────────────────

  if (!wallet && !singleKeyWallet) {
    return (
      <div className="flex flex-1 items-center justify-center">
        <div className="text-center">
          <p className="text-muted-foreground">No wallet selected</p>
          <Button variant="outline" className="mt-4" onClick={handleBack}>
            Back to Wallets
          </Button>
        </div>
      </div>
    );
  }

  const walletAlias =
    wallet?.alias?.trim() ||
    singleKeyWallet?.alias?.trim() ||
    "Unnamed Wallet";

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
          <p className="text-sm text-destructive flex-1">
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

      {showAdvanced ? (
        // ─── Advanced mode ───────────────────────────────────────
        <div className="space-y-6">
          {/* Wallet info */}
          <div className="text-sm text-muted-foreground">
            Wallet: <span className="font-medium text-foreground">{walletAlias}</span>
          </div>

          {/* Wallet unlock gate */}
          {!walletUnlocked && wallet?.usesPassword && (
            <div className="space-y-3">
              <p className="text-sm text-warning">
                Wallet is locked. Please unlock to continue.
              </p>
              <Button
                variant="outline"
                onClick={() => setUnlockOpen(true)}
              >
                Unlock Wallet
              </Button>
            </div>
          )}

          {walletUnlocked && (
            <>
              {/* Source type */}
              <div className="space-y-3">
                <Label className="text-base font-semibold">Source Type</Label>
                <p className="text-sm text-muted-foreground">
                  Select whether to send from Core wallet or Platform addresses
                </p>
                <div className="flex gap-4">
                  <label className="flex items-center gap-2 cursor-pointer">
                    <input
                      type="radio"
                      name="advSourceType"
                      checked={advSourceType === "core"}
                      onChange={() => {
                        setAdvSourceType("core");
                        setAdvInputs([]);
                      }}
                      className="accent-primary"
                    />
                    Core Wallet
                  </label>
                  <label
                    className={`flex items-center gap-2 ${platformAddresses.length === 0 ? "opacity-50 cursor-not-allowed" : "cursor-pointer"}`}
                  >
                    <input
                      type="radio"
                      name="advSourceType"
                      checked={advSourceType === "platform"}
                      disabled={platformAddresses.length === 0}
                      onChange={() => {
                        setAdvSourceType("platform");
                        setAdvInputs([]);
                      }}
                      className="accent-primary"
                    />
                    Platform Addresses
                    {platformAddresses.length === 0 && (
                      <span className="text-xs italic text-muted-foreground">
                        (no addresses with balance)
                      </span>
                    )}
                  </label>
                </div>
              </div>

              <Separator />

              {/* Inputs */}
              <div className="space-y-3">
                <Label className="text-base font-semibold">
                  {advSourceType === "core"
                    ? "Core Address Inputs"
                    : "Platform Address Inputs"}
                </Label>
                <p className="text-sm text-muted-foreground">
                  {advSourceType === "core"
                    ? "Select core addresses and amounts to send from each"
                    : "Select platform addresses and amounts to send from each"}
                </p>

                {advInputs.map((input, idx) => (
                  <div
                    key={input.address}
                    className="rounded-md border p-3 space-y-2"
                  >
                    <div className="flex items-center justify-between">
                      <code className="text-xs break-all">{input.address}</code>
                      <div className="flex items-center gap-2">
                        <span className="text-xs text-success font-medium">
                          {advSourceType === "core"
                            ? formatDash(input.balance)
                            : formatCredits(input.balance)}
                        </span>
                        <Button
                          variant="ghost"
                          size="icon"
                          className="h-6 w-6"
                          onClick={() => removeAdvancedInput(idx)}
                          aria-label="Remove input"
                        >
                          <Trash2 className="h-3 w-3" />
                        </Button>
                      </div>
                    </div>
                    <div className="flex items-center gap-2">
                      <Label className="text-xs">Amount:</Label>
                      <Input
                        type="text"
                        inputMode="decimal"
                        value={input.amount}
                        onChange={(e) =>
                          updateAdvancedInputAmount(idx, e.target.value)
                        }
                        placeholder="0.0"
                        className="w-32 h-8 text-sm"
                      />
                      <span className="text-xs text-muted-foreground">
                        DASH
                      </span>
                    </div>
                  </div>
                ))}

                {/* Add input selector */}
                {advSourceType === "core" &&
                  coreAddressesWithBalance.filter(
                    (a) => !advInputs.some((i) => i.address === a.address),
                  ).length > 0 && (
                    <Select
                      onValueChange={(addr) => {
                        const found = coreAddressesWithBalance.find(
                          (a) => a.address === addr,
                        );
                        if (found) addAdvancedCoreInput(found);
                      }}
                    >
                      <SelectTrigger className="w-full">
                        <SelectValue placeholder="+ Add Core Address" />
                      </SelectTrigger>
                      <SelectContent>
                        {coreAddressesWithBalance
                          .filter(
                            (a) =>
                              !advInputs.some((i) => i.address === a.address),
                          )
                          .map((a) => (
                            <SelectItem key={a.address} value={a.address}>
                              {a.address.slice(0, 12)}... ({formatDash(a.balance)})
                            </SelectItem>
                          ))}
                      </SelectContent>
                    </Select>
                  )}

                {advSourceType === "platform" &&
                  platformAddresses.filter(
                    (a) => !advInputs.some((i) => i.address === a.address),
                  ).length > 0 && (
                    <Select
                      onValueChange={(addr) => {
                        const found = platformAddresses.find(
                          (a) => a.address === addr,
                        );
                        if (found) addAdvancedPlatformInput(found);
                      }}
                    >
                      <SelectTrigger className="w-full">
                        <SelectValue placeholder="+ Add Platform Address" />
                      </SelectTrigger>
                      <SelectContent>
                        {platformAddresses
                          .filter(
                            (a) =>
                              !advInputs.some((i) => i.address === a.address),
                          )
                          .map((a) => (
                            <SelectItem key={a.address} value={a.address}>
                              {a.address.slice(0, 20)}... (
                              {formatCredits(a.balance)})
                            </SelectItem>
                          ))}
                      </SelectContent>
                    </Select>
                  )}
              </div>

              <Separator />

              {/* Outputs */}
              <div className="space-y-3">
                <Label className="text-base font-semibold">
                  Outputs (Send To)
                </Label>
                <p className="text-sm text-muted-foreground">
                  {advSourceType === "core"
                    ? "Add Core or Platform destination addresses"
                    : "Add Platform or Core destination addresses"}
                </p>

                {advOutputs.map((output, idx) => {
                  const oType = detectAddressType(output.address);
                  return (
                    <div key={idx} className="rounded-md border p-3 space-y-2">
                      <div className="flex items-center gap-2">
                        <Label className="text-xs shrink-0">To:</Label>
                        <Input
                          value={output.address}
                          onChange={(e) =>
                            updateAdvancedOutput(idx, "address", e.target.value)
                          }
                          placeholder="Enter address (X.../y.../evo1.../tevo1...)"
                          className="flex-1 h-8 text-sm"
                        />
                        {oType !== "unknown" && (
                          <Badge
                            variant="outline"
                            className={
                              oType === "core"
                                ? "text-primary border-primary"
                                : "text-purple-500 border-purple-500"
                            }
                          >
                            {oType === "core" ? "Core" : "Platform"}
                          </Badge>
                        )}
                      </div>
                      <div className="flex items-center gap-2">
                        <Label className="text-xs shrink-0">Amount:</Label>
                        <Input
                          type="text"
                          inputMode="decimal"
                          value={output.amount}
                          onChange={(e) =>
                            updateAdvancedOutput(idx, "amount", e.target.value)
                          }
                          placeholder="0.0"
                          className="w-32 h-8 text-sm"
                        />
                        <span className="text-xs text-muted-foreground">
                          DASH
                        </span>
                        {advOutputs.length > 1 && (
                          <Button
                            variant="ghost"
                            size="icon"
                            className="h-6 w-6 ml-auto"
                            onClick={() => removeAdvancedOutput(idx)}
                            aria-label="Remove output"
                          >
                            <Trash2 className="h-3 w-3" />
                          </Button>
                        )}
                      </div>
                    </div>
                  );
                })}

                <Button
                  variant="outline"
                  size="sm"
                  onClick={addAdvancedOutput}
                >
                  <Plus className="h-3 w-3 mr-1" /> Add Output
                </Button>
              </div>

              {/* Fee strategy (for platform operations) */}
              {(advSourceType === "platform" ||
                advOutputs.some(
                  (o) => detectAddressType(o.address) === "platform",
                )) && (
                <>
                  <Separator />
                  <div className="space-y-3">
                    <Label className="text-base font-semibold">
                      Fee Strategy
                    </Label>
                    <Select
                      value={advFeeStrategy}
                      onValueChange={(v) => setAdvFeeStrategy(v as FeeStrategy)}
                    >
                      <SelectTrigger className="w-64">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="deductFromFirstInput">
                          Deduct from first input
                        </SelectItem>
                        <SelectItem value="deductFromLastInput">
                          Deduct from last input
                        </SelectItem>
                        <SelectItem value="reduceFirstOutput">
                          Reduce first output
                        </SelectItem>
                        <SelectItem value="reduceLastOutput">
                          Reduce last output
                        </SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                </>
              )}

              <Separator />

              {/* Send button */}
              <div className="flex gap-3">
                <Button variant="outline" onClick={handleBack}>
                  Cancel
                </Button>
                <Button
                  onClick={handleAdvancedSendClick}
                  disabled={
                    !walletUnlocked ||
                    advInputs.length === 0 ||
                    !advOutputs.some(
                      (o) => o.address.trim() && o.amount.trim(),
                    )
                  }
                  className="min-w-[160px]"
                >
                  <Send className="h-4 w-4 mr-2" />
                  Send
                </Button>
              </div>
            </>
          )}
        </div>
      ) : (
        // ─── Simple mode ─────────────────────────────────────────
        <div className="space-y-6">
          {/* Wallet info (skip for identity source) */}
          {!isIdentitySource && (
            <div className="text-sm text-muted-foreground">
              Wallet:{" "}
              <span className="font-medium text-foreground">
                {walletAlias}
              </span>
            </div>
          )}

          {/* Wallet unlock gate (skip for identity source) */}
          {!isIdentitySource && !walletUnlocked && wallet?.usesPassword && (
            <div className="space-y-3">
              <p className="text-sm text-warning">
                Wallet is locked. Please unlock to continue.
              </p>
              <Button variant="outline" onClick={() => setUnlockOpen(true)}>
                Unlock Wallet
              </Button>
            </div>
          )}

          {/* Source selection */}
          <div className="space-y-3">
            <Label className="text-sm font-semibold">Send from</Label>

            {/* Core wallet option */}
            <button
              type="button"
              className={`w-full flex items-center gap-3 rounded-md border p-3 text-left transition-colors ${
                source?.type === "core"
                  ? "border-primary bg-primary/5"
                  : "hover:bg-muted/50"
              }`}
              onClick={() => setSource({ type: "core" })}
            >
              <input
                type="radio"
                checked={source?.type === "core"}
                readOnly
                className="accent-primary"
              />
              <span className="font-medium flex-1">Core Wallet</span>
              <span className="text-sm font-medium text-success">
                {formatDash(coreBalance)}
              </span>
            </button>

            {/* Platform addresses option */}
            {platformAddresses.length > 0 && (
              <button
                type="button"
                className={`w-full flex items-center gap-3 rounded-md border p-3 text-left transition-colors ${
                  source?.type === "platform"
                    ? "border-primary bg-primary/5"
                    : "hover:bg-muted/50"
                }`}
                onClick={() =>
                  setSource({ type: "platform", addresses: platformAddresses })
                }
              >
                <input
                  type="radio"
                  checked={source?.type === "platform"}
                  readOnly
                  className="accent-primary"
                />
                <span className="font-medium flex-1">Platform Addresses</span>
                <span className="text-sm font-medium text-success">
                  {formatCredits(totalPlatformBalance)}
                </span>
              </button>
            )}

            {/* Identity options */}
            {identities.map((identity) => (
              <button
                key={identity.id}
                type="button"
                className={`w-full flex items-center gap-3 rounded-md border p-3 text-left transition-colors ${
                  source?.type === "identity" &&
                  source.identity.id === identity.id
                    ? "border-primary bg-primary/5"
                    : "hover:bg-muted/50"
                }`}
                onClick={() => {
                  setSource({ type: "identity", identity });
                  // Auto-select first withdrawal key
                  const withdrawalKeys = identity.keys.filter(
                    (k) => k.purpose === "transfer" || k.purpose === "authentication",
                  );
                  if (withdrawalKeys.length > 0 && withdrawalKeys[0]) {
                    setSelectedKeyId(withdrawalKeys[0].keyId);
                  }
                }}
              >
                <input
                  type="radio"
                  checked={
                    source?.type === "identity" &&
                    source.identity.id === identity.id
                  }
                  readOnly
                  className="accent-primary"
                />
                <span className="font-medium flex-1 truncate">
                  Identity:{" "}
                  {identity.alias || identity.id.slice(0, 12) + "..."}
                </span>
                <span className="text-sm font-medium text-success">
                  {formatCredits(identity.balance)}
                </span>
              </button>
            ))}
          </div>

          {/* Identity key selection */}
          {isIdentitySource && source.type === "identity" && (
            <div className="space-y-2">
              <Label className="text-sm">Signing Key</Label>
              <Select
                value={selectedKeyId?.toString() ?? ""}
                onValueChange={(v) => setSelectedKeyId(parseInt(v))}
              >
                <SelectTrigger className="w-full">
                  <SelectValue placeholder="Select signing key" />
                </SelectTrigger>
                <SelectContent>
                  {source.identity.keys
                    .filter(
                      (k) =>
                        k.purpose === "transfer" ||
                        k.purpose === "authentication",
                    )
                    .map((k) => (
                      <SelectItem key={k.keyId} value={k.keyId.toString()}>
                        Key #{k.keyId} — {k.purpose} ({k.keyType})
                      </SelectItem>
                    ))}
                </SelectContent>
              </Select>
            </div>
          )}

          <Separator />

          {/* Destination address */}
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <Label className="text-sm font-semibold">Send to</Label>
              {destType !== "unknown" && (
                <Badge
                  variant="outline"
                  className={
                    destType === "core"
                      ? "text-primary border-primary"
                      : "text-purple-500 border-purple-500"
                  }
                >
                  {destType === "core" ? "Core Address" : "Platform Address"}
                </Badge>
              )}
            </div>
            <Input
              value={destinationAddress}
              onChange={(e) => setDestinationAddress(e.target.value)}
              placeholder="Enter address (X.../y.../evo1.../tevo1...)"
              aria-label="Destination address"
            />
            {destinationAddress.trim() && destType === "unknown" && !addressError && (
              <p className="text-xs text-destructive">
                Invalid address format
              </p>
            )}
            {addressError && (
              <p className="text-xs text-destructive">
                {addressError}
              </p>
            )}
            {isIdentitySource && destType === "platform" && (
              <p className="text-xs text-warning">
                Identity can only withdraw to Core addresses
              </p>
            )}
          </div>

          <Separator />

          {/* Amount input */}
          <div className="space-y-2">
            <AmountInput
              value={amountValue}
              onChange={setAmountValue}
              label="Amount"
              placeholder="Enter amount"
              decimalPlaces={8}
              unitName="DASH"
              maxAmount={
                maxAmountCredits !== null
                  ? Math.round(maxAmountCredits / CREDITS_PER_DUFF)
                  : null
              }
              showMaxButton
              onMaxClick={handleMaxClick}
              showValidationErrors
            />

            {/* Transaction type hint */}
            {txTypeDesc !== "Send" && destinationAddress.trim() && (
              <p className="text-xs text-muted-foreground italic">
                Transaction type: {txTypeDesc}
              </p>
            )}

            {/* Subtract fee checkbox (Core→Core only) */}
            {source?.type === "core" && destType === "core" && (
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
            )}
          </div>

          {/* Platform source breakdown */}
          {source?.type === "platform" &&
            amountValue &&
            parseFloat(amountValue) > 0 && (
              <div className="rounded-md border bg-muted/30 p-3 space-y-2">
                <p className="text-xs text-muted-foreground">
                  Source breakdown:
                </p>
                {(() => {
                  const credits = creditsFromDashString(amountValue);
                  if (!credits) return null;
                  const sorted = [...platformAddresses].sort(
                    (a, b) => b.balance - a.balance,
                  );
                  let remaining = credits;
                  return sorted.map((addr) => {
                    if (remaining <= 0) return null;
                    if (addr.address === destinationAddress.trim()) return null;
                    const use = Math.min(addr.balance, remaining);
                    remaining -= use;
                    const short =
                      addr.address.length > 18
                        ? `${addr.address.slice(0, 12)}...${addr.address.slice(-6)}`
                        : addr.address;
                    return (
                      <div
                        key={addr.address}
                        className="flex items-center justify-between text-xs"
                      >
                        <code>{short}</code>
                        <span className="text-success font-medium">
                          {formatCredits(use)}
                        </span>
                      </div>
                    );
                  });
                })()}
                <p className="text-[10px] text-muted-foreground italic">
                  Use Advanced Options to customize which addresses to send
                  from.
                </p>
              </div>
            )}

          <Separator />

          {/* Send button */}
          <div className="flex gap-3">
            <Button variant="outline" onClick={handleBack}>
              Cancel
            </Button>
            <Button
              onClick={handleSendClick}
              disabled={!canSend}
              className="min-w-[160px]"
            >
              <Send className="h-4 w-4 mr-2" />
              {txTypeDesc}
            </Button>
          </div>
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
          loading={unlockLoading}
          onResult={handleUnlockResult}
        />
      )}

      {/* Send confirmation dialog */}
      <ConfirmationDialog
        open={showConfirmDialog}
        onOpenChange={setShowConfirmDialog}
        title="Confirm Transaction"
        message={confirmMessage}
        confirmText="Send"
        cancelText="Cancel"
        onResult={handleConfirmResult}
      />
    </Island>
  );
}
