import { useCallback, useEffect, useMemo, useState } from "react";
import { Island } from "@/components/layout";
import { LoadingSpinner } from "@/components/feedback";
import { IdentityListPanel } from "@/components/identity/IdentityListPanel";
import { IdentityDetailPanel } from "@/components/identity/IdentityDetailPanel";
import { KeyManagementScreen } from "@/components/identity/KeyManagementScreen";
import { KeyInfoScreen } from "@/components/identity/KeyInfoScreen";
import { AddKeyDialog } from "@/components/identity/AddKeyDialog";
import {
  WithdrawScreen,
  type WithdrawStatus,
} from "@/components/identity/WithdrawScreen";
import {
  TransferScreen,
  type TransferStatus,
} from "@/components/identity/TransferScreen";
import {
  CreateIdentityScreen,
  type CreateIdentityStatus,
} from "@/components/identity/CreateIdentityScreen";
import {
  TopUpIdentityScreen,
  type TopUpStatus,
} from "@/components/identity/TopUpIdentityScreen";
import {
  LoadIdentityScreen,
  type LoadIdentityStatus,
} from "@/components/identity/LoadIdentityScreen";
import {
  RegisterDpnsNameScreen,
  type RegisterDpnsNameStatus,
} from "@/components/identity/RegisterDpnsNameScreen";
import { useIdentityStore } from "@/stores/identityStore";
import { useWalletStore } from "@/stores/walletStore";
import { commands } from "@/bindings";
import type {
  QualifiedIdentityDto,
  IdentityKeyDto,
  RegisterIdentityFundingMethodDto,
  KeySpecDto,
  TopUpIdentityFundingMethodDto,
  AddKeyToIdentityInput,
} from "@/bindings";
import type { AddKeyStatus } from "@/components/identity/AddKeyDialog";
import type { IdentityOption } from "@/components/shared/IdentitySelector";
import { toast } from "sonner";

// ─── Sub-view types ───────────────────────────────────────────────

type SubView =
  | { type: "detail" }
  | { type: "keys" }
  | { type: "keyInfo"; keyId: number }
  | { type: "addKey" }
  | { type: "withdraw" }
  | { type: "transfer" }
  | { type: "createIdentity" }
  | { type: "topUp" }
  | { type: "loadIdentity" }
  | { type: "registerDpns" };

// ─── IdentitiesScreen ─────────────────────────────────────────────

export function IdentitiesScreen() {
  // Identity store
  const {
    identities,
    selectedIdentityId,
    loading,
    refreshingIds,
    refreshingAll,
    error,
    loadIdentities,
    selectIdentity,
    refreshIdentity,
    refreshAllIdentities,
    setAlias,
    reorderIdentityUp,
    reorderIdentityDown,
    removeIdentity,
    reloadIdentity,
    subscribeToUpdates,
    clearError,
  } = useIdentityStore();

  // Wallet store (for wallet name resolution)
  const { hdWallets, singleKeyWallets, loadWallets } = useWalletStore();

  // Sub-view navigation
  const [subView, setSubView] = useState<SubView>({ type: "detail" });

  // Withdraw state
  const [withdrawStatus, setWithdrawStatus] = useState<WithdrawStatus>({
    type: "form",
  });

  // Transfer state
  const [transferStatus, setTransferStatus] = useState<TransferStatus>({
    type: "form",
  });

  // Create identity state
  const [createIdentityStatus, setCreateIdentityStatus] =
    useState<CreateIdentityStatus>({ type: "form" });

  // Top-up state
  const [topUpStatus, setTopUpStatus] = useState<TopUpStatus>({ type: "form" });

  // Load identity state
  const [loadIdentityStatus, setLoadIdentityStatus] =
    useState<LoadIdentityStatus>({ type: "form" });

  // Add key state
  const [addKeyStatus, setAddKeyStatus] = useState<AddKeyStatus>({
    type: "idle",
  });

  // Register DPNS name state
  const [registerDpnsStatus, setRegisterDpnsStatus] =
    useState<RegisterDpnsNameStatus>({ type: "form" });

  // Key info state
  const [keyInfoState, setKeyInfoState] = useState<{
    isSubmitting: boolean;
    error: string | null;
    success: string | null;
  }>({ isSubmitting: false, error: null, success: null });

  // Load identities and wallets on mount
  useEffect(() => {
    loadIdentities();
    loadWallets();
  }, [loadIdentities, loadWallets]);

  // Subscribe to identity update events
  useEffect(() => {
    let cleanup: (() => void) | undefined;
    subscribeToUpdates()
      .then((unsub) => {
        cleanup = unsub;
      })
      .catch(() => {});
    return () => cleanup?.();
  }, [subscribeToUpdates]);

  // Show toast on error
  useEffect(() => {
    if (error) {
      toast.error(error);
      clearError();
    }
  }, [error, clearError]);

  // Reset sub-view when selection changes
  useEffect(() => {
    setSubView({ type: "detail" });
    setWithdrawStatus({ type: "form" });
    setTransferStatus({ type: "form" });
    setCreateIdentityStatus({ type: "form" });
    setTopUpStatus({ type: "form" });
    setLoadIdentityStatus({ type: "form" });
    setAddKeyStatus({ type: "idle" });
    setRegisterDpnsStatus({ type: "form" });
    setKeyInfoState({ isSubmitting: false, error: null, success: null });
  }, [selectedIdentityId]);

  // ─── Derived state ──────────────────────────────────────────────

  const selectedIdentity: QualifiedIdentityDto | null = useMemo(
    () => identities.find((i) => i.id === selectedIdentityId) ?? null,
    [identities, selectedIdentityId],
  );

  const selectedKey: IdentityKeyDto | null = useMemo(() => {
    if (subView.type !== "keyInfo" || !selectedIdentity) return null;
    return (
      selectedIdentity.keys.find((k) => k.keyId === subView.keyId) ?? null
    );
  }, [subView, selectedIdentity]);

  // Build wallet name map: seed hash → display name
  const walletNames: Record<string, string> = useMemo(() => {
    const names: Record<string, string> = {};
    for (const w of hdWallets) {
      names[w.seedHash] = w.alias?.trim() || w.seedHash.slice(0, 10);
    }
    for (const w of singleKeyWallets) {
      names[w.keyHash] = w.alias?.trim() || w.keyHash.slice(0, 10);
    }
    return names;
  }, [hdWallets, singleKeyWallets]);

  // Build known identities list for TransferScreen
  const knownIdentities: IdentityOption[] = useMemo(() => {
    return identities
      .filter((i) => i.id !== selectedIdentityId)
      .map((i) => ({
        id: i.id,
        displayName: i.alias?.trim() || i.id.slice(0, 12),
      }));
  }, [identities, selectedIdentityId]);

  // ─── Navigation callbacks ──────────────────────────────────────

  const handleViewKeys = useCallback((_identityId: string) => {
    setSubView({ type: "keys" });
  }, []);

  const handleViewKey = useCallback(
    (_identityId: string, keyId: number) => {
      setSubView({ type: "keyInfo", keyId });
      setKeyInfoState({ isSubmitting: false, error: null, success: null });
    },
    [],
  );

  const handleAddKey = useCallback(() => {
    setSubView({ type: "addKey" });
    setAddKeyStatus({ type: "idle" });
  }, []);

  const handleWithdraw = useCallback((_identityId: string) => {
    setSubView({ type: "withdraw" });
    setWithdrawStatus({ type: "form" });
  }, []);

  const handleTransfer = useCallback((_identityId: string) => {
    setSubView({ type: "transfer" });
    setTransferStatus({ type: "form" });
  }, []);

  const handleTopUp = useCallback((_identityId: string) => {
    setSubView({ type: "topUp" });
    setTopUpStatus({ type: "form" });
  }, []);

  const handleRegisterDpns = useCallback((_identityId: string) => {
    setSubView({ type: "registerDpns" });
    setRegisterDpnsStatus({ type: "form" });
  }, []);

  const handleCreateIdentity = useCallback(() => {
    setSubView({ type: "createIdentity" });
    setCreateIdentityStatus({ type: "form" });
  }, []);

  const handleLoadIdentity = useCallback(() => {
    setSubView({ type: "loadIdentity" });
    setLoadIdentityStatus({ type: "form" });
  }, []);

  const handleNavigateToWallet = useCallback((_seedHash: string) => {
    // TODO: Navigate to wallet screen (requires cross-screen navigation)
    toast.info("Navigate to wallet — coming in a future task");
  }, []);

  const handleBackToDetail = useCallback(() => {
    setSubView({ type: "detail" });
  }, []);

  const handleBackToKeys = useCallback(() => {
    setSubView({ type: "keys" });
    setKeyInfoState({ isSubmitting: false, error: null, success: null });
  }, []);

  // ─── Create identity callbacks ────────────────────────────────────

  const handleCreateIdentitySubmit = useCallback(
    async (params: {
      walletSeedHash: string;
      identityIndex: number;
      alias: string;
      masterKeyType: string;
      keySpecs: KeySpecDto[];
      useDefaultKeys: boolean;
      fundingMethod: RegisterIdentityFundingMethodDto;
    }) => {
      setCreateIdentityStatus({
        type: "waitingForPlatform",
        startedAt: Date.now(),
      });
      try {
        const result = await commands.identityRegister({
          walletSeedHash: params.walletSeedHash,
          identityIndex: params.identityIndex,
          alias: params.alias,
          masterKeyType: params.masterKeyType,
          keySpecs: params.keySpecs,
          useDefaultKeys: params.useDefaultKeys,
          fundingMethod: params.fundingMethod,
        });
        if (result.status === "ok") {
          setCreateIdentityStatus({ type: "success" });
          // Reload identities to pick up the new one
          loadIdentities();
        } else {
          setCreateIdentityStatus({ type: "error", message: result.error });
        }
      } catch (e) {
        setCreateIdentityStatus({
          type: "error",
          message: e instanceof Error ? e.message : String(e),
        });
      }
    },
    [loadIdentities],
  );

  // ─── Top-up callbacks ──────────────────────────────────────────

  const handleTopUpSubmit = useCallback(
    async (params: {
      identityId: string;
      walletSeedHash: string;
      identityIndex: number;
      fundingMethod: TopUpIdentityFundingMethodDto;
    }) => {
      setTopUpStatus({ type: "waitingForPlatform", startedAt: Date.now() });
      try {
        const result = await commands.identityTopUp({
          identityId: params.identityId,
          walletSeedHash: params.walletSeedHash,
          identityIndex: params.identityIndex,
          fundingMethod: params.fundingMethod,
        });
        if (result.status === "ok") {
          setTopUpStatus({ type: "success" });
          reloadIdentity(params.identityId);
        } else {
          setTopUpStatus({ type: "error", message: result.error });
        }
      } catch (e) {
        setTopUpStatus({
          type: "error",
          message: e instanceof Error ? e.message : String(e),
        });
      }
    },
    [reloadIdentity],
  );

  const handleTopUpFromPlatformAddress = useCallback(
    async (params: {
      identityId: string;
      walletSeedHash: string;
      outputs: { address: string; amount: number }[];
    }) => {
      setTopUpStatus({ type: "waitingForPlatform", startedAt: Date.now() });
      try {
        const result = await commands.identityTopUpFromPlatformAddresses({
          identityId: params.identityId,
          walletSeedHash: params.walletSeedHash,
          outputs: params.outputs,
        });
        if (result.status === "ok") {
          setTopUpStatus({ type: "success" });
          reloadIdentity(params.identityId);
        } else {
          setTopUpStatus({ type: "error", message: result.error });
        }
      } catch (e) {
        setTopUpStatus({
          type: "error",
          message: e instanceof Error ? e.message : String(e),
        });
      }
    },
    [reloadIdentity],
  );

  // ─── Load identity callbacks ──────────────────────────────────

  const handleLoadById = useCallback(
    async (params: {
      identityId: string;
      identityType: string;
      alias: string;
      votingPrivateKey: string;
      ownerPrivateKey: string;
      payoutAddressPrivateKey: string;
      keys: string[];
      deriveKeysFromWallets: boolean;
      selectedWalletSeedHash: string | null;
    }) => {
      setLoadIdentityStatus({ type: "loading", startedAt: Date.now() });
      try {
        const result = await commands.identityLoad({
          identityId: params.identityId,
          identityType: params.identityType as "user" | "masternode" | "evonode",
          alias: params.alias,
          votingPrivateKey: params.votingPrivateKey,
          ownerPrivateKey: params.ownerPrivateKey,
          payoutAddressPrivateKey: params.payoutAddressPrivateKey,
          keys: params.keys,
          deriveKeysFromWallets: params.deriveKeysFromWallets,
          selectedWalletSeedHash: params.selectedWalletSeedHash,
        });
        if (result.status === "ok") {
          setLoadIdentityStatus({
            type: "success",
            message: "Identity loaded successfully",
          });
          loadIdentities();
        } else {
          setLoadIdentityStatus({ type: "error", message: result.error });
        }
      } catch (e) {
        setLoadIdentityStatus({
          type: "error",
          message: e instanceof Error ? e.message : String(e),
        });
      }
    },
    [loadIdentities],
  );

  const handleSearchFromWallet = useCallback(
    async (params: { walletSeedHash: string; identityIndex: number }) => {
      setLoadIdentityStatus({ type: "loading", startedAt: Date.now() });
      try {
        const result = await commands.identitySearchFromWallet({
          walletSeedHash: params.walletSeedHash,
          identityIndex: params.identityIndex,
        });
        if (result.status === "ok") {
          setLoadIdentityStatus({
            type: "success",
            message: "Identity search completed",
          });
          loadIdentities();
        } else {
          setLoadIdentityStatus({ type: "error", message: result.error });
        }
      } catch (e) {
        setLoadIdentityStatus({
          type: "error",
          message: e instanceof Error ? e.message : String(e),
        });
      }
    },
    [loadIdentities],
  );

  const handleSearchUpToIndex = useCallback(
    async (params: { walletSeedHash: string; maxIdentityIndex: number }) => {
      setLoadIdentityStatus({ type: "loading", startedAt: Date.now() });
      try {
        const result = await commands.identitySearchUpToIndex({
          walletSeedHash: params.walletSeedHash,
          maxIdentityIndex: params.maxIdentityIndex,
        });
        if (result.status === "ok") {
          setLoadIdentityStatus({
            type: "success",
            message: "Wallet search completed",
          });
          loadIdentities();
        } else {
          setLoadIdentityStatus({ type: "error", message: result.error });
        }
      } catch (e) {
        setLoadIdentityStatus({
          type: "error",
          message: e instanceof Error ? e.message : String(e),
        });
      }
    },
    [loadIdentities],
  );

  const handleSearchByDpnsName = useCallback(
    async (params: { name: string; walletSeedHash: string | null }) => {
      setLoadIdentityStatus({ type: "loading", startedAt: Date.now() });
      try {
        const result = await commands.identitySearchByDpnsName({
          name: params.name,
          walletSeedHash: params.walletSeedHash,
        });
        if (result.status === "ok") {
          setLoadIdentityStatus({
            type: "success",
            message: `Found identity for "${params.name}.dash"`,
          });
          loadIdentities();
        } else {
          setLoadIdentityStatus({ type: "error", message: result.error });
        }
      } catch (e) {
        setLoadIdentityStatus({
          type: "error",
          message: e instanceof Error ? e.message : String(e),
        });
      }
    },
    [loadIdentities],
  );

  // ─── Register DPNS name callbacks ─────────────────────────────────

  const handleRegisterDpnsSubmit = useCallback(
    async (params: { identityId: string; name: string }) => {
      setRegisterDpnsStatus({ type: "registering", startedAt: Date.now() });
      try {
        const result = await commands.identityRegisterDpnsName({
          identityId: params.identityId,
          name: params.name,
        });
        if (result.status === "ok") {
          // The actual result comes via TaskResultEvent — but we set a
          // preliminary success here. The identityStore subscription will
          // reload the identity when the event arrives.
          setRegisterDpnsStatus({
            type: "success",
            contested: false,
            feeEstimated: null,
            feeActual: null,
          });
          reloadIdentity(params.identityId);
        } else {
          setRegisterDpnsStatus({ type: "error", message: result.error });
        }
      } catch (e) {
        setRegisterDpnsStatus({
          type: "error",
          message: e instanceof Error ? e.message : String(e),
        });
      }
    },
    [reloadIdentity],
  );

  // ─── Withdraw callbacks ─────────────────────────────────────────

  const handleWithdrawSubmit = useCallback(
    async (params: {
      identityId: string;
      toAddress: string | null;
      credits: number;
      keyId: number | null;
    }) => {
      setWithdrawStatus({ type: "sending", startedAt: Date.now() });
      try {
        const result = await commands.identityWithdraw({
          identityId: params.identityId,
          toAddress: params.toAddress,
          credits: params.credits,
          keyId: params.keyId,
        });
        if (result.status === "ok") {
          setWithdrawStatus({ type: "success" });
          // Reload the identity to reflect the new balance
          reloadIdentity(params.identityId);
        } else {
          setWithdrawStatus({ type: "error", message: result.error });
        }
      } catch (e) {
        setWithdrawStatus({
          type: "error",
          message: e instanceof Error ? e.message : String(e),
        });
      }
    },
    [reloadIdentity],
  );

  // ─── Transfer callbacks ─────────────────────────────────────────

  const handleTransferToIdentity = useCallback(
    async (params: {
      fromIdentityId: string;
      toIdentityId: string;
      credits: number;
      keyId: number | null;
    }) => {
      setTransferStatus({ type: "sending", startedAt: Date.now() });
      try {
        const result = await commands.identityTransfer({
          fromIdentityId: params.fromIdentityId,
          toIdentityId: params.toIdentityId,
          credits: params.credits,
          keyId: params.keyId,
        });
        if (result.status === "ok") {
          setTransferStatus({ type: "success" });
          reloadIdentity(params.fromIdentityId);
        } else {
          setTransferStatus({ type: "error", message: result.error });
        }
      } catch (e) {
        setTransferStatus({
          type: "error",
          message: e instanceof Error ? e.message : String(e),
        });
      }
    },
    [reloadIdentity],
  );

  const handleTransferToAddress = useCallback(
    async (params: {
      identityId: string;
      address: string;
      credits: number;
      keyId: number | null;
    }) => {
      setTransferStatus({ type: "sending", startedAt: Date.now() });
      try {
        const result = await commands.identityTransferToAddresses({
          identityId: params.identityId,
          outputs: [{ address: params.address, amount: params.credits }],
          keyId: params.keyId,
        });
        if (result.status === "ok") {
          setTransferStatus({ type: "success" });
          reloadIdentity(params.identityId);
        } else {
          setTransferStatus({ type: "error", message: result.error });
        }
      } catch (e) {
        setTransferStatus({
          type: "error",
          message: e instanceof Error ? e.message : String(e),
        });
      }
    },
    [reloadIdentity],
  );

  // ─── Add key callbacks ──────────────────────────────────────────

  const handleAddKeySubmit = useCallback(
    async (params: {
      purpose: string;
      securityLevel: string;
      keyType: string;
      privateKeyHex: string;
      contractBounds?: {
        contractId: string;
        documentTypeName?: string;
      };
    }) => {
      if (!selectedIdentityId) return;
      setAddKeyStatus({ type: "submitting", startedAt: Date.now() });
      try {
        // Transform contract bounds from dialog format to tagged union for IPC
        let contractBounds: AddKeyToIdentityInput["contractBounds"] = null;
        if (params.contractBounds) {
          contractBounds = params.contractBounds.documentTypeName
            ? {
                type: "singleContractDocumentType" as const,
                contractId: params.contractBounds.contractId,
                documentTypeName: params.contractBounds.documentTypeName,
              }
            : {
                type: "singleContract" as const,
                contractId: params.contractBounds.contractId,
              };
        }

        const result = await commands.identityAddKey({
          identityId: selectedIdentityId,
          purpose: params.purpose,
          securityLevel: params.securityLevel,
          keyType: params.keyType,
          privateKeyHex: params.privateKeyHex,
          contractBounds,
        });
        if (result.status === "ok") {
          setAddKeyStatus({ type: "success" });
          reloadIdentity(selectedIdentityId);
        } else {
          setAddKeyStatus({ type: "error", message: result.error });
        }
      } catch (e) {
        setAddKeyStatus({
          type: "error",
          message: e instanceof Error ? e.message : String(e),
        });
      }
    },
    [selectedIdentityId, reloadIdentity],
  );

  // ─── Key info callbacks ──────────────────────────────────────────

  const handleDisableKey = useCallback(
    async (keyId: number) => {
      if (!selectedIdentityId) return;
      setKeyInfoState((s) => ({ ...s, isSubmitting: true, error: null }));
      try {
        const result = await commands.identityDisableKeys({
          identityId: selectedIdentityId,
          keyIds: [keyId],
        });
        if (result.status === "ok") {
          setKeyInfoState({
            isSubmitting: false,
            error: null,
            success: "Key disabled successfully",
          });
          reloadIdentity(selectedIdentityId);
        } else {
          setKeyInfoState({
            isSubmitting: false,
            error: result.error,
            success: null,
          });
        }
      } catch (e) {
        setKeyInfoState({
          isSubmitting: false,
          error: e instanceof Error ? e.message : String(e),
          success: null,
        });
      }
    },
    [selectedIdentityId, reloadIdentity],
  );

  const handleReplaceKey = useCallback(
    async (keyId: number, newKeyType: string, newPrivateKeyHex: string) => {
      if (!selectedIdentityId) return;
      setKeyInfoState((s) => ({ ...s, isSubmitting: true, error: null }));
      try {
        const result = await commands.identityReplaceKey({
          identityId: selectedIdentityId,
          oldKeyId: keyId,
          newKeyType,
          newPurpose: "AUTHENTICATION",
          newSecurityLevel: "MASTER",
          newPrivateKeyHex,
        });
        if (result.status === "ok") {
          setKeyInfoState({
            isSubmitting: false,
            error: null,
            success: "Key replaced successfully",
          });
          reloadIdentity(selectedIdentityId);
        } else {
          setKeyInfoState({
            isSubmitting: false,
            error: result.error,
            success: null,
          });
        }
      } catch (e) {
        setKeyInfoState({
          isSubmitting: false,
          error: e instanceof Error ? e.message : String(e),
          success: null,
        });
      }
    },
    [selectedIdentityId, reloadIdentity],
  );

  const handleSignMessage = useCallback(
    async (_keyId: number, _message: string): Promise<string> => {
      // TODO: Add identitySignMessage IPC command when backend supports it
      throw new Error("Message signing is not yet implemented");
    },
    [],
  );

  // ─── Render ──────────────────────────────────────────────────────

  if (loading) {
    return (
      <div className="flex flex-1 items-center justify-center">
        <LoadingSpinner label="Loading identities..." />
      </div>
    );
  }

  return (
    <div className="flex flex-1 gap-3 min-h-0">
      {/* Left Panel — Identity List */}
      <Island noPadding className="w-[300px] shrink-0 flex flex-col overflow-hidden">
        <IdentityListPanel
          identities={identities}
          selectedIdentityId={selectedIdentityId}
          refreshingIds={refreshingIds}
          refreshingAll={refreshingAll}
          onSelectIdentity={selectIdentity}
          onSetAlias={setAlias}
          onReorderUp={reorderIdentityUp}
          onReorderDown={reorderIdentityDown}
          onRemoveIdentity={removeIdentity}
          onRefreshIdentity={refreshIdentity}
          onRefreshAll={refreshAllIdentities}
          onViewKeys={handleViewKeys}
          onRegisterDpns={handleRegisterDpns}
          onTopUp={handleTopUp}
          onWithdraw={handleWithdraw}
          onTransfer={handleTransfer}
          onCreateIdentity={handleCreateIdentity}
          onLoadIdentity={handleLoadIdentity}
        />
      </Island>

      {/* Right Panel — Detail / Sub-views */}
      <Island className="flex-1 min-w-0 overflow-auto">
        {subView.type === "createIdentity" ? (
          renderCreateIdentity()
        ) : subView.type === "loadIdentity" ? (
          renderLoadIdentity()
        ) : selectedIdentity ? (
          renderRightPanel()
        ) : (
          <div className="flex flex-1 items-center justify-center h-full text-muted-foreground">
            <p className="text-sm">Select an identity to view details</p>
          </div>
        )}
      </Island>
    </div>
  );

  // ─── Create identity renderer ───────────────────────────────────

  function renderCreateIdentity() {
    return (
      <CreateIdentityScreen
        wallets={hdWallets}
        status={createIdentityStatus}
        onSubmit={handleCreateIdentitySubmit}
        onDismissError={() => setCreateIdentityStatus({ type: "form" })}
        onBack={handleBackToDetail}
        onBackToIdentities={() => {
          setSubView({ type: "detail" });
          setCreateIdentityStatus({ type: "form" });
          loadIdentities();
        }}
        onRegisterDpns={() => {
          setSubView({ type: "registerDpns" });
          setRegisterDpnsStatus({ type: "form" });
        }}
      />
    );
  }

  // ─── Load identity renderer ────────────────────────────────────

  function renderLoadIdentity() {
    return (
      <LoadIdentityScreen
        wallets={hdWallets}
        status={loadIdentityStatus}
        onLoadById={handleLoadById}
        onSearchFromWallet={handleSearchFromWallet}
        onSearchUpToIndex={handleSearchUpToIndex}
        onSearchByDpnsName={handleSearchByDpnsName}
        onDismissError={() => setLoadIdentityStatus({ type: "form" })}
        onBack={handleBackToDetail}
        onLoadAnother={() => setLoadIdentityStatus({ type: "form" })}
      />
    );
  }

  // ─── Right panel renderer ────────────────────────────────────────

  function renderRightPanel() {
    if (!selectedIdentity) return null;

    switch (subView.type) {
      case "detail":
        return (
          <IdentityDetailPanel
            identity={selectedIdentity}
            isRefreshing={refreshingIds.has(selectedIdentity.id)}
            walletNames={walletNames}
            onRefresh={() => refreshIdentity(selectedIdentity.id)}
            onTopUp={handleTopUp}
            onWithdraw={handleWithdraw}
            onTransfer={handleTransfer}
            onRegisterDpns={handleRegisterDpns}
            onViewKeys={handleViewKeys}
            onViewKey={handleViewKey}
            onNavigateToWallet={handleNavigateToWallet}
          />
        );

      case "keys":
        return (
          <KeyManagementScreen
            identity={selectedIdentity}
            onViewKey={(keyId) =>
              setSubView({ type: "keyInfo", keyId })
            }
            onAddKey={handleAddKey}
            onBack={handleBackToDetail}
          />
        );

      case "keyInfo":
        return selectedKey ? (
          <KeyInfoScreen
            identity={selectedIdentity}
            keyData={selectedKey}
            onBack={handleBackToKeys}
            onDisableKey={handleDisableKey}
            onReplaceKey={handleReplaceKey}
            onSignMessage={handleSignMessage}
            isSubmitting={keyInfoState.isSubmitting}
            error={keyInfoState.error}
            success={keyInfoState.success}
            onClearError={() =>
              setKeyInfoState((s) => ({ ...s, error: null }))
            }
            onClearSuccess={() =>
              setKeyInfoState((s) => ({ ...s, success: null }))
            }
          />
        ) : (
          <div className="flex flex-1 items-center justify-center h-full text-muted-foreground">
            <p className="text-sm">Key not found</p>
          </div>
        );

      case "addKey":
        return (
          <AddKeyDialog
            identity={selectedIdentity}
            status={addKeyStatus}
            onSubmit={handleAddKeySubmit}
            onDismissError={() => setAddKeyStatus({ type: "idle" })}
            onBackToKeys={handleBackToKeys}
            onAddAnother={() => setAddKeyStatus({ type: "idle" })}
            onBack={handleBackToKeys}
          />
        );

      case "withdraw":
        return (
          <WithdrawScreen
            identity={selectedIdentity}
            status={withdrawStatus}
            onSubmit={handleWithdrawSubmit}
            onDismissError={() => setWithdrawStatus({ type: "form" })}
            onBack={handleBackToDetail}
          />
        );

      case "transfer":
        return (
          <TransferScreen
            identity={selectedIdentity}
            status={transferStatus}
            knownIdentities={knownIdentities}
            onSubmitToIdentity={handleTransferToIdentity}
            onSubmitToAddress={handleTransferToAddress}
            onDismissError={() => setTransferStatus({ type: "form" })}
            onBack={handleBackToDetail}
          />
        );

      case "topUp":
        return (
          <TopUpIdentityScreen
            identity={selectedIdentity}
            wallets={hdWallets}
            status={topUpStatus}
            onSubmit={handleTopUpSubmit}
            onSubmitPlatformAddress={handleTopUpFromPlatformAddress}
            onDismissError={() => setTopUpStatus({ type: "form" })}
            onBack={handleBackToDetail}
          />
        );

      case "registerDpns":
        return (
          <RegisterDpnsNameScreen
            identities={identities}
            preselectedIdentityId={selectedIdentityId}
            status={registerDpnsStatus}
            source="identities"
            onSubmit={handleRegisterDpnsSubmit}
            onDismissError={() => setRegisterDpnsStatus({ type: "form" })}
            onBack={handleBackToDetail}
            onRegisterAnother={() => setRegisterDpnsStatus({ type: "form" })}
          />
        );
    }
  }
}
