import { useCallback, useState } from "react";
import { useRouterState } from "@tanstack/react-router";
import { commands } from "@/bindings";
import { TokenOperationForm } from "@/components/token/TokenOperationForm";
import type {
  ConfirmationConfig,
  GroupActionContext,
} from "@/components/token/TokenOperationForm";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

/**
 * Token Unfreeze screen — allows unfreezing a previously frozen identity's tokens.
 *
 * Reads token context from route search params:
 *   tokenId, contractId, tokenPosition, identityId, name, balance, decimals
 *
 * Optional group action params (from Group Actions screen):
 *   groupActionId, details
 */
export function TokenUnfreezeScreen() {
  const search = useRouterState({
    select: (s) => s.location.search as Record<string, string>,
  });

  const tokenContext = {
    tokenId: search.tokenId ?? "",
    contractId: search.contractId ?? "",
    tokenPosition: Number(search.tokenPosition ?? "0"),
    name: search.name ?? null,
    balance: search.balance ?? "0",
    decimals: Number(search.decimals ?? "8"),
    identityId: search.identityId ?? "",
  };

  // Group action context
  const groupActionId = search.groupActionId;
  const isGroupSigning = !!groupActionId;

  // Parse group action details for pre-populated unfreeze identity
  const groupUnfreezeIdentity =
    search.details
      ? (() => {
          try {
            const d = JSON.parse(search.details) as Record<string, unknown>;
            return typeof d.unfreezeIdentityId === "string"
              ? d.unfreezeIdentityId
              : "";
          } catch {
            return "";
          }
        })()
      : "";

  const groupAction: GroupActionContext | undefined = isGroupSigning
    ? { groupActionId, hasGroup: true, isUnilateral: false }
    : undefined;

  // Form state
  const [unfreezeIdentityId, setUnfreezeIdentityId] = useState(groupUnfreezeIdentity);

  // Validation
  const isValid = unfreezeIdentityId.trim().length > 0;
  const validationMessage =
    unfreezeIdentityId.length > 0 && !isValid
      ? "Please enter an identity ID to unfreeze."
      : undefined;

  // Confirmation
  const confirmation: ConfirmationConfig | undefined = isValid
    ? {
        title: "Confirm Unfreeze",
        description: `Are you sure you want to unfreeze identity ${unfreezeIdentityId.slice(0, 16)}...?`,
        confirmLabel: isGroupSigning ? "Sign Unfreeze" : "Unfreeze",
      }
    : undefined;

  // Build group info
  const buildGroupInfo = useCallback(
    (identityId: string, keyId: number) => {
      if (groupActionId) {
        return {
          type: "other_signer",
          action_id: groupActionId,
          signer_identity_id: identityId,
          signer_key_id: keyId,
        };
      }
      return null;
    },
    [groupActionId],
  );

  // Submit
  const handleSubmit = useCallback(
    async (params: {
      identityId: string;
      keyId: number;
      publicNote: string | null;
    }) => {
      const groupInfo = buildGroupInfo(params.identityId, params.keyId);
      return commands.tokenUnfreeze({
        operation: {
          identityId: params.identityId,
          contractId: tokenContext.contractId,
          tokenPosition: tokenContext.tokenPosition,
          keyId: params.keyId,
          publicNote: params.publicNote,
        },
        unfreezeIdentityId,
        groupInfo: groupInfo as unknown as null,
      });
    },
    [
      tokenContext.contractId,
      tokenContext.tokenPosition,
      unfreezeIdentityId,
      buildGroupInfo,
    ],
  );

  // Reset
  const handleDoAnother = useCallback(() => {
    setUnfreezeIdentityId("");
  }, []);

  return (
    <TokenOperationForm
      actionName="Unfreeze"
      tokenContext={tokenContext}
      groupAction={groupAction}
      isValid={isValid}
      validationMessage={validationMessage}
      confirmation={confirmation}
      onSubmit={handleSubmit}
      resultType="Token"
      successMessage="Identity tokens unfrozen successfully."
      doAnotherLabel="Unfreeze Another"
      onDoAnother={handleDoAnother}
    >
      {isGroupSigning ? (
        <div className="rounded-md border border-border bg-muted/30 p-3 text-sm">
          <span className="text-muted-foreground">Identity to unfreeze: </span>
          <span className="font-mono font-medium">{unfreezeIdentityId || "N/A"}</span>
        </div>
      ) : (
        <div className="space-y-2">
          <Label htmlFor="unfreeze-identity-id">Identity ID to Unfreeze</Label>
          <p className="text-xs text-muted-foreground">
            Enter the identity ID of the frozen identity you want to unfreeze.
          </p>
          <Input
            id="unfreeze-identity-id"
            placeholder="Enter frozen identity ID (Base58 or Hex)"
            value={unfreezeIdentityId}
            onChange={(e) => setUnfreezeIdentityId(e.target.value)}
            className="font-mono"
          />
        </div>
      )}
    </TokenOperationForm>
  );
}
