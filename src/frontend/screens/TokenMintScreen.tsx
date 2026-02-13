import { useCallback, useEffect, useMemo, useState } from "react";
import { useRouterState } from "@tanstack/react-router";
import { commands } from "@/bindings";
import type { MintingConfigDto } from "@/bindings";
import { TokenOperationForm } from "@/components/token/TokenOperationForm";
import type {
  ConfirmationConfig,
  GroupActionContext,
} from "@/components/token/TokenOperationForm";
import { estimateDocumentBatch } from "@/lib/feeEstimation";

/**
 * Token Mint screen — allows minting new tokens.
 *
 * Reads token context from route search params:
 *   tokenId, contractId, tokenPosition, identityId, name, balance, decimals
 *
 * Optional group action params (from Group Actions screen):
 *   groupActionId, groupPosition, details
 *
 * Fetches the token's minting destination config on mount to determine:
 * - Whether the recipient input should be shown (allowChoosingDestination)
 * - Whether there's a default destination identity (auto-populated)
 * - Whether the recipient is required or optional
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

  // Minting destination config from backend
  const [mintingConfig, setMintingConfig] = useState<MintingConfigDto | null>(
    null,
  );
  const [configLoading, setConfigLoading] = useState(true);

  // Fetch minting config on mount
  useEffect(() => {
    if (!tokenContext.contractId) {
      setConfigLoading(false);
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        const result = await commands.tokenGetMintingConfig({
          contractId: tokenContext.contractId,
          tokenPosition: tokenContext.tokenPosition,
        });
        if (!cancelled && result.status === "ok") {
          setMintingConfig(result.data);
        }
      } catch {
        // If the config can't be fetched, fall back to showing the recipient input
      } finally {
        if (!cancelled) setConfigLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [tokenContext.contractId, tokenContext.tokenPosition]);

  // Determine recipient input behavior from minting config
  const allowChoosingDestination =
    mintingConfig?.allowChoosingDestination ?? true;
  const defaultDestinationId =
    mintingConfig?.defaultDestinationIdentityId ?? null;

  // Show recipient input only if choosing is allowed (or still loading/fallback)
  const showRecipientInput =
    !isGroupSigning && (configLoading || allowChoosingDestination);

  // Recipient is always optional: when empty, tokens mint to the sender identity.
  // When there's a configured default destination, the placeholder indicates it.
  const recipientOptional = true;

  // Form state — auto-populate recipient with default destination if configured
  const [amount, setAmount] = useState(groupAmount);
  const [recipientId, setRecipientId] = useState(groupRecipient);
  const [recipientInitialized, setRecipientInitialized] = useState(false);

  // Auto-populate recipient when minting config loads and there's a default
  useEffect(() => {
    if (
      !recipientInitialized &&
      !isGroupSigning &&
      defaultDestinationId &&
      !recipientId
    ) {
      setRecipientId(defaultDestinationId);
      setRecipientInitialized(true);
    }
  }, [
    defaultDestinationId,
    recipientId,
    isGroupSigning,
    recipientInitialized,
  ]);

  // Validation — amount must be > 0
  const amountNum = Number(amount);
  const isAmountValid = amount !== "" && amountNum > 0;

  const isValid = isAmountValid;

  // Validation message
  let validationMessage: string | undefined;
  if (amount !== "" && !isAmountValid) {
    validationMessage = "Amount must be greater than 0.";
  }

  // Recipient placeholder text
  const recipientPlaceholder = defaultDestinationId
    ? `Default: ${defaultDestinationId.slice(0, 12)}...`
    : "Leave empty to mint to yourself";

  // Confirmation dialog
  const confirmation: ConfirmationConfig | undefined = isValid
    ? {
        title: "Confirm Mint",
        description: `Mint ${amount} tokens${recipientId ? ` to ${recipientId.slice(0, 12)}...` : ""}?`,
        confirmLabel: isGroupSigning ? "Sign Mint" : "Mint",
      }
    : undefined;

  // Build group info JSON for the IPC call
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

  // Submit handler
  const handleSubmit = useCallback(
    async (params: {
      identityId: string;
      keyId: number;
      publicNote: string | null;
    }) => {
      const groupInfo = buildGroupInfo();
      return commands.tokenMint({
        operation: {
          identityId: params.identityId,
          contractId: tokenContext.contractId,
          tokenPosition: tokenContext.tokenPosition,
          keyId: params.keyId,
          publicNote: params.publicNote,
        },
        amount,
        recipientId: recipientId || null,
        groupInfo: groupInfo,
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
    setRecipientId(defaultDestinationId ?? "");
  }, [defaultDestinationId]);

  return (
    <TokenOperationForm
      actionName="Mint"
      tokenContext={tokenContext}
      estimatedFee={estimateDocumentBatch(1)}
      showAmountInput={!isGroupSigning}
      amount={amount}
      onAmountChange={setAmount}
      amountLabel="Amount to Mint"
      showRecipientInput={showRecipientInput}
      recipientId={recipientId}
      onRecipientChange={
        allowChoosingDestination ? setRecipientId : undefined
      }
      recipientLabel="Recipient Identity ID"
      recipientOptional={recipientOptional}
      recipientPlaceholder={recipientPlaceholder}
      groupAction={groupAction}
      isValid={isValid}
      validationMessage={validationMessage}
      confirmation={confirmation}
      onSubmit={handleSubmit}
      resultEventType="tokenCompleted"
      successMessage="Tokens minted successfully."
      doAnotherLabel="Mint More"
      onDoAnother={handleDoAnother}
    >
      {/* Show minting config info when recipient is not allowed to be chosen */}
      {!configLoading &&
        !allowChoosingDestination &&
        defaultDestinationId &&
        !isGroupSigning && (
          <div className="rounded-md border border-border bg-muted/30 p-3 text-sm">
            <span className="text-muted-foreground">
              Minted tokens will be sent to:{" "}
            </span>
            <span className="font-mono font-medium">
              {defaultDestinationId.slice(0, 12)}...
              {defaultDestinationId.slice(-8)}
            </span>
          </div>
        )}
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
