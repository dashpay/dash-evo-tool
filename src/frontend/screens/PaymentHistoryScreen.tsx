import { useEffect, useMemo } from "react";
import {
  ArrowDownLeft,
  ArrowUpRight,
  CreditCard,
  RefreshCw,
  Loader2,
  Copy,
  AlertCircle,
} from "lucide-react";
import { Island } from "@/components/layout/Island";
import { EmptyState } from "@/components/feedback/EmptyState";
import { formatAmount } from "@/components/shared/AmountInput";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import { useDashPayStore } from "@/stores/dashpayStore";
import { cn } from "@/lib/utils";
import type { StoredPaymentDto } from "@/bindings";

// ─── Constants ────────────────────────────────────────────────────────

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

      <Separator />

      {/* Empty state */}
      {payments.length === 0 && !paymentsLoading ? (
        <EmptyState
          icon={CreditCard}
          title="No Payment History"
          description="No payments have been made with this identity."
        />
      ) : (
        /* Payment list */
        <div className="flex flex-col gap-2 overflow-y-auto" role="list">
          {payments.map((payment) => (
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

        {/* TX ID + Timestamp */}
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
            •{" "}
            {payment.confirmedAt
              ? formatTimestamp(payment.confirmedAt)
              : payment.createdAt
                ? formatTimestamp(payment.createdAt)
                : "Pending"}
          </span>
          {payment.status && payment.status !== "confirmed" && (
            <Badge variant="outline" className="text-[10px] px-1.5 py-0">
              {payment.status}
            </Badge>
          )}
        </div>
      </div>
    </div>
  );
}
