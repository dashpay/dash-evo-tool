import { cn } from "@/lib/utils";

interface ProgressBarProps {
  /** Progress value between 0 and 100. */
  value: number;
  /** Maximum value (defaults to 100). */
  max?: number;
  /** Optional label shown above the bar. */
  label?: string;
  /** Show percentage text. Defaults to false. */
  showPercent?: boolean;
  /** Visual variant. Defaults to "primary". */
  variant?: "primary" | "success" | "warning" | "destructive";
  className?: string;
}

const variantClasses = {
  primary: "bg-primary",
  success: "bg-success",
  warning: "bg-warning",
  destructive: "bg-destructive",
} as const;

/**
 * Horizontal progress bar for known-duration operations (e.g. SPV sync).
 *
 * Design spec from 2.1 META:
 * - Progress bar for known-duration operations (SPV sync)
 */
export function ProgressBar({
  value,
  max = 100,
  label,
  showPercent = false,
  variant = "primary",
  className,
}: ProgressBarProps) {
  const percent = Math.min(100, Math.max(0, (value / max) * 100));

  return (
    <div className={cn("w-full", className)}>
      {(label || showPercent) && (
        <div className="mb-1.5 flex items-center justify-between text-sm">
          {label && (
            <span className="font-medium text-foreground">{label}</span>
          )}
          {showPercent && (
            <span className="text-muted-foreground">
              {Math.round(percent)}%
            </span>
          )}
        </div>
      )}
      <div
        className="h-2 w-full overflow-hidden rounded-full bg-muted"
        role="progressbar"
        aria-valuenow={Math.round(percent)}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label={label ?? "Progress"}
      >
        <div
          className={cn(
            "h-full rounded-full transition-[width] duration-300 ease-out",
            variantClasses[variant],
          )}
          style={{ width: `${percent}%` }}
        />
      </div>
    </div>
  );
}
