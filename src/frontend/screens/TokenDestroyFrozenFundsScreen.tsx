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
 * Destroy Frozen Funds screen — permanently destroys all frozen tokens held by a
 * target identity for this token contract. This action cannot be undone.
 *
 * Reads token context from route search params:
 *   tokenId, contractId, tokenPosition, identityId, name, balance, decimals
 *
 * Optional group action params (from Group Actions screen):
 *   groupActionId, details
 */
export function TokenDestroyFrozenFundsScreen() {
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

  // Parse group action details for pre-populated frozen identity
  const groupFrozenIdentity =
    search.details
      ? (() => {
          try {
            const d = JSON.parse(search.details) as Record<string, unknown>;
            return typeof d.frozenIdentityId === "string"
              ? d.frozenIdentityId
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
  const [frozenIdentityId, setFrozenIdentityId] = useState(groupFrozenIdentity);

  // Validation
  const isValid = frozenIdentityId.trim().length > 0;
  const validationMessage =
    frozenIdentityId.length > 0 && !isValid
      ? "Please enter a frozen identity ID."
      : undefined;

  // Destructive confirmation — this action cannot be undone
  const confirmation: ConfirmationConfig | undefined = isValid
    ? {
        title: "Confirm Destroy Frozen Funds",
        description: `Are you sure you want to destroy all frozen funds for identity ${frozenIdentityId.slice(0, 16)}...? This action cannot be undone.`,
        confirmLabel: isGroupSigning ? "Sign Destroy" : "Destroy",
        destructive: true,
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
      return commands.tokenDestroyFrozenFunds({
        operation: {
          identityId: params.identityId,
          contractId: tokenContext.contractId,
          tokenPosition: tokenContext.tokenPosition,
          keyId: params.keyId,
          publicNote: params.publicNote,
        },
        frozenIdentityId,
        groupInfo: groupInfo as unknown as null,
      });
    },
    [
      tokenContext.contractId,
      tokenContext.tokenPosition,
      frozenIdentityId,
      buildGroupInfo,
    ],
  );

  // Reset
  const handleDoAnother = useCallback(() => {
    setFrozenIdentityId("");
  }, []);

  return (
    <TokenOperationForm
      actionName="Destroy Frozen Funds"
      tokenContext={tokenContext}
      groupAction={groupAction}
      isValid={isValid}
      validationMessage={validationMessage}
      confirmation={confirmation}
      onSubmit={handleSubmit}
      resultType="Token"
      successMessage="Frozen funds destroyed successfully."
      doAnotherLabel="Destroy More"
      onDoAnother={handleDoAnother}
    >
      {isGroupSigning ? (
        <div className="rounded-md border border-border bg-muted/30 p-3 text-sm">
          <span className="text-muted-foreground">Frozen identity: </span>
          <span className="font-mono font-medium">{frozenIdentityId || "N/A"}</span>
        </div>
      ) : (
        <div className="space-y-2">
          <Label htmlFor="frozen-identity-id">Frozen Identity ID</Label>
          <p className="text-xs text-muted-foreground">
            Enter the identity ID of the frozen identity whose funds will be permanently destroyed.
          </p>
          <Input
            id="frozen-identity-id"
            placeholder="Enter frozen identity ID (Base58 or Hex)"
            value={frozenIdentityId}
            onChange={(e) => setFrozenIdentityId(e.target.value)}
            className="font-mono"
          />
        </div>
      )}
    </TokenOperationForm>
  );
}
