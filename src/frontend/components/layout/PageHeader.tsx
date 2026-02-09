import { cn } from "@/lib/utils";
import { ChevronRight } from "lucide-react";

export interface Breadcrumb {
  label: string;
  /** If provided, breadcrumb is clickable. */
  onClick?: () => void;
}

interface PageHeaderProps {
  /** Page title displayed prominently. */
  title: string;
  /** Optional breadcrumb trail (e.g. ["Wallets", "HD Wallet #1"]). */
  breadcrumbs?: Breadcrumb[];
  /** Action buttons rendered on the right side. */
  actions?: React.ReactNode;
  /** Optional subtitle below the title. */
  subtitle?: string;
  className?: string;
}

/**
 * Page header with title, optional breadcrumbs, and right-aligned action buttons.
 * Matches the egui top_panel.rs pattern: left breadcrumbs + title, right action buttons.
 *
 * Used at the top of Island content areas.
 */
export function PageHeader({
  title,
  breadcrumbs,
  actions,
  subtitle,
  className,
}: PageHeaderProps) {
  return (
    <div
      className={cn(
        "flex items-start justify-between gap-4",
        className,
      )}
    >
      {/* Left: breadcrumbs + title */}
      <div className="min-w-0 flex-1">
        {breadcrumbs && breadcrumbs.length > 0 && (
          <nav
            aria-label="Breadcrumb"
            className="mb-1 flex items-center gap-1 text-sm text-muted-foreground"
          >
            {breadcrumbs.map((crumb, i) => (
              <span key={i} className="flex items-center gap-1">
                {i > 0 && (
                  <ChevronRight
                    className="h-3.5 w-3.5 flex-shrink-0 text-muted-foreground/60"
                    aria-hidden="true"
                  />
                )}
                {crumb.onClick ? (
                  <button
                    type="button"
                    onClick={crumb.onClick}
                    className="hover:text-foreground transition-colors"
                  >
                    {crumb.label}
                  </button>
                ) : (
                  <span>{crumb.label}</span>
                )}
              </span>
            ))}
          </nav>
        )}
        <h1 className="truncate text-2xl font-bold tracking-tight">
          {title}
        </h1>
        {subtitle && (
          <p className="mt-1 text-sm text-muted-foreground">{subtitle}</p>
        )}
      </div>

      {/* Right: action buttons */}
      {actions && (
        <div className="flex flex-shrink-0 items-center gap-2">
          {actions}
        </div>
      )}
    </div>
  );
}
