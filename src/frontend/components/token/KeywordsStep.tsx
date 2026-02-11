import { useCallback } from "react";
import { Plus, X, Info } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";

// ─── Types ──────────────────────────────────────────────────────────────────

export interface KeywordsState {
  keywords: string[];
  currentInput: string;
}

// ─── Defaults ───────────────────────────────────────────────────────────────

export function createDefaultKeywordsState(): KeywordsState {
  return {
    keywords: [],
    currentInput: "",
  };
}

// ─── Validation ─────────────────────────────────────────────────────────────

export interface KeywordsValidation {
  valid: boolean;
  errors: Record<string, string>;
}

export function validateKeywords(state: KeywordsState): KeywordsValidation {
  const errors: Record<string, string> = {};

  // Validate each keyword
  for (let i = 0; i < state.keywords.length; i++) {
    const keyword = state.keywords[i]?.trim() ?? "";
    if (keyword.length < 3) {
      errors[`keyword_${i}`] = "Keyword must be at least 3 characters";
    } else if (keyword.length > 50) {
      errors[`keyword_${i}`] = "Keyword must be at most 50 characters";
    }
  }

  // Duplicate check (case-insensitive)
  const seen = new Set<string>();
  for (let i = 0; i < state.keywords.length; i++) {
    const lower = state.keywords[i]?.trim().toLowerCase() ?? "";
    if (lower && seen.has(lower)) {
      errors[`keyword_${i}_dup`] = `Duplicate keyword: "${state.keywords[i] ?? ""}"`;
    }
    if (lower) seen.add(lower);
  }

  return {
    valid: Object.keys(errors).length === 0,
    errors,
  };
}

/** Validate a single keyword input before adding it */
export function validateSingleKeyword(keyword: string, existing: string[]): string | null {
  const trimmed = keyword.trim();
  if (!trimmed) return "Keyword cannot be empty";
  if (trimmed.length < 3) return "Keyword must be at least 3 characters";
  if (trimmed.length > 50) return "Keyword must be at most 50 characters";
  if (existing.some((k) => k.trim().toLowerCase() === trimmed.toLowerCase())) {
    return `Duplicate keyword: "${trimmed}"`;
  }
  return null;
}

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

// ─── KeywordsStep ───────────────────────────────────────────────────────────

export interface KeywordsStepProps {
  state: KeywordsState;
  onChange: (state: KeywordsState) => void;
}

export function KeywordsStep({ state, onChange }: KeywordsStepProps) {
  const validation = validateKeywords(state);

  const updateInput = useCallback(
    (value: string) => {
      onChange({ ...state, currentInput: value });
    },
    [state, onChange],
  );

  const addKeyword = useCallback(() => {
    const trimmed = state.currentInput.trim();
    if (!trimmed) return;

    // Support comma-separated input
    const parts = trimmed.split(",").map((s) => s.trim()).filter(Boolean);
    const newKeywords = [...state.keywords];
    for (const part of parts) {
      const err = validateSingleKeyword(part, newKeywords);
      if (!err) {
        newKeywords.push(part);
      }
    }
    onChange({ ...state, keywords: newKeywords, currentInput: "" });
  }, [state, onChange]);

  const removeKeyword = useCallback(
    (index: number) => {
      onChange({
        ...state,
        keywords: state.keywords.filter((_, i) => i !== index),
      });
    },
    [state, onChange],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLInputElement>) => {
      if (e.key === "Enter") {
        e.preventDefault();
        addKeyword();
      }
    },
    [addKeyword],
  );

  // Compute the error for the current input (for real-time feedback)
  const inputError = state.currentInput.trim()
    ? validateSingleKeyword(state.currentInput.trim(), state.keywords)
    : null;

  // Estimate cost
  const keywordCost = state.keywords.length * 0.1;

  return (
    <div className="space-y-4" data-testid="keywords-step">
      <p className="text-sm text-muted-foreground">
        Add searchable keywords to help others discover your token.
        Keywords are optional but help with token discoverability.
      </p>

      {/* Cost info */}
      {state.keywords.length > 0 && (
        <div className="flex items-center gap-1.5 text-xs text-muted-foreground bg-muted/50 rounded px-3 py-2" data-testid="keywords-cost">
          <Info className="h-3.5 w-3.5 shrink-0" />
          <span>
            {state.keywords.length} keyword{state.keywords.length !== 1 ? "s" : ""} — estimated cost:{" "}
            <span className="font-medium text-foreground">{keywordCost.toFixed(1)} Dash</span>
          </span>
        </div>
      )}

      {/* Keyword input */}
      <div className="space-y-1">
        <div className="flex items-center gap-1">
          <Label htmlFor="keyword-input">Add Keyword</Label>
          <InfoTooltip text="Each searchable keyword costs 0.1 Dash. Keywords must be between 3 and 50 characters. You can enter multiple keywords separated by commas." />
        </div>
        <div className="flex gap-2">
          <Input
            id="keyword-input"
            data-testid="keyword-input"
            value={state.currentInput}
            onChange={(e) => updateInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="e.g. gaming, reward, loyalty"
          />
          <Button
            variant="outline"
            onClick={addKeyword}
            disabled={!state.currentInput.trim() || !!inputError}
            data-testid="add-keyword"
          >
            <Plus className="h-4 w-4 mr-1" />
            Add
          </Button>
        </div>
        {inputError && state.currentInput.trim() && (
          <p className="text-xs text-destructive" data-testid="keyword-input-error">
            {inputError}
          </p>
        )}
      </div>

      {/* Keywords list */}
      {state.keywords.length > 0 && (
        <div className="space-y-2">
          <Label>Keywords ({state.keywords.length})</Label>
          <div className="flex flex-wrap gap-2" data-testid="keywords-list">
            {state.keywords.map((keyword, index) => {
              const hasError =
                validation.errors[`keyword_${index}`] ||
                validation.errors[`keyword_${index}_dup`];
              return (
                <Badge
                  key={index}
                  variant={hasError ? "destructive" : "secondary"}
                  className="flex items-center gap-1 pl-3 pr-1 py-1"
                  data-testid={`keyword-badge-${index}`}
                >
                  <span className="text-sm">{keyword}</span>
                  <button
                    type="button"
                    onClick={() => removeKeyword(index)}
                    className="inline-flex items-center justify-center rounded-full p-0.5 hover:bg-foreground/10 transition-colors"
                    aria-label={`Remove keyword "${keyword}"`}
                    data-testid={`keyword-remove-${index}`}
                  >
                    <X className="h-3 w-3" />
                  </button>
                </Badge>
              );
            })}
          </div>
          {/* Show aggregate errors */}
          {Object.entries(validation.errors).map(([key, msg]) => (
            <p key={key} className="text-xs text-destructive">
              {msg}
            </p>
          ))}
        </div>
      )}

      {/* Empty state */}
      {state.keywords.length === 0 && (
        <div
          className="rounded-lg border border-dashed p-6 text-center text-muted-foreground"
          data-testid="keywords-empty-state"
        >
          <p className="text-sm">No keywords added yet.</p>
          <p className="text-xs mt-1">
            Keywords are optional. Each keyword costs 0.1 Dash and helps others find your token.
          </p>
        </div>
      )}
    </div>
  );
}
