import { useCallback, useState } from "react";
import {
  Info,
  Plus,
  Trash2,
  X,
} from "lucide-react";
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
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { Separator } from "@/components/ui/separator";

// ─── Types ─────────────────────────────────────────────────────────────────

export type DistributionIntervalType = "timeBased" | "epochBased" | "blockBased";

export type IntervalTimeUnit = "second" | "minute" | "hour" | "day" | "week" | "year";

export type DistributionFunction =
  | "fixedAmount"
  | "stepDecreasing"
  | "stepwise"
  | "linear"
  | "polynomial"
  | "exponential"
  | "logarithmic"
  | "invertedLogarithmic";

export type DistributionRecipient = "contractOwner" | "identity" | "evonodes";

export interface StepwiseEntry {
  startStep: string;
  amount: string;
}

export interface PreProgrammedEntry {
  days: string;
  hours: string;
  minutes: string;
  identityId: string;
  amount: string;
}

export interface PerpetualDistributionState {
  enabled: boolean;
  intervalType: DistributionIntervalType;
  intervalAmount: string;
  intervalTimeUnit: IntervalTimeUnit;
  distributionFunction: DistributionFunction;
  recipient: DistributionRecipient;
  recipientIdentityId: string;

  // FixedAmount
  fixedAmount: string;

  // StepDecreasing
  stepCount: string;
  decreaseNumerator: string;
  decreaseDenominator: string;
  startPeriodOffset: string;
  initialEmission: string;
  stepDecreasingMinValue: string;
  stepDecreasingMaxIntervalCount: string;
  trailingAmount: string;

  // Stepwise
  stepwiseEntries: StepwiseEntry[];

  // Linear
  linearA: string;
  linearD: string;
  linearStartStep: string;
  linearStartingAmount: string;
  linearMinValue: string;
  linearMaxValue: string;

  // Polynomial
  polyA: string;
  polyM: string;
  polyN: string;
  polyD: string;
  polyS: string;
  polyO: string;
  polyB: string;
  polyMinValue: string;
  polyMaxValue: string;

  // Exponential
  expA: string;
  expM: string;
  expN: string;
  expD: string;
  expS: string;
  expO: string;
  expB: string;
  expMinValue: string;
  expMaxValue: string;

  // Logarithmic
  logA: string;
  logD: string;
  logM: string;
  logN: string;
  logS: string;
  logO: string;
  logB: string;
  logMinValue: string;
  logMaxValue: string;

  // Inverted Logarithmic
  invLogA: string;
  invLogD: string;
  invLogM: string;
  invLogN: string;
  invLogS: string;
  invLogO: string;
  invLogB: string;
  invLogMinValue: string;
  invLogMaxValue: string;
}

export interface PreProgrammedDistributionState {
  enabled: boolean;
  entries: PreProgrammedEntry[];
}

export interface DistributionState {
  perpetual: PerpetualDistributionState;
  preProgrammed: PreProgrammedDistributionState;
}

export function createDefaultDistributionState(): DistributionState {
  return {
    perpetual: {
      enabled: false,
      intervalType: "timeBased",
      intervalAmount: "",
      intervalTimeUnit: "day",
      distributionFunction: "fixedAmount",
      recipient: "contractOwner",
      recipientIdentityId: "",
      fixedAmount: "",
      stepCount: "",
      decreaseNumerator: "",
      decreaseDenominator: "",
      startPeriodOffset: "",
      initialEmission: "",
      stepDecreasingMinValue: "",
      stepDecreasingMaxIntervalCount: "",
      trailingAmount: "",
      stepwiseEntries: [],
      linearA: "",
      linearD: "",
      linearStartStep: "",
      linearStartingAmount: "",
      linearMinValue: "",
      linearMaxValue: "",
      polyA: "",
      polyM: "",
      polyN: "",
      polyD: "",
      polyS: "",
      polyO: "",
      polyB: "",
      polyMinValue: "",
      polyMaxValue: "",
      expA: "",
      expM: "",
      expN: "",
      expD: "",
      expS: "",
      expO: "",
      expB: "",
      expMinValue: "",
      expMaxValue: "",
      logA: "",
      logD: "",
      logM: "",
      logN: "",
      logS: "",
      logO: "",
      logB: "",
      logMinValue: "",
      logMaxValue: "",
      invLogA: "",
      invLogD: "",
      invLogM: "",
      invLogN: "",
      invLogS: "",
      invLogO: "",
      invLogB: "",
      invLogMinValue: "",
      invLogMaxValue: "",
    },
    preProgrammed: {
      enabled: false,
      entries: [],
    },
  };
}

// ─── Display helpers ───────────────────────────────────────────────────────

export const INTERVAL_TYPE_OPTIONS: { value: DistributionIntervalType; label: string }[] = [
  { value: "timeBased", label: "Time-Based" },
  { value: "epochBased", label: "Epoch-Based" },
  { value: "blockBased", label: "Block-Based" },
];

export const TIME_UNIT_OPTIONS: { value: IntervalTimeUnit; label: string; pluralLabel: string }[] = [
  { value: "second", label: "Second", pluralLabel: "Seconds" },
  { value: "minute", label: "Minute", pluralLabel: "Minutes" },
  { value: "hour", label: "Hour", pluralLabel: "Hours" },
  { value: "day", label: "Day", pluralLabel: "Days" },
  { value: "week", label: "Week", pluralLabel: "Weeks" },
  { value: "year", label: "Year", pluralLabel: "Years" },
];

export const DISTRIBUTION_FUNCTION_OPTIONS: { value: DistributionFunction; label: string; formula: string }[] = [
  { value: "fixedAmount", label: "Fixed Amount", formula: "f(x) = n" },
  { value: "stepDecreasing", label: "Step Decreasing", formula: "Gradually decreasing emissions" },
  { value: "stepwise", label: "Stepwise", formula: "Fixed amounts at specific intervals" },
  { value: "linear", label: "Linear", formula: "f(x) = a·x + b" },
  { value: "polynomial", label: "Polynomial", formula: "f(x) = a·x^(m/n) + o + b" },
  { value: "exponential", label: "Exponential", formula: "f(x) = a·e^(m/n·x) + o + b" },
  { value: "logarithmic", label: "Logarithmic", formula: "f(x) = a·log(m/n, x) + o + b" },
  { value: "invertedLogarithmic", label: "Inverted Logarithmic", formula: "f(x) = a/log(m/n, x) + o + b" },
];

export const RECIPIENT_OPTIONS: { value: DistributionRecipient; label: string }[] = [
  { value: "contractOwner", label: "Contract Owner" },
  { value: "identity", label: "Specific Identity" },
  { value: "evonodes", label: "Evonodes by Participation" },
];

export function getTimeUnitLabel(unit: IntervalTimeUnit, amount: string): string {
  const num = parseInt(amount, 10);
  const opt = TIME_UNIT_OPTIONS.find((o) => o.value === unit);
  if (!opt) return unit;
  return num === 1 ? opt.label : opt.pluralLabel;
}

/** Sanitize string to only contain digits */
function sanitizeU64(value: string): string {
  return value.replace(/[^0-9]/g, "");
}

/** Sanitize string to only contain digits, minus, and plus signs */
function sanitizeI64(value: string): string {
  return value.replace(/[^0-9\-+]/g, "");
}

// ─── Formula SVG visualization ─────────────────────────────────────────────

export function FormulaVisualization({ fn }: { fn: DistributionFunction }) {
  // SVG representations of each distribution function curve
  const curves: Record<DistributionFunction, { path: string; label: string }> = {
    fixedAmount: {
      path: "M 20 60 L 280 60",
      label: "f(x) = n  (constant)",
    },
    stepDecreasing: {
      path: "M 20 30 L 70 30 L 70 45 L 120 45 L 120 55 L 170 55 L 170 62 L 220 62 L 220 67 L 280 67",
      label: "Step-decreasing emissions",
    },
    stepwise: {
      path: "M 20 30 L 80 30 L 80 60 L 140 60 L 140 40 L 200 40 L 200 70 L 280 70",
      label: "Fixed amounts at intervals",
    },
    linear: {
      path: "M 20 80 L 280 20",
      label: "f(x) = a·x + b",
    },
    polynomial: {
      path: "M 20 80 Q 100 78 150 60 Q 200 35 250 15 L 280 10",
      label: "f(x) = a·x^(m/n) + o + b",
    },
    exponential: {
      path: "M 20 80 Q 80 78 120 72 Q 160 60 200 35 Q 240 10 280 5",
      label: "f(x) = a·e^(m/n·x) + o + b",
    },
    logarithmic: {
      path: "M 20 80 Q 60 40 100 30 Q 160 18 220 14 L 280 12",
      label: "f(x) = a·log(m/n, x) + o + b",
    },
    invertedLogarithmic: {
      path: "M 20 10 Q 60 40 100 55 Q 160 68 220 74 L 280 78",
      label: "f(x) = a/log(m/n, x) + o + b",
    },
  };

  const curve = curves[fn];
  if (!curve) return null;

  return (
    <div className="my-3 rounded-lg border bg-muted/50 p-3" data-testid="formula-visualization">
      <svg viewBox="0 0 300 90" className="w-full max-w-sm h-20" aria-label={`Formula: ${curve.label}`}>
        {/* Axes */}
        <line x1="15" y1="85" x2="285" y2="85" stroke="currentColor" strokeOpacity="0.2" strokeWidth="1" />
        <line x1="15" y1="5" x2="15" y2="85" stroke="currentColor" strokeOpacity="0.2" strokeWidth="1" />
        {/* Curve */}
        <path d={curve.path} fill="none" stroke="hsl(var(--primary))" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" />
        {/* Axis labels */}
        <text x="150" y="95" textAnchor="middle" fontSize="8" fill="currentColor" opacity="0.4">intervals</text>
        <text x="5" y="50" textAnchor="middle" fontSize="8" fill="currentColor" opacity="0.4" transform="rotate(-90 5 50)">tokens</text>
      </svg>
      <p className="text-xs text-muted-foreground text-center mt-1">{curve.label}</p>
    </div>
  );
}

// ─── Info tooltip ──────────────────────────────────────────────────────────

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

// ─── Param field helper ────────────────────────────────────────────────────

interface ParamFieldProps {
  label: string;
  value: string;
  onChange: (val: string) => void;
  placeholder?: string;
  sanitize?: "u64" | "i64";
  tooltip?: string;
  testId?: string;
}

function ParamField({ label, value, onChange, placeholder, sanitize, tooltip, testId }: ParamFieldProps) {
  const handleChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      let val = e.target.value;
      if (sanitize === "u64") val = sanitizeU64(val);
      else if (sanitize === "i64") val = sanitizeI64(val);
      onChange(val);
    },
    [onChange, sanitize],
  );

  return (
    <div className="flex items-center gap-2">
      <div className="flex items-center gap-1 min-w-0 shrink-0">
        <Label className="text-xs whitespace-nowrap">{label}</Label>
        {tooltip && <InfoTooltip text={tooltip} />}
      </div>
      <Input
        value={value}
        onChange={handleChange}
        placeholder={placeholder ?? ""}
        className="h-8 text-sm flex-1"
        inputMode={sanitize === "i64" ? "text" : "numeric"}
        data-testid={testId}
      />
    </div>
  );
}

// ─── Distribution function info ────────────────────────────────────────────

const DISTRIBUTION_INFO_TEXT = `**Fixed Amount** — Emits a constant number of tokens for every period. Formula: f(x) = n

**Step Decreasing** — Gradually decreasing emissions with step-based decay. Emissions decrease by a ratio (n/d) every step count intervals.

**Stepwise** — Emissions change at specific interval boundaries. Define fixed amounts for different step ranges.

**Linear** — Linear emission function. Formula: f(x) = a·x + b. Positive slope means increasing emissions, negative means decreasing.

**Polynomial** — Power-based emissions. Formula: f(x) = a·x^(m/n) + o + b. Flexible curves for growth or decay.

**Exponential** — Exponential growth or decay. Formula: f(x) = a·e^(m/n·x) + o + b. Rapid increase or decrease.

**Logarithmic** — Logarithmic growth. Formula: f(x) = a·log(m/n, x) + o + b. Growth slows as x increases.

**Inverted Logarithmic** — Inverse logarithmic curve. Formula: f(x) = a/log(m/n, x) + o + b. Opposite of logarithmic behavior.`;

// ─── DistributionStep ──────────────────────────────────────────────────────

export interface DistributionStepProps {
  state: DistributionState;
  onChange: (state: DistributionState) => void;
}

export function DistributionStep({ state, onChange }: DistributionStepProps) {
  const [infoDialogOpen, setInfoDialogOpen] = useState(false);

  const updatePerpetual = useCallback(
    (updates: Partial<PerpetualDistributionState>) => {
      onChange({
        ...state,
        perpetual: { ...state.perpetual, ...updates },
      });
    },
    [state, onChange],
  );

  const updatePreProgrammed = useCallback(
    (updates: Partial<PreProgrammedDistributionState>) => {
      onChange({
        ...state,
        preProgrammed: { ...state.preProgrammed, ...updates },
      });
    },
    [state, onChange],
  );

  const addStepwiseEntry = useCallback(() => {
    updatePerpetual({
      stepwiseEntries: [...state.perpetual.stepwiseEntries, { startStep: "0", amount: "0" }],
    });
  }, [state.perpetual.stepwiseEntries, updatePerpetual]);

  const removeStepwiseEntry = useCallback(
    (index: number) => {
      updatePerpetual({
        stepwiseEntries: state.perpetual.stepwiseEntries.filter((_, i) => i !== index),
      });
    },
    [state.perpetual.stepwiseEntries, updatePerpetual],
  );

  const updateStepwiseEntry = useCallback(
    (index: number, field: keyof StepwiseEntry, value: string) => {
      const entries = [...state.perpetual.stepwiseEntries];
      entries[index] = { ...entries[index], [field]: sanitizeU64(value) };
      updatePerpetual({ stepwiseEntries: entries });
    },
    [state.perpetual.stepwiseEntries, updatePerpetual],
  );

  const addPreProgrammedEntry = useCallback(() => {
    updatePreProgrammed({
      entries: [
        ...state.preProgrammed.entries,
        { days: "0", hours: "0", minutes: "0", identityId: "", amount: "" },
      ],
    });
  }, [state.preProgrammed.entries, updatePreProgrammed]);

  const removePreProgrammedEntry = useCallback(
    (index: number) => {
      updatePreProgrammed({
        entries: state.preProgrammed.entries.filter((_, i) => i !== index),
      });
    },
    [state.preProgrammed.entries, updatePreProgrammed],
  );

  const updatePreProgrammedEntry = useCallback(
    (index: number, field: keyof PreProgrammedEntry, value: string) => {
      const entries = [...state.preProgrammed.entries];
      let sanitized = value;
      if (field === "days" || field === "hours" || field === "minutes" || field === "amount") {
        sanitized = sanitizeU64(value);
      }
      entries[index] = { ...entries[index], [field]: sanitized };
      updatePreProgrammed({ entries });
    },
    [state.preProgrammed.entries, updatePreProgrammed],
  );

  const perp = state.perpetual;

  return (
    <div className="space-y-6" data-testid="distribution-step">
      {/* ── Perpetual Distribution ── */}
      <div className="space-y-4">
        <div className="flex items-center gap-2">
          <Checkbox
            id="enable-perpetual"
            data-testid="enable-perpetual"
            checked={perp.enabled}
            onCheckedChange={(checked) => {
              updatePerpetual({
                enabled: checked === true,
                ...(checked === true && !perp.enabled ? { intervalType: "timeBased" } : {}),
              });
            }}
          />
          <Label htmlFor="enable-perpetual" className="cursor-pointer font-medium">
            Enable Perpetual Distribution
          </Label>
          <InfoTooltip text="Perpetual distributions emit tokens on a recurring schedule (every N blocks, time units, or epochs)." />
        </div>

        {perp.enabled && (
          <div className="ml-6 space-y-4 border-l-2 border-border pl-4">
            {/* Interval Type */}
            <div className="space-y-2">
              <Label className="text-sm">Interval Type</Label>
              <Select
                value={perp.intervalType}
                onValueChange={(val) => updatePerpetual({ intervalType: val as DistributionIntervalType })}
              >
                <SelectTrigger className="w-48" data-testid="interval-type-select">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {INTERVAL_TYPE_OPTIONS.map((opt) => (
                    <SelectItem key={opt.value} value={opt.value}>
                      {opt.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            {/* Interval amount + unit */}
            <div className="space-y-2">
              <Label className="text-sm">
                {perp.intervalType === "timeBased"
                  ? "Distributes every"
                  : perp.intervalType === "epochBased"
                    ? "Epoch Interval"
                    : "Block Interval"}
              </Label>
              <div className="flex items-center gap-2">
                <Input
                  value={perp.intervalAmount}
                  onChange={(e) => updatePerpetual({ intervalAmount: sanitizeU64(e.target.value) })}
                  placeholder="e.g. 1"
                  className="w-24 h-8 text-sm"
                  inputMode="numeric"
                  data-testid="interval-amount"
                />
                {perp.intervalType === "timeBased" && (
                  <Select
                    value={perp.intervalTimeUnit}
                    onValueChange={(val) => updatePerpetual({ intervalTimeUnit: val as IntervalTimeUnit })}
                  >
                    <SelectTrigger className="w-32" data-testid="time-unit-select">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {TIME_UNIT_OPTIONS.map((opt) => (
                        <SelectItem key={opt.value} value={opt.value}>
                          {getTimeUnitLabel(opt.value, perp.intervalAmount)}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                )}
                {perp.intervalType === "epochBased" && (
                  <span className="text-sm text-muted-foreground">epochs</span>
                )}
                {perp.intervalType === "blockBased" && (
                  <span className="text-sm text-muted-foreground">blocks</span>
                )}
              </div>
            </div>

            <Separator />

            {/* Distribution Function */}
            <div className="space-y-2">
              <div className="flex items-center gap-2">
                <Label className="text-sm">Distribution Function</Label>
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-6 w-6 p-0"
                  onClick={() => setInfoDialogOpen(true)}
                  data-testid="function-info-button"
                  aria-label="Distribution function info"
                >
                  <Info className="h-3.5 w-3.5" />
                </Button>
              </div>
              <Select
                value={perp.distributionFunction}
                onValueChange={(val) => updatePerpetual({ distributionFunction: val as DistributionFunction })}
              >
                <SelectTrigger className="w-64" data-testid="function-select">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {DISTRIBUTION_FUNCTION_OPTIONS.map((opt) => (
                    <SelectItem key={opt.value} value={opt.value}>
                      {opt.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>

              {/* Formula visualization */}
              <FormulaVisualization fn={perp.distributionFunction} />
            </div>

            {/* Function-specific parameters */}
            <div className="space-y-3" data-testid="function-params">
              {perp.distributionFunction === "fixedAmount" && (
                <ParamField
                  label="Fixed Amount per Interval"
                  value={perp.fixedAmount}
                  onChange={(val) => updatePerpetual({ fixedAmount: val })}
                  sanitize="u64"
                  testId="param-fixed-amount"
                />
              )}

              {perp.distributionFunction === "stepDecreasing" && (
                <StepDecreasingParams state={perp} onUpdate={updatePerpetual} />
              )}

              {perp.distributionFunction === "stepwise" && (
                <StepwiseParams
                  entries={perp.stepwiseEntries}
                  onAdd={addStepwiseEntry}
                  onRemove={removeStepwiseEntry}
                  onUpdate={updateStepwiseEntry}
                />
              )}

              {perp.distributionFunction === "linear" && (
                <LinearParams state={perp} onUpdate={updatePerpetual} />
              )}

              {perp.distributionFunction === "polynomial" && (
                <PolynomialParams state={perp} onUpdate={updatePerpetual} />
              )}

              {perp.distributionFunction === "exponential" && (
                <ExponentialParams state={perp} onUpdate={updatePerpetual} />
              )}

              {perp.distributionFunction === "logarithmic" && (
                <LogarithmicParams state={perp} onUpdate={updatePerpetual} />
              )}

              {perp.distributionFunction === "invertedLogarithmic" && (
                <InvertedLogarithmicParams state={perp} onUpdate={updatePerpetual} />
              )}
            </div>

            <Separator />

            {/* Recipient */}
            <div className="space-y-2">
              <Label className="text-sm">Recipient</Label>
              <div className="flex items-center gap-2">
                <Select
                  value={perp.recipient}
                  onValueChange={(val) => updatePerpetual({ recipient: val as DistributionRecipient })}
                >
                  <SelectTrigger className="w-52" data-testid="recipient-select">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {RECIPIENT_OPTIONS.map((opt) => (
                      <SelectItem key={opt.value} value={opt.value}>
                        {opt.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              {perp.recipient === "identity" && (
                <Input
                  value={perp.recipientIdentityId}
                  onChange={(e) => updatePerpetual({ recipientIdentityId: e.target.value })}
                  placeholder="Enter base58 identity ID"
                  className="h-8 text-sm font-mono"
                  data-testid="recipient-identity-input"
                />
              )}
            </div>
          </div>
        )}
      </div>

      <Separator />

      {/* ── Pre-Programmed Distribution ── */}
      <div className="space-y-4">
        <div className="flex items-center gap-2">
          <Checkbox
            id="enable-preprogrammed"
            data-testid="enable-preprogrammed"
            checked={state.preProgrammed.enabled}
            onCheckedChange={(checked) => updatePreProgrammed({ enabled: checked === true })}
          />
          <Label htmlFor="enable-preprogrammed" className="cursor-pointer font-medium">
            Enable Pre-Programmed Distribution
          </Label>
          <InfoTooltip text="Pre-programmed distributions are one-time emissions at specific future time offsets after the token is registered." />
        </div>

        {state.preProgrammed.enabled && (
          <div className="ml-6 space-y-3 border-l-2 border-border pl-4">
            {state.preProgrammed.entries.map((entry, index) => (
              <div
                key={index}
                className="flex flex-wrap items-end gap-2 rounded-lg border bg-card p-3"
                data-testid={`preprogrammed-entry-${index}`}
              >
                <div className="space-y-1">
                  <Label className="text-xs">Days</Label>
                  <Input
                    value={entry.days}
                    onChange={(e) => updatePreProgrammedEntry(index, "days", e.target.value)}
                    className="w-20 h-8 text-sm"
                    inputMode="numeric"
                    data-testid={`preprogrammed-days-${index}`}
                  />
                </div>
                <div className="space-y-1">
                  <Label className="text-xs">Hours</Label>
                  <Input
                    value={entry.hours}
                    onChange={(e) => updatePreProgrammedEntry(index, "hours", e.target.value)}
                    className="w-20 h-8 text-sm"
                    inputMode="numeric"
                    data-testid={`preprogrammed-hours-${index}`}
                  />
                </div>
                <div className="space-y-1">
                  <Label className="text-xs">Minutes</Label>
                  <Input
                    value={entry.minutes}
                    onChange={(e) => updatePreProgrammedEntry(index, "minutes", e.target.value)}
                    className="w-20 h-8 text-sm"
                    inputMode="numeric"
                    data-testid={`preprogrammed-minutes-${index}`}
                  />
                </div>
                <div className="space-y-1 flex-1 min-w-[180px]">
                  <Label className="text-xs">Identity (base58)</Label>
                  <Input
                    value={entry.identityId}
                    onChange={(e) => updatePreProgrammedEntry(index, "identityId", e.target.value)}
                    placeholder="Enter base58 identity"
                    className="h-8 text-sm font-mono"
                    data-testid={`preprogrammed-identity-${index}`}
                  />
                </div>
                <div className="space-y-1">
                  <Label className="text-xs">Amount</Label>
                  <Input
                    value={entry.amount}
                    onChange={(e) => updatePreProgrammedEntry(index, "amount", e.target.value)}
                    placeholder="Token amount"
                    className="w-28 h-8 text-sm"
                    inputMode="numeric"
                    data-testid={`preprogrammed-amount-${index}`}
                  />
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-8 w-8 p-0 text-destructive hover:text-destructive"
                  onClick={() => removePreProgrammedEntry(index)}
                  aria-label={`Remove entry ${index + 1}`}
                  data-testid={`preprogrammed-remove-${index}`}
                >
                  <Trash2 className="h-4 w-4" />
                </Button>
              </div>
            ))}

            <Button
              variant="outline"
              size="sm"
              onClick={addPreProgrammedEntry}
              data-testid="add-preprogrammed-entry"
            >
              <Plus className="h-4 w-4 mr-1" />
              Add Distribution Entry
            </Button>
          </div>
        )}
      </div>

      {/* ── Distribution Function Info Dialog ── */}
      <Dialog open={infoDialogOpen} onOpenChange={setInfoDialogOpen}>
        <DialogContent className="max-w-lg max-h-[80vh] overflow-y-auto" data-testid="function-info-dialog">
          <DialogHeader>
            <DialogTitle>Distribution Function Types</DialogTitle>
            <DialogDescription>Overview of available distribution function formulas and their use cases.</DialogDescription>
          </DialogHeader>
          <div className="space-y-4 text-sm">
            {DISTRIBUTION_INFO_TEXT.split("\n\n").map((block, i) => {
              const boldMatch = block.match(/^\*\*(.+?)\*\* — (.+)$/);
              if (boldMatch) {
                return (
                  <div key={i}>
                    <p>
                      <strong>{boldMatch[1]}</strong> — {boldMatch[2]}
                    </p>
                  </div>
                );
              }
              return <p key={i}>{block}</p>;
            })}
          </div>
          <Button variant="outline" onClick={() => setInfoDialogOpen(false)} className="mt-2">
            <X className="h-4 w-4 mr-1" />
            Close
          </Button>
        </DialogContent>
      </Dialog>
    </div>
  );
}

// ─── Function-specific parameter components ────────────────────────────────

interface FunctionParamsProps {
  state: PerpetualDistributionState;
  onUpdate: (updates: Partial<PerpetualDistributionState>) => void;
}

function StepDecreasingParams({ state, onUpdate }: FunctionParamsProps) {
  return (
    <div className="space-y-2" data-testid="step-decreasing-params">
      <ParamField
        label="Step Count"
        value={state.stepCount}
        onChange={(val) => onUpdate({ stepCount: val })}
        sanitize="u64"
        testId="param-step-count"
      />
      <ParamField
        label="Decrease Numerator (n < 65,536)"
        value={state.decreaseNumerator}
        onChange={(val) => onUpdate({ decreaseNumerator: val })}
        sanitize="u64"
        testId="param-decrease-numerator"
      />
      <ParamField
        label="Decrease Denominator (d < 65,536)"
        value={state.decreaseDenominator}
        onChange={(val) => onUpdate({ decreaseDenominator: val })}
        sanitize="u64"
        testId="param-decrease-denominator"
      />
      <ParamField
        label="Start Period Offset (optional)"
        value={state.startPeriodOffset}
        onChange={(val) => onUpdate({ startPeriodOffset: val })}
        sanitize="i64"
        placeholder="None"
        testId="param-start-period-offset"
      />
      <ParamField
        label="Initial Token Emission"
        value={state.initialEmission}
        onChange={(val) => onUpdate({ initialEmission: val })}
        sanitize="u64"
        testId="param-initial-emission"
      />
      <ParamField
        label="Minimum Emission Value (optional)"
        value={state.stepDecreasingMinValue}
        onChange={(val) => onUpdate({ stepDecreasingMinValue: val })}
        sanitize="u64"
        placeholder="None"
        testId="param-step-min-value"
      />
      <ParamField
        label="Max Interval Count (optional)"
        value={state.stepDecreasingMaxIntervalCount}
        onChange={(val) => onUpdate({ stepDecreasingMaxIntervalCount: val })}
        sanitize="u64"
        placeholder="None"
        testId="param-max-interval-count"
      />
      <ParamField
        label="Trailing Distribution Amount"
        value={state.trailingAmount}
        onChange={(val) => onUpdate({ trailingAmount: val })}
        sanitize="u64"
        testId="param-trailing-amount"
      />
    </div>
  );
}

interface StepwiseParamsProps {
  entries: StepwiseEntry[];
  onAdd: () => void;
  onRemove: (index: number) => void;
  onUpdate: (index: number, field: keyof StepwiseEntry, value: string) => void;
}

function StepwiseParams({ entries, onAdd, onRemove, onUpdate }: StepwiseParamsProps) {
  return (
    <div className="space-y-2" data-testid="stepwise-params">
      {entries.map((entry, index) => (
        <div key={index} className="flex items-center gap-2" data-testid={`stepwise-entry-${index}`}>
          <span className="text-xs text-muted-foreground shrink-0">#{index}</span>
          <div className="flex items-center gap-1">
            <Label className="text-xs shrink-0">Start Step:</Label>
            <Input
              value={entry.startStep}
              onChange={(e) => onUpdate(index, "startStep", e.target.value)}
              className="w-20 h-7 text-xs"
              inputMode="numeric"
              data-testid={`stepwise-start-${index}`}
            />
          </div>
          <div className="flex items-center gap-1">
            <Label className="text-xs shrink-0">Amount:</Label>
            <Input
              value={entry.amount}
              onChange={(e) => onUpdate(index, "amount", e.target.value)}
              className="w-20 h-7 text-xs"
              inputMode="numeric"
              data-testid={`stepwise-amount-${index}`}
            />
          </div>
          <Button
            variant="ghost"
            size="sm"
            className="h-7 w-7 p-0 text-destructive hover:text-destructive"
            onClick={() => onRemove(index)}
            aria-label={`Remove step ${index}`}
            data-testid={`stepwise-remove-${index}`}
          >
            <Trash2 className="h-3.5 w-3.5" />
          </Button>
        </div>
      ))}
      <Button variant="outline" size="sm" onClick={onAdd} data-testid="add-stepwise-entry">
        <Plus className="h-3.5 w-3.5 mr-1" />
        Add Step
      </Button>
    </div>
  );
}

function LinearParams({ state, onUpdate }: FunctionParamsProps) {
  return (
    <div className="space-y-2" data-testid="linear-params">
      <ParamField
        label="Slope Numerator (a, -255 ≤ a ≤ 256)"
        value={state.linearA}
        onChange={(val) => onUpdate({ linearA: val })}
        sanitize="i64"
        testId="param-linear-a"
      />
      <ParamField
        label="Slope Divisor (d)"
        value={state.linearD}
        onChange={(val) => onUpdate({ linearD: val })}
        sanitize="u64"
        testId="param-linear-d"
      />
      <ParamField
        label="Start Step (optional)"
        value={state.linearStartStep}
        onChange={(val) => onUpdate({ linearStartStep: val })}
        sanitize="i64"
        placeholder="None"
        testId="param-linear-start-step"
      />
      <ParamField
        label="Starting Amount (b)"
        value={state.linearStartingAmount}
        onChange={(val) => onUpdate({ linearStartingAmount: val })}
        sanitize="i64"
        testId="param-linear-starting-amount"
      />
      <ParamField
        label="Min Emission Value (optional)"
        value={state.linearMinValue}
        onChange={(val) => onUpdate({ linearMinValue: val })}
        sanitize="u64"
        placeholder="None"
        testId="param-linear-min-value"
      />
      <ParamField
        label="Max Emission Value (optional)"
        value={state.linearMaxValue}
        onChange={(val) => onUpdate({ linearMaxValue: val })}
        sanitize="u64"
        placeholder="None"
        testId="param-linear-max-value"
      />
    </div>
  );
}

function PolynomialParams({ state, onUpdate }: FunctionParamsProps) {
  return (
    <div className="space-y-2" data-testid="polynomial-params">
      <ParamField
        label="Scaling Factor (a, -255 ≤ a ≤ 256)"
        value={state.polyA}
        onChange={(val) => onUpdate({ polyA: val })}
        sanitize="i64"
        testId="param-poly-a"
      />
      <ParamField
        label="Exponent Numerator (m, -8 ≤ m ≤ 8)"
        value={state.polyM}
        onChange={(val) => onUpdate({ polyM: val })}
        sanitize="i64"
        testId="param-poly-m"
      />
      <ParamField
        label="Exponent Denominator (n, 0 < n ≤ 32)"
        value={state.polyN}
        onChange={(val) => onUpdate({ polyN: val })}
        sanitize="u64"
        testId="param-poly-n"
      />
      <ParamField
        label="Divisor (d)"
        value={state.polyD}
        onChange={(val) => onUpdate({ polyD: val })}
        sanitize="u64"
        testId="param-poly-d"
      />
      <ParamField
        label="Start Period Offset (optional)"
        value={state.polyS}
        onChange={(val) => onUpdate({ polyS: val })}
        sanitize="u64"
        placeholder="None"
        testId="param-poly-s"
      />
      <ParamField
        label="Offset (o)"
        value={state.polyO}
        onChange={(val) => onUpdate({ polyO: val })}
        sanitize="i64"
        testId="param-poly-o"
      />
      <ParamField
        label="Initial Token Emission (b)"
        value={state.polyB}
        onChange={(val) => onUpdate({ polyB: val })}
        sanitize="u64"
        testId="param-poly-b"
      />
      <ParamField
        label="Min Emission Value (optional)"
        value={state.polyMinValue}
        onChange={(val) => onUpdate({ polyMinValue: val })}
        sanitize="u64"
        placeholder="None"
        testId="param-poly-min-value"
      />
      <ParamField
        label="Max Emission Value (optional)"
        value={state.polyMaxValue}
        onChange={(val) => onUpdate({ polyMaxValue: val })}
        sanitize="u64"
        placeholder="None"
        testId="param-poly-max-value"
      />
    </div>
  );
}

function ExponentialParams({ state, onUpdate }: FunctionParamsProps) {
  return (
    <div className="space-y-2" data-testid="exponential-params">
      <ParamField
        label="Scaling Factor (a, 0 < a ≤ 256)"
        value={state.expA}
        onChange={(val) => onUpdate({ expA: val })}
        sanitize="u64"
        testId="param-exp-a"
      />
      <ParamField
        label="Exponent Rate Numerator (m, -8 ≤ m ≤ 8, m ≠ 0)"
        value={state.expM}
        onChange={(val) => onUpdate({ expM: val })}
        sanitize="i64"
        testId="param-exp-m"
      />
      <ParamField
        label="Exponent Rate Denominator (n, 0 < n ≤ 32)"
        value={state.expN}
        onChange={(val) => onUpdate({ expN: val })}
        sanitize="u64"
        testId="param-exp-n"
      />
      <ParamField
        label="Divisor (d, d ≠ 0)"
        value={state.expD}
        onChange={(val) => onUpdate({ expD: val })}
        sanitize="u64"
        testId="param-exp-d"
      />
      <ParamField
        label="Start Period Offset (optional)"
        value={state.expS}
        onChange={(val) => onUpdate({ expS: val })}
        sanitize="u64"
        placeholder="None"
        testId="param-exp-s"
      />
      <ParamField
        label="Offset (o)"
        value={state.expO}
        onChange={(val) => onUpdate({ expO: val })}
        sanitize="i64"
        testId="param-exp-o"
      />
      <ParamField
        label="Base Amount (b)"
        value={state.expB}
        onChange={(val) => onUpdate({ expB: val })}
        sanitize="i64"
        testId="param-exp-b"
      />
      <ParamField
        label="Min Emission Value (optional)"
        value={state.expMinValue}
        onChange={(val) => onUpdate({ expMinValue: val })}
        sanitize="u64"
        placeholder="None"
        testId="param-exp-min-value"
      />
      <ParamField
        label="Max Emission Value (optional)"
        value={state.expMaxValue}
        onChange={(val) => onUpdate({ expMaxValue: val })}
        sanitize="u64"
        placeholder="None"
        testId="param-exp-max-value"
      />
    </div>
  );
}

function LogarithmicParams({ state, onUpdate }: FunctionParamsProps) {
  return (
    <div className="space-y-2" data-testid="logarithmic-params">
      <ParamField
        label="Scaling Factor (a, -32,766 ≤ a ≤ 32,767)"
        value={state.logA}
        onChange={(val) => onUpdate({ logA: val })}
        sanitize="i64"
        testId="param-log-a"
      />
      <ParamField
        label="Divisor (d)"
        value={state.logD}
        onChange={(val) => onUpdate({ logD: val })}
        sanitize="u64"
        testId="param-log-d"
      />
      <ParamField
        label="Exponent Numerator (m)"
        value={state.logM}
        onChange={(val) => onUpdate({ logM: val })}
        sanitize="u64"
        testId="param-log-m"
      />
      <ParamField
        label="Exponent Denominator (n)"
        value={state.logN}
        onChange={(val) => onUpdate({ logN: val })}
        sanitize="u64"
        testId="param-log-n"
      />
      <ParamField
        label="Start Period Offset (optional)"
        value={state.logS}
        onChange={(val) => onUpdate({ logS: val })}
        sanitize="u64"
        placeholder="None"
        testId="param-log-s"
      />
      <ParamField
        label="Offset (o)"
        value={state.logO}
        onChange={(val) => onUpdate({ logO: val })}
        sanitize="i64"
        testId="param-log-o"
      />
      <ParamField
        label="Base Amount (b)"
        value={state.logB}
        onChange={(val) => onUpdate({ logB: val })}
        sanitize="u64"
        testId="param-log-b"
      />
      <ParamField
        label="Min Emission Value (optional)"
        value={state.logMinValue}
        onChange={(val) => onUpdate({ logMinValue: val })}
        sanitize="u64"
        placeholder="None"
        testId="param-log-min-value"
      />
      <ParamField
        label="Max Emission Value (optional)"
        value={state.logMaxValue}
        onChange={(val) => onUpdate({ logMaxValue: val })}
        sanitize="u64"
        placeholder="None"
        testId="param-log-max-value"
      />
    </div>
  );
}

function InvertedLogarithmicParams({ state, onUpdate }: FunctionParamsProps) {
  return (
    <div className="space-y-2" data-testid="inverted-logarithmic-params">
      <ParamField
        label="Scaling Factor (a, -32,766 ≤ a ≤ 32,767)"
        value={state.invLogA}
        onChange={(val) => onUpdate({ invLogA: val })}
        sanitize="i64"
        testId="param-inv-log-a"
      />
      <ParamField
        label="Divisor (d)"
        value={state.invLogD}
        onChange={(val) => onUpdate({ invLogD: val })}
        sanitize="u64"
        testId="param-inv-log-d"
      />
      <ParamField
        label="Exponent Numerator (m)"
        value={state.invLogM}
        onChange={(val) => onUpdate({ invLogM: val })}
        sanitize="u64"
        testId="param-inv-log-m"
      />
      <ParamField
        label="Exponent Denominator (n)"
        value={state.invLogN}
        onChange={(val) => onUpdate({ invLogN: val })}
        sanitize="u64"
        testId="param-inv-log-n"
      />
      <ParamField
        label="Start Period Offset (optional)"
        value={state.invLogS}
        onChange={(val) => onUpdate({ invLogS: val })}
        sanitize="u64"
        placeholder="None"
        testId="param-inv-log-s"
      />
      <ParamField
        label="Offset (o)"
        value={state.invLogO}
        onChange={(val) => onUpdate({ invLogO: val })}
        sanitize="i64"
        testId="param-inv-log-o"
      />
      <ParamField
        label="Base Amount (b)"
        value={state.invLogB}
        onChange={(val) => onUpdate({ invLogB: val })}
        sanitize="u64"
        testId="param-inv-log-b"
      />
      <ParamField
        label="Min Emission Value (optional)"
        value={state.invLogMinValue}
        onChange={(val) => onUpdate({ invLogMinValue: val })}
        sanitize="u64"
        placeholder="None"
        testId="param-inv-log-min-value"
      />
      <ParamField
        label="Max Emission Value (optional)"
        value={state.invLogMaxValue}
        onChange={(val) => onUpdate({ invLogMaxValue: val })}
        sanitize="u64"
        placeholder="None"
        testId="param-inv-log-max-value"
      />
    </div>
  );
}
