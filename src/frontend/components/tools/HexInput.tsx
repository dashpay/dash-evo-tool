import { useCallback, useMemo, useState } from "react";
import { cn } from "@/lib/utils";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";

/** Detected input format for hex/base64/CSV byte data. */
export type InputFormat = "hex" | "base64" | "csv" | "unknown";

interface HexInputProps {
  /** Current text value. */
  value: string;
  /** Called when the text value changes. */
  onChange: (value: string) => void;
  /** Optional label displayed above the textarea. */
  label?: string;
  /** Placeholder text. */
  placeholder?: string;
  /** Number of visible text rows. Defaults to 6. */
  rows?: number;
  /** Whether the input is disabled. */
  disabled?: boolean;
  /** Additional CSS class for the outer container. */
  className?: string;
  /** Optional error message displayed below the input. */
  error?: string;
}

/**
 * Detects the format of a byte-encoded string.
 *
 * Supports:
 * - **hex**: even-length string of 0-9a-fA-F characters (with optional 0x prefix, spaces/newlines)
 * - **base64**: valid base64 characters with optional padding
 * - **csv**: comma-separated integers (0-255)
 */
export function detectFormat(input: string): InputFormat {
  const trimmed = input.trim();
  if (!trimmed) return "unknown";

  // Strip 0x prefix and whitespace for hex detection
  const hexClean = trimmed.replace(/^0x/i, "").replace(/[\s\n\r]/g, "");
  if (/^[0-9a-fA-F]+$/.test(hexClean) && hexClean.length % 2 === 0 && hexClean.length > 0) {
    return "hex";
  }

  // CSV: comma-separated numbers 0-255
  if (/^(\d{1,3}\s*,\s*)*\d{1,3}$/.test(trimmed)) {
    const nums = trimmed.split(",").map((s) => parseInt(s.trim(), 10));
    if (nums.every((n) => n >= 0 && n <= 255)) {
      return "csv";
    }
  }

  // Base64
  if (/^[A-Za-z0-9+/\n\r]+=*$/.test(trimmed.replace(/[\s]/g, ""))) {
    return "base64";
  }

  return "unknown";
}

/**
 * Decode input bytes to hex string for display/processing.
 *
 * Returns null if the input cannot be decoded.
 */
export function decodeToHex(input: string, format: InputFormat): string | null {
  const trimmed = input.trim();
  if (!trimmed) return null;

  switch (format) {
    case "hex": {
      const hexClean = trimmed.replace(/^0x/i, "").replace(/[\s\n\r]/g, "");
      return hexClean.toLowerCase();
    }
    case "base64": {
      try {
        const binary = atob(trimmed.replace(/[\s\n\r]/g, ""));
        return Array.from(binary, (c) =>
          c.charCodeAt(0).toString(16).padStart(2, "0"),
        ).join("");
      } catch {
        return null;
      }
    }
    case "csv": {
      const nums = trimmed.split(",").map((s) => parseInt(s.trim(), 10));
      if (nums.some((n) => isNaN(n) || n < 0 || n > 255)) return null;
      return nums.map((n) => n.toString(16).padStart(2, "0")).join("");
    }
    default:
      return null;
  }
}

/**
 * Multiline textarea with auto-detected format (hex/base64/CSV) and byte decoding.
 *
 * Used by visualizer tools that accept raw byte data in multiple formats.
 * Shows a format badge indicating the detected input type.
 */
export function HexInput({
  value,
  onChange,
  label,
  placeholder = "Enter hex, base64, or comma-separated bytes...",
  rows = 6,
  disabled = false,
  className,
  error,
}: HexInputProps) {
  const [focused, setFocused] = useState(false);

  const format = useMemo(() => detectFormat(value), [value]);

  const handleChange = useCallback(
    (e: React.ChangeEvent<HTMLTextAreaElement>) => {
      onChange(e.target.value);
    },
    [onChange],
  );

  const formatLabel: Record<InputFormat, string> = {
    hex: "Hex",
    base64: "Base64",
    csv: "CSV",
    unknown: "Unknown",
  };

  const formatVariant: Record<InputFormat, "default" | "secondary" | "outline" | "destructive"> = {
    hex: "default",
    base64: "secondary",
    csv: "outline",
    unknown: "destructive",
  };

  return (
    <div className={cn("space-y-2", className)}>
      {/* Label + format badge */}
      <div className="flex items-center justify-between">
        {label && (
          <Label className="text-sm font-medium">{label}</Label>
        )}
        {value.trim() && (
          <Badge variant={formatVariant[format]} className="text-xs">
            {formatLabel[format]}
          </Badge>
        )}
      </div>

      {/* Textarea */}
      <textarea
        value={value}
        onChange={handleChange}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        placeholder={placeholder}
        rows={rows}
        disabled={disabled}
        spellCheck={false}
        autoComplete="off"
        autoCorrect="off"
        className={cn(
          "w-full rounded-md border bg-background px-3 py-2 font-mono text-sm",
          "resize-y transition-colors placeholder:text-muted-foreground",
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
          "disabled:cursor-not-allowed disabled:opacity-50",
          focused && "ring-2 ring-ring ring-offset-2",
          error && "border-destructive",
        )}
        aria-label={label || "Byte data input"}
        aria-invalid={!!error}
        aria-describedby={error ? "hex-input-error" : undefined}
      />

      {/* Error message */}
      {error && (
        <p id="hex-input-error" className="text-sm text-destructive" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
