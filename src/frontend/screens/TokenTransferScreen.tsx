import { useCallback, useState } from "react";
import { useRouterState } from "@tanstack/react-router";
import { commands } from "@/bindings";
import { TokenOperationForm } from "@/components/token/TokenOperationForm";
import type { ConfirmationConfig } from "@/components/token/TokenOperationForm";

/**
 * Token Transfer screen — allows transferring tokens to another identity.
 *
 * Reads token context from route search params:
 *   tokenId, contractId, tokenPosition, identityId, name, balance, decimals
 */
export function TokenTransferScreen() {
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

  // Form state
  const [amount, setAmount] = useState("");
  const [recipientId, setRecipientId] = useState("");

  // Validation
  const amountNum = Number(amount);
  const balanceNum = Number(tokenContext.balance);
  const isAmountValid = amount !== "" && amountNum > 0;
  const isAmountWithinBalance = isAmountValid && amountNum <= balanceNum;
  const isRecipientValid = recipientId.trim().length > 0;
  const isValid = isAmountWithinBalance && isRecipientValid;

  // Validation message
  let validationMessage: string | undefined;
  if (amount !== "" && !isAmountValid) {
    validationMessage = "Amount must be greater than 0.";
  } else if (amount !== "" && isAmountValid && !isAmountWithinBalance) {
    validationMessage = "Amount exceeds available balance.";
  } else if (recipientId.trim().length > 0 && !isRecipientValid) {
    validationMessage = "Please enter a valid recipient identity ID.";
  }

  // Confirmation dialog
  const confirmation: ConfirmationConfig | undefined =
    isValid
      ? {
          title: "Confirm Transfer",
          description: `Transfer ${amount} tokens to ${recipientId.slice(0, 12)}...?`,
          confirmLabel: "Transfer",
        }
      : undefined;

  // Submit handler
  const handleSubmit = useCallback(
    async (params: { identityId: string; keyId: number; publicNote: string | null }) => {
      return commands.tokenTransfer({
        operation: {
          identityId: params.identityId,
          contractId: tokenContext.contractId,
          tokenPosition: tokenContext.tokenPosition,
          keyId: params.keyId,
          publicNote: params.publicNote,
        },
        recipientId,
        amount,
      });
    },
    [tokenContext.contractId, tokenContext.tokenPosition, recipientId, amount],
  );

  // Reset form for "do another"
  const handleDoAnother = useCallback(() => {
    setAmount("");
    setRecipientId("");
  }, []);

  return (
    <TokenOperationForm
      actionName="Transfer"
      tokenContext={tokenContext}
      showAmountInput
      amount={amount}
      onAmountChange={setAmount}
      maxAmount={tokenContext.balance}
      amountLabel="Amount to Transfer"
      showRecipientInput
      recipientId={recipientId}
      onRecipientChange={setRecipientId}
      recipientLabel="Recipient Identity ID"
      isValid={isValid}
      validationMessage={validationMessage}
      confirmation={confirmation}
      onSubmit={handleSubmit}
      resultEventType="tokenCompleted"
      successMessage="Tokens transferred successfully."
      doAnotherLabel="Transfer More"
      onDoAnother={handleDoAnother}
    />
  );
}
