import { useCallback, useMemo, useState } from "react";
import { QRCodeSVG } from "qrcode.react";
import { Plus } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/shared/CopyButton";
import { formatAmount } from "@/components/shared/AmountInput";

const DUFFS_DECIMAL_PLACES = 8;
const CREDITS_PER_DUFF = 1000;

/** An address entry for the receive dialog. */
export interface ReceiveAddress {
  /** The display address string */
  address: string;
  /** Balance in smallest unit (duffs for Core, credits for Platform) */
  balance: number;
}

/** Which type of wallet is being received for */
export type ReceiveWalletType = "hd" | "singleKey";

interface ReceiveDialogProps {
  /** Whether the dialog is open */
  open: boolean;
  /** Called when the dialog should close */
  onOpenChange: (open: boolean) => void;
  /** Wallet display name */
  walletName: string;
  /** Type of wallet */
  walletType: ReceiveWalletType;
  /** Core (L1) addresses with balances in duffs */
  coreAddresses: ReceiveAddress[];
  /** Platform (L2) addresses with balances in credits */
  platformAddresses: ReceiveAddress[];
  /** Called when user requests a new Core address */
  onNewCoreAddress?: () => void;
  /** Called when user requests a new Platform address */
  onNewPlatformAddress?: () => void;
  /** Loading state for new address generation */
  generatingAddress?: boolean;
}

/**
 * Receive dialog for displaying wallet receive addresses with QR codes.
 *
 * Matches egui ReceiveDialogState behavior:
 * - Core/Platform tabs for HD wallets (Core only for single-key)
 * - QR code display (220x220)
 * - Address selector dropdown when multiple addresses exist
 * - Full address display with copy button
 * - Balance display per address
 * - New Address generation button
 *
 * @see egui: src/ui/wallets/wallets_screen/dialogs.rs lines 193-550
 */
export function ReceiveDialog({
  open,
  onOpenChange,
  walletName,
  walletType,
  coreAddresses,
  platformAddresses,
  onNewCoreAddress,
  onNewPlatformAddress,
  generatingAddress = false,
}: ReceiveDialogProps) {
  // Use a key to reset inner state when dialog reopens
  const [openCount, setOpenCount] = useState(0);
  const handleOpenChange = useCallback(
    (value: boolean) => {
      if (value) setOpenCount((c) => c + 1);
      onOpenChange(value);
    },
    [onOpenChange],
  );

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Receive — {walletName}</DialogTitle>
          <DialogDescription className="sr-only">
            Display receive addresses and QR codes
          </DialogDescription>
        </DialogHeader>
        <ReceiveDialogContent
          key={openCount}
          walletType={walletType}
          coreAddresses={coreAddresses}
          platformAddresses={platformAddresses}
          onNewCoreAddress={onNewCoreAddress}
          onNewPlatformAddress={onNewPlatformAddress}
          generatingAddress={generatingAddress}
        />
      </DialogContent>
    </Dialog>
  );
}

/** Inner content that resets state when remounted via key. */
function ReceiveDialogContent({
  walletType,
  coreAddresses,
  platformAddresses,
  onNewCoreAddress,
  onNewPlatformAddress,
  generatingAddress = false,
}: {
  walletType: ReceiveWalletType;
  coreAddresses: ReceiveAddress[];
  platformAddresses: ReceiveAddress[];
  onNewCoreAddress?: () => void;
  onNewPlatformAddress?: () => void;
  generatingAddress?: boolean;
}) {
  const [activeTab, setActiveTab] = useState<"core" | "platform">("core");
  const [selectedCoreIndex, setSelectedCoreIndex] = useState(0);
  const [selectedPlatformIndex, setSelectedPlatformIndex] = useState(0);

  // Clamp indices to valid range (computed)
  const clampedCoreIndex =
    coreAddresses.length > 0
      ? Math.min(selectedCoreIndex, coreAddresses.length - 1)
      : 0;
  const clampedPlatformIndex =
    platformAddresses.length > 0
      ? Math.min(selectedPlatformIndex, platformAddresses.length - 1)
      : 0;

  const handleCoreIndexChange = useCallback((value: string) => {
    setSelectedCoreIndex(parseInt(value, 10));
  }, []);

  const handlePlatformIndexChange = useCallback((value: string) => {
    setSelectedPlatformIndex(parseInt(value, 10));
  }, []);

  // For single-key wallets, only show Core tab
  const showPlatformTab = walletType === "hd" && platformAddresses.length > 0;

  if (showPlatformTab) {
    return (
      <Tabs
        value={activeTab}
        onValueChange={(v) => setActiveTab(v as "core" | "platform")}
      >
        <TabsList className="w-full">
          <TabsTrigger value="core" className="flex-1">
            Core
          </TabsTrigger>
          <TabsTrigger value="platform" className="flex-1">
            Platform
          </TabsTrigger>
        </TabsList>
        <TabsContent value="core" className="mt-4">
          <AddressPanel
            addresses={coreAddresses}
            selectedIndex={clampedCoreIndex}
            onSelectIndex={handleCoreIndexChange}
            onNewAddress={onNewCoreAddress}
            generatingAddress={generatingAddress}
            balanceUnit="duffs"
            infoText="Send Dash to this address to fund your wallet."
          />
        </TabsContent>
        <TabsContent value="platform" className="mt-4">
          <AddressPanel
            addresses={platformAddresses}
            selectedIndex={clampedPlatformIndex}
            onSelectIndex={handlePlatformIndexChange}
            onNewAddress={onNewPlatformAddress}
            generatingAddress={generatingAddress}
            balanceUnit="credits"
            infoText="Send credits to this Platform address."
          />
        </TabsContent>
      </Tabs>
    );
  }

  return (
    <div className="mt-2">
      <AddressPanel
        addresses={coreAddresses}
        selectedIndex={clampedCoreIndex}
        onSelectIndex={handleCoreIndexChange}
        onNewAddress={onNewCoreAddress}
        generatingAddress={generatingAddress}
        balanceUnit="duffs"
        infoText="Send Dash to this address to fund your wallet."
      />
    </div>
  );
}

// ── Address Panel Sub-Component ──────────────────────────────────────

interface AddressPanelProps {
  addresses: ReceiveAddress[];
  selectedIndex: number;
  onSelectIndex: (value: string) => void;
  onNewAddress?: () => void;
  generatingAddress?: boolean;
  balanceUnit: "duffs" | "credits";
  infoText: string;
}

function AddressPanel({
  addresses,
  selectedIndex,
  onSelectIndex,
  onNewAddress,
  generatingAddress = false,
  balanceUnit,
  infoText,
}: AddressPanelProps) {
  const selectedAddress = addresses[selectedIndex] ?? null;

  const formattedBalance = useMemo(() => {
    if (!selectedAddress) return "0.00000000 DASH";
    if (balanceUnit === "duffs") {
      return `${formatAmount(selectedAddress.balance, DUFFS_DECIMAL_PLACES)} DASH`;
    }
    // Credits → DASH: credits / CREDITS_PER_DUFF / 10^8
    const duffs = selectedAddress.balance / CREDITS_PER_DUFF;
    return `${formatAmount(duffs, DUFFS_DECIMAL_PLACES)} DASH`;
  }, [selectedAddress, balanceUnit]);

  const truncateAddress = useCallback(
    (addr: string, maxLen: number = 20) => {
      if (addr.length <= maxLen) return addr;
      const half = Math.floor((maxLen - 3) / 2);
      return `${addr.slice(0, half)}...${addr.slice(-half)}`;
    },
    [],
  );

  if (addresses.length === 0) {
    return (
      <div className="flex flex-col items-center gap-4 py-8">
        <p className="text-sm text-muted-foreground">No addresses available.</p>
        {onNewAddress && (
          <Button
            variant="outline"
            size="sm"
            onClick={onNewAddress}
            disabled={generatingAddress}
          >
            <Plus className="mr-1 size-4" />
            {generatingAddress ? "Generating..." : "Generate Address"}
          </Button>
        )}
      </div>
    );
  }

  return (
    <div className="flex flex-col items-center gap-4">
      {/* QR Code */}
      <div
        className="rounded-lg border bg-white p-3"
        role="img"
        aria-label={`QR code for address ${selectedAddress?.address ?? ""}`}
      >
        <QRCodeSVG
          value={selectedAddress?.address ?? ""}
          size={220}
          level="M"
          includeMargin={false}
        />
      </div>

      {/* Address Selector (only if multiple) */}
      {addresses.length > 1 && (
        <Select
          value={String(selectedIndex)}
          onValueChange={onSelectIndex}
        >
          <SelectTrigger className="w-full" aria-label="Select address">
            <SelectValue placeholder="Select address" />
          </SelectTrigger>
          <SelectContent>
            {addresses.map((addr, i) => (
              <SelectItem key={i} value={String(i)}>
                <span className="font-mono text-xs">
                  {truncateAddress(addr.address)}
                </span>
                <span className="ml-2 text-xs text-muted-foreground">
                  ({formatAmount(
                    balanceUnit === "credits"
                      ? addr.balance / CREDITS_PER_DUFF
                      : addr.balance,
                    DUFFS_DECIMAL_PLACES,
                  )}{" "}
                  DASH)
                </span>
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      )}

      {/* Full Address Display */}
      {selectedAddress && (
        <div className="w-full space-y-3">
          <div className="flex items-center gap-2">
            <code className="flex-1 break-all rounded bg-muted px-3 py-2 text-xs font-mono">
              {selectedAddress.address}
            </code>
            <CopyButton value={selectedAddress.address} label="Copy" size="sm" />
          </div>

          {/* Balance */}
          <div className="flex items-center justify-between text-sm">
            <span className="text-muted-foreground">Balance</span>
            <span className="font-medium">{formattedBalance}</span>
          </div>

          {/* Action Buttons */}
          <div className="flex gap-2">
            {onNewAddress && (
              <Button
                variant="outline"
                size="sm"
                onClick={onNewAddress}
                disabled={generatingAddress}
                className="flex-1"
              >
                <Plus className="mr-1 size-4" />
                {generatingAddress ? "Generating..." : "New Address"}
              </Button>
            )}
          </div>

          {/* Info Text */}
          <p className="text-center text-xs text-muted-foreground">{infoText}</p>
        </div>
      )}
    </div>
  );
}
