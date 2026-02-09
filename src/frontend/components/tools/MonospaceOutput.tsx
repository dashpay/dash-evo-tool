import { cn } from "@/lib/utils";
import { CopyButton } from "@/components/shared/CopyButton";

interface MonospaceOutputProps {
  /** Text content to display. */
  value: string;
  /** Optional label displayed above the output. */
  label?: string;
  /** Maximum height in pixels before scrolling. Defaults to 400. */
  maxHeight?: number;
  /** Whether to show the copy button. Defaults to true. */
  showCopy?: boolean;
  /** Whether to wrap long lines. Defaults to true. */
  wrap?: boolean;
  /** Additional CSS class for the outer container. */
  className?: string;
}

/**
 * Scrollable, selectable monospace text area with optional copy button.
 *
 * Used by tool screens to display formatted output like deserialized data,
 * proof structures, and platform info results.
 */
export function MonospaceOutput({
  value,
  label,
  maxHeight = 400,
  showCopy = true,
  wrap = true,
  className,
}: MonospaceOutputProps) {
  return (
    <div className={cn("space-y-2", className)}>
      {/* Label + copy */}
      {(label || showCopy) && (
        <div className="flex items-center justify-between">
          {label && (
            <span className="text-sm font-medium text-foreground">
              {label}
            </span>
          )}
          {showCopy && value && (
            <CopyButton value={value} label="Copy" size="sm" />
          )}
        </div>
      )}

      {/* Output area */}
      <div
        className={cn(
          "rounded-md border bg-muted/30 p-3 font-mono text-sm",
          "overflow-auto select-text",
        )}
        style={{ maxHeight }}
        role="log"
        aria-label={label || "Output"}
        tabIndex={0}
      >
        {value ? (
          <pre
            className={cn(
              "m-0",
              wrap ? "whitespace-pre-wrap break-all" : "whitespace-pre",
            )}
          >
            {value}
          </pre>
        ) : (
          <span className="text-muted-foreground italic">No output</span>
        )}
      </div>
    </div>
  );
}
