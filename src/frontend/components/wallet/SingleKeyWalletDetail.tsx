import { useState, useMemo } from "react";
import {
  Send,
  Download,
  RefreshCw,
  Loader2,
  ChevronLeft,
  ChevronRight,
  ChevronsLeft,
  ChevronsRight,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { CopyButton } from "@/components/shared/CopyButton";
import { EmptyState } from "@/components/feedback/EmptyState";
import { formatAmount } from "@/components/shared/AmountInput";
import type { SingleKeyWalletDto, UtxoDto } from "@/bindings";

// ─── Constants ─────────────────────────────────────────────────────

const DUFFS_DECIMAL_PLACES = 8;
const UTXOS_PER_PAGE = 50;

// ─── Props ─────────────────────────────────────────────────────────

interface SingleKeyWalletDetailProps {
  /** The single-key wallet to display */
  wallet: SingleKeyWalletDto;
  /** Whether the wallet is currently being refreshed */
  refreshing?: boolean;
  /** Called when the user clicks Send */
  onSend?: () => void;
  /** Called when the user clicks Receive */
  onReceive?: () => void;
  /** Called when the user clicks Refresh */
  onRefresh?: () => void;
  /** Additional CSS class */
  className?: string;
}

// ─── Helpers ───────────────────────────────────────────────────────

function formatDash(duffs: number): string {
  return formatAmount(duffs, DUFFS_DECIMAL_PLACES);
}

// ─── UTXO Card Sub-Component ──────────────────────────────────────

interface UtxoCardProps {
  utxo: UtxoDto;
}

function UtxoCard({ utxo }: UtxoCardProps) {
  return (
    <div className="rounded-md border border-border bg-muted/30 p-3 space-y-1" data-testid="utxo-card">
      <div className="flex items-center gap-1">
        <span className="font-mono text-xs text-muted-foreground truncate" title={`${utxo.txid}:${utxo.vout}`}>
          {utxo.txid}:{utxo.vout}
        </span>
        <CopyButton value={`${utxo.txid}:${utxo.vout}`} />
      </div>
      <div className="font-mono text-sm font-semibold">
        {formatDash(utxo.amount)} DASH
      </div>
    </div>
  );
}

// ─── Pagination Controls Sub-Component ────────────────────────────

interface PaginationControlsProps {
  currentPage: number;
  totalPages: number;
  startIndex: number;
  endIndex: number;
  totalItems: number;
  onFirst: () => void;
  onPrev: () => void;
  onNext: () => void;
  onLast: () => void;
}

function PaginationControls({
  currentPage,
  totalPages,
  startIndex,
  endIndex,
  totalItems,
  onFirst,
  onPrev,
  onNext,
  onLast,
}: PaginationControlsProps) {
  const isFirstPage = currentPage === 0;
  const isLastPage = currentPage >= totalPages - 1;

  return (
    <div className="flex items-center justify-between" role="navigation" aria-label="UTXO pagination">
      <div className="flex items-center gap-1">
        <Button
          variant="outline"
          size="xs"
          onClick={onFirst}
          disabled={isFirstPage}
          aria-label="First page"
        >
          <ChevronsLeft className="size-3.5" />
          First
        </Button>
        <Button
          variant="outline"
          size="xs"
          onClick={onPrev}
          disabled={isFirstPage}
          aria-label="Previous page"
        >
          <ChevronLeft className="size-3.5" />
          Prev
        </Button>
      </div>

      <span className="text-xs text-muted-foreground">
        Page {currentPage + 1} of {totalPages} ({startIndex + 1}-{endIndex} of {totalItems})
      </span>

      <div className="flex items-center gap-1">
        <Button
          variant="outline"
          size="xs"
          onClick={onNext}
          disabled={isLastPage}
          aria-label="Next page"
        >
          Next
          <ChevronRight className="size-3.5" />
        </Button>
        <Button
          variant="outline"
          size="xs"
          onClick={onLast}
          disabled={isLastPage}
          aria-label="Last page"
        >
          Last
          <ChevronsRight className="size-3.5" />
        </Button>
      </div>
    </div>
  );
}

// ─── SingleKeyWalletDetail ─────────────────────────────────────────

export function SingleKeyWalletDetail({
  wallet,
  refreshing = false,
  onSend,
  onReceive,
  onRefresh,
  className,
}: SingleKeyWalletDetailProps) {
  const [utxoPage, setUtxoPage] = useState(0);

  // Computed values
  const displayName = wallet.alias?.trim() || "Unnamed Key";

  // Sort UTXOs by amount descending for consistent display
  const sortedUtxos = useMemo(() => {
    return [...wallet.utxos].sort((a, b) => b.amount - a.amount);
  }, [wallet.utxos]);

  // Pagination
  const totalUtxos = sortedUtxos.length;
  const totalPages = Math.max(1, Math.ceil(totalUtxos / UTXOS_PER_PAGE));

  // Auto-correct page if out of bounds
  const effectivePage = Math.min(utxoPage, totalPages - 1);
  if (effectivePage !== utxoPage) {
    setUtxoPage(effectivePage);
  }

  const startIndex = effectivePage * UTXOS_PER_PAGE;
  const endIndex = Math.min(startIndex + UTXOS_PER_PAGE, totalUtxos);
  const pageUtxos = sortedUtxos.slice(startIndex, endIndex);

  return (
    <div className={cn("flex flex-col gap-4", className)}>
      {/* Header Section */}
      <div className="space-y-2">
        <div className="flex items-center justify-between">
          <h2 className="text-xl font-bold truncate" title={displayName}>
            {displayName}
          </h2>
          {refreshing && (
            <Loader2
              className="size-5 animate-spin text-primary"
              aria-label="Refreshing wallet"
            />
          )}
        </div>

        {/* Address */}
        <div className="flex items-center gap-1">
          <span className="font-mono text-sm text-muted-foreground truncate" title={wallet.address}>
            {wallet.address}
          </span>
          <CopyButton value={wallet.address} />
        </div>

        {/* Balance */}
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-sm">
          <div>
            <span className="text-muted-foreground">Balance: </span>
            <span className="font-mono font-medium">
              {formatDash(wallet.totalBalance)} DASH
            </span>
          </div>
          {wallet.unconfirmedBalance > 0 && (
            <Badge variant="outline" className="text-xs">
              +{formatDash(wallet.unconfirmedBalance)} pending
            </Badge>
          )}
        </div>
      </div>

      {/* Action Bar */}
      <div className="flex items-center gap-2 flex-wrap">
        {onSend && (
          <Button size="sm" onClick={onSend}>
            <Send className="size-3.5" />
            Send
          </Button>
        )}
        {onReceive && (
          <Button variant="outline" size="sm" onClick={onReceive}>
            <Download className="size-3.5" />
            Receive
          </Button>
        )}
        {onRefresh && (
          <Button
            variant="outline"
            size="sm"
            onClick={onRefresh}
            disabled={refreshing}
          >
            <RefreshCw
              className={cn("size-3.5", refreshing && "animate-spin")}
            />
            Refresh
          </Button>
        )}
      </div>

      {/* UTXO Section */}
      <div className="space-y-3">
        <h3 className="text-sm font-semibold text-muted-foreground">
          UTXOs ({totalUtxos})
        </h3>

        {totalUtxos === 0 ? (
          <EmptyState
            title="No UTXOs available"
            description="Click 'Refresh' to load UTXOs from Core."
          />
        ) : (
          <>
            {/* Pagination Controls (top) */}
            {totalPages > 1 && (
              <PaginationControls
                currentPage={effectivePage}
                totalPages={totalPages}
                startIndex={startIndex}
                endIndex={endIndex}
                totalItems={totalUtxos}
                onFirst={() => setUtxoPage(0)}
                onPrev={() => setUtxoPage((p) => Math.max(0, p - 1))}
                onNext={() => setUtxoPage((p) => Math.min(totalPages - 1, p + 1))}
                onLast={() => setUtxoPage(totalPages - 1)}
              />
            )}

            {/* UTXO Cards */}
            <div className="space-y-2 max-h-[400px] overflow-auto">
              {pageUtxos.map((utxo) => (
                <UtxoCard key={`${utxo.txid}:${utxo.vout}`} utxo={utxo} />
              ))}
            </div>

            {/* Pagination Controls (bottom) — only if many pages */}
            {totalPages > 2 && (
              <PaginationControls
                currentPage={effectivePage}
                totalPages={totalPages}
                startIndex={startIndex}
                endIndex={endIndex}
                totalItems={totalUtxos}
                onFirst={() => setUtxoPage(0)}
                onPrev={() => setUtxoPage((p) => Math.max(0, p - 1))}
                onNext={() => setUtxoPage((p) => Math.min(totalPages - 1, p + 1))}
                onLast={() => setUtxoPage(totalPages - 1)}
              />
            )}
          </>
        )}
      </div>
    </div>
  );
}
