import { useCallback, useMemo, useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Button } from "@/components/ui/button";
import { formatAmount } from "@/components/shared/AmountInput";
import { commands, events } from "@/bindings";
import type { WalletDto } from "@/bindings";
import { toast } from "sonner";

const DUFFS_DECIMAL_PLACES = 8;
const CREDITS_PER_DUFF = 1000;

interface FundAssetLockDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  wallet: WalletDto;
  assetLockIndex: number;
}

/** Wait for a dispatched backend task to complete. */
async function waitForTask(taskId: string, timeoutMs = 60000): Promise<void> {
  return new Promise<void>(async (resolve, reject) => {
    let resolved = false;

    const timer = setTimeout(() => {
      if (resolved) return;
      resolved = true;
      unsubResult();
      unsubError();
      reject(new Error("Task timed out"));
    }, timeoutMs);

    const done = (fn: () => void) => {
      if (resolved) return;
      resolved = true;
      clearTimeout(timer);
      unsubResult();
      unsubError();
      fn();
    };

    const unsubResult = await events.taskResultEvent.listen((event) => {
      if (event.payload.taskId !== taskId) return;
      done(() => resolve());
    });
    const unsubError = await events.taskErrorEvent.listen((event) => {
      if (event.payload.taskId !== taskId) return;
      done(() => reject(new Error(event.payload.message)));
    });

    if (resolved) {
      unsubResult();
      unsubError();
    }
  });
}

function truncateString(s: string, maxLen: number = 20): string {
  if (s.length <= maxLen) return s;
  const half = Math.floor((maxLen - 3) / 2);
  return `${s.slice(0, half)}...${s.slice(-half)}`;
}

/**
 * Dialog for funding a platform address from an existing proved asset lock.
 *
 * @see egui: src/ui/wallets/wallets_screen/dialogs.rs
 */
export function FundAssetLockDialog({
  open,
  onOpenChange,
  wallet,
  assetLockIndex,
}: FundAssetLockDialogProps) {
  const [selectedAddress, setSelectedAddress] = useState<string>("");
  const [processing, setProcessing] = useState(false);
  const [statusMessage, setStatusMessage] = useState<{
    type: "error" | "success";
    text: string;
  } | null>(null);

  const assetLock = wallet.unusedAssetLocks[assetLockIndex] ?? null;

  const assetLockAmountDash = useMemo(() => {
    if (!assetLock) return "0";
    // amount is in credits; convert to duffs then format
    const duffs = assetLock.amount / CREDITS_PER_DUFF;
    return formatAmount(duffs, DUFFS_DECIMAL_PLACES);
  }, [assetLock]);

  const handleFund = useCallback(async () => {
    if (!selectedAddress || !assetLock) return;
    setProcessing(true);
    setStatusMessage(null);

    try {
      const result = await commands.walletFundPlatformFromAssetLock({
        walletSeedHash: wallet.seedHash,
        assetLockIndex,
        destinationAddress: selectedAddress,
        amount: null, // use full asset lock amount
      });

      if (result.status === "ok") {
        await waitForTask(result.data.taskId);
        toast.success("Platform address funded from asset lock");
        onOpenChange(false);
      } else {
        setStatusMessage({ type: "error", text: result.error });
      }
    } catch (e) {
      setStatusMessage({
        type: "error",
        text: e instanceof Error ? e.message : String(e),
      });
    } finally {
      setProcessing(false);
    }
  }, [selectedAddress, assetLock, wallet.seedHash, assetLockIndex, onOpenChange]);

  if (!assetLock) return null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Fund Platform Address from Asset Lock</DialogTitle>
          <DialogDescription className="sr-only">
            Select a platform address to fund from an asset lock
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          {/* Asset lock info */}
          <div className="space-y-1 text-sm">
            <div className="flex justify-between">
              <span className="text-muted-foreground">Asset Lock TX</span>
              <code className="font-mono text-xs">
                {truncateString(assetLock.txid, 24)}
              </code>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Amount</span>
              <span className="font-medium">{assetLockAmountDash} DASH</span>
            </div>
          </div>

          {/* Platform address selector */}
          <div className="space-y-2">
            <label className="text-sm font-medium">
              Destination Platform Address
            </label>
            {wallet.platformAddresses.length === 0 ? (
              <p className="text-sm text-muted-foreground">
                No platform addresses available. Generate one first.
              </p>
            ) : (
              <Select value={selectedAddress} onValueChange={setSelectedAddress}>
                <SelectTrigger className="w-full" aria-label="Select platform address">
                  <SelectValue placeholder="Select a platform address" />
                </SelectTrigger>
                <SelectContent>
                  {wallet.platformAddresses.map((addr) => {
                    const balanceDuffs = addr.balance / CREDITS_PER_DUFF;
                    return (
                      <SelectItem key={addr.address} value={addr.address}>
                        <span className="font-mono text-xs">
                          {truncateString(addr.address)}
                        </span>
                        <span className="ml-2 text-xs text-muted-foreground">
                          ({formatAmount(balanceDuffs, DUFFS_DECIMAL_PLACES)} DASH)
                        </span>
                      </SelectItem>
                    );
                  })}
                </SelectContent>
              </Select>
            )}
          </div>

          {/* Status message */}
          {statusMessage && (
            <p
              className={`text-sm ${statusMessage.type === "error" ? "text-destructive" : "text-green-600"}`}
            >
              {statusMessage.text}
            </p>
          )}
        </div>

        <DialogFooter className="gap-2 sm:gap-0">
          <Button
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={processing}
          >
            Cancel
          </Button>
          <Button
            onClick={handleFund}
            disabled={!selectedAddress || processing || wallet.platformAddresses.length === 0}
          >
            {processing ? "Funding..." : "Fund"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
