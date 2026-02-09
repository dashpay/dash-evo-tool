import { cn } from "@/lib/utils";
import type { LucideIcon } from "lucide-react";
import { Button } from "@/components/ui/button";

interface EmptyStateProps {
  /** Lucide icon component to display above the message. */
  icon?: LucideIcon;
  /** Primary message (e.g. "No wallets yet"). */
  title: string;
  /** Optional secondary description text. */
  description?: string;
  /** Label for the call-to-action button. */
  actionLabel?: string;
  /** Callback when the action button is clicked. */
  onAction?: () => void;
  className?: string;
}

/**
 * Centered empty-state placeholder for lists and content areas.
 *
 * Design spec from 2.1 META:
 * - Centered layout with muted icon + descriptive text + action button
 * - e.g. "No wallets yet" → "Create Wallet" button
 */
export function EmptyState({
  icon: Icon,
  title,
  description,
  actionLabel,
  onAction,
  className,
}: EmptyStateProps) {
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center gap-3 py-16 text-center",
        className,
      )}
      role="status"
    >
      {Icon && (
        <Icon
          className="h-12 w-12 text-muted-foreground/50"
          aria-hidden="true"
        />
      )}
      <div className="space-y-1">
        <p className="text-lg font-medium text-foreground">{title}</p>
        {description && (
          <p className="text-sm text-muted-foreground">{description}</p>
        )}
      </div>
      {actionLabel && onAction && (
        <Button onClick={onAction} className="mt-2">
          {actionLabel}
        </Button>
      )}
    </div>
  );
}
