import { useCallback, useId, useMemo, useState } from "react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";

interface AmountInputProps {
  /** Current string value of the input */
  value: string;
  /** Called when the input value changes */
  onChange: (value: string) => void;
  /** Label text */
  label?: string;
  /** Placeholder / hint text */
  placeholder?: string;
  /** Number of decimal places (e.g. 8 for DASH, 0 for credits) */
  decimalPlaces?: number;
  /** Unit name for display in errors (e.g. "DASH", "credits") */
  unitName?: string;
  /** Maximum allowed amount (in smallest unit, e.g. duffs). null = no max. */
  maxAmount?: number | null;
  /** Hint text explaining why max is limited (e.g. "fees reserved") */
  maxExceededHint?: string;
  /** Minimum allowed amount (in smallest unit). null = no min. Default: 1 */
  minAmount?: number | null;
  /** Show "Max" button */
  showMaxButton?: boolean;
  /** Called when "Max" button is clicked */
  onMaxClick?: () => void;
  /** Whether to show validation errors inline */
  showValidationErrors?: boolean;
  /** Additional CSS class */
  className?: string;
  /** Disable the input */
  disabled?: boolean;
}

/**
 * Parse a decimal string with given decimal places into smallest unit integer.
 * e.g. "1.5" with 8 decimals = 150000000
 */
function parseAmount(
  value: string,
  decimalPlaces: number,
): number | null {
  if (!value.trim()) return null;

  const trimmed = value.trim();
  // Allow numbers and optional single decimal point
  if (!/^\d*\.?\d*$/.test(trimmed)) return null;
  if (trimmed === ".") return null;

  const parts = trimmed.split(".");
  const intPart = parts[0] || "0";
  const decPart = parts[1] || "";

  // Too many decimal places
  if (decPart.length > decimalPlaces) return null;

  // Pad decimal part to full precision
  const paddedDec = decPart.padEnd(decimalPlaces, "0");
  const result = parseInt(intPart + paddedDec, 10);
  return isNaN(result) ? null : result;
}

/**
 * Format an amount in smallest units to decimal string.
 */
export function formatAmount(amount: number, decimalPlaces: number): string {
  if (decimalPlaces === 0) return amount.toString();
  const divisor = Math.pow(10, decimalPlaces);
  return (amount / divisor).toFixed(decimalPlaces);
}

/** 1 DASH = 100,000,000,000 Platform credits */
export const CREDITS_PER_DASH = 100_000_000_000;

/** 1 duff = 1,000 Platform credits */
export const CREDITS_PER_DUFF = 1000;

/** Format a Platform credits amount as a DASH decimal string (8 places). */
export function formatCreditsAsDash(credits: number): string {
  return (credits / CREDITS_PER_DASH).toFixed(8);
}

/**
 * Validate an amount value and return an error message if invalid.
 */
function validateAmount(
  value: string,
  decimalPlaces: number,
  minAmount: number | null,
  maxAmount: number | null,
  unitName: string,
  maxExceededHint?: string,
): string | null {
  if (!value.trim()) return null; // Empty is valid (no amount entered)

  const parsed = parseAmount(value, decimalPlaces);
  if (parsed === null) {
    return "Invalid amount format";
  }

  if (maxAmount !== null && parsed > maxAmount) {
    const maxFormatted = formatAmount(maxAmount, decimalPlaces);
    const msg = `Amount exceeds maximum ${maxFormatted} ${unitName}`;
    return maxExceededHint ? `${msg}. ${maxExceededHint}` : msg;
  }

  if (minAmount !== null && parsed < minAmount) {
    const minFormatted = formatAmount(minAmount, decimalPlaces);
    return `Amount must be at least ${minFormatted} ${unitName}`;
  }

  return null;
}

/**
 * Cryptocurrency amount input with decimal validation, min/max bounds,
 * and optional "Max" button.
 *
 * Matches egui AmountInput behavior:
 * - Decimal parsing with configurable precision
 * - Min/max validation with error messages
 * - Optional Max button to fill maximum amount
 * - Unit name in error messages
 */
export function AmountInput({
  value,
  onChange,
  label,
  placeholder,
  decimalPlaces = 8,
  unitName = "DASH",
  maxAmount = null,
  maxExceededHint,
  minAmount = 1,
  showMaxButton = false,
  onMaxClick,
  showValidationErrors = true,
  className,
  disabled = false,
}: AmountInputProps) {
  const id = useId();
  const [touched, setTouched] = useState(false);

  const error = useMemo(
    () =>
      touched
        ? validateAmount(
            value,
            decimalPlaces,
            minAmount,
            maxAmount,
            unitName,
            maxExceededHint,
          )
        : null,
    [
      value,
      decimalPlaces,
      minAmount,
      maxAmount,
      unitName,
      maxExceededHint,
      touched,
    ],
  );

  const handleChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const newValue = e.target.value;
      // Allow only digits and at most one decimal point
      if (newValue && !/^\d*\.?\d*$/.test(newValue)) return;
      onChange(newValue);
    },
    [onChange],
  );

  const handleBlur = useCallback(() => {
    setTouched(true);
  }, []);

  const hasError = showValidationErrors && !!error;

  return (
    <div className={cn("space-y-1.5", className)}>
      {label && (
        <Label htmlFor={id} className="text-sm font-medium">
          {label}
        </Label>
      )}
      <div className="flex items-center gap-2">
        <Input
          id={id}
          type="text"
          inputMode="decimal"
          value={value}
          onChange={handleChange}
          onBlur={handleBlur}
          placeholder={placeholder}
          aria-invalid={hasError}
          aria-describedby={hasError ? `${id}-error` : undefined}
          disabled={disabled}
        />
        {showMaxButton && (
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={onMaxClick}
            disabled={disabled}
            aria-label="Set maximum amount"
          >
            Max
          </Button>
        )}
      </div>
      {hasError && (
        <p id={`${id}-error`} className="text-sm text-destructive" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

/**
 * Hook to manage AmountInput state.
 * Returns the string value, onChange handler, and parsed amount.
 */
export function useAmountInput(decimalPlaces: number = 8) {
  const [value, setValue] = useState("");

  const parsedAmount = useMemo(
    () => parseAmount(value, decimalPlaces),
    [value, decimalPlaces],
  );

  const isValid = useMemo(() => {
    if (!value.trim()) return false;
    return parsedAmount !== null;
  }, [value, parsedAmount]);

  return { value, setValue, parsedAmount, isValid };
}
