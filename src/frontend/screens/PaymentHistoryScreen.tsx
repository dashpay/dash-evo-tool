import { useEffect, useMemo } from "react";
import {
  ArrowDownLeft,
  ArrowUpRight,
  CreditCard,
  RefreshCw,
  Loader2,
  Copy,
  AlertCircle,
  Search,
  ChevronDown,
  ArrowUpDown,
} from "lucide-react";
import { Island } from "@/components/layout/Island";
import { EmptyState } from "@/components/feedback/EmptyState";
import { formatAmount } from "@/components/shared/AmountInput";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  useDashPayStore,
  type PaymentDirectionFilter,
  type PaymentSortField,
} from "@/stores/dashpayStore";
import { cn } from "@/lib/utils";
import type { StoredPaymentDto } from "@/bindings";

// ─── Constants ────────────────────────────────────────────────────────

const DIRECTION_LABELS: Record<PaymentDirectionFilter, string> = {
  all: "All",
  incoming: "Incoming",
  outgoing: "Outgoing",
};

const SORT_LABELS: Record<PaymentSortField, string> = {
  date: "Date",
  amount: "Amount",
  contact: "Contact Name",
};

const STATUS_BADGE_STYLES: Record<
  string,
  { label: string; className: string } | undefined
> = {
  confirmed: {
    label: "confirmed",
    className: "text-green-600 border-green-600/30",
  },
  pending: {
    label: "pending",
    className: "text-yellow-600 border-yellow-600/30",
  },
  failed: {
    label: "failed",
    className: "text-destructive border-destructive/30",
  },
};

// ─── Helpers ──────────────────────────────────────────────────────────

function formatTimestamp(unixSeconds: number): string {
  const now = Date.now() / 1000;
  const diff = now - unixSeconds;

  if (diff < 60) return "just now";
  if (diff < 3600) {
    const mins = Math.floor(diff / 60);
    return `${mins}m ago`;
  }
  if (diff < 86400) {
    const hours = Math.floor(diff / 3600);
    return `${hours}h ago`;
  }
  if (diff < 604800) {
    const days = Math.floor(diff / 86400);
    return `${days}d ago`;
  }

  const date = new Date(unixSeconds * 1000);
  return date.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

function truncateId(id: string): string {
  if (id.length <= 16) return id;
  return `${id.slice(0, 8)}…${id.slice(-8)}`;
}

function resolveContactName(
  payment: StoredPaymentDto,
  selectedIdentityId: string,
  contactsMap: Map<string, string>,
): string {
  const isIncoming = payment.toIdentityId === selectedIdentityId;
  const counterpartyId = isIncoming
    ? payment.fromIdentityId
    : payment.toIdentityId;

  const name = contactsMap.get(counterpartyId);
  if (name) return name;

  return `Unknown (${counterpartyId.slice(0, 8)})`;
}

function copyToClipboard(text: string) {
  navigator.clipboard.writeText(text);
}

function getPaymentTimestamp(payment: StoredPaymentDto): number {
  return payment.confirmedAt ?? payment.createdAt ?? 0;
}

function formatPaymentTime(payment: StoredPaymentDto): string {
  const ts = getPaymentTimestamp(payment);
  return ts > 0 ? formatTimestamp(ts) : "Pending";
}

// ─── Component ────────────────────────────────────────────────────────

export function PaymentHistoryScreen() {
  const selectedIdentityId = useDashPayStore((s) => s.selectedIdentityId);
  const payments = useDashPayStore((s) => s.payments);
  const paymentsLoading = useDashPayStore((s) => s.paymentsLoading);
  const paymentsRefreshing = useDashPayStore((s) => s.paymentsRefreshing);
  const paymentsError = useDashPayStore((s) => s.paymentsError);
  const contacts = useDashPayStore((s) => s.contacts);
  const loadPayments = useDashPayStore((s) => s.loadPayments);
  const refreshPayments = useDashPayStore((s) => s.refreshPayments);

  // Filter/sort state
  const directionFilter = useDashPayStore((s) => s.paymentDirectionFilter);
  const searchQuery = useDashPayStore((s) => s.paymentSearchQuery);
  const sortField = useDashPayStore((s) => s.paymentSortField);
  const sortOrder = useDashPayStore((s) => s.paymentSortOrder);
  const setDirectionFilter = useDashPayStore(
    (s) => s.setPaymentDirectionFilter,
  );
  const setSearchQuery = useDashPayStore((s) => s.setPaymentSearchQuery);
  const setSortField = useDashPayStore((s) => s.setPaymentSortField);
  const toggleSortOrder = useDashPayStore((s) => s.togglePaymentSortOrder);

  // Build a map of contact identity IDs → display names for fast lookups
  const contactsMap = useMemo(() => {
    const map = new Map<string, string>();
    for (const c of contacts) {
      const name = c.displayName || c.username || null;
      if (name) {
        map.set(c.contactIdentityId, name);
      }
    }
    return map;
  }, [contacts]);

  // Filtered and sorted payments
  const filteredPayments = useMemo(() => {
    if (!selectedIdentityId) return [];

    let result = payments;

    // Direction filter
    if (directionFilter !== "all") {
      result = result.filter((p) => {
        const isIncoming = p.toIdentityId === selectedIdentityId;
        return directionFilter === "incoming" ? isIncoming : !isIncoming;
      });
    }

    // Search filter (by contact name, memo, or tx ID)
    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase();
      result = result.filter((p) => {
        const name = resolveContactName(p, selectedIdentityId, contactsMap);
        return (
          name.toLowerCase().includes(q) ||
          (p.memo?.toLowerCase().includes(q) ?? false) ||
          p.txId.toLowerCase().includes(q)
        );
      });
    }

    // Sort
    const sorted = [...result].sort((a, b) => {
      let cmp = 0;
      switch (sortField) {
        case "date":
          cmp = getPaymentTimestamp(b) - getPaymentTimestamp(a);
          break;
        case "amount":
          cmp = b.amount - a.amount;
          break;
        case "contact": {
          const aName = resolveContactName(
            a,
            selectedIdentityId,
            contactsMap,
          ).toLowerCase();
          const bName = resolveContactName(
            b,
            selectedIdentityId,
            contactsMap,
          ).toLowerCase();
          cmp = aName.localeCompare(bName);
          break;
        }
      }
      return sortOrder === "descending" ? -cmp : cmp;
    });

    return sorted;
  }, [
    payments,
    selectedIdentityId,
    directionFilter,
    searchQuery,
    sortField,
    sortOrder,
    contactsMap,
  ]);

  // Load payments from local DB on mount / identity change
  useEffect(() => {
    if (selectedIdentityId) {
      loadPayments(100);
    }
  }, [selectedIdentityId, loadPayments]);

  // ── No identity selected ──
  if (!selectedIdentityId) {
    return (
      <Island className="flex-1">
        <EmptyState
          icon={CreditCard}
          title="No Identity Selected"
          description="Please select an identity from the sidebar to view payment history."
        />
      </Island>
    );
  }

  // ── Loading state ──
  if (paymentsLoading && payments.length === 0) {
    return (
      <Island className="flex-1">
        <div className="flex flex-col items-center justify-center gap-3 py-16">
          <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
          <p className="text-sm text-muted-foreground">
            Loading payment history…
          </p>
        </div>
      </Island>
    );
  }

  const hasFilters =
    directionFilter !== "all" || searchQuery.trim().length > 0;

  return (
    <Island className="flex-1 flex flex-col gap-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold">Payment History</h2>
        <Button
          variant="outline"
          size="sm"
          onClick={() => refreshPayments()}
          disabled={paymentsRefreshing}
          aria-label="Refresh payment history from Platform"
        >
          {paymentsRefreshing ? (
            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
          ) : (
            <RefreshCw className="mr-2 h-4 w-4" />
          )}
          {paymentsRefreshing ? "Refreshing…" : "Refresh"}
        </Button>
      </div>

      {/* Error banner */}
      {paymentsError && (
        <div className="flex items-start gap-2 rounded-md bg-destructive/10 border border-destructive/30 px-3 py-2 text-sm text-destructive">
          <AlertCircle className="h-4 w-4 mt-0.5 shrink-0" />
          <span>{paymentsError}</span>
        </div>
      )}

      {/* Filter / Sort controls */}
      <div className="flex flex-wrap items-center gap-2">
        {/* Search input */}
        <div className="relative max-w-xs flex-1 min-w-[180px]">
          <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground" />
          <Input
            placeholder="Search payments…"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="h-9 pl-8 text-sm"
            aria-label="Search payments"
          />
        </div>

        {/* Direction filter dropdown */}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline" size="sm" className="h-9">
              {DIRECTION_LABELS[directionFilter]}
              <ChevronDown className="ml-1.5 h-3.5 w-3.5" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start">
            {(
              Object.keys(DIRECTION_LABELS) as PaymentDirectionFilter[]
            ).map((f) => (
              <DropdownMenuItem
                key={f}
                onClick={() => setDirectionFilter(f)}
                className={cn(f === directionFilter && "bg-accent")}
              >
                {DIRECTION_LABELS[f]}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>

        {/* Sort dropdown */}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline" size="sm" className="h-9">
              Sort: {SORT_LABELS[sortField]}
              <ChevronDown className="ml-1.5 h-3.5 w-3.5" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start">
            {(Object.keys(SORT_LABELS) as PaymentSortField[]).map((s) => (
              <DropdownMenuItem
                key={s}
                onClick={() => setSortField(s)}
                className={cn(s === sortField && "bg-accent")}
              >
                {SORT_LABELS[s]}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>

        {/* Sort order toggle */}
        <Button
          variant="ghost"
          size="sm"
          className="h-9 px-2"
          onClick={toggleSortOrder}
          aria-label={`Sort ${sortOrder === "ascending" ? "descending" : "ascending"}`}
        >
          <ArrowUpDown className="h-3.5 w-3.5" />
          <span className="ml-1 text-xs text-muted-foreground">
            {sortOrder === "ascending" ? "Asc" : "Desc"}
          </span>
        </Button>
      </div>

      <Separator />

      {/* Empty state */}
      {payments.length === 0 && !paymentsLoading ? (
        <EmptyState
          icon={CreditCard}
          title="No Payment History"
          description="No payments have been made with this identity."
        />
      ) : filteredPayments.length === 0 && hasFilters ? (
        <EmptyState
          icon={Search}
          title="No Matching Payments"
          description="No payments match your current filters. Try adjusting your search or filter."
        />
      ) : (
        /* Payment list */
        <div className="flex flex-col gap-2 overflow-y-auto" role="list">
          {filteredPayments.map((payment) => (
            <PaymentCard
              key={`${payment.txId}-${payment.id}`}
              payment={payment}
              selectedIdentityId={selectedIdentityId}
              contactsMap={contactsMap}
            />
          ))}
        </div>
      )}
    </Island>
  );
}

// ─── PaymentCard ──────────────────────────────────────────────────────

interface PaymentCardProps {
  payment: StoredPaymentDto;
  selectedIdentityId: string;
  contactsMap: Map<string, string>;
}

function PaymentCard({
  payment,
  selectedIdentityId,
  contactsMap,
}: PaymentCardProps) {
  const isIncoming = payment.toIdentityId === selectedIdentityId;
  const contactName = resolveContactName(
    payment,
    selectedIdentityId,
    contactsMap,
  );
  const amountFormatted = formatAmount(payment.amount, 8);

  return (
    <div
      role="listitem"
      className="flex items-start gap-3 rounded-lg border bg-card p-3 hover:bg-accent/50 transition-colors"
    >
      {/* Avatar placeholder */}
      <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground">
        <CreditCard className="h-5 w-5" />
      </div>

      {/* Direction indicator */}
      <div className="flex shrink-0 items-center pt-0.5">
        {isIncoming ? (
          <ArrowDownLeft
            className="h-4 w-4 text-green-500"
            aria-label="Incoming"
          />
        ) : (
          <ArrowUpRight
            className="h-4 w-4 text-red-500"
            aria-label="Outgoing"
          />
        )}
      </div>

      {/* Content */}
      <div className="flex-1 min-w-0">
        <div className="flex items-center justify-between gap-2">
          <span className="font-medium truncate">{contactName}</span>
          <span
            className={cn(
              "text-sm font-semibold whitespace-nowrap",
              isIncoming ? "text-green-500" : "text-red-500",
            )}
          >
            {isIncoming ? "+" : "-"}
            {amountFormatted} Dash
          </span>
        </div>

        {/* Memo */}
        {payment.memo && (
          <p className="text-sm text-muted-foreground italic truncate mt-0.5">
            &ldquo;{payment.memo}&rdquo;
          </p>
        )}

        {/* TX ID + Timestamp + Status */}
        <div className="flex items-center gap-2 mt-1">
          <button
            className="text-xs text-muted-foreground font-mono hover:text-foreground transition-colors cursor-pointer inline-flex items-center gap-1"
            onClick={() => copyToClipboard(payment.txId)}
            title="Copy transaction ID"
            aria-label={`Copy transaction ID ${truncateId(payment.txId)}`}
          >
            {truncateId(payment.txId)}
            <Copy className="h-3 w-3" />
          </button>
          <span className="text-xs text-muted-foreground">
            • {formatPaymentTime(payment)}
          </span>
          {STATUS_BADGE_STYLES[payment.status] && (
            <Badge
              variant="outline"
              className={cn(
                "text-[10px] px-1.5 py-0",
                STATUS_BADGE_STYLES[payment.status]!.className,
              )}
            >
              {STATUS_BADGE_STYLES[payment.status]!.label}
            </Badge>
          )}
        </div>
      </div>
    </div>
  );
}
