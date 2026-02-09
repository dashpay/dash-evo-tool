import { cn } from "@/lib/utils";
import {
  Users,
  FileText,
  Coins,
  Wallet,
  Wrench,
  Settings,
  MessageCircle,
  ChevronLeft,
  ChevronRight,
} from "lucide-react";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Badge } from "@/components/ui/badge";
import type { NetworkDto } from "@/bindings";

export interface NavItem {
  id: string;
  label: string;
  icon: React.ComponentType<{ className?: string }>;
  path: string;
}

export const navItems: NavItem[] = [
  { id: "dashpay", label: "DashPay", icon: MessageCircle, path: "/dashpay" },
  { id: "identities", label: "Identities", icon: Users, path: "/identities" },
  { id: "contracts", label: "Contracts", icon: FileText, path: "/contracts" },
  { id: "tokens", label: "Tokens", icon: Coins, path: "/tokens" },
  { id: "wallets", label: "Wallets", icon: Wallet, path: "/wallets" },
  { id: "tools", label: "Tools", icon: Wrench, path: "/tools" },
  { id: "settings", label: "Settings", icon: Settings, path: "/settings" },
];

/** Maps a route path to the active sidebar section ID */
export function getActiveSectionFromPath(pathname: string): string | undefined {
  // Match against nav items by finding the longest prefix match
  for (const item of navItems) {
    if (pathname === item.path || pathname.startsWith(item.path + "/")) {
      return item.id;
    }
  }
  return undefined;
}

const networkLabels: Record<NetworkDto, string> = {
  dash: "Mainnet",
  testnet: "Testnet",
  devnet: "Devnet",
  regtest: "Local Network",
};

const networkColorClasses: Record<NetworkDto, string> = {
  dash: "bg-network-mainnet",
  testnet: "bg-network-testnet",
  devnet: "bg-network-devnet",
  regtest: "bg-network-local",
};

interface SidebarProps {
  /** Currently active section ID (e.g. "identities", "wallets") */
  activeSection?: string;
  /** Called when user clicks a nav item */
  onNavigate: (path: string) => void;
  /** Current network (null = not initialized yet) */
  network?: NetworkDto | null;
  /** Whether developer mode is enabled */
  developerMode?: boolean;
  /** Whether sidebar is collapsed (icon-only) */
  collapsed?: boolean;
  /** Toggle collapsed state */
  onToggleCollapsed?: () => void;
  className?: string;
}

export function Sidebar({
  activeSection,
  onNavigate,
  network = null,
  developerMode = false,
  collapsed = false,
  onToggleCollapsed,
  className,
}: SidebarProps) {
  return (
    <div
      className={cn(
        "flex h-full flex-col border-r border-sidebar-border bg-sidebar transition-[width] duration-200",
        collapsed ? "w-[72px]" : "w-[200px]",
        className,
      )}
      data-testid="sidebar"
    >
      {/* Navigation items */}
      <nav className="flex flex-1 flex-col gap-1 p-2 pt-4" aria-label="Main navigation">
        {navItems.map((item) => {
          const isActive = activeSection === item.id;
          const Icon = item.icon;

          const button = (
            <button
              key={item.id}
              type="button"
              onClick={() => onNavigate(item.path)}
              className={cn(
                "flex w-full items-center gap-3 rounded-md px-3 py-2.5 text-sm font-medium transition-colors",
                "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                isActive
                  ? "bg-sidebar-primary text-sidebar-primary-foreground shadow-glow"
                  : "text-sidebar-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
                collapsed && "justify-center px-0",
              )}
              aria-current={isActive ? "page" : undefined}
              data-testid={`nav-${item.id}`}
            >
              <Icon className={cn("h-5 w-5 flex-shrink-0", collapsed && "h-5 w-5")} />
              {!collapsed && <span className="truncate">{item.label}</span>}
            </button>
          );

          if (collapsed) {
            return (
              <Tooltip key={item.id} delayDuration={0}>
                <TooltipTrigger asChild>{button}</TooltipTrigger>
                <TooltipContent side="right" sideOffset={8}>
                  {item.label}
                </TooltipContent>
              </Tooltip>
            );
          }

          return button;
        })}
      </nav>

      {/* Bottom area: network badge, developer mode, collapse toggle */}
      <div className="flex flex-col gap-2 border-t border-sidebar-border p-2">
        {/* Developer mode badge */}
        {developerMode && (
          <button
            type="button"
            onClick={() => onNavigate("/settings")}
            className={cn(
              "flex items-center gap-2 rounded-md px-3 py-1.5 text-xs font-medium text-purple-600 transition-colors hover:bg-sidebar-accent dark:text-purple-400",
              collapsed && "justify-center px-0",
            )}
            data-testid="dev-mode-badge"
          >
            <Wrench className="h-3.5 w-3.5 flex-shrink-0" />
            {!collapsed && <span>Dev Mode</span>}
          </button>
        )}

        {/* Network badge (hidden on mainnet) */}
        {network && network !== "dash" && (
          <div className={cn("flex items-center", collapsed ? "justify-center" : "px-3")}>
            <Badge
              className={cn(
                "network-badge",
                networkColorClasses[network],
              )}
              data-testid="network-badge"
            >
              {collapsed
                ? networkLabels[network]?.charAt(0)
                : networkLabels[network]}
            </Badge>
          </div>
        )}

        {/* Collapse toggle */}
        {onToggleCollapsed && (
          <button
            type="button"
            onClick={onToggleCollapsed}
            className={cn(
              "flex items-center gap-2 rounded-md px-3 py-1.5 text-xs text-muted-foreground transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
              collapsed && "justify-center px-0",
            )}
            aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
            data-testid="sidebar-collapse-toggle"
          >
            {collapsed ? (
              <ChevronRight className="h-4 w-4" />
            ) : (
              <>
                <ChevronLeft className="h-4 w-4" />
                <span>Collapse</span>
              </>
            )}
          </button>
        )}
      </div>
    </div>
  );
}
