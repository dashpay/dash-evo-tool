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
 * Destroy Frozen Funds screen — permanently destroys all frozen tokens held by a
 * target identity for this token contract. This action cannot be undone.
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

  // Fetch frozen identities from Platform
  const { frozenIdentities, loading: loadingFrozen } = useFrozenIdentities(
    tokenContext.tokenId,
  );

  // Form state
  const [frozenIdentityId, setFrozenIdentityId] = useState(groupFrozenIdentity);
  const [useManualInput, setUseManualInput] = useState(false);

  // Handle select change
  const handleSelectChange = useCallback((value: string) => {
    if (value === "__other__") {
      setUseManualInput(true);
      setFrozenIdentityId("");
    } else {
      setUseManualInput(false);
      setFrozenIdentityId(value);
    }
  }, []);

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
    setUseManualInput(false);
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
      ) : loadingFrozen ? (
        <div className="space-y-2">
          <Label>Frozen Identity ID</Label>
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            Loading frozen identities from Platform...
          </div>
        </div>
      ) : frozenIdentities.length > 0 && !useManualInput ? (
        <div className="space-y-2">
          <Label htmlFor="frozen-identity-select">Frozen Identity ID</Label>
          <p className="text-xs text-muted-foreground">
            Select a frozen identity whose funds will be permanently destroyed, or choose &quot;Other&quot; to enter an ID manually.
          </p>
          <Select
            value={frozenIdentityId}
            onValueChange={handleSelectChange}
          >
            <SelectTrigger id="frozen-identity-select" data-testid="frozen-identity-select">
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
          <Label htmlFor="frozen-identity-id">Frozen Identity ID</Label>
          <p className="text-xs text-muted-foreground">
            {frozenIdentities.length > 0
              ? "Enter the identity ID manually."
              : "No frozen identities found among loaded identities. Enter the identity ID of the frozen identity whose funds will be permanently destroyed."}
          </p>
          <Input
            id="frozen-identity-id"
            placeholder="Enter frozen identity ID (Base58 or Hex)"
            value={frozenIdentityId}
            onChange={(e) => setFrozenIdentityId(e.target.value)}
            className="font-mono"
          />
          {frozenIdentities.length > 0 && (
            <button
              type="button"
              className="text-xs text-primary hover:underline"
              onClick={() => {
                setUseManualInput(false);
                setFrozenIdentityId("");
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
