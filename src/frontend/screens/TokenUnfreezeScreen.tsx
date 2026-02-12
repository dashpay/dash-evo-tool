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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Loader2 } from "lucide-react";
import { useFrozenIdentities } from "@/hooks/useFrozenIdentities";

/**
 * Token Unfreeze screen — allows unfreezing a previously frozen identity's tokens.
 *
 * On mount, queries Platform for frozen identities and shows them in a dropdown.
 * Falls back to manual text input via "Other" option.
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

  // Fetch frozen identities from Platform
  const { frozenIdentities, loading: loadingFrozen } = useFrozenIdentities(
    tokenContext.tokenId,
  );

  // Form state
  const [unfreezeIdentityId, setUnfreezeIdentityId] = useState(groupUnfreezeIdentity);
  const [useManualInput, setUseManualInput] = useState(false);

  // Handle select change
  const handleSelectChange = useCallback((value: string) => {
    if (value === "__other__") {
      setUseManualInput(true);
      setUnfreezeIdentityId("");
    } else {
      setUseManualInput(false);
      setUnfreezeIdentityId(value);
    }
  }, []);

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
      return commands.tokenUnfreeze({
        operation: {
          identityId: params.identityId,
          contractId: tokenContext.contractId,
          tokenPosition: tokenContext.tokenPosition,
          keyId: params.keyId,
          publicNote: params.publicNote,
        },
        unfreezeIdentityId,
        groupInfo: groupInfo,
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
    setUseManualInput(false);
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
      resultEventType="tokenCompleted"
      successMessage="Identity tokens unfrozen successfully."
      doAnotherLabel="Unfreeze Another"
      onDoAnother={handleDoAnother}
    >
      {isGroupSigning ? (
        <div className="rounded-md border border-border bg-muted/30 p-3 text-sm">
          <span className="text-muted-foreground">Identity to unfreeze: </span>
          <span className="font-mono font-medium">{unfreezeIdentityId || "N/A"}</span>
        </div>
      ) : loadingFrozen ? (
        <div className="space-y-2">
          <Label>Identity ID to Unfreeze</Label>
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            Loading frozen identities from Platform...
          </div>
        </div>
      ) : frozenIdentities.length > 0 && !useManualInput ? (
        <div className="space-y-2">
          <Label htmlFor="unfreeze-identity-select">Identity ID to Unfreeze</Label>
          <p className="text-xs text-muted-foreground">
            Select a frozen identity to unfreeze, or choose &quot;Other&quot; to enter an ID manually.
          </p>
          <Select
            value={unfreezeIdentityId}
            onValueChange={handleSelectChange}
          >
            <SelectTrigger id="unfreeze-identity-select" data-testid="unfreeze-identity-select">
              <SelectValue placeholder="Select frozen identity..." />
            </SelectTrigger>
            <SelectContent>
              {frozenIdentities.map((fi) => (
                <SelectItem key={fi.id} value={fi.id}>
                  <span className="font-mono text-xs">{fi.label}</span>
                </SelectItem>
              ))}
              <SelectItem value="__other__">Other (enter manually)</SelectItem>
            </SelectContent>
          </Select>
        </div>
      ) : (
        <div className="space-y-2">
          <Label htmlFor="unfreeze-identity-id">Identity ID to Unfreeze</Label>
          <p className="text-xs text-muted-foreground">
            {frozenIdentities.length > 0
              ? "Enter the identity ID manually."
              : "No frozen identities found among loaded identities. Enter the identity ID of the frozen identity you want to unfreeze."}
          </p>
          <Input
            id="unfreeze-identity-id"
            placeholder="Enter frozen identity ID (Base58 or Hex)"
            value={unfreezeIdentityId}
            onChange={(e) => setUnfreezeIdentityId(e.target.value)}
            className="font-mono"
          />
          {frozenIdentities.length > 0 && (
            <button
              type="button"
              className="text-xs text-primary hover:underline"
              onClick={() => {
                setUseManualInput(false);
                setUnfreezeIdentityId("");
              }}
            >
              Back to dropdown
            </button>
          )}
        </div>
      )}
    </TokenOperationForm>
  );
}
