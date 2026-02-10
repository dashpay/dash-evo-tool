import { useCallback, useMemo } from "react";
import { ChevronDown, ChevronRight, Info } from "lucide-react";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";

// ─── Types ──────────────────────────────────────────────────────────────────

export interface HistoryState {
  keepTransferHistory: boolean;
  keepFreezingHistory: boolean;
  keepMintingHistory: boolean;
  keepBurningHistory: boolean;
  keepDirectPricingHistory: boolean;
  keepDirectPurchaseHistory: boolean;
  showAdvanced: boolean;
}

// ─── Defaults ───────────────────────────────────────────────────────────────

export function createDefaultHistoryState(): HistoryState {
  return {
    keepTransferHistory: true,
    keepFreezingHistory: true,
    keepMintingHistory: true,
    keepBurningHistory: true,
    keepDirectPricingHistory: true,
    keepDirectPurchaseHistory: true,
    showAdvanced: false,
  };
}

// ─── Parent state helper ────────────────────────────────────────────────────

export type TriState = true | false | "indeterminate";

export function computeParentState(state: HistoryState): TriState {
  const flags = [
    state.keepTransferHistory,
    state.keepFreezingHistory,
    state.keepMintingHistory,
    state.keepBurningHistory,
    state.keepDirectPricingHistory,
    state.keepDirectPurchaseHistory,
  ];
  const allOn = flags.every(Boolean);
  const noneOn = flags.every((f) => !f);
  if (allOn) return true;
  if (noneOn) return false;
  return "indeterminate";
}

// ─── Sub-checkbox definitions ───────────────────────────────────────────────

interface HistoryField {
  key: keyof Omit<HistoryState, "showAdvanced">;
  label: string;
}

const HISTORY_FIELDS: HistoryField[] = [
  { key: "keepTransferHistory", label: "Transfers" },
  { key: "keepFreezingHistory", label: "Freezes / unfreezes" },
  { key: "keepMintingHistory", label: "Mints" },
  { key: "keepBurningHistory", label: "Burns" },
  { key: "keepDirectPricingHistory", label: "Direct-pricing changes" },
  { key: "keepDirectPurchaseHistory", label: "Direct purchases" },
];

// ─── InfoTooltip ────────────────────────────────────────────────────────────

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

// ─── HistoryStep ────────────────────────────────────────────────────────────

export interface HistoryStepProps {
  state: HistoryState;
  onChange: (state: HistoryState) => void;
}

export function HistoryStep({ state, onChange }: HistoryStepProps) {
  const parentState = useMemo(() => computeParentState(state), [state]);

  const toggleParent = useCallback(() => {
    // Clicking when all on → all off; otherwise → all on
    const newVal = parentState !== true;
    onChange({
      ...state,
      keepTransferHistory: newVal,
      keepFreezingHistory: newVal,
      keepMintingHistory: newVal,
      keepBurningHistory: newVal,
      keepDirectPricingHistory: newVal,
      keepDirectPurchaseHistory: newVal,
    });
  }, [state, onChange, parentState]);

  const toggleAdvanced = useCallback(() => {
    onChange({ ...state, showAdvanced: !state.showAdvanced });
  }, [state, onChange]);

  const updateField = useCallback(
    (key: keyof Omit<HistoryState, "showAdvanced">, value: boolean) => {
      onChange({ ...state, [key]: value });
    },
    [state, onChange],
  );

  return (
    <div className="space-y-4" data-testid="history-step">
      <p className="text-sm text-muted-foreground">
        Configure which token operations should have their history recorded on the blockchain.
        Keeping history allows participants to audit past actions.
      </p>

      {/* Parent tri-state checkbox */}
      <div className="flex items-center gap-3">
        <Checkbox
          id="keep-history-parent"
          data-testid="keep-history-parent"
          checked={parentState === true ? true : parentState === false ? false : "indeterminate"}
          onCheckedChange={toggleParent}
        />
        <div className="flex items-center gap-1">
          <Label htmlFor="keep-history-parent" className="cursor-pointer font-medium">
            Keep history
          </Label>
          <InfoTooltip text="Enable or disable history recording for all token operations at once. Use the Advanced section to configure individual operation types." />
        </div>

        {/* Advanced toggle */}
        <button
          type="button"
          className="flex items-center gap-1 text-xs font-medium text-primary hover:underline ml-4"
          onClick={toggleAdvanced}
          data-testid="history-advanced-toggle"
        >
          {state.showAdvanced ? (
            <ChevronDown className="h-3.5 w-3.5" />
          ) : (
            <ChevronRight className="h-3.5 w-3.5" />
          )}
          Advanced
        </button>
      </div>

      {/* Summary when not expanded */}
      {!state.showAdvanced && (
        <p className="text-xs text-muted-foreground pl-7" data-testid="history-summary">
          {parentState === true
            ? "All history is being recorded."
            : parentState === false
              ? "No history is being recorded."
              : `${HISTORY_FIELDS.filter((f) => state[f.key]).length} of ${HISTORY_FIELDS.length} operation types are being recorded.`}
        </p>
      )}

      {/* Advanced sub-checkboxes */}
      {state.showAdvanced && (
        <div className="space-y-3 pl-7 border-l-2 border-border ml-2" data-testid="history-advanced-section">
          {HISTORY_FIELDS.map((field) => (
            <div key={field.key} className="flex items-center gap-2">
              <Checkbox
                id={`history-${field.key}`}
                data-testid={`history-${field.key}`}
                checked={state[field.key]}
                onCheckedChange={(checked) =>
                  updateField(field.key, checked === true)
                }
              />
              <Label htmlFor={`history-${field.key}`} className="cursor-pointer text-sm">
                {field.label}
              </Label>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
