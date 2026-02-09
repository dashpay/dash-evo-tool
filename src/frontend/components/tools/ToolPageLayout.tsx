import { cn } from "@/lib/utils";
import { ArrowLeft } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Island } from "@/components/layout/Island";
import { useNavigate } from "@tanstack/react-router";

interface ToolPageLayoutProps {
  /** Tool page title displayed at the top. */
  title: string;
  /** Optional subtitle below the title. */
  subtitle?: string;
  /** Optional action buttons rendered in the header right side. */
  actions?: React.ReactNode;
  /** Page content. */
  children: React.ReactNode;
  /** Additional CSS class for the outer container. */
  className?: string;
  /** Whether to show the back navigation to tools index. Defaults to true. */
  showBack?: boolean;
}

/**
 * Consistent page layout for tool screens.
 *
 * Provides: back navigation to /tools, title + subtitle header, optional action buttons,
 * and an Island wrapper for content. Used by all individual tool screens.
 */
export function ToolPageLayout({
  title,
  subtitle,
  actions,
  children,
  className,
  showBack = true,
}: ToolPageLayoutProps) {
  const navigate = useNavigate();

  return (
    <div className={cn("flex flex-1 flex-col gap-3 min-h-0", className)}>
      <Island className="flex-1 flex flex-col min-h-0 overflow-auto">
        {/* Header */}
        <div className="flex items-start justify-between gap-4 mb-6">
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              {showBack && (
                <Button
                  variant="ghost"
                  size="icon-sm"
                  onClick={() => navigate({ to: "/tools" })}
                  aria-label="Back to Tools"
                >
                  <ArrowLeft className="size-4" />
                </Button>
              )}
              <div className="min-w-0">
                <h1 className="truncate text-2xl font-bold tracking-tight">
                  {title}
                </h1>
                {subtitle && (
                  <p className="mt-0.5 text-sm text-muted-foreground">
                    {subtitle}
                  </p>
                )}
              </div>
            </div>
          </div>
          {actions && (
            <div className="flex flex-shrink-0 items-center gap-2">
              {actions}
            </div>
          )}
        </div>

        {/* Content */}
        <div className="flex-1 min-h-0">{children}</div>
      </Island>
    </div>
  );
}
