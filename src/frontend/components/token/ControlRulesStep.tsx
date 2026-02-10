import { useCallback } from "react";
import { ChevronDown, ChevronRight, Info } from "lucide-react";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

// ─── Types ─────────────────────────────────────────────────────────────────

export type ActionTaker =
  | "noOne"
  | "contractOwner"
  | "identity"
  | "mainGroup"
  | "group";

export interface ChangeControlRulesState {
  authorizedActionTaker: ActionTaker;
  authorizedIdentityId: string;
  authorizedGroupPosition: string;
  adminActionTaker: ActionTaker;
  adminIdentityId: string;
  adminGroupPosition: string;
  changingAuthorizedToNoOneAllowed: boolean;
  changingAdminToNoOneAllowed: boolean;
  selfChangingAdminAllowed: boolean;
}

export interface MintExtrasState {
  destinationDefaultToContractOwner: boolean;
  destinationOtherIdentityEnabled: boolean;
  destinationOtherIdentityId: string;
  allowChoosingDestination: boolean;
  destinationIdentityRules: ChangeControlRulesState;
  destinationIdentityRulesExpanded: boolean;
  choosingDestinationRules: ChangeControlRulesState;
  choosingDestinationRulesExpanded: boolean;
}

export interface ControlRulesState {
  manualMinting: ChangeControlRulesState;
  manualMintingExpanded: boolean;
  mintExtras: MintExtrasState;

  manualBurning: ChangeControlRulesState;
  manualBurningExpanded: boolean;

  freeze: ChangeControlRulesState;
  freezeExpanded: boolean;

  unfreeze: ChangeControlRulesState;
  unfreezeExpanded: boolean;

  destroyFrozenFunds: ChangeControlRulesState;
  destroyFrozenFundsExpanded: boolean;

  emergencyAction: ChangeControlRulesState;
  emergencyActionExpanded: boolean;

  maxSupplyChange: ChangeControlRulesState;
  maxSupplyChangeExpanded: boolean;

  conventionsChange: ChangeControlRulesState;
  conventionsChangeExpanded: boolean;

  marketplace: ChangeControlRulesState;
  marketplaceExpanded: boolean;

  directPurchasePricing: ChangeControlRulesState;
  directPurchasePricingExpanded: boolean;

  mainControlGroupChange: ActionTaker;
  mainControlGroupChangeIdentityId: string;
  mainControlGroupChangeGroupPosition: string;
  mainControlGroupChangeExpanded: boolean;
}

// ─── Defaults ──────────────────────────────────────────────────────────────

export function createDefaultControlRules(): ChangeControlRulesState {
  return {
    authorizedActionTaker: "noOne",
    authorizedIdentityId: "",
    authorizedGroupPosition: "",
    adminActionTaker: "noOne",
    adminIdentityId: "",
    adminGroupPosition: "",
    changingAuthorizedToNoOneAllowed: true,
    changingAdminToNoOneAllowed: true,
    selfChangingAdminAllowed: true,
  };
}

export function createDefaultMintExtras(): MintExtrasState {
  return {
    destinationDefaultToContractOwner: true,
    destinationOtherIdentityEnabled: false,
    destinationOtherIdentityId: "",
    allowChoosingDestination: false,
    destinationIdentityRules: createDefaultControlRules(),
    destinationIdentityRulesExpanded: false,
    choosingDestinationRules: createDefaultControlRules(),
    choosingDestinationRulesExpanded: false,
  };
}

export function createDefaultControlRulesState(): ControlRulesState {
  return {
    manualMinting: createDefaultControlRules(),
    manualMintingExpanded: false,
    mintExtras: createDefaultMintExtras(),

    manualBurning: createDefaultControlRules(),
    manualBurningExpanded: false,

    freeze: createDefaultControlRules(),
    freezeExpanded: false,

    unfreeze: createDefaultControlRules(),
    unfreezeExpanded: false,

    destroyFrozenFunds: createDefaultControlRules(),
    destroyFrozenFundsExpanded: false,

    emergencyAction: createDefaultControlRules(),
    emergencyActionExpanded: false,

    maxSupplyChange: createDefaultControlRules(),
    maxSupplyChangeExpanded: false,

    conventionsChange: createDefaultControlRules(),
    conventionsChangeExpanded: false,

    marketplace: createDefaultControlRules(),
    marketplaceExpanded: false,

    directPurchasePricing: createDefaultControlRules(),
    directPurchasePricingExpanded: false,

    mainControlGroupChange: "noOne",
    mainControlGroupChangeIdentityId: "",
    mainControlGroupChangeGroupPosition: "",
    mainControlGroupChangeExpanded: false,
  };
}

// ─── Validation ────────────────────────────────────────────────────────────

export interface ControlRulesValidation {
  valid: boolean;
  errors: Record<string, string>;
}

export function validateIdentityId(id: string): boolean {
  // Base58 characters
  return /^[123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz]{32,44}$/.test(id);
}

function validateGroupPosition(pos: string): boolean {
  return /^\d+$/.test(pos);
}

function validateRules(
  rules: ChangeControlRulesState,
  prefix: string,
  errors: Record<string, string>,
  groups: GroupEntry[],
): void {
  if (rules.authorizedActionTaker === "identity" && rules.authorizedIdentityId.trim()) {
    if (!validateIdentityId(rules.authorizedIdentityId.trim())) {
      errors[`${prefix}.authorizedIdentityId`] = "Invalid Base58 identity ID";
    }
  }
  if (rules.adminActionTaker === "identity" && rules.adminIdentityId.trim()) {
    if (!validateIdentityId(rules.adminIdentityId.trim())) {
      errors[`${prefix}.adminIdentityId`] = "Invalid Base58 identity ID";
    }
  }
  if (rules.authorizedActionTaker === "group") {
    const pos = rules.authorizedGroupPosition.trim();
    if (pos && !validateGroupPosition(pos)) {
      errors[`${prefix}.authorizedGroupPosition`] = "Must be a number";
    } else if (pos && groups.length > 0 && parseInt(pos, 10) >= groups.length) {
      errors[`${prefix}.authorizedGroupPosition`] = `Group position must be < ${groups.length}`;
    }
  }
  if (rules.adminActionTaker === "group") {
    const pos = rules.adminGroupPosition.trim();
    if (pos && !validateGroupPosition(pos)) {
      errors[`${prefix}.adminGroupPosition`] = "Must be a number";
    } else if (pos && groups.length > 0 && parseInt(pos, 10) >= groups.length) {
      errors[`${prefix}.adminGroupPosition`] = `Group position must be < ${groups.length}`;
    }
  }
}

export function validateControlRules(
  state: ControlRulesState,
  groups: GroupEntry[] = [],
): ControlRulesValidation {
  const errors: Record<string, string> = {};

  validateRules(state.manualMinting, "manualMinting", errors, groups);
  validateRules(state.manualBurning, "manualBurning", errors, groups);
  validateRules(state.freeze, "freeze", errors, groups);
  validateRules(state.unfreeze, "unfreeze", errors, groups);
  validateRules(state.destroyFrozenFunds, "destroyFrozenFunds", errors, groups);
  validateRules(state.emergencyAction, "emergencyAction", errors, groups);
  validateRules(state.maxSupplyChange, "maxSupplyChange", errors, groups);
  validateRules(state.conventionsChange, "conventionsChange", errors, groups);
  validateRules(state.marketplace, "marketplace", errors, groups);
  validateRules(state.directPurchasePricing, "directPurchasePricing", errors, groups);
  validateRules(state.mintExtras.destinationIdentityRules, "mintExtras.destinationIdentityRules", errors, groups);
  validateRules(state.mintExtras.choosingDestinationRules, "mintExtras.choosingDestinationRules", errors, groups);

  // Mint destination validation
  if (state.manualMinting.authorizedActionTaker !== "noOne") {
    const ext = state.mintExtras;
    if (!ext.destinationDefaultToContractOwner && !ext.destinationOtherIdentityEnabled && !ext.allowChoosingDestination) {
      errors["mintExtras.destination"] = "At least one minting destination mode must be enabled";
    }
    if (ext.destinationOtherIdentityEnabled && ext.destinationOtherIdentityId.trim()) {
      if (!validateIdentityId(ext.destinationOtherIdentityId.trim())) {
        errors["mintExtras.destinationOtherIdentityId"] = "Invalid Base58 identity ID";
      }
    }
  }

  // Main control group change
  if (state.mainControlGroupChange === "identity" && state.mainControlGroupChangeIdentityId.trim()) {
    if (!validateIdentityId(state.mainControlGroupChangeIdentityId.trim())) {
      errors["mainControlGroupChange.identityId"] = "Invalid Base58 identity ID";
    }
  }
  if (state.mainControlGroupChange === "group") {
    const pos = state.mainControlGroupChangeGroupPosition.trim();
    if (pos && !validateGroupPosition(pos)) {
      errors["mainControlGroupChange.groupPosition"] = "Must be a number";
    }
  }

  return { valid: Object.keys(errors).length === 0, errors };
}

// ─── Group entry (passed from Groups step) ─────────────────────────────────

export interface GroupEntry {
  requiredPower: string;
  members: Array<{ identityId: string; power: string }>;
}

// ─── Action Taker display helpers ──────────────────────────────────────────

const ACTION_TAKER_OPTIONS: { value: ActionTaker; label: string }[] = [
  { value: "noOne", label: "No One" },
  { value: "contractOwner", label: "Contract Owner" },
  { value: "identity", label: "Identity" },
  { value: "mainGroup", label: "Main Group" },
  { value: "group", label: "Specific Group" },
];

// ─── InfoTooltip helper ─────────────────────────────────────────────────────

function InfoTooltip({ text }: { text: string }) {
  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            className="inline-flex items-center justify-center rounded-full p-0.5 text-muted-foreground hover:text-foreground transition-colors"
            aria-label="More information"
          >
            <Info className="h-3.5 w-3.5" />
          </button>
        </TooltipTrigger>
        <TooltipContent side="right" className="max-w-xs whitespace-pre-line">
          {text}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}

// ─── ActionTakerSelector ────────────────────────────────────────────────────

interface ActionTakerSelectorProps {
  label: string;
  value: ActionTaker;
  onChange: (value: ActionTaker) => void;
  identityId: string;
  onIdentityIdChange: (value: string) => void;
  groupPosition: string;
  onGroupPositionChange: (value: string) => void;
  groups: GroupEntry[];
  testIdPrefix: string;
  error?: string;
  identityError?: string;
  groupError?: string;
}

function ActionTakerSelector({
  label,
  value,
  onChange,
  identityId,
  onIdentityIdChange,
  groupPosition,
  onGroupPositionChange,
  groups,
  testIdPrefix,
  error,
  identityError,
  groupError,
}: ActionTakerSelectorProps) {
  const groupOptions = groups.length > 0
    ? ACTION_TAKER_OPTIONS
    : ACTION_TAKER_OPTIONS.map((opt) =>
        opt.value === "group"
          ? { ...opt, label: "Specific Group (No Groups Added Yet)" }
          : opt,
      );

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <Label className="text-xs whitespace-nowrap min-w-[140px]">{label}</Label>
        <Select
          value={value}
          onValueChange={(val) => onChange(val as ActionTaker)}
        >
          <SelectTrigger
            className="h-8 text-xs flex-1"
            data-testid={`${testIdPrefix}-select`}
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {groupOptions.map((opt) => (
              <SelectItem key={opt.value} value={opt.value}>
                {opt.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      {error && <p className="text-xs text-destructive pl-[148px]">{error}</p>}
      {value === "identity" && (
        <div className="pl-[148px]">
          <Input
            value={identityId}
            onChange={(e) => onIdentityIdChange(e.target.value)}
            placeholder="Base58 identity ID"
            className="h-7 text-xs font-mono"
            data-testid={`${testIdPrefix}-identity-input`}
          />
          {identityId.trim() && (
            <span
              className={cn(
                "text-xs",
                validateIdentityId(identityId.trim())
                  ? "text-green-600"
                  : "text-destructive",
              )}
              data-testid={`${testIdPrefix}-identity-status`}
            >
              {validateIdentityId(identityId.trim()) ? "\u2714" : "\u2718"} {validateIdentityId(identityId.trim()) ? "Valid" : "Invalid"} format
            </span>
          )}
          {identityError && (
            <p className="text-xs text-destructive">{identityError}</p>
          )}
        </div>
      )}
      {value === "group" && (
        <div className="pl-[148px]">
          <Input
            value={groupPosition}
            onChange={(e) =>
              onGroupPositionChange(e.target.value.replace(/[^0-9]/g, ""))
            }
            placeholder="Group position (0, 1, 2...)"
            className="h-7 text-xs w-40"
            inputMode="numeric"
            data-testid={`${testIdPrefix}-group-input`}
          />
          {groupError && (
            <p className="text-xs text-destructive">{groupError}</p>
          )}
        </div>
      )}
    </div>
  );
}

// ─── ControlRuleSection ─────────────────────────────────────────────────────

interface ControlRuleSectionProps {
  title: string;
  tooltip: string;
  rules: ChangeControlRulesState;
  onRulesChange: (rules: ChangeControlRulesState) => void;
  expanded: boolean;
  onExpandedChange: (expanded: boolean) => void;
  groups: GroupEntry[];
  testIdPrefix: string;
  errors: Record<string, string>;
  children?: React.ReactNode;
}

function ControlRuleSection({
  title,
  tooltip,
  rules,
  onRulesChange,
  expanded,
  onExpandedChange,
  groups,
  testIdPrefix,
  errors,
  children,
}: ControlRuleSectionProps) {
  const updateRules = useCallback(
    (updates: Partial<ChangeControlRulesState>) => {
      onRulesChange({ ...rules, ...updates });
    },
    [rules, onRulesChange],
  );

  const actionTakerLabel =
    ACTION_TAKER_OPTIONS.find((o) => o.value === rules.authorizedActionTaker)?.label ?? "No One";

  return (
    <div
      className="rounded-lg border bg-card"
      data-testid={`${testIdPrefix}-section`}
    >
      <div className="flex items-center gap-2 w-full px-3 py-2.5">
        <button
          type="button"
          className="flex items-center gap-2 flex-1 text-left hover:bg-accent/50 transition-colors rounded-lg -mx-3 -my-2.5 px-3 py-2.5"
          onClick={() => onExpandedChange(!expanded)}
          data-testid={`${testIdPrefix}-toggle`}
        >
          {expanded ? (
            <ChevronDown className="h-4 w-4 shrink-0" />
          ) : (
            <ChevronRight className="h-4 w-4 shrink-0" />
          )}
          <span className="text-sm font-medium flex-1">{title}</span>
          <span className="text-xs text-muted-foreground">{actionTakerLabel}</span>
        </button>
        <InfoTooltip text={tooltip} />
      </div>

      {expanded && (
        <div className="px-3 pb-3 space-y-3 border-t pt-3">
          <ActionTakerSelector
            label="Authorized to act:"
            value={rules.authorizedActionTaker}
            onChange={(val) => updateRules({ authorizedActionTaker: val })}
            identityId={rules.authorizedIdentityId}
            onIdentityIdChange={(val) => updateRules({ authorizedIdentityId: val })}
            groupPosition={rules.authorizedGroupPosition}
            onGroupPositionChange={(val) => updateRules({ authorizedGroupPosition: val })}
            groups={groups}
            testIdPrefix={`${testIdPrefix}-authorized`}
            identityError={errors[`${testIdPrefix}.authorizedIdentityId`]}
            groupError={errors[`${testIdPrefix}.authorizedGroupPosition`]}
          />

          <ActionTakerSelector
            label="Authorized to change:"
            value={rules.adminActionTaker}
            onChange={(val) => updateRules({ adminActionTaker: val })}
            identityId={rules.adminIdentityId}
            onIdentityIdChange={(val) => updateRules({ adminIdentityId: val })}
            groupPosition={rules.adminGroupPosition}
            onGroupPositionChange={(val) => updateRules({ adminGroupPosition: val })}
            groups={groups}
            testIdPrefix={`${testIdPrefix}-admin`}
            identityError={errors[`${testIdPrefix}.adminIdentityId`]}
            groupError={errors[`${testIdPrefix}.adminGroupPosition`]}
          />

          <div className="space-y-2 pt-1">
            <div className="flex items-center gap-2">
              <Checkbox
                id={`${testIdPrefix}-authorized-no-one`}
                checked={rules.changingAuthorizedToNoOneAllowed}
                onCheckedChange={(checked) =>
                  updateRules({ changingAuthorizedToNoOneAllowed: checked === true })
                }
                data-testid={`${testIdPrefix}-authorized-no-one-checkbox`}
              />
              <Label
                htmlFor={`${testIdPrefix}-authorized-no-one`}
                className="text-xs cursor-pointer"
              >
                Changing authorized action takers to no one allowed
              </Label>
            </div>

            <div className="flex items-center gap-2">
              <Checkbox
                id={`${testIdPrefix}-admin-no-one`}
                checked={rules.changingAdminToNoOneAllowed}
                onCheckedChange={(checked) =>
                  updateRules({ changingAdminToNoOneAllowed: checked === true })
                }
                data-testid={`${testIdPrefix}-admin-no-one-checkbox`}
              />
              <Label
                htmlFor={`${testIdPrefix}-admin-no-one`}
                className="text-xs cursor-pointer"
              >
                Changing admin action takers to no one allowed
              </Label>
            </div>

            <div className="flex items-center gap-2">
              <Checkbox
                id={`${testIdPrefix}-self-changing`}
                checked={rules.selfChangingAdminAllowed}
                onCheckedChange={(checked) =>
                  updateRules({ selfChangingAdminAllowed: checked === true })
                }
                data-testid={`${testIdPrefix}-self-changing-checkbox`}
              />
              <Label
                htmlFor={`${testIdPrefix}-self-changing`}
                className="text-xs cursor-pointer"
              >
                Self-changing admin action takers allowed
              </Label>
            </div>
          </div>

          {children}
        </div>
      )}
    </div>
  );
}

// ─── MintExtrasSection ──────────────────────────────────────────────────────

interface MintExtrasSectionProps {
  extras: MintExtrasState;
  onChange: (extras: MintExtrasState) => void;
  groups: GroupEntry[];
  errors: Record<string, string>;
}

function MintExtrasSection({ extras, onChange, groups, errors }: MintExtrasSectionProps) {
  const updateExtras = useCallback(
    (updates: Partial<MintExtrasState>) => {
      onChange({ ...extras, ...updates });
    },
    [extras, onChange],
  );

  return (
    <div className="space-y-3 border-t pt-3 mt-3" data-testid="mint-extras">
      <p className="text-xs font-medium text-muted-foreground">Minting Destination Options</p>

      {errors["mintExtras.destination"] && (
        <p className="text-xs text-destructive" data-testid="mint-destination-error">
          {errors["mintExtras.destination"]}
        </p>
      )}

      <div className="flex items-center gap-2">
        <Checkbox
          id="mint-default-contract-owner"
          checked={extras.destinationDefaultToContractOwner}
          onCheckedChange={(checked) => {
            updateExtras({
              destinationDefaultToContractOwner: checked === true,
              ...(checked === true ? { destinationOtherIdentityEnabled: false } : {}),
            });
          }}
          data-testid="mint-default-contract-owner"
        />
        <Label htmlFor="mint-default-contract-owner" className="text-xs cursor-pointer">
          Default destination: Contract Owner
        </Label>
      </div>

      <div className="space-y-2">
        <div className="flex items-center gap-2">
          <Checkbox
            id="mint-other-identity"
            checked={extras.destinationOtherIdentityEnabled}
            onCheckedChange={(checked) => {
              updateExtras({
                destinationOtherIdentityEnabled: checked === true,
                ...(checked === true ? { destinationDefaultToContractOwner: false } : {}),
              });
            }}
            data-testid="mint-other-identity"
          />
          <Label htmlFor="mint-other-identity" className="text-xs cursor-pointer">
            Default destination: Specific Identity
          </Label>
        </div>

        {extras.destinationOtherIdentityEnabled && (
          <div className="ml-6 space-y-1">
            <Input
              value={extras.destinationOtherIdentityId}
              onChange={(e) =>
                updateExtras({ destinationOtherIdentityId: e.target.value })
              }
              placeholder="Base58 identity ID"
              className="h-7 text-xs font-mono"
              data-testid="mint-destination-identity-input"
            />
            {errors["mintExtras.destinationOtherIdentityId"] && (
              <p className="text-xs text-destructive">
                {errors["mintExtras.destinationOtherIdentityId"]}
              </p>
            )}

            {/* Nested rules for changing default destination identity */}
            <div className="mt-2">
              <button
                type="button"
                className="flex items-center gap-1 text-xs text-primary hover:underline"
                onClick={() =>
                  updateExtras({
                    destinationIdentityRulesExpanded: !extras.destinationIdentityRulesExpanded,
                  })
                }
                data-testid="mint-destination-rules-toggle"
              >
                {extras.destinationIdentityRulesExpanded ? (
                  <ChevronDown className="h-3 w-3" />
                ) : (
                  <ChevronRight className="h-3 w-3" />
                )}
                Change Destination Identity Rules
              </button>
              {extras.destinationIdentityRulesExpanded && (
                <div className="ml-4 mt-2 space-y-2 border-l-2 border-border pl-3">
                  <NestedRulesEditor
                    rules={extras.destinationIdentityRules}
                    onChange={(rules) =>
                      updateExtras({ destinationIdentityRules: rules })
                    }
                    groups={groups}
                    testIdPrefix="mint-dest-identity-rules"
                    errors={errors}
                  />
                </div>
              )}
            </div>
          </div>
        )}
      </div>

      <div className="space-y-2">
        <div className="flex items-center gap-2">
          <Checkbox
            id="mint-allow-choosing"
            checked={extras.allowChoosingDestination}
            onCheckedChange={(checked) =>
              updateExtras({ allowChoosingDestination: checked === true })
            }
            data-testid="mint-allow-choosing"
          />
          <Label htmlFor="mint-allow-choosing" className="text-xs cursor-pointer">
            Allow choosing destination on each mint
          </Label>
        </div>

        {extras.allowChoosingDestination && (
          <div className="ml-6">
            <button
              type="button"
              className="flex items-center gap-1 text-xs text-primary hover:underline"
              onClick={() =>
                updateExtras({
                  choosingDestinationRulesExpanded: !extras.choosingDestinationRulesExpanded,
                })
              }
              data-testid="mint-choosing-rules-toggle"
            >
              {extras.choosingDestinationRulesExpanded ? (
                <ChevronDown className="h-3 w-3" />
              ) : (
                <ChevronRight className="h-3 w-3" />
              )}
              Change Choosing Destination Rules
            </button>
            {extras.choosingDestinationRulesExpanded && (
              <div className="ml-4 mt-2 space-y-2 border-l-2 border-border pl-3">
                <NestedRulesEditor
                  rules={extras.choosingDestinationRules}
                  onChange={(rules) =>
                    updateExtras({ choosingDestinationRules: rules })
                  }
                  groups={groups}
                  testIdPrefix="mint-choosing-rules"
                  errors={errors}
                />
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

// ─── NestedRulesEditor ──────────────────────────────────────────────────────

interface NestedRulesEditorProps {
  rules: ChangeControlRulesState;
  onChange: (rules: ChangeControlRulesState) => void;
  groups: GroupEntry[];
  testIdPrefix: string;
  errors: Record<string, string>;
}

function NestedRulesEditor({
  rules,
  onChange,
  groups,
  testIdPrefix,
  errors,
}: NestedRulesEditorProps) {
  const updateRules = useCallback(
    (updates: Partial<ChangeControlRulesState>) => {
      onChange({ ...rules, ...updates });
    },
    [rules, onChange],
  );

  return (
    <div className="space-y-2" data-testid={`${testIdPrefix}-editor`}>
      <ActionTakerSelector
        label="Authorized to act:"
        value={rules.authorizedActionTaker}
        onChange={(val) => updateRules({ authorizedActionTaker: val })}
        identityId={rules.authorizedIdentityId}
        onIdentityIdChange={(val) => updateRules({ authorizedIdentityId: val })}
        groupPosition={rules.authorizedGroupPosition}
        onGroupPositionChange={(val) => updateRules({ authorizedGroupPosition: val })}
        groups={groups}
        testIdPrefix={`${testIdPrefix}-authorized`}
        identityError={errors[`${testIdPrefix}.authorizedIdentityId`]}
        groupError={errors[`${testIdPrefix}.authorizedGroupPosition`]}
      />

      <ActionTakerSelector
        label="Authorized to change:"
        value={rules.adminActionTaker}
        onChange={(val) => updateRules({ adminActionTaker: val })}
        identityId={rules.adminIdentityId}
        onIdentityIdChange={(val) => updateRules({ adminIdentityId: val })}
        groupPosition={rules.adminGroupPosition}
        onGroupPositionChange={(val) => updateRules({ adminGroupPosition: val })}
        groups={groups}
        testIdPrefix={`${testIdPrefix}-admin`}
        identityError={errors[`${testIdPrefix}.adminIdentityId`]}
        groupError={errors[`${testIdPrefix}.adminGroupPosition`]}
      />

      <div className="space-y-2 pt-1">
        <div className="flex items-center gap-2">
          <Checkbox
            id={`${testIdPrefix}-authorized-no-one`}
            checked={rules.changingAuthorizedToNoOneAllowed}
            onCheckedChange={(checked) =>
              updateRules({ changingAuthorizedToNoOneAllowed: checked === true })
            }
            data-testid={`${testIdPrefix}-authorized-no-one-checkbox`}
          />
          <Label
            htmlFor={`${testIdPrefix}-authorized-no-one`}
            className="text-xs cursor-pointer"
          >
            Changing authorized to no one allowed
          </Label>
        </div>
        <div className="flex items-center gap-2">
          <Checkbox
            id={`${testIdPrefix}-admin-no-one`}
            checked={rules.changingAdminToNoOneAllowed}
            onCheckedChange={(checked) =>
              updateRules({ changingAdminToNoOneAllowed: checked === true })
            }
            data-testid={`${testIdPrefix}-admin-no-one-checkbox`}
          />
          <Label
            htmlFor={`${testIdPrefix}-admin-no-one`}
            className="text-xs cursor-pointer"
          >
            Changing admin to no one allowed
          </Label>
        </div>
        <div className="flex items-center gap-2">
          <Checkbox
            id={`${testIdPrefix}-self-changing`}
            checked={rules.selfChangingAdminAllowed}
            onCheckedChange={(checked) =>
              updateRules({ selfChangingAdminAllowed: checked === true })
            }
            data-testid={`${testIdPrefix}-self-changing-checkbox`}
          />
          <Label
            htmlFor={`${testIdPrefix}-self-changing`}
            className="text-xs cursor-pointer"
          >
            Self-changing admin allowed
          </Label>
        </div>
      </div>
    </div>
  );
}

// ─── ControlRulesStep ───────────────────────────────────────────────────────

export interface ControlRulesStepProps {
  state: ControlRulesState;
  onChange: (state: ControlRulesState) => void;
  groups?: GroupEntry[];
}

export function ControlRulesStep({
  state,
  onChange,
  groups = [],
}: ControlRulesStepProps) {
  const validation = validateControlRules(state, groups);

  const updateRule = useCallback(
    <K extends keyof ControlRulesState>(key: K, value: ControlRulesState[K]) => {
      onChange({ ...state, [key]: value });
    },
    [state, onChange],
  );

  return (
    <div className="space-y-3" data-testid="control-rules-step">
      <p className="text-sm text-muted-foreground mb-4">
        Configure who can perform actions on this token and who can change these rules.
        Each rule has an action taker (who can perform the action) and an admin (who can change the rule).
      </p>

      {/* Manual Minting */}
      <ControlRuleSection
        title="Manual Minting"
        tooltip="Controls who can manually create (mint) new tokens. Minted tokens are added to the circulating supply."
        rules={state.manualMinting}
        onRulesChange={(rules) => updateRule("manualMinting", rules)}
        expanded={state.manualMintingExpanded}
        onExpandedChange={(expanded) => updateRule("manualMintingExpanded", expanded)}
        groups={groups}
        testIdPrefix="manualMinting"
        errors={validation.errors}
      >
        {state.manualMinting.authorizedActionTaker !== "noOne" && (
          <MintExtrasSection
            extras={state.mintExtras}
            onChange={(extras) => updateRule("mintExtras", extras)}
            groups={groups}
            errors={validation.errors}
          />
        )}
      </ControlRuleSection>

      {/* Manual Burning */}
      <ControlRuleSection
        title="Manual Burning"
        tooltip="Controls who can manually destroy (burn) tokens. Burned tokens are removed from the circulating supply."
        rules={state.manualBurning}
        onRulesChange={(rules) => updateRule("manualBurning", rules)}
        expanded={state.manualBurningExpanded}
        onExpandedChange={(expanded) => updateRule("manualBurningExpanded", expanded)}
        groups={groups}
        testIdPrefix="manualBurning"
        errors={validation.errors}
      />

      {/* Freeze */}
      <ControlRuleSection
        title="Freeze"
        tooltip="Controls who can freeze identities, preventing them from sending tokens. Frozen identities can still receive tokens if 'Allow transfers to frozen identities' is enabled in Basic Info."
        rules={state.freeze}
        onRulesChange={(rules) => updateRule("freeze", rules)}
        expanded={state.freezeExpanded}
        onExpandedChange={(expanded) => updateRule("freezeExpanded", expanded)}
        groups={groups}
        testIdPrefix="freeze"
        errors={validation.errors}
      />

      {/* Unfreeze */}
      <ControlRuleSection
        title="Unfreeze"
        tooltip="Controls who can unfreeze previously frozen identities, restoring their ability to send tokens."
        rules={state.unfreeze}
        onRulesChange={(rules) => updateRule("unfreeze", rules)}
        expanded={state.unfreezeExpanded}
        onExpandedChange={(expanded) => updateRule("unfreezeExpanded", expanded)}
        groups={groups}
        testIdPrefix="unfreeze"
        errors={validation.errors}
      />

      {/* Destroy Frozen Funds */}
      <ControlRuleSection
        title="Destroy Frozen Funds"
        tooltip="Controls who can permanently destroy tokens held by frozen identities. This is an irreversible action."
        rules={state.destroyFrozenFunds}
        onRulesChange={(rules) => updateRule("destroyFrozenFunds", rules)}
        expanded={state.destroyFrozenFundsExpanded}
        onExpandedChange={(expanded) => updateRule("destroyFrozenFundsExpanded", expanded)}
        groups={groups}
        testIdPrefix="destroyFrozenFunds"
        errors={validation.errors}
      />

      {/* Emergency Action (Pause/Resume) */}
      <ControlRuleSection
        title="Emergency Action (Pause/Resume)"
        tooltip="Controls who can pause and resume the token. When paused, all transfers are disabled."
        rules={state.emergencyAction}
        onRulesChange={(rules) => updateRule("emergencyAction", rules)}
        expanded={state.emergencyActionExpanded}
        onExpandedChange={(expanded) => updateRule("emergencyActionExpanded", expanded)}
        groups={groups}
        testIdPrefix="emergencyAction"
        errors={validation.errors}
      />

      {/* Max Supply Change */}
      <ControlRuleSection
        title="Max Supply Change"
        tooltip="Controls who can change the maximum supply limit for the token."
        rules={state.maxSupplyChange}
        onRulesChange={(rules) => updateRule("maxSupplyChange", rules)}
        expanded={state.maxSupplyChangeExpanded}
        onExpandedChange={(expanded) => updateRule("maxSupplyChangeExpanded", expanded)}
        groups={groups}
        testIdPrefix="maxSupplyChange"
        errors={validation.errors}
      />

      {/* Conventions Change */}
      <ControlRuleSection
        title="Conventions Change"
        tooltip="Controls who can change token conventions (name, description, etc.)."
        rules={state.conventionsChange}
        onRulesChange={(rules) => updateRule("conventionsChange", rules)}
        expanded={state.conventionsChangeExpanded}
        onExpandedChange={(expanded) => updateRule("conventionsChangeExpanded", expanded)}
        groups={groups}
        testIdPrefix="conventionsChange"
        errors={validation.errors}
      />

      {/* Marketplace Trade Mode Change */}
      <ControlRuleSection
        title="Marketplace Trade Mode Change"
        tooltip="Controls who can change marketplace trading rules. Note: Marketplace trading is not yet supported on Dash Platform."
        rules={state.marketplace}
        onRulesChange={(rules) => updateRule("marketplace", rules)}
        expanded={state.marketplaceExpanded}
        onExpandedChange={(expanded) => updateRule("marketplaceExpanded", expanded)}
        groups={groups}
        testIdPrefix="marketplace"
        errors={validation.errors}
      />

      {/* Direct Purchase Pricing Change */}
      <ControlRuleSection
        title="Direct Purchase Pricing Change"
        tooltip="Controls who can change the direct purchase pricing for this token."
        rules={state.directPurchasePricing}
        onRulesChange={(rules) => updateRule("directPurchasePricing", rules)}
        expanded={state.directPurchasePricingExpanded}
        onExpandedChange={(expanded) => updateRule("directPurchasePricingExpanded", expanded)}
        groups={groups}
        testIdPrefix="directPurchasePricing"
        errors={validation.errors}
      />

      {/* Main Control Group Change */}
      <div
        className="rounded-lg border bg-card"
        data-testid="mainControlGroupChange-section"
      >
        <div className="flex items-center gap-2 w-full px-3 py-2.5">
          <button
            type="button"
            className="flex items-center gap-2 flex-1 text-left hover:bg-accent/50 transition-colors rounded-lg -mx-3 -my-2.5 px-3 py-2.5"
            onClick={() =>
              updateRule("mainControlGroupChangeExpanded", !state.mainControlGroupChangeExpanded)
            }
            data-testid="mainControlGroupChange-toggle"
          >
            {state.mainControlGroupChangeExpanded ? (
              <ChevronDown className="h-4 w-4 shrink-0" />
            ) : (
              <ChevronRight className="h-4 w-4 shrink-0" />
            )}
            <span className="text-sm font-medium flex-1">Main Control Group Change</span>
            <span className="text-xs text-muted-foreground">
              {ACTION_TAKER_OPTIONS.find((o) => o.value === state.mainControlGroupChange)?.label ?? "No One"}
            </span>
          </button>
          <InfoTooltip text="Controls who can change the main control group for this token contract." />
        </div>

        {state.mainControlGroupChangeExpanded && (
          <div className="px-3 pb-3 border-t pt-3">
            <ActionTakerSelector
              label="Authorized:"
              value={state.mainControlGroupChange}
              onChange={(val) => updateRule("mainControlGroupChange", val)}
              identityId={state.mainControlGroupChangeIdentityId}
              onIdentityIdChange={(val) =>
                updateRule("mainControlGroupChangeIdentityId", val)
              }
              groupPosition={state.mainControlGroupChangeGroupPosition}
              onGroupPositionChange={(val) =>
                updateRule("mainControlGroupChangeGroupPosition", val)
              }
              groups={groups}
              testIdPrefix="mainControlGroupChange"
              identityError={validation.errors["mainControlGroupChange.identityId"]}
              groupError={validation.errors["mainControlGroupChange.groupPosition"]}
            />
          </div>
        )}
      </div>
    </div>
  );
}
