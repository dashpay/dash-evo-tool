import { useEffect, useMemo, useCallback } from "react";
import {
  Outlet,
  useNavigate,
  useRouterState,
} from "@tanstack/react-router";
import { Island } from "@/components/layout/Island";
import { cn } from "@/lib/utils";
import { Swords, History, Badge, Clock } from "lucide-react";

// ─── Subscreen tabs ──────────────────────────────────────────────────

interface SubscreenTab {
  id: string;
  label: string;
  icon: React.ElementType;
  path: string;
}

const tabs: SubscreenTab[] = [
  { id: "active", label: "Active Contests", icon: Swords, path: "/contracts/dpns/active" },
  { id: "past", label: "Past Contests", icon: History, path: "/contracts/dpns/past" },
  { id: "owned", label: "Owned Names", icon: Badge, path: "/contracts/dpns/owned" },
  { id: "scheduled", label: "Scheduled Votes", icon: Clock, path: "/contracts/dpns/scheduled" },
];

// ─── DpnsScreen ─────────────────────────────────────────────────────

export function DpnsScreen() {
  const navigate = useNavigate();
  const routerState = useRouterState();
  const pathname = routerState.location.pathname;

  // Redirect /contracts/dpns to /contracts/dpns/active
  useEffect(() => {
    if (pathname === "/contracts/dpns" || pathname === "/contracts/dpns/") {
      navigate({ to: "/contracts/dpns/active", replace: true });
    }
  }, [pathname, navigate]);

  // Active tab from pathname
  const activeTabId = useMemo(() => {
    const match = tabs.find(
      (t) => pathname === t.path || pathname.startsWith(t.path + "/"),
    );
    return match?.id ?? "active";
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
        <nav className="flex flex-col gap-1 p-2" aria-label="DPNS sections">
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
