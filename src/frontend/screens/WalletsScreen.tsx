import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Island } from "@/components/layout";
import { LoadingSpinner } from "@/components/feedback";
import {
  WalletListPanel,
  HdWalletDetail,
  SingleKeyWalletDetail,
  ReceiveDialog,
  FundAssetLockDialog,
  PrivateKeyDialog,
} from "@/components/wallet";
import type { ReceiveAddress } from "@/components/wallet/ReceiveDialog";
import { WalletUnlockDialog } from "@/components/shared/WalletUnlockDialog";
import type { WalletUnlockResult } from "@/components/shared/WalletUnlockDialog";
import { useWalletStore } from "@/stores/walletStore";
import { commands } from "@/bindings";
import type { WalletDto, SingleKeyWalletDto } from "@/bindings";
import { toast } from "sonner";
import { toastError } from "@/lib/toastError";

// ─── Helpers ───────────────────────────────────────────────────────

function buildCoreAddresses(wallet: WalletDto): ReceiveAddress[] {
  return wallet.addresses
    .filter((a) => {
      // Only include external/funds addresses (BIP44 external chain m/44'/[coin_type]'/X'/0/...)
      return /m\/44'\/\d+'\/\d+'\/0\//.test(a.derivationPath);
    })
    .map((a) => ({ address: a.address, balance: a.balance }));
}

function buildPlatformAddresses(wallet: WalletDto): ReceiveAddress[] {
  return wallet.platformAddresses.map((a) => ({
    address: a.address,
    balance: a.balance,
  }));
}

function buildSingleKeyCoreAddresses(
  wallet: SingleKeyWalletDto,
): ReceiveAddress[] {
  const totalBalance =
    (wallet.totalBalance ?? 0) + (wallet.unconfirmedBalance ?? 0);
  return [{ address: wallet.address, balance: totalBalance }];
}

// ─── WalletsScreen ────────────────────────────────────────────────

export function WalletsScreen() {
  const navigate = useNavigate();

  // Store state
  const {
    hdWallets,
    singleKeyWallets,
    selectedWallet,
    loading,
    refreshing,
    error,
    refreshMode,
    loadWallets,
    selectWallet,
    refreshSelectedWallet,
    setRefreshMode,
    setHdWalletAlias,
    setSingleKeyWalletAlias,
    removeHdWallet,
    removeSingleKeyWallet,
    notifyUnlocked,
    notifyLocked,
    unlockWallet,
    lockWallet,
    subscribeToUpdates,
    clearError,
  } = useWalletStore();

  // Developer mode
  const [isDeveloperMode, setIsDeveloperMode] = useState(false);

  // Dialog state
  const [receiveDialogOpen, setReceiveDialogOpen] = useState(false);
  const [privateKeyDialog, setPrivateKeyDialog] = useState<{
    open: boolean;
    address: string;
    wif: string;
  }>({ open: false, address: "", wif: "" });
  const [generatingAddress, setGeneratingAddress] = useState(false);

  // Wallet unlock for view-key flow
  const [viewKeyUnlock, setViewKeyUnlock] = useState<{
    open: boolean;
    seedHash: string;
    alias: string | null;
    passwordHint: string | null;
    address: string;
    derivationPath: string;
    error?: string | null;
  } | null>(null);

  // Fund from asset lock dialog
  const [fundAssetLockDialog, setFundAssetLockDialog] = useState<{
    open: boolean;
    assetLockIndex: number;
  } | null>(null);

  // Load wallets and developer mode on mount
  useEffect(() => {
    loadWallets();
    commands
      .contextIsDeveloperMode()
      .then((result) => {
        setIsDeveloperMode(result);
      })
      .catch(() => {});
  }, [loadWallets]);

  // Subscribe to wallet update events
  useEffect(() => {
    let cleanup: (() => void) | undefined;
    subscribeToUpdates()
      .then((unsub) => {
        cleanup = unsub;
      })
      .catch((e) => console.error("Failed to subscribe to wallet events:", e));
    return () => cleanup?.();
  }, [subscribeToUpdates]);

  // Show toast on error
  useEffect(() => {
    if (error) {
      toastError(error);
      clearError();
    }
  }, [error, clearError]);

  // ─── Find selected wallet objects ────────────────────────────────

  const selectedHdWallet: WalletDto | null =
    selectedWallet?.type === "hd"
      ? (hdWallets.find((w) => w.seedHash === selectedWallet.seedHash) ?? null)
      : null;

  const selectedSingleKeyWallet: SingleKeyWalletDto | null =
    selectedWallet?.type === "singleKey"
      ? (singleKeyWallets.find(
          (w) => w.keyHash === selectedWallet.keyHash,
        ) ?? null)
      : null;

  // ─── Navigation callbacks ────────────────────────────────────────

  const handleCreateWallet = useCallback(() => {
    navigate({ to: "/wallets/create" as string });
  }, [navigate]);

  const handleImportWallet = useCallback(() => {
    navigate({ to: "/wallets/import" as string });
  }, [navigate]);

  const handleSend = useCallback(() => {
    if (selectedWallet?.type === "hd") {
      navigate({ to: "/wallets/send/hd" as string });
    } else if (selectedWallet?.type === "singleKey") {
      navigate({ to: "/wallets/send-single-key" as string });
    }
  }, [navigate, selectedWallet]);

  // ─── Receive dialog ──────────────────────────────────────────────

  const handleReceive = useCallback(() => {
    setReceiveDialogOpen(true);
  }, []);

  const handleNewCoreAddress = useCallback(async () => {
    if (!selectedHdWallet) return;
    setGeneratingAddress(true);
    try {
      const result = await commands.walletGenerateReceiveAddress({
        walletSeedHash: selectedHdWallet.seedHash,
      });
      if (result.status === "ok") {
        toast.success("New address generated");
        await useWalletStore.getState().reloadHdWallet(selectedHdWallet.seedHash);
      } else {
        toastError(result.error);
      }
    } catch (e) {
      toastError(e instanceof Error ? e.message : String(e));
    } finally {
      setGeneratingAddress(false);
    }
  }, [selectedHdWallet]);

  // ─── Refresh ─────────────────────────────────────────────────────

  const handleRefresh = useCallback(() => {
    refreshSelectedWallet();
  }, [refreshSelectedWallet]);

  // ─── Add address ─────────────────────────────────────────────────

  const handleAddAddress = useCallback(async () => {
    if (!selectedHdWallet) return;
    try {
      const result = await commands.walletGenerateReceiveAddress({
        walletSeedHash: selectedHdWallet.seedHash,
      });
      if (result.status === "ok") {
        toast.success("New receiving address added");
        await useWalletStore.getState().reloadHdWallet(selectedHdWallet.seedHash);
      } else {
        toastError(result.error);
      }
    } catch (e) {
      toastError(e instanceof Error ? e.message : String(e));
    }
  }, [selectedHdWallet]);

  // ─── View private key ────────────────────────────────────────────

  const fetchAndShowPrivateKey = useCallback(
    async (seedHash: string, address: string, derivationPath: string) => {
      try {
        const result = await commands.walletGetPrivateKey({
          walletSeedHash: seedHash,
          address,
          derivationPath,
        });
        if (result.status === "ok") {
          setPrivateKeyDialog({ open: true, address, wif: result.data });
        } else {
          toastError(result.error);
        }
      } catch (e) {
        toastError(e instanceof Error ? e.message : String(e));
      }
    },
    [],
  );

  const handleViewKey = useCallback(
    (address: string, derivationPath: string) => {
      if (!selectedHdWallet) return;
      if (selectedHdWallet.usesPassword) {
        // Need to unlock first
        setViewKeyUnlock({
          open: true,
          seedHash: selectedHdWallet.seedHash,
          alias: selectedHdWallet.alias,
          passwordHint: selectedHdWallet.passwordHint,
          address,
          derivationPath,
        });
      } else {
        // No password — fetch key directly
        fetchAndShowPrivateKey(selectedHdWallet.seedHash, address, derivationPath);
      }
    },
    [selectedHdWallet, fetchAndShowPrivateKey],
  );

  const handleViewKeyUnlockResult = useCallback(
    async (result: WalletUnlockResult) => {
      if (result.status === "unlocked" && viewKeyUnlock) {
        const err = await unlockWallet(
          { type: "hd", seedHash: viewKeyUnlock.seedHash },
          result.password,
        );
        if (err) {
          setViewKeyUnlock((prev) => prev ? { ...prev, error: err } : null);
          return;
        }
        await fetchAndShowPrivateKey(
          viewKeyUnlock.seedHash,
          viewKeyUnlock.address,
          viewKeyUnlock.derivationPath,
        );
      }
      setViewKeyUnlock(null);
    },
    [viewKeyUnlock, unlockWallet, fetchAndShowPrivateKey],
  );

  // ─── Asset lock callbacks ────────────────────────────────────────

  const handleCreateAssetLock = useCallback(() => {
    navigate({ to: "/wallets/asset-locks/create" as string });
  }, [navigate]);

  const handleSearchAssetLocks = useCallback(async () => {
    if (!selectedHdWallet) return;
    try {
      const result = await commands.coreRecoverAssetLocks({
        walletSeedHash: selectedHdWallet.seedHash,
      });
      if (result.status === "ok") {
        toast.success("Asset lock search dispatched");
      } else {
        toastError(result.error);
      }
    } catch (e) {
      toastError(e instanceof Error ? e.message : String(e));
    }
  }, [selectedHdWallet]);

  const handleViewAssetLock = useCallback(
    (txid: string) => {
      navigate({ to: `/wallets/asset-locks/${txid}` as string });
    },
    [navigate],
  );

  const handleFundAssetLock = useCallback(
    (assetLockIndex: number) => {
      if (!selectedHdWallet) return;
      setFundAssetLockDialog({ open: true, assetLockIndex });
    },
    [selectedHdWallet],
  );

  // ─── Wallet lock/unlock callbacks ────────────────────────────────

  const handleUnlockWallet = useCallback(
    async (seedHash: string, password: string): Promise<string | null> => {
      return unlockWallet({ type: "hd", seedHash }, password);
    },
    [unlockWallet],
  );

  const handleLockWallet = useCallback(
    async (seedHash: string) => {
      await lockWallet({ type: "hd", seedHash });
    },
    [lockWallet],
  );

  const handleUnlockSingleKeyWallet = useCallback(
    async (keyHash: string, password: string): Promise<string | null> => {
      return unlockWallet({ type: "singleKey", keyHash }, password);
    },
    [unlockWallet],
  );

  const handleLockSingleKeyWallet = useCallback(
    async (keyHash: string) => {
      await lockWallet({ type: "singleKey", keyHash });
    },
    [lockWallet],
  );

  // ─── Receive dialog addresses ────────────────────────────────────

  let receiveWalletName: string;
  let receiveCoreAddresses: ReceiveAddress[];
  let receivePlatformAddresses: ReceiveAddress[];
  let receiveWalletType: "hd" | "singleKey";

  if (selectedHdWallet) {
    receiveWalletName = selectedHdWallet.alias?.trim() || "Unnamed Wallet";
    receiveCoreAddresses = buildCoreAddresses(selectedHdWallet);
    receivePlatformAddresses = buildPlatformAddresses(selectedHdWallet);
    receiveWalletType = "hd";
  } else if (selectedSingleKeyWallet) {
    receiveWalletName = selectedSingleKeyWallet.alias?.trim() || "Unnamed Key";
    receiveCoreAddresses = buildSingleKeyCoreAddresses(selectedSingleKeyWallet);
    receivePlatformAddresses = [];
    receiveWalletType = "singleKey";
  } else {
    receiveWalletName = "Wallet";
    receiveCoreAddresses = [];
    receivePlatformAddresses = [];
    receiveWalletType = "singleKey";
  }

  // ─── Render ──────────────────────────────────────────────────────

  if (loading) {
    return (
      <div className="flex flex-1 items-center justify-center">
        <LoadingSpinner label="Loading wallets..." />
      </div>
    );
  }

  return (
    <div className="flex flex-1 gap-3 min-h-0">
      {/* Left Panel — Wallet List */}
      <Island noPadding className="w-[300px] shrink-0 flex flex-col overflow-hidden">
        <WalletListPanel
          hdWallets={hdWallets}
          singleKeyWallets={singleKeyWallets}
          selectedWallet={selectedWallet}
          onSelectWallet={selectWallet}
          onRenameHdWallet={setHdWalletAlias}
          onRenameSingleKeyWallet={setSingleKeyWalletAlias}
          onRemoveHdWallet={removeHdWallet}
          onRemoveSingleKeyWallet={removeSingleKeyWallet}
          onUnlockWallet={handleUnlockWallet}
          onLockWallet={handleLockWallet}
          onUnlockSingleKeyWallet={handleUnlockSingleKeyWallet}
          onLockSingleKeyWallet={handleLockSingleKeyWallet}
          onCreateWallet={handleCreateWallet}
          onImportWallet={handleImportWallet}
        />
      </Island>

      {/* Right Panel — Detail View */}
      <Island className="flex-1 min-w-0 overflow-auto">
        {selectedHdWallet ? (
          <HdWalletDetail
            wallet={selectedHdWallet}
            refreshing={refreshing}
            isDeveloperMode={isDeveloperMode}
            refreshMode={refreshMode}
            onRefreshModeChange={setRefreshMode}
            onSend={handleSend}
            onReceive={handleReceive}
            onRefresh={handleRefresh}
            onAddAddress={handleAddAddress}
            onViewKey={handleViewKey}
            onCreateAssetLock={handleCreateAssetLock}
            onSearchAssetLocks={handleSearchAssetLocks}
            onViewAssetLock={handleViewAssetLock}
            onFundAssetLock={handleFundAssetLock}
          />
        ) : selectedSingleKeyWallet ? (
          <SingleKeyWalletDetail
            wallet={selectedSingleKeyWallet}
            refreshing={refreshing}
            onSend={handleSend}
            onReceive={handleReceive}
            onRefresh={handleRefresh}
          />
        ) : (
          <div className="flex flex-1 items-center justify-center h-full text-muted-foreground">
            <p className="text-sm">Select a wallet to view details</p>
          </div>
        )}
      </Island>

      {/* Receive Dialog */}
      <ReceiveDialog
        open={receiveDialogOpen}
        onOpenChange={setReceiveDialogOpen}
        walletName={receiveWalletName}
        walletType={receiveWalletType}
        coreAddresses={receiveCoreAddresses}
        platformAddresses={receivePlatformAddresses}
        onNewCoreAddress={selectedHdWallet ? handleNewCoreAddress : undefined}
        generatingAddress={generatingAddress}
      />

      {/* Private Key Dialog */}
      <PrivateKeyDialog
        open={privateKeyDialog.open}
        onOpenChange={(open) => {
          if (!open) setPrivateKeyDialog({ open: false, address: "", wif: "" });
        }}
        address={privateKeyDialog.address}
        privateKeyWif={privateKeyDialog.wif}
      />

      {/* View Key Unlock Dialog */}
      {viewKeyUnlock && (
        <WalletUnlockDialog
          open={viewKeyUnlock.open}
          onOpenChange={(open) => {
            if (!open) setViewKeyUnlock(null);
          }}
          walletAlias={viewKeyUnlock.alias ?? "Wallet"}
          passwordHint={viewKeyUnlock.passwordHint}
          error={viewKeyUnlock.error}
          onResult={handleViewKeyUnlockResult}
        />
      )}

      {/* Fund from Asset Lock Dialog */}
      {fundAssetLockDialog && selectedHdWallet && (
        <FundAssetLockDialog
          open={fundAssetLockDialog.open}
          onOpenChange={(open) => {
            if (!open) setFundAssetLockDialog(null);
          }}
          wallet={selectedHdWallet}
          assetLockIndex={fundAssetLockDialog.assetLockIndex}
        />
      )}
    </div>
  );
}
