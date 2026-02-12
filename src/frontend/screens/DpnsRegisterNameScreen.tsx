import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  RegisterDpnsNameScreen,
  type RegisterDpnsNameStatus,
  isContestedName,
} from "@/components/identity/RegisterDpnsNameScreen";
import { useIdentityStore } from "@/stores/identityStore";
import { useContestStore } from "@/stores/contestStore";
import { useWalletStore } from "@/stores/walletStore";
import { commands } from "@/bindings";
import { LoadingSpinner } from "@/components/feedback";
import {
  WalletUnlockDialog,
  type WalletUnlockResult,
} from "@/components/shared/WalletUnlockDialog";

export function DpnsRegisterNameScreen() {
  const navigate = useNavigate();

  // Identity store
  const identities = useIdentityStore((s) => s.identities);
  const identitiesLoading = useIdentityStore((s) => s.loading);
  const loadIdentities = useIdentityStore((s) => s.loadIdentities);
  const reloadIdentity = useIdentityStore((s) => s.reloadIdentity);
  const subscribeIdentityUpdates = useIdentityStore((s) => s.subscribeToUpdates);

  // Contest store — for refreshing DPNS names after registration
  const { refreshDpnsNames } = useContestStore();

  // Wallet store — for wallet lock state
  const hdWallets = useWalletStore((s) => s.hdWallets);
  const singleKeyWallets = useWalletStore((s) => s.singleKeyWallets);
  const loadWallets = useWalletStore((s) => s.loadWallets);

  // Registration status
  const [status, setStatus] = useState<RegisterDpnsNameStatus>({
    type: "form",
  });

  // Track which identity is selected (synced from child component via onSubmit identity ID)
  const [selectedIdentityId, setSelectedIdentityId] = useState<string | null>(
    null,
  );

  // Wallet unlock state
  const [walletUnlockOpen, setWalletUnlockOpen] = useState(false);
  const [walletUnlockError, setWalletUnlockError] = useState<string | null>(
    null,
  );
  const [walletUnlockedHashes, setWalletUnlockedHashes] = useState<
    Set<string>
  >(new Set());

  // Resolve which identity is currently selected
  const currentIdentityId =
    selectedIdentityId ?? identities[0]?.id ?? null;

  // Find associated wallet for current identity
  const currentIdentity = currentIdentityId
    ? identities.find((i) => i.id === currentIdentityId)
    : null;
  const associatedWallet = (() => {
    if (!currentIdentity) return null;
    const hashes = currentIdentity.associatedWalletHashes;
    if (hashes.length === 0) return null;
    for (const hash of hashes) {
      const hd = hdWallets.find((w) => w.seedHash === hash);
      if (hd)
        return {
          seedHash: hd.seedHash,
          alias: hd.alias,
          usesPassword: hd.usesPassword,
          passwordHint: hd.passwordHint,
        };
    }
    for (const hash of hashes) {
      const sk = singleKeyWallets.find((w) => w.keyHash === hash);
      if (sk)
        return {
          seedHash: sk.keyHash,
          alias: sk.alias,
          usesPassword: sk.usesPassword,
          passwordHint: null,
        };
    }
    return null;
  })();

  // Whether wallet is locked
  const walletLocked =
    !!associatedWallet &&
    associatedWallet.usesPassword &&
    !walletUnlockedHashes.has(associatedWallet.seedHash);

  // Load identities and wallets on mount
  useEffect(() => {
    loadIdentities();
    loadWallets();
  }, [loadIdentities, loadWallets]);

  // Subscribe to identity updates
  useEffect(() => {
    let cleanup: (() => void) | undefined;
    subscribeIdentityUpdates()
      .then((unsub) => {
        cleanup = unsub;
      })
      .catch((e) => console.error("Failed to subscribe to identity events:", e));
    return () => {
      cleanup?.();
    };
  }, [subscribeIdentityUpdates]);

  // Track identity selection from child component
  const handleIdentityChange = useCallback((identityId: string) => {
    setSelectedIdentityId(identityId);
  }, []);

  const handleSubmit = useCallback(
    async (params: { identityId: string; name: string }) => {
      setStatus({ type: "registering", startedAt: Date.now() });
      try {
        const result = await commands.identityRegisterDpnsName({
          identityId: params.identityId,
          name: params.name,
        });
        if (result.status === "ok") {
          setStatus({
            type: "success",
            contested: isContestedName(params.name),
            feeEstimated: null,
            feeActual: null,
          });
          // Reload identity (updates balance, DPNS names)
          reloadIdentity(params.identityId);
          // Refresh local DPNS names list
          refreshDpnsNames();
        } else {
          setStatus({ type: "error", message: result.error });
        }
      } catch (e) {
        setStatus({
          type: "error",
          message: e instanceof Error ? e.message : String(e),
        });
      }
    },
    [reloadIdentity, refreshDpnsNames],
  );

  const handleDismissError = useCallback(() => {
    setStatus({ type: "form" });
  }, []);

  const handleBack = useCallback(() => {
    navigate({ to: "/contracts/dpns-active" });
  }, [navigate]);

  const handleRegisterAnother = useCallback(() => {
    setStatus({ type: "form" });
  }, []);

  // Wallet unlock handlers
  const handleRequestUnlock = useCallback(() => {
    setWalletUnlockOpen(true);
    setWalletUnlockError(null);
  }, []);

  const handleWalletUnlockResult = useCallback(
    async (result: WalletUnlockResult) => {
      if (result.status === "unlocked" && associatedWallet) {
        setWalletUnlockError(null);
        try {
          await commands.walletNotifyUnlocked(associatedWallet.seedHash);
          setWalletUnlockedHashes(
            (prev) => new Set([...prev, associatedWallet.seedHash]),
          );
        } catch (e) {
          setWalletUnlockError(
            e instanceof Error ? e.message : String(e),
          );
          return;
        }
      }
      setWalletUnlockOpen(false);
    },
    [associatedWallet],
  );

  // Show loading while identities are loading
  if (identitiesLoading && identities.length === 0) {
    return (
      <div className="flex flex-1 items-center justify-center">
        <LoadingSpinner label="Loading identities..." />
      </div>
    );
  }

  return (
    <div className="flex flex-1 flex-col p-6 max-w-2xl">
      <RegisterDpnsNameScreen
        identities={identities}
        status={status}
        source="dpns"
        walletLocked={walletLocked}
        onRequestUnlock={handleRequestUnlock}
        onIdentityChange={handleIdentityChange}
        onSubmit={handleSubmit}
        onDismissError={handleDismissError}
        onBack={handleBack}
        onRegisterAnother={handleRegisterAnother}
      />

      {/* Wallet unlock dialog */}
      {associatedWallet && (
        <WalletUnlockDialog
          open={walletUnlockOpen}
          onOpenChange={setWalletUnlockOpen}
          walletAlias={
            associatedWallet.alias ||
            associatedWallet.seedHash.slice(0, 10)
          }
          passwordHint={associatedWallet.passwordHint ?? null}
          error={walletUnlockError}
          onResult={handleWalletUnlockResult}
        />
      )}
    </div>
  );
}
