import { useCallback, useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  Check,
  ChevronDown,
  ChevronUp,
  Info,
  Minus,
  Plus,
} from "lucide-react";
import {
  DistributionStep,
  createDefaultDistributionState,
} from "./DistributionStep";
import type { DistributionState } from "./DistributionStep";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
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

// ─── Language definitions ───────────────────────────────────────────────────

export interface TokenLanguage {
  code: string;
  label: string;
}

/**
 * All supported token name languages, matching egui's TokenNameLanguage enum.
 * First entry is English (default / always-present).
 */
export const TOKEN_LANGUAGES: TokenLanguage[] = [
  { code: "en", label: "English" },
  { code: "ar", label: "Arabic" },
  { code: "bn", label: "Bengali" },
  { code: "my", label: "Burmese" },
  { code: "zh", label: "Chinese" },
  { code: "cs", label: "Czech" },
  { code: "nl", label: "Dutch" },
  { code: "fa", label: "Farsi (Persian)" },
  { code: "fil", label: "Filipino (Tagalog)" },
  { code: "fr", label: "French" },
  { code: "de", label: "German" },
  { code: "el", label: "Greek" },
  { code: "gu", label: "Gujarati" },
  { code: "ha", label: "Hausa" },
  { code: "he", label: "Hebrew" },
  { code: "hi", label: "Hindi" },
  { code: "hu", label: "Hungarian" },
  { code: "ig", label: "Igbo" },
  { code: "id", label: "Indonesian" },
  { code: "it", label: "Italian" },
  { code: "ja", label: "Japanese" },
  { code: "jv", label: "Javanese" },
  { code: "kn", label: "Kannada" },
  { code: "km", label: "Khmer" },
  { code: "ko", label: "Korean" },
  { code: "ms", label: "Malay" },
  { code: "ml", label: "Malayalam" },
  { code: "zh-cmn", label: "Mandarin Chinese" },
  { code: "mr", label: "Marathi" },
  { code: "ne", label: "Nepali" },
  { code: "or", label: "Oriya" },
  { code: "ps", label: "Pashto" },
  { code: "pl", label: "Polish" },
  { code: "pt", label: "Portuguese" },
  { code: "pa", label: "Punjabi" },
  { code: "ro", label: "Romanian" },
  { code: "ru", label: "Russian" },
  { code: "sr", label: "Serbian" },
  { code: "sd", label: "Sindhi" },
  { code: "si", label: "Sinhala" },
  { code: "so", label: "Somali" },
  { code: "es", label: "Spanish" },
  { code: "sw", label: "Swahili" },
  { code: "sv", label: "Swedish" },
  { code: "ta", label: "Tamil" },
  { code: "te", label: "Telugu" },
  { code: "th", label: "Thai" },
  { code: "tr", label: "Turkish" },
  { code: "uk", label: "Ukrainian" },
  { code: "ur", label: "Urdu" },
  { code: "vi", label: "Vietnamese" },
  { code: "yo", label: "Yoruba" },
];

// ─── Preset definitions ─────────────────────────────────────────────────────

export type TokenPreset =
  | "mostRestrictive"
  | "onlyEmergency"
  | "mintingAndBurning"
  | "advancedActions"
  | "allAllowed";

export interface TokenPresetOption {
  value: TokenPreset;
  label: string;
  description: string;
}

export const TOKEN_PRESETS: TokenPresetOption[] = [
  {
    value: "mostRestrictive",
    label: "Most Restrictive",
    description: "No actions allowed after creation",
  },
  {
    value: "onlyEmergency",
    label: "Only Emergency Action",
    description: "Can pause/unpause token",
  },
  {
    value: "mintingAndBurning",
    label: "Minting and Burning",
    description: "Can mint and burn tokens",
  },
  {
    value: "advancedActions",
    label: "Advanced Actions",
    description: "Mint, burn, freeze, and more",
  },
  {
    value: "allAllowed",
    label: "All Allowed",
    description: "All actions enabled",
  },
];

// ─── Token name entry ───────────────────────────────────────────────────────

export interface TokenNameEntry {
  singular: string;
  plural: string;
  languageCode: string;
}

// ─── BasicInfo state ────────────────────────────────────────────────────────

export interface BasicInfoState {
  names: TokenNameEntry[];
  description: string;
  baseSupply: string;
  maxSupply: string;
  decimals: string;
  shouldCapitalize: boolean;
  startPaused: boolean;
  allowTransfersToFrozen: boolean;
  contractKeywords: string;
  preset: TokenPreset | null;
}

export function createDefaultBasicInfo(): BasicInfoState {
  return {
    names: [{ singular: "", plural: "", languageCode: "en" }],
    description: "",
    baseSupply: "",
    maxSupply: "",
    decimals: "8",
    shouldCapitalize: false,
    startPaused: false,
    allowTransfersToFrozen: false,
    contractKeywords: "",
    preset: null,
  };
}

// ─── Validation ─────────────────────────────────────────────────────────────

export interface BasicInfoValidation {
  valid: boolean;
  errors: Record<string, string>;
}

export function validateBasicInfo(state: BasicInfoState): BasicInfoValidation {
  const errors: Record<string, string> = {};

  // Token name validation (first entry required)
  const firstName = state.names[0]?.singular?.trim() ?? "";
  if (!firstName) {
    errors.tokenName = "Token name is required";
  } else if (firstName.length < 3) {
    errors.tokenName = "Token name must be at least 3 characters";
  } else if (firstName.length > 50) {
    errors.tokenName = "Token name must be at most 50 characters";
  }

  // Validate additional names
  for (let i = 1; i < state.names.length; i++) {
    const name = state.names[i].singular.trim();
    if (name && (name.length < 3 || name.length > 50)) {
      errors[`tokenName_${i}`] =
        `Name must be between 3 and 50 characters`;
    }
    const plural = state.names[i].plural.trim();
    if (plural && (plural.length < 3 || plural.length > 50)) {
      errors[`tokenPluralName_${i}`] =
        `Plural name must be between 3 and 50 characters`;
    }
  }

  // Duplicate language check
  const usedLangs = new Set<string>();
  for (let i = 0; i < state.names.length; i++) {
    const lang = state.names[i].languageCode;
    if (usedLangs.has(lang)) {
      errors[`language_${i}`] = "Duplicate language";
    }
    usedLangs.add(lang);
  }

  // Base supply validation
  const baseSupply = state.baseSupply.trim();
  if (!baseSupply) {
    errors.baseSupply = "Base supply is required";
  } else if (!/^\d+(\.\d+)?$/.test(baseSupply)) {
    errors.baseSupply = "Base supply must be a valid number";
  } else if (Number(baseSupply) <= 0) {
    errors.baseSupply = "Base supply must be greater than 0";
  }

  // Max supply validation (optional)
  const maxSupply = state.maxSupply.trim();
  if (maxSupply) {
    if (!/^\d+(\.\d+)?$/.test(maxSupply)) {
      errors.maxSupply = "Max supply must be a valid number";
    } else if (Number(maxSupply) < 0) {
      errors.maxSupply = "Max supply cannot be negative";
    }
  }

  // Decimals validation
  const decimals = state.decimals.trim();
  if (!decimals) {
    errors.decimals = "Decimals is required";
  } else if (!/^\d+$/.test(decimals)) {
    errors.decimals = "Decimals must be a whole number";
  } else {
    const val = parseInt(decimals, 10);
    if (val < 0 || val > 99) {
      errors.decimals = "Decimals must be between 0 and 99";
    }
  }

  // Description validation (optional but max 100)
  if (state.description.length > 100) {
    errors.description = "Description must be at most 100 characters";
  }

  // Contract keywords validation
  if (state.contractKeywords.trim()) {
    const keywords = state.contractKeywords
      .split(",")
      .map((k) => k.trim())
      .filter(Boolean);
    for (const keyword of keywords) {
      if (keyword.length < 3 || keyword.length > 50) {
        errors.contractKeywords =
          "Each keyword must be between 3 and 50 characters";
        break;
      }
    }
    // Duplicate check
    const seen = new Set<string>();
    for (const keyword of keywords) {
      if (seen.has(keyword.toLowerCase())) {
        errors.contractKeywords = `Duplicate keyword: "${keyword}"`;
        break;
      }
      seen.add(keyword.toLowerCase());
    }
  }

  return {
    valid: Object.keys(errors).length === 0,
    errors,
  };
}

// ─── Step definitions ───────────────────────────────────────────────────────

export const WIZARD_STEPS = [
  { key: "basicInfo", label: "Basic Info" },
  { key: "distribution", label: "Distribution" },
  { key: "controlRules", label: "Control Rules" },
  { key: "groups", label: "Groups" },
  { key: "history", label: "History" },
  { key: "keywords", label: "Keywords" },
  { key: "review", label: "Review & Create" },
] as const;

export type WizardStepKey = (typeof WIZARD_STEPS)[number]["key"];

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

// ─── TokenCreatorWizard ─────────────────────────────────────────────────────

export interface TokenCreatorWizardProps {
  /** Called when the user clicks Cancel or back-arrow at step 0 */
  onCancel: () => void;
}

export function TokenCreatorWizard({ onCancel }: TokenCreatorWizardProps) {
  const [currentStep, setCurrentStep] = useState(0);
  const [basicInfo, setBasicInfo] = useState<BasicInfoState>(
    createDefaultBasicInfo,
  );
  const [distribution, setDistribution] = useState<DistributionState>(
    createDefaultDistributionState,
  );

  const canGoNext = useCallback(() => {
    if (currentStep === 0) {
      return validateBasicInfo(basicInfo).valid;
    }
    return true;
  }, [currentStep, basicInfo]);

  const handleNext = useCallback(() => {
    if (currentStep < WIZARD_STEPS.length - 1 && canGoNext()) {
      setCurrentStep((s) => s + 1);
    }
  }, [currentStep, canGoNext]);

  const handlePrevious = useCallback(() => {
    if (currentStep > 0) {
      setCurrentStep((s) => s - 1);
    }
  }, [currentStep]);

  const handleCancel = useCallback(() => {
    onCancel();
  }, [onCancel]);

  return (
    <div className="flex flex-col h-full" data-testid="token-creator-wizard">
      {/* Step indicator */}
      <div className="flex items-center gap-1 px-1 py-3 mb-4 overflow-x-auto">
        {WIZARD_STEPS.map((step, index) => (
          <div key={step.key} className="flex items-center">
            {index > 0 && (
              <div
                className={cn(
                  "h-px w-6 mx-1",
                  index <= currentStep ? "bg-primary" : "bg-border",
                )}
              />
            )}
            <button
              type="button"
              onClick={() => {
                // Allow clicking on completed steps or current step
                if (index <= currentStep) setCurrentStep(index);
              }}
              disabled={index > currentStep}
              className={cn(
                "flex items-center gap-1.5 rounded-full px-3 py-1.5 text-xs font-medium transition-colors whitespace-nowrap",
                index === currentStep &&
                  "bg-primary text-primary-foreground",
                index < currentStep &&
                  "bg-primary/10 text-primary cursor-pointer hover:bg-primary/20",
                index > currentStep &&
                  "bg-muted text-muted-foreground cursor-not-allowed",
              )}
            >
              {index < currentStep ? (
                <Check className="h-3.5 w-3.5" />
              ) : (
                <span>{index + 1}</span>
              )}
              {step.label}
            </button>
          </div>
        ))}
      </div>

      {/* Step content */}
      <div className="flex-1 min-h-0 overflow-y-auto">
        {currentStep === 0 && (
          <BasicInfoStep state={basicInfo} onChange={setBasicInfo} />
        )}
        {currentStep === 1 && (
          <DistributionStep state={distribution} onChange={setDistribution} />
        )}
        {currentStep === 2 && (
          <PlaceholderStep label="Control Rules" />
        )}
        {currentStep === 3 && (
          <PlaceholderStep label="Groups" />
        )}
        {currentStep === 4 && (
          <PlaceholderStep label="History" />
        )}
        {currentStep === 5 && (
          <PlaceholderStep label="Keywords" />
        )}
        {currentStep === 6 && (
          <PlaceholderStep label="Review & Create" />
        )}
      </div>

      {/* Navigation buttons */}
      <div className="flex items-center justify-between border-t pt-4 mt-4">
        <Button
          variant="outline"
          onClick={currentStep === 0 ? handleCancel : handlePrevious}
          data-testid="wizard-back"
        >
          <ArrowLeft className="h-4 w-4 mr-1" />
          {currentStep === 0 ? "Cancel" : "Previous"}
        </Button>

        <span className="text-sm text-muted-foreground">
          Step {currentStep + 1} of {WIZARD_STEPS.length}
        </span>

        {currentStep < WIZARD_STEPS.length - 1 ? (
          <Button
            onClick={handleNext}
            disabled={!canGoNext()}
            data-testid="wizard-next"
          >
            Next
            <ArrowRight className="h-4 w-4 ml-1" />
          </Button>
        ) : (
          <Button data-testid="wizard-create" disabled>
            Create Token
          </Button>
        )}
      </div>
    </div>
  );
}

// ─── PlaceholderStep ────────────────────────────────────────────────────────

function PlaceholderStep({ label }: { label: string }) {
  return (
    <div className="flex items-center justify-center h-48 text-muted-foreground" data-testid={`placeholder-step-${label.toLowerCase().replace(/[^a-z0-9]/g, "-")}`}>
      {label} — coming soon
    </div>
  );
}

// ─── BasicInfoStep ──────────────────────────────────────────────────────────

interface BasicInfoStepProps {
  state: BasicInfoState;
  onChange: (state: BasicInfoState) => void;
}

export function BasicInfoStep({ state, onChange }: BasicInfoStepProps) {
  const validation = validateBasicInfo(state);

  const updateField = useCallback(
    <K extends keyof BasicInfoState>(key: K, value: BasicInfoState[K]) => {
      onChange({ ...state, [key]: value });
    },
    [state, onChange],
  );

  const updateName = useCallback(
    (
      index: number,
      field: keyof TokenNameEntry,
      value: string,
    ) => {
      const newNames = [...state.names];
      newNames[index] = { ...newNames[index], [field]: value };
      onChange({ ...state, names: newNames });
    },
    [state, onChange],
  );

  const addLanguage = useCallback(() => {
    const usedCodes = new Set(state.names.map((n) => n.languageCode));
    const next = TOKEN_LANGUAGES.find((l) => !usedCodes.has(l.code));
    if (next) {
      onChange({
        ...state,
        names: [
          ...state.names,
          { singular: "", plural: "", languageCode: next.code },
        ],
      });
    }
  }, [state, onChange]);

  const removeLanguage = useCallback(
    (index: number) => {
      if (index === 0) return; // Cannot remove first (English)
      const newNames = state.names.filter((_, i) => i !== index);
      onChange({ ...state, names: newNames });
    },
    [state, onChange],
  );

  const [advancedExpanded, setAdvancedExpanded] = useState(false);

  // Compute how many additional languages are still available
  const usedCodes = new Set(state.names.map((n) => n.languageCode));
  const canAddLanguage = TOKEN_LANGUAGES.some((l) => !usedCodes.has(l.code));

  // Decimals example text
  const pluralName =
    state.names[0]?.plural?.trim() ||
    (state.names[0]?.singular?.trim()
      ? state.names[0].singular.trim() + "s"
      : "<Token Name>");
  const decimalsExample =
    state.decimals === "0"
      ? `Non-fractional token (e.g. 0, 1, 2 or 10 ${pluralName})`
      : `Fractional token (e.g. 0.2 ${pluralName})`;

  return (
    <div className="space-y-6" data-testid="basic-info-step">
      {/* Token Name(s) */}
      <div className="space-y-3">
        {state.names.map((entry, index) => (
          <div key={index} className="space-y-2">
            <div className="flex items-end gap-2">
              <div className="flex-1 space-y-1">
                <div className="flex items-center gap-1">
                  <Label htmlFor={`token-name-singular-${index}`}>
                    Token Name (singular){index === 0 ? "*" : ""}
                  </Label>
                  {index === 0 && (
                    <InfoTooltip text="The name of your token (e.g., 'MyCoin', 'GameToken'). Must be between 3 and 50 characters." />
                  )}
                </div>
                <Input
                  id={`token-name-singular-${index}`}
                  data-testid={`token-name-singular-${index}`}
                  value={entry.singular}
                  onChange={(e) =>
                    updateName(index, "singular", e.target.value)
                  }
                  placeholder="Token name"
                  maxLength={50}
                />
                {index === 0 && validation.errors.tokenName && (
                  <p className="text-xs text-destructive">
                    {validation.errors.tokenName}
                  </p>
                )}
                {index > 0 && validation.errors[`tokenName_${index}`] && (
                  <p className="text-xs text-destructive">
                    {validation.errors[`tokenName_${index}`]}
                  </p>
                )}
              </div>

              <div className="w-40">
                <Select
                  value={entry.languageCode}
                  onValueChange={(val) =>
                    updateName(index, "languageCode", val)
                  }
                >
                  <SelectTrigger data-testid={`language-select-${index}`}>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {TOKEN_LANGUAGES.filter(
                      (l) =>
                        l.code === entry.languageCode ||
                        !usedCodes.has(l.code),
                    ).map((lang) => (
                      <SelectItem key={lang.code} value={lang.code}>
                        {lang.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              {canAddLanguage && index === state.names.length - 1 && (
                <Button
                  variant="outline"
                  size="icon"
                  onClick={addLanguage}
                  title="Add language"
                  data-testid="add-language"
                >
                  <Plus className="h-4 w-4" />
                </Button>
              )}

              {index > 0 && (
                <Button
                  variant="outline"
                  size="icon"
                  onClick={() => removeLanguage(index)}
                  title="Remove language"
                  data-testid={`remove-language-${index}`}
                >
                  <Minus className="h-4 w-4" />
                </Button>
              )}
            </div>

            {/* Plural name */}
            <div className="flex-1 space-y-1 pl-0">
              <Label htmlFor={`token-name-plural-${index}`}>
                Token Name (plural){index === 0 ? "*" : ""}
              </Label>
              <Input
                id={`token-name-plural-${index}`}
                data-testid={`token-name-plural-${index}`}
                value={entry.plural}
                onChange={(e) => updateName(index, "plural", e.target.value)}
                placeholder={
                  entry.singular
                    ? `${entry.singular}s`
                    : "Plural form"
                }
                maxLength={50}
              />
              {validation.errors[`tokenPluralName_${index}`] && (
                <p className="text-xs text-destructive">
                  {validation.errors[`tokenPluralName_${index}`]}
                </p>
              )}
            </div>

            {validation.errors[`language_${index}`] && (
              <p className="text-xs text-destructive">
                {validation.errors[`language_${index}`]}
              </p>
            )}
          </div>
        ))}
      </div>

      {/* Description */}
      <div className="space-y-1">
        <div className="flex items-center gap-1">
          <Label htmlFor="token-description">
            Description
          </Label>
          <InfoTooltip text="An optional description explaining what your token is for. Maximum 100 characters." />
        </div>
        <Input
          id="token-description"
          data-testid="token-description"
          value={state.description}
          onChange={(e) => updateField("description", e.target.value)}
          placeholder="What is this token for?"
          maxLength={100}
        />
        <div className="flex justify-between">
          {validation.errors.description ? (
            <p className="text-xs text-destructive">
              {validation.errors.description}
            </p>
          ) : (
            <span />
          )}
          <span className="text-xs text-muted-foreground">
            {state.description.length}/100
          </span>
        </div>
      </div>

      {/* Base Supply */}
      <div className="space-y-1">
        <div className="flex items-center gap-1">
          <Label htmlFor="base-supply">Base Supply*</Label>
          <InfoTooltip text="The number of tokens to create when the token is registered. These tokens will be owned by you (the token creator). You can mint more tokens later if minting is enabled." />
        </div>
        <Input
          id="base-supply"
          data-testid="base-supply"
          value={state.baseSupply}
          onChange={(e) => {
            const val = e.target.value.replace(/[^0-9.]/g, "");
            updateField("baseSupply", val);
          }}
          placeholder="e.g. 1000000"
          inputMode="decimal"
        />
        {validation.errors.baseSupply && (
          <p className="text-xs text-destructive">
            {validation.errors.baseSupply}
          </p>
        )}
      </div>

      {/* Max Supply */}
      <div className="space-y-1">
        <div className="flex items-center gap-1">
          <Label htmlFor="max-supply">Max Supply</Label>
          <InfoTooltip text="The maximum number of tokens that can ever exist. Leave empty or set to 0 for no maximum (unlimited supply)." />
        </div>
        <Input
          id="max-supply"
          data-testid="max-supply"
          value={state.maxSupply}
          onChange={(e) => {
            const val = e.target.value.replace(/[^0-9.]/g, "");
            updateField("maxSupply", val);
          }}
          placeholder="Unlimited (leave empty)"
          inputMode="decimal"
        />
        {validation.errors.maxSupply && (
          <p className="text-xs text-destructive">
            {validation.errors.maxSupply}
          </p>
        )}
      </div>

      {/* Token Preset */}
      <div className="space-y-1">
        <div className="flex items-center gap-1">
          <Label htmlFor="token-preset">Token Preset</Label>
          <InfoTooltip
            text={
              "Choose a preset that determines what actions are allowed on your token.\n\n" +
              "- Most Restrictive: No actions allowed after creation\n" +
              "- Only Emergency Action: Can pause/unpause token\n" +
              "- Minting and Burning: Can mint and burn tokens\n" +
              "- Advanced Actions: Mint, burn, freeze, and more\n" +
              "- All Allowed: All actions enabled"
            }
          />
        </div>
        <Select
          value={state.preset ?? ""}
          onValueChange={(val) =>
            updateField("preset", (val || null) as TokenPreset | null)
          }
        >
          <SelectTrigger data-testid="token-preset-select">
            <SelectValue placeholder="Select a preset..." />
          </SelectTrigger>
          <SelectContent>
            {TOKEN_PRESETS.map((p) => (
              <SelectItem key={p.value} value={p.value}>
                {p.label} — {p.description}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {/* Advanced section (collapsible) */}
      <div>
        <button
          type="button"
          className="flex items-center gap-1 text-sm font-medium text-primary hover:underline"
          onClick={() => setAdvancedExpanded(!advancedExpanded)}
          data-testid="advanced-toggle"
        >
          {advancedExpanded ? (
            <ChevronUp className="h-4 w-4" />
          ) : (
            <ChevronDown className="h-4 w-4" />
          )}
          Advanced Options
        </button>

        {advancedExpanded && (
          <div className="mt-3 space-y-4 pl-2 border-l-2 border-border ml-2">
            {/* Start as Paused */}
            <div className="flex items-center gap-2">
              <Checkbox
                id="start-paused"
                data-testid="start-paused"
                checked={state.startPaused}
                onCheckedChange={(checked) =>
                  updateField("startPaused", checked === true)
                }
              />
              <div className="flex items-center gap-1">
                <Label htmlFor="start-paused" className="cursor-pointer">
                  Start as paused
                </Label>
                <InfoTooltip text="When enabled, the token will be created in a paused state, meaning transfers will be disabled by default. To allow transfers in the future, the token must be unpaused via an emergency action." />
              </div>
            </div>

            {/* Name should be capitalized */}
            <div className="flex items-center gap-2">
              <Checkbox
                id="should-capitalize"
                data-testid="should-capitalize"
                checked={state.shouldCapitalize}
                onCheckedChange={(checked) =>
                  updateField("shouldCapitalize", checked === true)
                }
              />
              <div className="flex items-center gap-1">
                <Label htmlFor="should-capitalize" className="cursor-pointer">
                  Name should be capitalized
                </Label>
                <InfoTooltip text="Informs client applications whether to capitalize the token name by default." />
              </div>
            </div>

            {/* Allow transfers to frozen identities */}
            <div className="flex items-center gap-2">
              <Checkbox
                id="allow-frozen-transfers"
                data-testid="allow-frozen-transfers"
                checked={state.allowTransfersToFrozen}
                onCheckedChange={(checked) =>
                  updateField("allowTransfersToFrozen", checked === true)
                }
              />
              <div className="flex items-center gap-1">
                <Label
                  htmlFor="allow-frozen-transfers"
                  className="cursor-pointer"
                >
                  Allow transfers to frozen identities
                </Label>
                <InfoTooltip text="When enabled, tokens can be transferred TO identities that are frozen. Frozen identities still cannot send tokens." />
              </div>
            </div>

            {/* Max Decimals */}
            <div className="space-y-1">
              <div className="flex items-center gap-1">
                <Label htmlFor="decimals">Max Decimals</Label>
                <InfoTooltip text="The decimal places of the token. For example, Dash and Bitcoin use 8. If 0, the token is non-fractional." />
              </div>
              <div className="flex items-center gap-2">
                <Input
                  id="decimals"
                  data-testid="decimals"
                  value={state.decimals}
                  onChange={(e) => {
                    const val = e.target.value.replace(/[^0-9]/g, "");
                    updateField("decimals", val.slice(0, 2));
                  }}
                  className="w-20"
                  inputMode="numeric"
                  maxLength={2}
                />
                <span className="text-xs text-muted-foreground">
                  {decimalsExample}
                </span>
              </div>
              {validation.errors.decimals && (
                <p className="text-xs text-destructive">
                  {validation.errors.decimals}
                </p>
              )}
            </div>

            {/* Contract Keywords */}
            <div className="space-y-1">
              <div className="flex items-center gap-1">
                <Label htmlFor="contract-keywords">
                  Contract Keywords (comma separated)
                </Label>
                <InfoTooltip text="Each searchable keyword costs 0.1 Dash. Keywords must be between 3 and 50 characters each." />
              </div>
              <Input
                id="contract-keywords"
                data-testid="contract-keywords"
                value={state.contractKeywords}
                onChange={(e) =>
                  updateField("contractKeywords", e.target.value)
                }
                placeholder="e.g. gaming, reward, loyalty"
              />
              {validation.errors.contractKeywords && (
                <p className="text-xs text-destructive">
                  {validation.errors.contractKeywords}
                </p>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
