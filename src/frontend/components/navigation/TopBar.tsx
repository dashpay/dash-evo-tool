import { cn } from "@/lib/utils";
import { ChevronRight } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ThemeToggle } from "@/components/theme";
import type { NetworkDto } from "@/bindings";

export interface BreadcrumbItem {
  label: string;
  onClick?: () => void;
}

export interface ActionButton {
  label: string;
  onClick: () => void;
  variant?: "default" | "destructive" | "outline" | "secondary" | "ghost";
}

export interface ActionGroup {
  label: string;
  actions: ActionButton[];
}

const networkLabels: Record<NetworkDto, string> = {
  dash: "Mainnet",
  testnet: "Testnet",
  devnet: "Devnet",
  regtest: "Local",
};

const networkBadgeClasses: Record<NetworkDto, string> = {
  dash: "bg-network-mainnet text-white",
  testnet: "bg-network-testnet text-white",
  devnet: "bg-network-devnet text-white",
  regtest: "bg-network-local text-white",
};

interface TopBarProps {
  /** Breadcrumb trail for current location */
  breadcrumbs?: BreadcrumbItem[];
  /** Whether Dash Core wallet is connected */
  connected?: boolean;
  /** Current network */
  network?: NetworkDto | null;
  /** Context-sensitive action buttons */
  actions?: ActionButton[];
  /** Grouped action menus (e.g. Documents, Contracts dropdowns) */
  actionGroups?: ActionGroup[];
  /** Called when disconnected connection dot is clicked (start Dash-Qt) */
  onConnectionClick?: () => void;
  className?: string;
}

export function TopBar({
  breadcrumbs = [],
  connected = false,
  network = null,
  actions = [],
  actionGroups = [],
  onConnectionClick,
  className,
}: TopBarProps) {
  return (
    <div
      className={cn(
        "island flex items-center justify-between gap-4 px-4 py-3",
        className,
      )}
      role="banner"
      data-testid="top-bar"
    >
      {/* Left: Connection status + Breadcrumbs */}
      <div className="flex min-w-0 flex-1 items-center gap-3">
        {/* Connection indicator */}
        <button
          type="button"
          onClick={connected ? undefined : onConnectionClick}
          className={cn(
            "flex-shrink-0 rounded-full p-1 transition-colors",
            !connected && "cursor-pointer hover:bg-accent",
          )}
          aria-label={
            connected
              ? "Connected to Dash Core Wallet"
              : "Disconnected from Dash Core Wallet. Click to start."
          }
          title={
            connected
              ? "Connected to Dash Core Wallet"
              : "Disconnected from Dash Core Wallet. Click to start."
          }
          data-testid="connection-indicator"
        >
          <span
            className={
              connected
                ? "connection-dot-connected"
                : "connection-dot-disconnected"
            }
          />
        </button>

        {/* Breadcrumb navigation */}
        {breadcrumbs.length > 0 && (
          <nav
            aria-label="Breadcrumb"
            className="flex min-w-0 items-center gap-1 text-sm"
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
                    className="truncate text-muted-foreground transition-colors hover:text-foreground"
                  >
                    {crumb.label}
                  </button>
                ) : (
                  <span className="truncate font-medium text-foreground">
                    {crumb.label}
                  </span>
                )}
              </span>
            ))}
          </nav>
        )}
      </div>

      {/* Right: Actions + Network badge + Theme toggle */}
      <div className="flex flex-shrink-0 items-center gap-2">
        {/* Action groups (dropdown menus) */}
        {actionGroups.map((group) => (
          <DropdownMenu key={group.label}>
            <DropdownMenuTrigger asChild>
              <Button
                variant="default"
                size="sm"
                className="bg-network-mainnet text-white hover:bg-network-mainnet/90"
                data-testid={`action-group-${group.label.toLowerCase()}`}
              >
                {group.label}
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              {group.actions.map((action) => (
                <DropdownMenuItem
                  key={action.label}
                  onClick={action.onClick}
                >
                  {action.label}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        ))}

        {/* Individual action buttons */}
        {actions.map((action) => (
          <Button
            key={action.label}
            variant={action.variant ?? "default"}
            size="sm"
            onClick={action.onClick}
            data-testid={`action-${action.label.toLowerCase().replace(/\s+/g, "-")}`}
          >
            {action.label}
          </Button>
        ))}

        {/* Network badge */}
        {network && (
          <Badge
            className={cn(
              "network-badge",
              networkBadgeClasses[network],
            )}
            data-testid="top-bar-network-badge"
          >
            {networkLabels[network]}
          </Badge>
        )}

        {/* Theme toggle */}
        <ThemeToggle />
      </div>
    </div>
  );
}
