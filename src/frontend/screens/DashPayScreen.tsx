import { useEffect, useMemo, useCallback } from "react";
import {
  Outlet,
  useNavigate,
  useRouterState,
} from "@tanstack/react-router";
import { Island } from "@/components/layout/Island";
import { EmptyState } from "@/components/feedback/EmptyState";
import {
  IdentitySelector,
  type IdentityOption,
} from "@/components/shared/IdentitySelector";
import { cn } from "@/lib/utils";
import { useDashPayStore } from "@/stores/dashpayStore";
import { useIdentityStore } from "@/stores/identityStore";
import {
  User,
  Users,
  CreditCard,
  Search,
  ShieldAlert,
} from "lucide-react";

// ─── Subscreen tabs ──────────────────────────────────────────────────

interface SubscreenTab {
  id: string;
  label: string;
  icon: React.ElementType;
  path: string;
  /** Only show in developer mode */
  devOnly?: boolean;
}

const tabs: SubscreenTab[] = [
  { id: "profile", label: "My Profile", icon: User, path: "/dashpay/profile" },
  { id: "contacts", label: "Contacts", icon: Users, path: "/dashpay/contacts" },
  {
    id: "payments",
    label: "Payment History",
    icon: CreditCard,
    path: "/dashpay/payments",
    devOnly: true,
  },
  {
    id: "search",
    label: "Search Profiles",
    icon: Search,
    path: "/dashpay/search",
  },
];

// ─── No-Identities Card ──────────────────────────────────────────────

function NoIdentitiesCard({ onLoadIdentity }: { onLoadIdentity: () => void }) {
  return (
    <Island className="flex-1">
      <EmptyState
        icon={ShieldAlert}
        title="No Identities Loaded"
        description="You need at least one identity to use DashPay features. Load an existing identity or create one from the Identities screen."
        actionLabel="Load Identity"
        onAction={onLoadIdentity}
      />
    </Island>
  );
}

// ─── DashPayScreen ───────────────────────────────────────────────────

export function DashPayScreen() {
  const navigate = useNavigate();
  const routerState = useRouterState();
  const pathname = routerState.location.pathname;

  // Stores
  const {
    selectedIdentityId,
    selectIdentity,
    subscribeToUpdates,
    loadProfile,
    loadContacts,
    loadContactRequests,
    loadPayments,
  } = useDashPayStore();
  const identities = useIdentityStore((s) => s.identities);
  const loadIdentities = useIdentityStore((s) => s.loadIdentities);
  const identitiesLoading = useIdentityStore((s) => s.loading);

  // Load identities on mount
  useEffect(() => {
    loadIdentities();
  }, [loadIdentities]);

  // Auto-select first identity if none selected
  useEffect(() => {
    if (!selectedIdentityId && identities.length > 0) {
      selectIdentity(identities[0]!.id);
    }
  }, [selectedIdentityId, identities, selectIdentity]);

  // Load data when identity changes
  useEffect(() => {
    if (selectedIdentityId) {
      loadProfile();
      loadContacts();
      loadContactRequests();
      loadPayments();
    }
  }, [selectedIdentityId, loadProfile, loadContacts, loadContactRequests, loadPayments]);

  // Subscribe to backend task events
  useEffect(() => {
    let unsubscribe: (() => void) | undefined;
    subscribeToUpdates()
      .then((unsub) => {
        unsubscribe = unsub;
      })
      .catch((e) => console.error("Failed to subscribe to DashPay events:", e));
    return () => unsubscribe?.();
  }, [subscribeToUpdates]);

  // Redirect /dashpay to /dashpay/profile
  useEffect(() => {
    if (pathname === "/dashpay" || pathname === "/dashpay/") {
      navigate({ to: "/dashpay/profile", replace: true });
    }
  }, [pathname, navigate]);

  // Identity options for selector
  const identityOptions: IdentityOption[] = useMemo(
    () =>
      identities.map((i) => ({
        id: i.id,
        displayName: i.alias?.trim() || i.id.slice(0, 12) + "...",
      })),
    [identities],
  );

  // Active tab from pathname
  const activeTabId = useMemo(() => {
    const match = tabs.find(
      (t) => pathname === t.path || pathname.startsWith(t.path + "/"),
    );
    return match?.id ?? "profile";
  }, [pathname]);

  const handleTabClick = useCallback(
    (path: string) => {
      navigate({ to: path });
    },
    [navigate],
  );

  const handleLoadIdentity = useCallback(() => {
    navigate({ to: "/identities" });
  }, [navigate]);

  const handleIdentityChange = useCallback(
    (id: string) => {
      selectIdentity(id || null);
    },
    [selectIdentity],
  );

  // Show no-identities state
  if (!identitiesLoading && identities.length === 0) {
    return <NoIdentitiesCard onLoadIdentity={handleLoadIdentity} />;
  }

  // Filter visible tabs (payments is dev-only — for now always show)
  const visibleTabs = tabs;

  return (
    <div className="flex flex-1 gap-3 min-h-0">
      {/* Left — Subscreen navigation sidebar */}
      <Island noPadding className="w-[220px] shrink-0 flex flex-col">
        {/* Identity selector at top */}
        <div className="p-4 pb-2">
          <IdentitySelector
            value={selectedIdentityId ?? ""}
            onChange={handleIdentityChange}
            identities={identityOptions}
            placeholder="Select identity"
            showOther={false}
            disabled={identitiesLoading}
          />
        </div>

        {/* Tab buttons */}
        <nav className="flex flex-col gap-1 p-2" aria-label="DashPay sections">
          {visibleTabs.map((tab) => {
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
