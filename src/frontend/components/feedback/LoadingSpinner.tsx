import { cn } from "@/lib/utils";
import { Loader2 } from "lucide-react";

interface LoadingSpinnerProps {
  /** Size of the spinner icon. Defaults to "md". */
  size?: "sm" | "md" | "lg";
  /** Optional label shown below the spinner. */
  label?: string;
  className?: string;
}

const sizeClasses = {
  sm: "h-4 w-4",
  md: "h-6 w-6",
  lg: "h-10 w-10",
} as const;

/**
 * Inline spinning indicator for async operations.
 *
 * Design spec from 2.1 META:
 * - Inline spinner for in-progress actions (button spinner)
 * - Uses Lucide Loader2 with CSS spin animation
 */
export function LoadingSpinner({
  size = "md",
  label,
  className,
}: LoadingSpinnerProps) {
  return (
    <div
      className={cn("inline-flex items-center gap-2", className)}
      role="status"
      aria-label={label ?? "Loading"}
    >
      <Loader2 className={cn("animate-spin text-muted-foreground", sizeClasses[size])} />
      {label && <span className="text-sm text-muted-foreground">{label}</span>}
    </div>
  );
}

interface LoadingOverlayProps {
  /** Whether the overlay is visible. */
  visible: boolean;
  /** Optional label shown below the spinner. */
  label?: string;
  className?: string;
}

/**
 * Full-area overlay spinner for blocking loading states.
 *
 * Design spec from 2.1 META:
 * - Full-page spinner for app initialization
 * - Semi-transparent backdrop
 */
export function LoadingOverlay({
  visible,
  label,
  className,
}: LoadingOverlayProps) {
  if (!visible) return null;

  return (
    <div
      className={cn(
        "absolute inset-0 z-50 flex flex-col items-center justify-center gap-3 bg-background/80 backdrop-blur-sm",
        className,
      )}
      role="status"
      aria-label={label ?? "Loading"}
    >
      <Loader2 className="h-10 w-10 animate-spin text-primary" />
      {label && (
        <p className="text-sm font-medium text-muted-foreground">{label}</p>
      )}
    </div>
  );
}
