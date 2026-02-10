import { useCallback, useMemo, useState } from "react";
import { useRouterState } from "@tanstack/react-router";
import { commands } from "@/bindings";
import { TokenOperationForm } from "@/components/token/TokenOperationForm";
import type {
  ConfirmationConfig,
  GroupActionContext,
} from "@/components/token/TokenOperationForm";

/**
 * Token Mint screen — allows minting new tokens.
 *
 * Reads token context from route search params:
 *   tokenId, contractId, tokenPosition, identityId, name, balance, decimals
 *
 * Optional group action params (from Group Actions screen):
 *   groupActionId, groupPosition, details
 */
export function TokenMintScreen() {
  const search = useRouterState({
    select: (s) => s.location.search as Record<string, string>,
  });

  // Token context from search params
  const tokenContext = {
    tokenId: search.tokenId ?? "",
    contractId: search.contractId ?? "",
    tokenPosition: Number(search.tokenPosition ?? "0"),
    name: search.name ?? null,
    balance: search.balance ?? "0",
    decimals: Number(search.decimals ?? "8"),
    identityId: search.identityId ?? "",
  };

  // Group action context (if coming from Group Actions screen)
  const groupActionId = search.groupActionId;
  const isGroupSigning = !!groupActionId;

  // Parse group action details if present
  const groupDetails = useMemo(() => {
    if (search.details) {
      try {
        return JSON.parse(search.details) as Record<string, unknown>;
      } catch {
        return null;
      }
    }
    return null;
  }, [search.details]);

  // Pre-populated amount from group action details
  const groupAmount =
    groupDetails && typeof groupDetails.amount === "string"
      ? groupDetails.amount
      : "";
  const groupRecipient =
    groupDetails && typeof groupDetails.recipientId === "string"
      ? groupDetails.recipientId
      : "";

  // Group action context for TokenOperationForm
  const groupAction: GroupActionContext | undefined = isGroupSigning
    ? {
        groupActionId,
        hasGroup: true,
        isUnilateral: false,
      }
    : undefined;

  // Form state
  const [amount, setAmount] = useState(groupAmount);
  const [recipientId, setRecipientId] = useState(groupRecipient);

  // Validation — amount must be > 0
  const amountNum = Number(amount);
  const isAmountValid = amount !== "" && amountNum > 0;
  const isValid = isAmountValid;

  // Validation message
  let validationMessage: string | undefined;
  if (amount !== "" && !isAmountValid) {
    validationMessage = "Amount must be greater than 0.";
  }

  // Confirmation dialog
  const confirmation: ConfirmationConfig | undefined = isValid
    ? {
        title: "Confirm Mint",
        description: `Mint ${amount} tokens${recipientId ? ` to ${recipientId.slice(0, 12)}...` : ""}?`,
        confirmLabel: isGroupSigning ? "Sign Mint" : "Mint",
      }
    : undefined;

  // Build group info JSON for the IPC call
  const buildGroupInfo = useCallback(
    (identityId: string, keyId: number) => {
      if (groupActionId) {
        // Signing an existing group action
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

  // Submit handler
  const handleSubmit = useCallback(
    async (params: {
      identityId: string;
      keyId: number;
      publicNote: string | null;
    }) => {
      const groupInfo = buildGroupInfo(params.identityId, params.keyId);
      return commands.tokenMint({
        operation: {
          identityId: params.identityId,
          contractId: tokenContext.contractId,
          tokenPosition: tokenContext.tokenPosition,
          keyId: params.keyId,
        },
        amount,
        recipientId: recipientId || null,
        groupInfo: groupInfo as unknown as null,
      });
    },
    [
      tokenContext.contractId,
      tokenContext.tokenPosition,
      amount,
      recipientId,
      buildGroupInfo,
    ],
  );

  // Reset form for "do another"
  const handleDoAnother = useCallback(() => {
    setAmount("");
    setRecipientId("");
  }, []);

  return (
    <TokenOperationForm
      actionName="Mint"
      tokenContext={tokenContext}
      showAmountInput={!isGroupSigning}
      amount={amount}
      onAmountChange={setAmount}
      amountLabel="Amount to Mint"
      showRecipientInput={!isGroupSigning}
      recipientId={recipientId}
      onRecipientChange={setRecipientId}
      recipientLabel="Recipient Identity ID"
      recipientOptional
      recipientPlaceholder="Leave empty to mint to yourself"
      groupAction={groupAction}
      isValid={isValid}
      validationMessage={validationMessage}
      confirmation={confirmation}
      onSubmit={handleSubmit}
      resultType="Token"
      successMessage="Tokens minted successfully."
      doAnotherLabel="Mint More"
      onDoAnother={handleDoAnother}
    >
      {isGroupSigning && groupAmount && (
        <div className="rounded-md border border-border bg-muted/30 p-3 text-sm">
          <span className="text-muted-foreground">Amount: </span>
          <span className="font-mono font-medium">{groupAmount}</span>
          {groupRecipient && (
            <>
              <br />
              <span className="text-muted-foreground">Recipient: </span>
              <span className="font-mono font-medium">
                {groupRecipient.slice(0, 12)}...{groupRecipient.slice(-8)}
              </span>
            </>
          )}
        </div>
      )}
    </TokenOperationForm>
  );
}
