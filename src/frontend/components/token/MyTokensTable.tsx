import { useState, useMemo, useCallback } from "react";
import {
  ArrowUpDown,
  ArrowUp,
  ArrowDown,
  ArrowLeft,
  MoreVertical,
  Send,
  Coins,
  Flame,
  Snowflake,
  Sun,
  Pause,
  Play,
  Gift,
  Eye,
  Tag,
  ShoppingCart,
  Settings,
  Trash2,
  Info,
  X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { EmptyState } from "@/components/feedback/EmptyState";
import { ConfirmationDialog } from "@/components/shared/ConfirmationDialog";
import type { TokenEntry } from "@/stores/tokenStore";
import type { TokenSortColumn, TokenSortOrder } from "@/stores/tokenStore";

// ─── Types ──────────────────────────────────────────────────────────

export type TokenAction =
  | "transfer"
  | "mint"
  | "burn"
  | "freeze"
  | "unfreeze"
  | "pause"
  | "resume"
  | "claim"
  | "viewClaims"
  | "setPrice"
  | "purchase"
  | "updateConfig"
  | "destroyFrozen"
  | "moreInfo"
  | "remove";

/** A summary of a unique token for the Level 1 list. */
export interface TokenSummary {
  tokenId: string;
  contractId: string;
  tokenPosition: number;
  name: string | null;
  decimals: number;
  /** Number of identities holding this token. */
  identityCount: number;
}

export interface MyTokensTableProps {
  /** Token entries to display (sorted by the store). */
  tokens: TokenEntry[];
  /** Current sort column. */
  sortColumn: TokenSortColumn;
  /** Current sort direction. */
  sortOrder: TokenSortOrder;
  /** Called when a sort column header is clicked. */
  onSortChange: (column: TokenSortColumn) => void;
  /** Called when an action is triggered on a token entry (includes identityId context). */
  onAction: (entry: TokenEntry, action: TokenAction) => void;
  /** Called when "More Info" is triggered (token-level, not identity-specific). */
  onMoreInfo: (tokenId: string) => void;
  /** Called to remove a token (after confirmation). */
  onRemove: (tokenId: string) => void;
}

// ─── Helpers ────────────────────────────────────────────────────────

/** Format a BigInt-style balance string with the given decimal places. */
function formatTokenBalance(balance: string, decimals: number): string {
  if (!balance || balance === "0") return "0";
  if (decimals === 0) return balance;

  // Pad with leading zeros if needed
  const padded = balance.padStart(decimals + 1, "0");
  const intPart = padded.slice(0, padded.length - decimals);
  const fracPart = padded.slice(padded.length - decimals);

  // Trim trailing zeros from fractional part
  const trimmedFrac = fracPart.replace(/0+$/, "");
  if (!trimmedFrac) return intPart;
  return `${intPart}.${trimmedFrac}`;
}

/** Truncate a hex string for display. */
function truncateId(id: string, chars = 8): string {
  if (id.length <= chars * 2 + 3) return id;
  return `${id.slice(0, chars)}...${id.slice(-chars)}`;
}

/** Group token entries by tokenId into summaries. */
function groupTokens(tokens: TokenEntry[]): TokenSummary[] {
  const map = new Map<string, TokenSummary>();
  for (const t of tokens) {
    if (!map.has(t.tokenId)) {
      map.set(t.tokenId, {
        tokenId: t.tokenId,
        contractId: t.contractId,
        tokenPosition: t.tokenPosition,
        name: t.name,
        decimals: t.decimals,
        identityCount: 0,
      });
    }
    map.get(t.tokenId)!.identityCount++;
  }
  return Array.from(map.values());
}

// ─── Sort indicator ─────────────────────────────────────────────────

function SortIndicator({
  column,
  activeColumn,
  sortOrder,
}: {
  column: TokenSortColumn;
  activeColumn: TokenSortColumn;
  sortOrder: TokenSortOrder;
}) {
  if (column !== activeColumn) {
    return <ArrowUpDown className="ml-1 h-3.5 w-3.5 text-muted-foreground/50" />;
  }
  return sortOrder === "ascending" ? (
    <ArrowUp className="ml-1 h-3.5 w-3.5" />
  ) : (
    <ArrowDown className="ml-1 h-3.5 w-3.5" />
  );
}

// ─── Action menu items config ───────────────────────────────────────

interface ActionMenuItem {
  action: TokenAction;
  label: string;
  icon: typeof Send;
  separatorBefore?: boolean;
  danger?: boolean;
}

const ACTION_MENU_ITEMS: ActionMenuItem[] = [
  { action: "transfer", label: "Transfer", icon: Send },
  { action: "mint", label: "Mint", icon: Coins },
  { action: "burn", label: "Burn", icon: Flame },
  { action: "freeze", label: "Freeze", icon: Snowflake, separatorBefore: true },
  { action: "unfreeze", label: "Unfreeze", icon: Sun },
  { action: "destroyFrozen", label: "Destroy Frozen Funds", icon: Trash2 },
  { action: "pause", label: "Pause", icon: Pause, separatorBefore: true },
  { action: "resume", label: "Resume", icon: Play },
  { action: "claim", label: "Claim", icon: Gift, separatorBefore: true },
  { action: "viewClaims", label: "View Claims", icon: Eye },
  { action: "setPrice", label: "Set Price", icon: Tag, separatorBefore: true },
  { action: "purchase", label: "Purchase", icon: ShoppingCart },
  { action: "updateConfig", label: "Update Config", icon: Settings, separatorBefore: true },
  { action: "moreInfo", label: "More Info", icon: Info, separatorBefore: true },
  { action: "remove", label: "Remove", icon: X, separatorBefore: true, danger: true },
];

// ─── Component ──────────────────────────────────────────────────────

export function MyTokensTable({
  tokens,
  sortColumn,
  sortOrder,
  onSortChange,
  onAction,
  onMoreInfo,
  onRemove,
}: MyTokensTableProps) {
  const [selectedTokenId, setSelectedTokenId] = useState<string | null>(null);
  const [removeDialogOpen, setRemoveDialogOpen] = useState(false);
  const [tokenToRemove, setTokenToRemove] = useState<TokenSummary | null>(null);

  // Group tokens by tokenId for Level 1
  const tokenSummaries = useMemo(() => groupTokens(tokens), [tokens]);

  // Get entries for the selected token (Level 2)
  const selectedTokenEntries = useMemo(() => {
    if (!selectedTokenId) return [];
    return tokens.filter((t) => t.tokenId === selectedTokenId);
  }, [tokens, selectedTokenId]);

  const selectedTokenName = useMemo(() => {
    if (!selectedTokenId) return null;
    const summary = tokenSummaries.find((s) => s.tokenId === selectedTokenId);
    return summary?.name ?? "Unnamed Token";
  }, [selectedTokenId, tokenSummaries]);

  const handleDrillDown = useCallback((tokenId: string) => {
    setSelectedTokenId(tokenId);
  }, []);

  const handleBack = useCallback(() => {
    setSelectedTokenId(null);
  }, []);

  const handleEntryAction = useCallback(
    (entry: TokenEntry, action: TokenAction) => {
      if (action === "moreInfo") {
        onMoreInfo(entry.tokenId);
        return;
      }
      if (action === "remove") {
        // Remove at token level — find summary for dialog
        const summary = tokenSummaries.find((s) => s.tokenId === entry.tokenId);
        if (summary) {
          setTokenToRemove(summary);
          setRemoveDialogOpen(true);
        }
        return;
      }
      onAction(entry, action);
    },
    [onAction, onMoreInfo, tokenSummaries],
  );

  const handleTokenLevelAction = useCallback(
    (summary: TokenSummary, action: "moreInfo" | "remove") => {
      if (action === "moreInfo") {
        onMoreInfo(summary.tokenId);
        return;
      }
      if (action === "remove") {
        setTokenToRemove(summary);
        setRemoveDialogOpen(true);
      }
    },
    [onMoreInfo],
  );

  const handleConfirmRemove = () => {
    if (tokenToRemove) {
      // If we're viewing the removed token's detail, go back
      if (selectedTokenId === tokenToRemove.tokenId) {
        setSelectedTokenId(null);
      }
      onRemove(tokenToRemove.tokenId);
    }
    setTokenToRemove(null);
    setRemoveDialogOpen(false);
  };

  // Empty state
  if (tokens.length === 0) {
    return (
      <EmptyState
        icon={Coins}
        title="No tokens yet"
        description="Add a token by ID or create a new one to get started."
      />
    );
  }

  return (
    <>
      {selectedTokenId ? (
        <TokenDetailView
          tokenName={selectedTokenName!}
          tokenId={selectedTokenId}
          entries={selectedTokenEntries}
          sortColumn={sortColumn}
          sortOrder={sortOrder}
          onSortChange={onSortChange}
          onBack={handleBack}
          onAction={handleEntryAction}
        />
      ) : (
        <TokenListView
          summaries={tokenSummaries}
          onDrillDown={handleDrillDown}
          onAction={handleTokenLevelAction}
        />
      )}

      <ConfirmationDialog
        open={removeDialogOpen}
        onOpenChange={setRemoveDialogOpen}
        title="Confirm Remove Token"
        message={`Are you sure you want to stop tracking the token "${tokenToRemove?.name ?? "Unknown"}"? You can re-add it later. Your actual token balance will not change with this action.`}
        confirmText="Confirm"
        cancelText="Cancel"
        danger
        onResult={(status) => {
          if (status === "confirmed") {
            handleConfirmRemove();
          } else {
            setTokenToRemove(null);
          }
        }}
      />
    </>
  );
}

// ─── Level 1: Token List ─────────────────────────────────────────────

function TokenListView({
  summaries,
  onDrillDown,
  onAction,
}: {
  summaries: TokenSummary[];
  onDrillDown: (tokenId: string) => void;
  onAction: (summary: TokenSummary, action: "moreInfo" | "remove") => void;
}) {
  return (
    <div className="w-full">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Token Name</TableHead>
            <TableHead className="w-[200px]">
              <span className="text-sm font-semibold">Token ID</span>
            </TableHead>
            <TableHead className="w-[100px] text-right">
              <span className="text-sm font-semibold">Identities</span>
            </TableHead>
            <TableHead className="w-[100px]">
              <span className="sr-only">Actions</span>
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {summaries.map((summary) => (
            <TokenSummaryRow
              key={summary.tokenId}
              summary={summary}
              onDrillDown={onDrillDown}
              onAction={onAction}
            />
          ))}
        </TableBody>
      </Table>
    </div>
  );
}

function TokenSummaryRow({
  summary,
  onDrillDown,
  onAction,
}: {
  summary: TokenSummary;
  onDrillDown: (tokenId: string) => void;
  onAction: (summary: TokenSummary, action: "moreInfo" | "remove") => void;
}) {
  const displayName = summary.name ?? "Unnamed Token";

  return (
    <TableRow>
      {/* Token Name (clickable → drill down) */}
      <TableCell>
        <Button
          variant="link"
          className="h-auto p-0 text-sm font-medium text-foreground hover:text-dash-blue"
          onClick={() => onDrillDown(summary.tokenId)}
        >
          {displayName}
        </Button>
      </TableCell>

      {/* Token ID */}
      <TableCell>
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="text-sm font-mono text-muted-foreground cursor-default">
              {truncateId(summary.tokenId)}
            </span>
          </TooltipTrigger>
          <TooltipContent>
            <p className="font-mono text-xs">{summary.tokenId}</p>
          </TooltipContent>
        </Tooltip>
      </TableCell>

      {/* Identity count */}
      <TableCell className="text-right">
        <span className="text-sm text-muted-foreground tabular-nums">
          {summary.identityCount}
        </span>
      </TableCell>

      {/* Actions: More Info + Remove */}
      <TableCell className="text-right">
        <div className="flex items-center justify-end gap-1">
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="h-8 w-8"
                aria-label={`More info for ${displayName}`}
                onClick={() => onAction(summary, "moreInfo")}
              >
                <Info className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>More Info</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="h-8 w-8 text-muted-foreground hover:text-destructive"
                aria-label={`Remove ${displayName}`}
                onClick={() => onAction(summary, "remove")}
              >
                <X className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Remove</TooltipContent>
          </Tooltip>
        </div>
      </TableCell>
    </TableRow>
  );
}

// ─── Level 2: Token Detail (Per-Identity Balances) ───────────────────

function TokenDetailView({
  tokenName,
  tokenId,
  entries,
  sortColumn,
  sortOrder,
  onSortChange,
  onBack,
  onAction,
}: {
  tokenName: string;
  tokenId: string;
  entries: TokenEntry[];
  sortColumn: TokenSortColumn;
  sortOrder: TokenSortOrder;
  onSortChange: (column: TokenSortColumn) => void;
  onBack: () => void;
  onAction: (entry: TokenEntry, action: TokenAction) => void;
}) {
  return (
    <div className="w-full">
      {/* Back button + token header */}
      <div className="flex items-center gap-3 mb-3 px-1">
        <Button
          variant="ghost"
          size="sm"
          className="h-8 gap-1.5"
          onClick={onBack}
        >
          <ArrowLeft className="h-4 w-4" />
          Back
        </Button>
        <div className="flex items-center gap-2">
          <Coins className="h-4 w-4 text-dash-blue" />
          <span className="text-sm font-semibold">{tokenName}</span>
          <Tooltip>
            <TooltipTrigger asChild>
              <span className="text-xs font-mono text-muted-foreground cursor-default">
                {truncateId(tokenId)}
              </span>
            </TooltipTrigger>
            <TooltipContent>
              <p className="font-mono text-xs">{tokenId}</p>
            </TooltipContent>
          </Tooltip>
        </div>
      </div>

      <Table>
        <TableHeader>
          <TableRow>
            <TableHead className="w-[200px]">
              <Button
                variant="ghost"
                size="sm"
                className="-ml-3 h-8 font-semibold"
                onClick={() => onSortChange("ownerAlias")}
              >
                Identity
                <SortIndicator
                  column="ownerAlias"
                  activeColumn={sortColumn}
                  sortOrder={sortOrder}
                />
              </Button>
            </TableHead>
            <TableHead>
              <span className="text-sm font-semibold">Identity ID</span>
            </TableHead>
            <TableHead className="w-[160px] text-right">
              <Button
                variant="ghost"
                size="sm"
                className="-mr-3 ml-auto h-8 font-semibold"
                onClick={() => onSortChange("balance")}
              >
                Balance
                <SortIndicator
                  column="balance"
                  activeColumn={sortColumn}
                  sortOrder={sortOrder}
                />
              </Button>
            </TableHead>
            <TableHead className="w-[60px]">
              <span className="sr-only">Actions</span>
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {entries.map((entry) => (
            <IdentityBalanceRow
              key={entry.identityId}
              entry={entry}
              onAction={onAction}
            />
          ))}
        </TableBody>
      </Table>
    </div>
  );
}

function IdentityBalanceRow({
  entry,
  onAction,
}: {
  entry: TokenEntry;
  onAction: (entry: TokenEntry, action: TokenAction) => void;
}) {
  const formattedBalance = useMemo(
    () => formatTokenBalance(entry.balance, entry.decimals),
    [entry.balance, entry.decimals],
  );

  return (
    <TableRow>
      {/* Identity Alias */}
      <TableCell>
        {entry.ownerAlias ? (
          <span className="text-sm font-medium">{entry.ownerAlias}</span>
        ) : (
          <span className="text-sm text-muted-foreground">—</span>
        )}
      </TableCell>

      {/* Identity ID */}
      <TableCell>
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="text-sm font-mono text-muted-foreground cursor-default">
              {truncateId(entry.identityId)}
            </span>
          </TooltipTrigger>
          <TooltipContent>
            <p className="font-mono text-xs">{entry.identityId}</p>
          </TooltipContent>
        </Tooltip>
      </TableCell>

      {/* Balance (right-aligned) */}
      <TableCell className="text-right">
        <span className="text-sm font-mono tabular-nums">
          {formattedBalance}
        </span>
      </TableCell>

      {/* Actions dropdown */}
      <TableCell className="text-center">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className="h-8 w-8"
              aria-label={`Actions for ${entry.ownerAlias ?? "identity"} ${entry.name ?? "token"}`}
            >
              <MoreVertical className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-52">
            {ACTION_MENU_ITEMS.map((item) => (
              <ActionMenuEntry
                key={item.action}
                item={item}
                onClick={() => onAction(entry, item.action)}
              />
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </TableCell>
    </TableRow>
  );
}

// ─── Action menu entry ──────────────────────────────────────────────

function ActionMenuEntry({
  item,
  onClick,
}: {
  item: ActionMenuItem;
  onClick: () => void;
}) {
  const Icon = item.icon;
  return (
    <>
      {item.separatorBefore && <DropdownMenuSeparator />}
      <DropdownMenuItem
        onClick={onClick}
        className={cn(item.danger && "text-destructive focus:text-destructive")}
      >
        <Icon className="mr-2 h-4 w-4" />
        {item.label}
      </DropdownMenuItem>
    </>
  );
}

// ─── Exports ────────────────────────────────────────────────────────

export { formatTokenBalance, truncateId, groupTokens };
