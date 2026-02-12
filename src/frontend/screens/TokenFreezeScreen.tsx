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
 * Token Freeze screen — allows freezing an identity's tokens for a particular contract.
 *
 * Reads token context from route search params:
 *   tokenId, contractId, tokenPosition, identityId, name, balance, decimals
 *
 * Optional group action params (from Group Actions screen):
 *   groupActionId, details
 */
export function TokenFreezeScreen() {
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

  // Parse group action details for pre-populated freeze identity
  const groupFreezeIdentity =
    search.details
      ? (() => {
          try {
            const d = JSON.parse(search.details) as Record<string, unknown>;
            return typeof d.freezeIdentityId === "string"
              ? d.freezeIdentityId
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
  const [freezeIdentityId, setFreezeIdentityId] = useState(groupFreezeIdentity);

  // Validation
  const isValid = freezeIdentityId.trim().length > 0;
  const validationMessage =
    freezeIdentityId.length > 0 && !isValid
      ? "Please enter an identity ID to freeze."
      : undefined;

  // Confirmation
  const confirmation: ConfirmationConfig | undefined = isValid
    ? {
        title: "Confirm Freeze",
        description: `Are you sure you want to freeze identity ${freezeIdentityId.slice(0, 16)}...?`,
        confirmLabel: isGroupSigning ? "Sign Freeze" : "Freeze",
      }
    : undefined;

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
      return commands.tokenFreeze({
        operation: {
          identityId: params.identityId,
          contractId: tokenContext.contractId,
          tokenPosition: tokenContext.tokenPosition,
          keyId: params.keyId,
          publicNote: params.publicNote,
        },
        freezeIdentityId,
        groupInfo: groupInfo,
      });
    },
    [
      tokenContext.contractId,
      tokenContext.tokenPosition,
      freezeIdentityId,
      buildGroupInfo,
    ],
  );

  // Reset
  const handleDoAnother = useCallback(() => {
    setFreezeIdentityId("");
  }, []);

  return (
    <TokenOperationForm
      actionName="Freeze"
      tokenContext={tokenContext}
      groupAction={groupAction}
      isValid={isValid}
      validationMessage={validationMessage}
      confirmation={confirmation}
      onSubmit={handleSubmit}
      resultEventType="tokenCompleted"
      successMessage="Identity tokens frozen successfully."
      doAnotherLabel="Freeze Another"
      onDoAnother={handleDoAnother}
    >
      {isGroupSigning ? (
        <div className="rounded-md border border-border bg-muted/30 p-3 text-sm">
          <span className="text-muted-foreground">Identity to freeze: </span>
          <span className="font-mono font-medium">{freezeIdentityId || "N/A"}</span>
        </div>
      ) : (
        <div className="space-y-2">
          <Label htmlFor="freeze-identity-id">Identity ID to Freeze</Label>
          <Input
            id="freeze-identity-id"
            placeholder="Enter identity ID (Base58 or Hex)"
            value={freezeIdentityId}
            onChange={(e) => setFreezeIdentityId(e.target.value)}
            className="font-mono"
          />
        </div>
      )}
    </TokenOperationForm>
  );
}
