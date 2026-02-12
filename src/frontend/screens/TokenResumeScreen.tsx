import { useCallback } from "react";
import { useRouterState } from "@tanstack/react-router";
import { commands } from "@/bindings";
import { TokenOperationForm } from "@/components/token/TokenOperationForm";
import type {
  ConfirmationConfig,
  GroupActionContext,
} from "@/components/token/TokenOperationForm";

/**
 * Token Resume screen — resumes token transfers after a pause.
 *
 * This is an emergency action that re-enables all transfers. Only contract owners
 * or authorized group members can resume tokens.
 *
 * Reads token context from route search params:
 *   tokenId, contractId, tokenPosition, identityId, name, balance, decimals
 *
 * Optional group action params (from Group Actions screen):
 *   groupActionId, details
 */
export function TokenResumeScreen() {
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
    title: "Confirm Resume",
    description:
      "Are you sure you want to resume token transfers for this contract?",
    confirmLabel: isGroupSigning ? "Sign Resume" : "Resume",
  };

  // Build group info
  const buildGroupInfo = useCallback(() => {
    const groupPos = Number(search.groupPosition ?? "0");
    if (groupActionId) {
      return {
        type: "otherSigner",
        groupContractPosition: groupPos,
        actionId: groupActionId,
        actionIsProposer: false,
      };
    }
    return null;
  }, [groupActionId, search.groupPosition]);

  // Submit
  const handleSubmit = useCallback(
    async (params: {
      identityId: string;
      keyId: number;
      publicNote: string | null;
    }) => {
      const groupInfo = buildGroupInfo();
      return commands.tokenResume({
        operation: {
          identityId: params.identityId,
          contractId: tokenContext.contractId,
          tokenPosition: tokenContext.tokenPosition,
          keyId: params.keyId,
          publicNote: params.publicNote,
        },
        groupInfo: groupInfo,
      });
    },
    [tokenContext.contractId, tokenContext.tokenPosition, buildGroupInfo],
  );

  return (
    <TokenOperationForm
      actionName="Resume"
      tokenContext={tokenContext}
      groupAction={groupAction}
      isValid={true}
      confirmation={confirmation}
      onSubmit={handleSubmit}
      resultEventType="tokenCompleted"
      successMessage="Token transfers have been resumed successfully."
      doAnotherLabel="Back to Tokens"
    />
  );
}
