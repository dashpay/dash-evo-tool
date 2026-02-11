import { useState, useCallback, useEffect, useMemo } from "react";
import { Outlet, useNavigate, useRouterState } from "@tanstack/react-router";
import { AppShell } from "@/components/layout";
import { ErrorBoundary } from "@/components/feedback";
import { Sidebar, TopBar, getActiveSectionFromPath, navItems } from "@/components/navigation";
import type { BreadcrumbItem } from "@/components/navigation";
import type { NetworkDto } from "@/bindings";
import { commands, events } from "@/bindings";
import type { ZmqConnectionStatusEvent } from "@/bindings";
import { useKeyboardShortcuts } from "@/hooks/useKeyboardShortcuts";

/** Build breadcrumbs from the current route path */
function buildBreadcrumbs(
  pathname: string,
  navigate: (path: string) => void,
): BreadcrumbItem[] {
  const crumbs: BreadcrumbItem[] = [];

  // Find the matching top-level nav item
  const activeNavItem = navItems.find(
    (item) =>
      pathname === item.path || pathname.startsWith(item.path + "/"),
  );

  if (!activeNavItem) return crumbs;

  // First crumb: the top-level section (clickable if there are sub-routes)
  const hasSubPath = pathname !== activeNavItem.path && pathname !== activeNavItem.path + "/";

  crumbs.push({
    label: activeNavItem.label,
    onClick: hasSubPath ? () => navigate(activeNavItem.path) : undefined,
  });

  // Second crumb: sub-route label if applicable
  if (hasSubPath) {
    const subPath = pathname.slice(activeNavItem.path.length + 1);
    const label = subPath
      .split("/")[0]!
      .replace(/-/g, " ")
      .replace(/\b\w/g, (c) => c.toUpperCase());
    crumbs.push({ label });
  }

  return crumbs;
}

export function AppLayout() {
  const navigate = useNavigate();
  const routerState = useRouterState();
  const pathname = routerState.location.pathname;

  const [collapsed, setCollapsed] = useState(false);
  const [network, setNetwork] = useState<NetworkDto | null>(null);
  const [connected, setConnected] = useState(false);
  const [developerMode, setDeveloperMode] = useState(false);

  // Load initial network info on mount
  useEffect(() => {
    commands
      .getNetworkInfo()
      .then((info) => {
        setNetwork(info.activeNetwork);
      })
      .catch(() => {
        // Not connected to backend (browser mode)
      });
  }, []);

  // Load developer mode status
  useEffect(() => {
    commands
      .contextIsDeveloperMode()
      .then((isDev) => {
        setDeveloperMode(isDev);
      })
      .catch(() => {
        // Not connected to backend
      });
  }, []);

  // Listen for ZMQ connection status events
  useEffect(() => {
    let cleanup: (() => void) | undefined;
    events.zmqConnectionStatusEvent
      .listen((event: { payload: ZmqConnectionStatusEvent }) => {
        if (event.payload.network === network) {
          setConnected(event.payload.connected);
        }
      })
      .then((unlisten) => {
        cleanup = unlisten;
      })
      .catch(() => {
        // Events not available in browser mode
      });
    return () => cleanup?.();
  }, [network]);

  const activeSection = getActiveSectionFromPath(pathname);
  const breadcrumbs = buildBreadcrumbs(pathname, (path: string) => {
    navigate({ to: path });
  });

  const handleNavigate = useCallback(
    (path: string) => {
      navigate({ to: path });
    },
    [navigate],
  );

  const handleConnectionClick = useCallback(() => {
    commands.coreStartDashQt({ dashQtPath: "", overwriteDashConf: false }).catch(() => {
      // Ignore errors (no Dash-Qt configured)
    });
  }, []);

  // Global keyboard shortcuts: number keys 1-7 navigate to sidebar sections,
  // [ and ] collapse/expand sidebar
  const shortcuts = useMemo(
    () => [
      ...navItems.map((item, index) => ({
        key: String(index + 1),
        description: `Navigate to ${item.label}`,
        action: () => handleNavigate(item.path),
      })),
      {
        key: "[",
        description: "Toggle sidebar collapsed",
        action: () => setCollapsed((c) => !c),
      },
    ],
    [handleNavigate],
  );
  useKeyboardShortcuts(shortcuts);

  return (
    <AppShell
      sidebar={
        <Sidebar
          activeSection={activeSection}
          onNavigate={handleNavigate}
          network={network}
          developerMode={developerMode}
          collapsed={collapsed}
          onToggleCollapsed={() => setCollapsed((c) => !c)}
        />
      }
    >
      {/* Top bar + content area */}
      <div className="flex flex-1 flex-col gap-3 overflow-hidden p-3">
        <TopBar
          breadcrumbs={breadcrumbs}
          connected={connected}
          network={network}
          onConnectionClick={handleConnectionClick}
        />
        {/* Main content — scrollable area */}
        <div className="flex min-h-0 flex-1 overflow-auto">
          <ErrorBoundary>
            <Outlet />
          </ErrorBoundary>
        </div>
      </div>
    </AppShell>
  );
}
