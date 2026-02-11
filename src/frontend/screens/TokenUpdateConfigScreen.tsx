import { useCallback, useMemo, useState } from "react";
import { useRouterState } from "@tanstack/react-router";
import { commands } from "@/bindings";
import { TokenOperationForm } from "@/components/token/TokenOperationForm";
import type {
  ConfirmationConfig,
  GroupActionContext,
} from "@/components/token/TokenOperationForm";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Checkbox } from "@/components/ui/checkbox";
import { Button } from "@/components/ui/button";

// ---------------------------------------------------------------------------
// Change Item Types
// ---------------------------------------------------------------------------

/**
 * All possible change item types. These correspond to the Rust enum
 * `TokenConfigurationChangeItem` variants.
 */
const CHANGE_ITEM_OPTIONS = [
  // Conventions
  { value: "Conventions", label: "Conventions", group: "Conventions" },
  {
    value: "ConventionsControlGroup",
    label: "Conventions Control Group",
    group: "Conventions",
  },
  {
    value: "ConventionsAdminGroup",
    label: "Conventions Admin Group",
    group: "Conventions",
  },

  // Max Supply
  { value: "MaxSupply", label: "Max Supply", group: "Max Supply" },
  {
    value: "MaxSupplyControlGroup",
    label: "Max Supply Control Group",
    group: "Max Supply",
  },
  {
    value: "MaxSupplyAdminGroup",
    label: "Max Supply Admin Group",
    group: "Max Supply",
  },

  // Distribution
  {
    value: "PerpetualDistribution",
    label: "Perpetual Distribution",
    group: "Distribution",
  },
  {
    value: "PerpetualDistributionControlGroup",
    label: "Perpetual Distribution Control Group",
    group: "Distribution",
  },
  {
    value: "PerpetualDistributionAdminGroup",
    label: "Perpetual Distribution Admin Group",
    group: "Distribution",
  },
  {
    value: "NewTokensDestinationIdentity",
    label: "New Tokens Destination",
    group: "Distribution",
  },
  {
    value: "NewTokensDestinationIdentityControlGroup",
    label: "New Tokens Destination Control Group",
    group: "Distribution",
  },
  {
    value: "NewTokensDestinationIdentityAdminGroup",
    label: "New Tokens Destination Admin Group",
    group: "Distribution",
  },
  {
    value: "MintingAllowChoosingDestination",
    label: "Minting Allow Choosing Destination",
    group: "Distribution",
  },
  {
    value: "MintingAllowChoosingDestinationControlGroup",
    label: "Minting Allow Choosing Destination Control Group",
    group: "Distribution",
  },
  {
    value: "MintingAllowChoosingDestinationAdminGroup",
    label: "Minting Allow Choosing Destination Admin Group",
    group: "Distribution",
  },

  // Token Actions
  { value: "ManualMinting", label: "Manual Minting", group: "Token Actions" },
  {
    value: "ManualMintingAdminGroup",
    label: "Manual Minting Admin Group",
    group: "Token Actions",
  },
  { value: "ManualBurning", label: "Manual Burning", group: "Token Actions" },
  {
    value: "ManualBurningAdminGroup",
    label: "Manual Burning Admin Group",
    group: "Token Actions",
  },
  { value: "Freeze", label: "Freeze", group: "Token Actions" },
  {
    value: "FreezeAdminGroup",
    label: "Freeze Admin Group",
    group: "Token Actions",
  },
  { value: "Unfreeze", label: "Unfreeze", group: "Token Actions" },
  {
    value: "UnfreezeAdminGroup",
    label: "Unfreeze Admin Group",
    group: "Token Actions",
  },
  {
    value: "DestroyFrozenFunds",
    label: "Destroy Frozen Funds",
    group: "Token Actions",
  },
  {
    value: "DestroyFrozenFundsAdminGroup",
    label: "Destroy Frozen Funds Admin Group",
    group: "Token Actions",
  },
  {
    value: "EmergencyAction",
    label: "Emergency Action",
    group: "Token Actions",
  },
  {
    value: "EmergencyActionAdminGroup",
    label: "Emergency Action Admin Group",
    group: "Token Actions",
  },

  // Marketplace
  {
    value: "MarketplaceTradeModeControlGroup",
    label: "Marketplace Trade Mode Management",
    group: "Marketplace",
  },
  {
    value: "MarketplaceTradeModeAdminGroup",
    label: "Marketplace Trade Mode Admin",
    group: "Marketplace",
  },

  // Main Control Group
  {
    value: "MainControlGroup",
    label: "Main Control Group",
    group: "Control Groups",
  },
] as const;

type ChangeItemType = (typeof CHANGE_ITEM_OPTIONS)[number]["value"];

/**
 * AuthorizedActionTakers types for the group/control editors.
 */
type ActionTakerType =
  | "NoOne"
  | "ContractOwner"
  | "MainGroup"
  | "Identity"
  | "Group";

/**
 * Items that use the AuthorizedActionTakers editor.
 */
const AAT_ITEMS = new Set<string>([
  "ConventionsControlGroup",
  "ConventionsAdminGroup",
  "MaxSupplyControlGroup",
  "MaxSupplyAdminGroup",
  "PerpetualDistributionControlGroup",
  "PerpetualDistributionAdminGroup",
  "NewTokensDestinationIdentityControlGroup",
  "NewTokensDestinationIdentityAdminGroup",
  "MintingAllowChoosingDestinationControlGroup",
  "MintingAllowChoosingDestinationAdminGroup",
  "ManualMinting",
  "ManualMintingAdminGroup",
  "ManualBurning",
  "ManualBurningAdminGroup",
  "Freeze",
  "FreezeAdminGroup",
  "Unfreeze",
  "UnfreezeAdminGroup",
  "DestroyFrozenFunds",
  "DestroyFrozenFundsAdminGroup",
  "EmergencyAction",
  "EmergencyActionAdminGroup",
  "MarketplaceTradeModeControlGroup",
  "MarketplaceTradeModeAdminGroup",
]);

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

/**
 * Token Update Config screen — allows updating various token configuration parameters.
 *
 * Reads token context from route search params:
 *   tokenId, contractId, tokenPosition, identityId, name, balance, decimals
 *
 * Optional group action params (from Group Actions screen):
 *   groupActionId, details
 */
export function TokenUpdateConfigScreen() {
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

  // Parse group action details for pre-populated change item
  const groupDetails = useMemo(() => {
    if (!search.details) return null;
    try {
      return JSON.parse(search.details) as Record<string, unknown>;
    } catch {
      return null;
    }
  }, [search.details]);

  const groupAction: GroupActionContext | undefined = isGroupSigning
    ? { groupActionId, hasGroup: true, isUnilateral: false }
    : undefined;

  // Form state
  const [changeItemType, setChangeItemType] = useState<ChangeItemType | "">(
    isGroupSigning && groupDetails?.changeItemType
      ? (groupDetails.changeItemType as ChangeItemType)
      : "",
  );

  // Per-variant editing state
  const [conventionsJson, setConventionsJson] = useState("");
  const [conventionsError, setConventionsError] = useState("");
  const [maxSupply, setMaxSupply] = useState("");
  const [mintingAllowChoose, setMintingAllowChoose] = useState(false);
  const [destinationIdentity, setDestinationIdentity] = useState("");
  const [mainControlGroup, setMainControlGroup] = useState("");

  // AuthorizedActionTakers state
  const [actionTakerType, setActionTakerType] =
    useState<ActionTakerType>("ContractOwner");
  const [actionTakerIdentity, setActionTakerIdentity] = useState("");
  const [actionTakerGroup, setActionTakerGroup] = useState("0");

  // Validation
  const isValid = useMemo(() => {
    if (!changeItemType) return false;

    if (changeItemType === "Conventions") {
      return conventionsJson.trim().length > 0 && conventionsError === "";
    }
    if (changeItemType === "PerpetualDistribution") {
      return false; // Cannot be modified
    }

    if (AAT_ITEMS.has(changeItemType)) {
      if (actionTakerType === "Identity") {
        return actionTakerIdentity.trim().length > 0;
      }
      return true;
    }

    return true;
  }, [
    changeItemType,
    conventionsJson,
    conventionsError,
    actionTakerType,
    actionTakerIdentity,
  ]);

  const validationMessage = !changeItemType
    ? "Select a configuration item to update."
    : changeItemType === "PerpetualDistribution"
      ? "The perpetual distribution cannot be modified."
      : undefined;

  // Build changeItemJson for the backend
  const buildChangeItemJson = useCallback(() => {
    if (!changeItemType) return null;

    // Conventions — value is the parsed JSON object
    if (changeItemType === "Conventions") {
      try {
        const parsed = JSON.parse(conventionsJson);
        return { Conventions: parsed };
      } catch {
        return null;
      }
    }

    // MaxSupply — optional u64
    if (changeItemType === "MaxSupply") {
      const val = maxSupply.trim();
      return { MaxSupply: val ? parseInt(val, 10) : null };
    }

    // MintingAllowChoosingDestination — boolean
    if (changeItemType === "MintingAllowChoosingDestination") {
      return { MintingAllowChoosingDestination: mintingAllowChoose };
    }

    // NewTokensDestinationIdentity — optional identifier
    if (changeItemType === "NewTokensDestinationIdentity") {
      const val = destinationIdentity.trim();
      return { NewTokensDestinationIdentity: val || null };
    }

    // PerpetualDistribution — cannot be modified
    if (changeItemType === "PerpetualDistribution") {
      return null;
    }

    // MainControlGroup — optional u16
    if (changeItemType === "MainControlGroup") {
      const val = mainControlGroup.trim();
      return { MainControlGroup: val ? parseInt(val, 10) : null };
    }

    // AuthorizedActionTakers variants
    if (AAT_ITEMS.has(changeItemType)) {
      let aat: unknown;
      switch (actionTakerType) {
        case "NoOne":
          aat = "NoOne";
          break;
        case "ContractOwner":
          aat = "ContractOwner";
          break;
        case "MainGroup":
          aat = "MainGroup";
          break;
        case "Identity":
          aat = { Identity: actionTakerIdentity.trim() };
          break;
        case "Group":
          aat = { Group: parseInt(actionTakerGroup, 10) || 0 };
          break;
        default:
          aat = "ContractOwner";
      }
      return { [changeItemType]: aat };
    }

    // TokenConfigurationNoChange
    return { TokenConfigurationNoChange: null };
  }, [
    changeItemType,
    conventionsJson,
    maxSupply,
    mintingAllowChoose,
    destinationIdentity,
    mainControlGroup,
    actionTakerType,
    actionTakerIdentity,
    actionTakerGroup,
  ]);

  // Confirmation
  const confirmation: ConfirmationConfig | undefined = isValid
    ? {
        title: "Confirm Update",
        description: `Update token configuration: ${CHANGE_ITEM_OPTIONS.find((o) => o.value === changeItemType)?.label ?? changeItemType}?`,
        confirmLabel: isGroupSigning ? "Sign Update" : "Update Config",
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
      const changeItemJson = buildChangeItemJson();
      if (!changeItemJson) {
        return {
          status: "error" as const,
          error: "Invalid change item configuration",
        };
      }

      const groupInfo = buildGroupInfo(params.identityId, params.keyId);

      return commands.tokenUpdateConfig({
        identityId: params.identityId,
        contractId: tokenContext.contractId,
        tokenId: tokenContext.tokenId,
        tokenAlias: tokenContext.name ?? "",
        tokenPosition: tokenContext.tokenPosition,
        changeItemJson: changeItemJson as unknown as null,
        keyId: params.keyId,
        publicNote: params.publicNote,
        groupInfo: groupInfo as unknown as null,
      });
    },
    [
      tokenContext.contractId,
      tokenContext.tokenId,
      tokenContext.name,
      tokenContext.tokenPosition,
      buildChangeItemJson,
      buildGroupInfo,
    ],
  );

  // Reset
  const handleDoAnother = useCallback(() => {
    setChangeItemType("");
    setConventionsJson("");
    setConventionsError("");
    setMaxSupply("");
    setMintingAllowChoose(false);
    setDestinationIdentity("");
    setMainControlGroup("");
    setActionTakerType("ContractOwner");
    setActionTakerIdentity("");
    setActionTakerGroup("0");
  }, []);

  // Group the options for the Select
  const groups = useMemo(() => {
    const map = new Map<string, (typeof CHANGE_ITEM_OPTIONS)[number][]>();
    for (const opt of CHANGE_ITEM_OPTIONS) {
      const arr = map.get(opt.group) ?? [];
      arr.push(opt);
      map.set(opt.group, arr);
    }
    return map;
  }, []);

  // Handle conventions JSON change
  const handleConventionsChange = useCallback((value: string) => {
    setConventionsJson(value);
    if (!value.trim()) {
      setConventionsError("");
      return;
    }
    try {
      JSON.parse(value);
      setConventionsError("");
    } catch (e) {
      setConventionsError(`Invalid JSON: ${e instanceof Error ? e.message : String(e)}`);
    }
  }, []);

  return (
    <TokenOperationForm
      actionName="Update Config"
      tokenContext={tokenContext}
      groupAction={groupAction}
      isValid={isValid}
      validationMessage={validationMessage}
      confirmation={confirmation}
      onSubmit={handleSubmit}
      resultType="Token"
      successMessage="Token configuration updated successfully."
      doAnotherLabel="Update Another Config"
      onDoAnother={handleDoAnother}
    >
      <div className="space-y-4">
        {/* Change Item Selector */}
        <div className="space-y-2">
          <Label htmlFor="change-item-type">Configuration Item</Label>
          {isGroupSigning ? (
            <div className="rounded-md border border-border bg-muted/30 p-3 text-sm">
              <span className="text-muted-foreground">Config item: </span>
              <span className="font-medium">
                {CHANGE_ITEM_OPTIONS.find((o) => o.value === changeItemType)
                  ?.label ?? (changeItemType || "N/A")}
              </span>
            </div>
          ) : (
            <Select
              value={changeItemType}
              onValueChange={(v) => setChangeItemType(v as ChangeItemType)}
            >
              <SelectTrigger id="change-item-type" data-testid="change-item-selector">
                <SelectValue placeholder="Select item to update..." />
              </SelectTrigger>
              <SelectContent>
                {Array.from(groups.entries()).map(([group, items], gi) => (
                  <SelectGroup key={group}>
                    {gi > 0 && <SelectSeparator />}
                    <SelectLabel>{group}</SelectLabel>
                    {items.map((opt) => (
                      <SelectItem key={opt.value} value={opt.value}>
                        {opt.label}
                      </SelectItem>
                    ))}
                  </SelectGroup>
                ))}
              </SelectContent>
            </Select>
          )}
        </div>

        {/* Per-variant editing UI */}
        {changeItemType && (
          <div className="space-y-3 rounded-md border border-border bg-muted/10 p-4">
            {renderVariantEditor({
              changeItemType,
              conventionsJson,
              conventionsError,
              onConventionsChange: handleConventionsChange,
              maxSupply,
              onMaxSupplyChange: setMaxSupply,
              mintingAllowChoose,
              onMintingAllowChooseChange: setMintingAllowChoose,
              destinationIdentity,
              onDestinationIdentityChange: setDestinationIdentity,
              mainControlGroup,
              onMainControlGroupChange: setMainControlGroup,
              actionTakerType,
              onActionTakerTypeChange: setActionTakerType,
              actionTakerIdentity,
              onActionTakerIdentityChange: setActionTakerIdentity,
              actionTakerGroup,
              onActionTakerGroupChange: setActionTakerGroup,
              isGroupSigning,
            })}
          </div>
        )}
      </div>
    </TokenOperationForm>
  );
}

// ---------------------------------------------------------------------------
// Per-variant Editor
// ---------------------------------------------------------------------------

interface VariantEditorProps {
  changeItemType: ChangeItemType;
  conventionsJson: string;
  conventionsError: string;
  onConventionsChange: (v: string) => void;
  maxSupply: string;
  onMaxSupplyChange: (v: string) => void;
  mintingAllowChoose: boolean;
  onMintingAllowChooseChange: (v: boolean) => void;
  destinationIdentity: string;
  onDestinationIdentityChange: (v: string) => void;
  mainControlGroup: string;
  onMainControlGroupChange: (v: string) => void;
  actionTakerType: ActionTakerType;
  onActionTakerTypeChange: (v: ActionTakerType) => void;
  actionTakerIdentity: string;
  onActionTakerIdentityChange: (v: string) => void;
  actionTakerGroup: string;
  onActionTakerGroupChange: (v: string) => void;
  isGroupSigning: boolean;
}

function renderVariantEditor(props: VariantEditorProps) {
  const { changeItemType } = props;

  // Conventions: JSON text editor
  if (changeItemType === "Conventions") {
    return (
      <div className="space-y-2">
        <Label>Token Conventions (JSON)</Label>
        <p className="text-xs text-muted-foreground">
          Update the JSON to change the token conventions (name, description,
          decimals, etc.).
        </p>
        <Textarea
          data-testid="conventions-json-editor"
          placeholder='{"localizedName": "...", "localizedDescription": "..."}'
          value={props.conventionsJson}
          onChange={(e) => props.onConventionsChange(e.target.value)}
          rows={8}
          className="font-mono text-sm"
        />
        {props.conventionsError && (
          <p className="text-sm text-destructive" data-testid="conventions-error">
            {props.conventionsError}
          </p>
        )}
        <Button
          variant="outline"
          size="sm"
          type="button"
          onClick={() => {
            props.onConventionsChange("");
          }}
        >
          Clear
        </Button>
      </div>
    );
  }

  // MaxSupply: numeric input
  if (changeItemType === "MaxSupply") {
    return (
      <div className="space-y-2">
        <Label htmlFor="max-supply-input">Max Supply</Label>
        <p className="text-xs text-muted-foreground">
          Set the maximum supply of tokens. Leave empty for no limit.
        </p>
        <Input
          id="max-supply-input"
          data-testid="max-supply-input"
          type="number"
          placeholder="Enter max supply (leave empty for no limit)"
          value={props.maxSupply}
          onChange={(e) => props.onMaxSupplyChange(e.target.value)}
          min={0}
        />
      </div>
    );
  }

  // MintingAllowChoosingDestination: checkbox
  if (changeItemType === "MintingAllowChoosingDestination") {
    return (
      <div className="flex items-center space-x-2">
        <Checkbox
          id="minting-allow-choose"
          data-testid="minting-allow-choose"
          checked={props.mintingAllowChoose}
          onCheckedChange={(checked) =>
            props.onMintingAllowChooseChange(checked === true)
          }
        />
        <Label htmlFor="minting-allow-choose">
          Allow user to choose destination when minting
        </Label>
      </div>
    );
  }

  // NewTokensDestinationIdentity: identity ID input
  if (changeItemType === "NewTokensDestinationIdentity") {
    return (
      <div className="space-y-2">
        <Label htmlFor="destination-identity">New Tokens Destination Identity</Label>
        <p className="text-xs text-muted-foreground">
          Set the identity that receives newly minted tokens. Leave empty to
          clear.
        </p>
        <Input
          id="destination-identity"
          data-testid="destination-identity-input"
          placeholder="Enter identity ID (Base58)"
          value={props.destinationIdentity}
          onChange={(e) => props.onDestinationIdentityChange(e.target.value)}
          className="font-mono"
        />
      </div>
    );
  }

  // PerpetualDistribution: read-only
  if (changeItemType === "PerpetualDistribution") {
    return (
      <div className="rounded-md border border-destructive/50 bg-destructive/5 p-3">
        <p className="text-sm text-destructive" data-testid="perpetual-distribution-warning">
          The perpetual distribution cannot be modified.
        </p>
      </div>
    );
  }

  // MainControlGroup: numeric input for group position
  if (changeItemType === "MainControlGroup") {
    return (
      <div className="space-y-2">
        <Label htmlFor="main-control-group">Main Control Group Position</Label>
        <p className="text-xs text-muted-foreground">
          Set the group position for the main control group.
        </p>
        <Input
          id="main-control-group"
          data-testid="main-control-group-input"
          type="number"
          placeholder="Enter group position"
          value={props.mainControlGroup}
          onChange={(e) => props.onMainControlGroupChange(e.target.value)}
          min={0}
        />
      </div>
    );
  }

  // AuthorizedActionTakers editor for all control/admin group items
  if (AAT_ITEMS.has(changeItemType)) {
    return (
      <AuthorizedActionTakersEditor
        actionTakerType={props.actionTakerType}
        onActionTakerTypeChange={props.onActionTakerTypeChange}
        actionTakerIdentity={props.actionTakerIdentity}
        onActionTakerIdentityChange={props.onActionTakerIdentityChange}
        actionTakerGroup={props.actionTakerGroup}
        onActionTakerGroupChange={props.onActionTakerGroupChange}
        isGroupSigning={props.isGroupSigning}
      />
    );
  }

  return (
    <p className="text-sm text-muted-foreground">
      No parameters to edit for this entry.
    </p>
  );
}

// ---------------------------------------------------------------------------
// AuthorizedActionTakers Editor
// ---------------------------------------------------------------------------

interface AATEditorProps {
  actionTakerType: ActionTakerType;
  onActionTakerTypeChange: (v: ActionTakerType) => void;
  actionTakerIdentity: string;
  onActionTakerIdentityChange: (v: string) => void;
  actionTakerGroup: string;
  onActionTakerGroupChange: (v: string) => void;
  isGroupSigning: boolean;
}

function AuthorizedActionTakersEditor(props: AATEditorProps) {
  return (
    <div className="space-y-3">
      <Label>Authorized Action Taker</Label>
      <Select
        value={props.actionTakerType}
        onValueChange={(v) =>
          props.onActionTakerTypeChange(v as ActionTakerType)
        }
        disabled={props.isGroupSigning}
      >
        <SelectTrigger data-testid="action-taker-selector">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="NoOne">No One</SelectItem>
          <SelectItem value="ContractOwner">Contract Owner</SelectItem>
          <SelectItem value="MainGroup">Main Group</SelectItem>
          <SelectItem value="Identity">Identity</SelectItem>
          <SelectItem value="Group">Group</SelectItem>
        </SelectContent>
      </Select>

      {/* Identity input */}
      {props.actionTakerType === "Identity" && (
        <div className="space-y-1">
          <Label htmlFor="aat-identity-input">Identity ID</Label>
          <Input
            id="aat-identity-input"
            data-testid="aat-identity-input"
            placeholder="Enter identity ID (Base58)"
            value={props.actionTakerIdentity}
            onChange={(e) => props.onActionTakerIdentityChange(e.target.value)}
            className="font-mono"
            disabled={props.isGroupSigning}
          />
        </div>
      )}

      {/* Group position input */}
      {props.actionTakerType === "Group" && (
        <div className="space-y-1">
          <Label htmlFor="aat-group-input">Group Position</Label>
          <Input
            id="aat-group-input"
            data-testid="aat-group-input"
            type="number"
            placeholder="Enter group position"
            value={props.actionTakerGroup}
            onChange={(e) => props.onActionTakerGroupChange(e.target.value)}
            min={0}
            disabled={props.isGroupSigning}
          />
        </div>
      )}
    </div>
  );
}
