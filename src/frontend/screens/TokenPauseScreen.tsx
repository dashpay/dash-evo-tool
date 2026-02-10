import { useCallback } from "react";
import { useRouterState } from "@tanstack/react-router";
import { commands } from "@/bindings";
import { TokenOperationForm } from "@/components/token/TokenOperationForm";
import type {
  ConfirmationConfig,
  GroupActionContext,
} from "@/components/token/TokenOperationForm";

/**
 * Token Pause screen — pauses all token transfers for a contract.
 *
 * This is an emergency action that stops all transfers. Only contract owners
 * or authorized group members can pause tokens.
 *
 * Reads token context from route search params:
 *   tokenId, contractId, tokenPosition, identityId, name, balance, decimals
 *
 * Optional group action params (from Group Actions screen):
 *   groupActionId, details
 */
export function TokenPauseScreen() {
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

  const groupAction: GroupActionContext | undefined = isGroupSigning
    ? { groupActionId, hasGroup: true, isUnilateral: false }
    : undefined;

  // Confirmation
  const confirmation: ConfirmationConfig = {
    title: "Confirm Pause",
    description:
      "Are you sure you want to pause all token transfers for this contract? This is an emergency action.",
    confirmLabel: isGroupSigning ? "Sign Pause" : "Pause",
  };

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
      return commands.tokenPause({
        operation: {
          identityId: params.identityId,
          contractId: tokenContext.contractId,
          tokenPosition: tokenContext.tokenPosition,
          keyId: params.keyId,
          publicNote: params.publicNote,
        },
        groupInfo: groupInfo as unknown as null,
      });
    },
    [tokenContext.contractId, tokenContext.tokenPosition, buildGroupInfo],
  );

  return (
    <TokenOperationForm
      actionName="Pause"
      tokenContext={tokenContext}
      groupAction={groupAction}
      isValid={true}
      confirmation={confirmation}
      onSubmit={handleSubmit}
      resultType="Token"
      successMessage="Token transfers have been paused successfully."
      doAnotherLabel="Back to Tokens"
    />
  );
}
