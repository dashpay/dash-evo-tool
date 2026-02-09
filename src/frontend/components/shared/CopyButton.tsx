import { useCallback, useState } from "react";
import { Check, Copy } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

interface CopyButtonProps {
  /** The text to copy to clipboard */
  value: string;
  /** Optional label to display next to icon */
  label?: string;
  /** Button size variant */
  size?: "default" | "sm" | "xs" | "icon" | "icon-xs" | "icon-sm";
  /** Additional CSS class */
  className?: string;
}

/**
 * Copy-to-clipboard button with visual feedback.
 *
 * Shows a check icon for 2 seconds after successful copy.
 */
export function CopyButton({
  value,
  label,
  size = "icon-xs",
  className,
}: CopyButtonProps) {
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // Fallback for environments without clipboard API
    }
  }, [value]);

  return (
    <Button
      type="button"
      variant="ghost"
      size={size}
      onClick={handleCopy}
      className={cn("text-muted-foreground", className)}
      aria-label={copied ? "Copied" : "Copy to clipboard"}
    >
      {copied ? (
        <Check className="size-3.5 text-success" />
      ) : (
        <Copy className="size-3.5" />
      )}
      {label && <span className="ml-1">{label}</span>}
    </Button>
  );
}
