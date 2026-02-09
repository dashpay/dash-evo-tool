import { useCallback, useEffect } from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  RegisterDpnsNameScreen,
  type RegisterDpnsNameStatus,
} from "@/components/identity/RegisterDpnsNameScreen";
import { useIdentityStore } from "@/stores/identityStore";
import { useContestStore } from "@/stores/contestStore";
import { commands } from "@/bindings";
import { LoadingSpinner } from "@/components/feedback";
import { useState } from "react";

export function DpnsRegisterNameScreen() {
  const navigate = useNavigate();

  // Identity store
  const {
    identities,
    loading: identitiesLoading,
    loadIdentities,
    reloadIdentity,
    subscribeToUpdates: subscribeIdentityUpdates,
  } = useIdentityStore();

  // Contest store — for refreshing DPNS names after registration
  const { refreshDpnsNames } = useContestStore();

  // Registration status
  const [status, setStatus] = useState<RegisterDpnsNameStatus>({
    type: "form",
  });

  // Load identities on mount
  useEffect(() => {
    loadIdentities();
  }, [loadIdentities]);

  // Subscribe to identity updates
  useEffect(() => {
    let cleanup: (() => void) | undefined;
    subscribeIdentityUpdates().then((unsub) => {
      cleanup = unsub;
    });
    return () => {
      cleanup?.();
    };
  }, [subscribeIdentityUpdates]);

  // Submit registration
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
            contested: false,
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
        onSubmit={handleSubmit}
        onDismissError={handleDismissError}
        onBack={handleBack}
        onRegisterAnother={handleRegisterAnother}
      />
    </div>
  );
}
