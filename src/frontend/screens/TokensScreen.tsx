import { useEffect, useMemo, useCallback } from "react";
import {
  Outlet,
  useNavigate,
  useRouterState,
} from "@tanstack/react-router";
import { Island } from "@/components/layout/Island";
import { cn } from "@/lib/utils";
import { Coins, Search, Sparkles } from "lucide-react";

// ─── Subscreen tabs ──────────────────────────────────────────────────

interface SubscreenTab {
  id: string;
  label: string;
  icon: React.ElementType;
  path: string;
}

const tabs: SubscreenTab[] = [
  { id: "my-tokens", label: "My Tokens", icon: Coins, path: "/tokens" },
  { id: "search", label: "Search Tokens", icon: Search, path: "/tokens/search" },
  { id: "creator", label: "Token Creator", icon: Sparkles, path: "/tokens/creator" },
];

// ─── TokensScreen ───────────────────────────────────────────────────

export function TokensScreen() {
  const navigate = useNavigate();
  const routerState = useRouterState();
  const pathname = routerState.location.pathname;

  // Redirect trailing slash /tokens/ → /tokens
  useEffect(() => {
    if (pathname === "/tokens/") {
      navigate({ to: "/tokens", replace: true });
    }
  }, [pathname, navigate]);

  // Active tab from pathname
  const activeTabId = useMemo(() => {
    // Check specific sub-paths first (longest match wins)
    for (const tab of tabs) {
      if (tab.path === "/tokens") continue; // check index last
      if (pathname === tab.path || pathname.startsWith(tab.path + "/")) {
        return tab.id;
      }
    }
    // Index route: exact match or child routes like /tokens/transfer
    if (pathname === "/tokens") return "my-tokens";
    // For operation routes (transfer, mint, burn, etc.) no tab is highlighted
    return null;
  }, [pathname]);

  const handleTabClick = useCallback(
    (path: string) => {
      navigate({ to: path });
    },
    [navigate],
  );

  return (
    <div className="flex flex-1 gap-3 min-h-0">
      {/* Left — Subscreen navigation sidebar */}
      <Island noPadding className="w-[220px] shrink-0 flex flex-col">
        <nav className="flex flex-col gap-1 p-2" aria-label="Tokens sections">
          {tabs.map((tab) => {
            const Icon = tab.icon;
            const isActive = tab.id === activeTabId;
            return (
              <button
                key={tab.id}
                type="button"
                onClick={() => handleTabClick(tab.path)}
                className={cn(
                  "flex items-center gap-3 rounded-md px-3 py-2.5 text-sm font-medium transition-colors text-left",
                  isActive
                    ? "bg-primary text-primary-foreground"
                    : "text-muted-foreground hover:bg-muted hover:text-foreground",
                )}
                aria-current={isActive ? "page" : undefined}
              >
                <Icon className="h-4 w-4 shrink-0" aria-hidden="true" />
                {tab.label}
              </button>
            );
          })}
        </nav>
      </Island>

      {/* Right — Active subscreen content */}
      <div className="flex flex-1 min-w-0">
        <Outlet />
      </div>
    </div>
  );
}
