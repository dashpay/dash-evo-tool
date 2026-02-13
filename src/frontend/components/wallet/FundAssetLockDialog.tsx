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
import { waitForTask } from "@/lib/utils";
import { commands } from "@/bindings";
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
  // Use openCount key to reset inner state when dialog reopens
  const [openCount, setOpenCount] = useState(0);
  const handleOpenChange = useCallback(
    (value: boolean) => {
      if (value) setOpenCount((c) => c + 1);
      onOpenChange(value);
    },
    [onOpenChange],
  );

  const assetLock = wallet.unusedAssetLocks[assetLockIndex] ?? null;

  if (!assetLock) {
    return (
      <Dialog open={open} onOpenChange={handleOpenChange}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Asset Lock Not Found</DialogTitle>
            <DialogDescription className="sr-only">
              Asset lock error
            </DialogDescription>
          </DialogHeader>
          <p className="text-sm text-destructive">
            The selected asset lock could not be found. It may have already been used.
          </p>
          <DialogFooter>
            <Button variant="outline" onClick={() => handleOpenChange(false)}>
              Close
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    );
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-md">
        <FundAssetLockContent
          key={openCount}
          wallet={wallet}
          assetLockIndex={assetLockIndex}
          assetLock={assetLock}
          onClose={() => handleOpenChange(false)}
        />
      </DialogContent>
    </Dialog>
  );
}

/** Inner content that resets state when remounted via key. */
function FundAssetLockContent({
  wallet,
  assetLockIndex,
  assetLock,
  onClose,
}: {
  wallet: WalletDto;
  assetLockIndex: number;
  assetLock: { txid: string; amount: number };
  onClose: () => void;
}) {
  const [selectedAddress, setSelectedAddress] = useState<string>("");
  const [processing, setProcessing] = useState(false);
  const [statusMessage, setStatusMessage] = useState<{
    type: "error" | "success";
    text: string;
  } | null>(null);

  const assetLockAmountDash = useMemo(() => {
    const duffs = assetLock.amount / CREDITS_PER_DUFF;
    return formatAmount(duffs, DUFFS_DECIMAL_PLACES);
  }, [assetLock]);

  const handleFund = useCallback(async () => {
    if (!selectedAddress) return;
    setProcessing(true);
    setStatusMessage(null);

    try {
      const result = await commands.walletFundPlatformFromAssetLock({
        walletSeedHash: wallet.seedHash,
        assetLockIndex,
        destinationAddress: selectedAddress,
        amount: null,
      });

      if (result.status === "ok") {
        await waitForTask(result.data.taskId);
        toast.success("Platform address funded from asset lock");
        onClose();
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
  }, [selectedAddress, wallet.seedHash, assetLockIndex, onClose]);

  return (
    <>
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
                    <SelectItem key={addr.bech32MAddress} value={addr.bech32MAddress}>
                      <span className="font-mono text-xs">
                        {truncateString(addr.bech32MAddress)}
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
            role="alert"
            className={`text-sm ${statusMessage.type === "error" ? "text-destructive" : "text-green-600"}`}
          >
            {statusMessage.text}
          </p>
        )}
      </div>

      <DialogFooter className="gap-2 sm:gap-0">
        <Button
          variant="outline"
          onClick={onClose}
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
    </>
  );
}
