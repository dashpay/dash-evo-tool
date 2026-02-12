import React, { useState, useMemo, useCallback } from "react";
import {
  DndContext,
  closestCenter,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import {
  SortableContext,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import {
  restrictToVerticalAxis,
  restrictToParentElement,
} from "@dnd-kit/modifiers";
import {
  ArrowUpDown,
  ArrowUp,
  ArrowDown,
  ArrowLeft,
  GripVertical,
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
  Calculator,
} from "lucide-react";
import { cn, displayId, hexToBase58 } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
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
import { LoadingSpinner } from "@/components/feedback";
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

/** Reward estimation data for a specific identity+token combination. */
export interface RewardEstimate {
  /** The estimated reward amount (formatted string). */
  amount: string;
  /** Full explanation text from the backend. */
  explanation: string;
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
  /** Whether to show the Rewards column in the Level 2 detail view. */
  showRewardsColumn?: boolean;
  /** Map of "identityId:tokenId" → reward estimate data. */
  rewardEstimates?: Map<string, RewardEstimate>;
  /** Set of "identityId:tokenId" keys currently being estimated. */
  estimatingRewards?: Set<string>;
  /** Called to estimate rewards for a specific identity+token. */
  onEstimateRewards?: (identityId: string, tokenId: string) => void;
  /** Called when a token is drag-and-dropped to a new position in the Level 1 list. */
  onReorder?: (activeTokenId: string, overTokenId: string) => Promise<void>;
}

// ─── Helpers ────────────────────────────────────────────────────────

/** Format a BigInt-style balance string with the given decimal places. */
function formatTokenBalance(balance: string | number, decimals: number): string {
  const balanceStr = String(balance);
  if (!balanceStr || balanceStr === "0") return "0";
  if (decimals === 0) return balanceStr;

  // Pad with leading zeros if needed
  const padded = balanceStr.padStart(decimals + 1, "0");
  const intPart = padded.slice(0, padded.length - decimals);
  const fracPart = padded.slice(padded.length - decimals);

  // Trim trailing zeros from fractional part
  const trimmedFrac = fracPart.replace(/0+$/, "");
  if (!trimmedFrac) return intPart;
  return `${intPart}.${trimmedFrac}`;
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
  showRewardsColumn = false,
  rewardEstimates,
  estimatingRewards,
  onEstimateRewards,
  onReorder,
}: MyTokensTableProps) {
  const [selectedTokenId, setSelectedTokenId] = useState<string | null>(null);
  const [removeDialogOpen, setRemoveDialogOpen] = useState(false);
  const [tokenToRemove, setTokenToRemove] = useState<TokenSummary | null>(null);
  const [explanationDialogOpen, setExplanationDialogOpen] = useState(false);
  const [explanationKey, setExplanationKey] = useState<string | null>(null);

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

  const handleShowExplanation = useCallback((identityId: string, tokenId: string) => {
    setExplanationKey(`${identityId}:${tokenId}`);
    setExplanationDialogOpen(true);
  }, []);

  const currentExplanation = explanationKey ? rewardEstimates?.get(explanationKey) : undefined;

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
          showRewardsColumn={showRewardsColumn}
          rewardEstimates={rewardEstimates}
          estimatingRewards={estimatingRewards}
          onEstimateRewards={onEstimateRewards}
          onShowExplanation={handleShowExplanation}
        />
      ) : (
        <TokenListView
          summaries={tokenSummaries}
          onDrillDown={handleDrillDown}
          onAction={handleTokenLevelAction}
          onReorder={onReorder}
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

      <RewardExplanationDialog
        open={explanationDialogOpen}
        onOpenChange={setExplanationDialogOpen}
        estimate={currentExplanation ?? null}
      />
    </>
  );
}

// ─── Level 1: Token List ─────────────────────────────────────────────

function TokenListView({
  summaries,
  onDrillDown,
  onAction,
  onReorder,
}: {
  summaries: TokenSummary[];
  onDrillDown: (tokenId: string) => void;
  onAction: (summary: TokenSummary, action: "moreInfo" | "remove") => void;
  onReorder?: (activeTokenId: string, overTokenId: string) => Promise<void>;
}) {
  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: { distance: 8 },
    }),
    useSensor(KeyboardSensor),
  );

  const tokenIds = useMemo(
    () => summaries.map((s) => s.tokenId),
    [summaries],
  );

  const handleDragEnd = useCallback(
    (event: DragEndEvent) => {
      const { active, over } = event;
      if (!over || active.id === over.id) return;
      if (onReorder) {
        onReorder(String(active.id), String(over.id));
      }
    },
    [onReorder],
  );

  return (
    <div className="w-full">
      <DndContext
        sensors={sensors}
        collisionDetection={closestCenter}
        modifiers={[restrictToVerticalAxis, restrictToParentElement]}
        onDragEnd={handleDragEnd}
      >
        <SortableContext items={tokenIds} strategy={verticalListSortingStrategy}>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-[40px]">
                  <span className="sr-only">Reorder</span>
                </TableHead>
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
                <SortableTokenRow
                  key={summary.tokenId}
                  summary={summary}
                  onDrillDown={onDrillDown}
                  onAction={onAction}
                />
              ))}
            </TableBody>
          </Table>
        </SortableContext>
      </DndContext>
    </div>
  );
}

function SortableTokenRow({
  summary,
  onDrillDown,
  onAction,
}: {
  summary: TokenSummary;
  onDrillDown: (tokenId: string) => void;
  onAction: (summary: TokenSummary, action: "moreInfo" | "remove") => void;
}) {
  const {
    attributes,
    listeners,
    setNodeRef,
    setActivatorNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: summary.tokenId });

  const style: React.CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : undefined,
    zIndex: isDragging ? 10 : undefined,
    position: isDragging ? "relative" : undefined,
  };

  const displayName = summary.name ?? "Unnamed Token";

  return (
    <TableRow ref={setNodeRef} style={style} className={isDragging ? "shadow-lg" : undefined}>
      {/* Drag handle */}
      <TableCell className="w-[40px] px-2">
        <div
          ref={setActivatorNodeRef}
          {...attributes}
          {...listeners}
          className="cursor-grab active:cursor-grabbing touch-none"
          aria-label="Drag to reorder"
        >
          <GripVertical className="h-4 w-4 text-muted-foreground/50 hover:text-muted-foreground" />
        </div>
      </TableCell>

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
              {displayId(summary.tokenId)}
            </span>
          </TooltipTrigger>
          <TooltipContent>
            <p className="font-mono text-xs">{hexToBase58(summary.tokenId)}</p>
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
  showRewardsColumn = false,
  rewardEstimates,
  estimatingRewards,
  onEstimateRewards,
  onShowExplanation,
}: {
  tokenName: string;
  tokenId: string;
  entries: TokenEntry[];
  sortColumn: TokenSortColumn;
  sortOrder: TokenSortOrder;
  onSortChange: (column: TokenSortColumn) => void;
  onBack: () => void;
  onAction: (entry: TokenEntry, action: TokenAction) => void;
  showRewardsColumn?: boolean;
  rewardEstimates?: Map<string, RewardEstimate>;
  estimatingRewards?: Set<string>;
  onEstimateRewards?: (identityId: string, tokenId: string) => void;
  onShowExplanation?: (identityId: string, tokenId: string) => void;
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
                {displayId(tokenId)}
              </span>
            </TooltipTrigger>
            <TooltipContent>
              <p className="font-mono text-xs">{hexToBase58(tokenId)}</p>
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
            {showRewardsColumn && (
              <TableHead className="w-[200px]">
                <span className="text-sm font-semibold">Rewards</span>
              </TableHead>
            )}
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
              showRewardsColumn={showRewardsColumn}
              rewardEstimate={rewardEstimates?.get(`${entry.identityId}:${entry.tokenId}`)}
              isEstimating={estimatingRewards?.has(`${entry.identityId}:${entry.tokenId}`) ?? false}
              onEstimateRewards={onEstimateRewards}
              onShowExplanation={onShowExplanation}
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
  showRewardsColumn = false,
  rewardEstimate,
  isEstimating = false,
  onEstimateRewards,
  onShowExplanation,
}: {
  entry: TokenEntry;
  onAction: (entry: TokenEntry, action: TokenAction) => void;
  showRewardsColumn?: boolean;
  rewardEstimate?: RewardEstimate;
  isEstimating?: boolean;
  onEstimateRewards?: (identityId: string, tokenId: string) => void;
  onShowExplanation?: (identityId: string, tokenId: string) => void;
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
              {displayId(entry.identityId)}
            </span>
          </TooltipTrigger>
          <TooltipContent>
            <p className="font-mono text-xs">{hexToBase58(entry.identityId)}</p>
          </TooltipContent>
        </Tooltip>
      </TableCell>

      {/* Balance (right-aligned) */}
      <TableCell className="text-right">
        <span className="text-sm font-mono tabular-nums">
          {formattedBalance}
        </span>
      </TableCell>

      {/* Rewards column */}
      {showRewardsColumn && (
        <TableCell>
          <div className="flex items-center gap-1.5">
            {rewardEstimate ? (
              <>
                <span className="text-sm font-mono tabular-nums" data-testid="reward-amount">
                  {rewardEstimate.amount}
                </span>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="h-6 w-6"
                      aria-label="Show reward details"
                      onClick={() => onShowExplanation?.(entry.identityId, entry.tokenId)}
                    >
                      <Info className="h-3.5 w-3.5" />
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>Show reward details</TooltipContent>
                </Tooltip>
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-6 px-2 text-xs"
                  disabled={isEstimating}
                  onClick={() => onEstimateRewards?.(entry.identityId, entry.tokenId)}
                >
                  {isEstimating ? (
                    <LoadingSpinner className="h-3 w-3" />
                  ) : (
                    "Estimate"
                  )}
                </Button>
              </>
            ) : (
              <Button
                variant="outline"
                size="sm"
                className="h-7 px-2.5 text-xs gap-1.5"
                disabled={isEstimating}
                onClick={() => onEstimateRewards?.(entry.identityId, entry.tokenId)}
                data-testid="estimate-rewards-button"
              >
                {isEstimating ? (
                  <LoadingSpinner className="h-3 w-3" />
                ) : (
                  <Calculator className="h-3 w-3" />
                )}
                Estimate
              </Button>
            )}
          </div>
        </TableCell>
      )}

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

// ─── Reward Explanation Dialog ───────────────────────────────────────

function RewardExplanationDialog({
  open,
  onOpenChange,
  estimate,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  estimate: RewardEstimate | null;
}) {
  const [expandedSections, setExpandedSections] = useState<Set<string>>(new Set());

  const toggleSection = useCallback((section: string) => {
    setExpandedSections((prev) => {
      const next = new Set(prev);
      if (next.has(section)) {
        next.delete(section);
      } else {
        next.add(section);
      }
      return next;
    });
  }, []);

  if (!estimate) return null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-[600px] max-h-[80vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Reward Estimation Details</DialogTitle>
          <DialogDescription>
            Estimated perpetual distribution rewards for this identity.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          {/* Total */}
          <div className="rounded-md border bg-muted/30 p-3">
            <span className="text-sm text-muted-foreground">Total Estimated Rewards</span>
            <p className="text-lg font-semibold font-mono mt-0.5" data-testid="reward-total">
              {estimate.amount}
            </p>
          </div>

          {/* Full explanation */}
          <div>
            <button
              type="button"
              className="flex items-center gap-1.5 text-sm font-medium hover:underline"
              onClick={() => toggleSection("explanation")}
            >
              {expandedSections.has("explanation") ? "▾" : "▸"} Explanation
            </button>
            {expandedSections.has("explanation") && (
              <div
                className="mt-2 rounded-md border bg-muted/20 p-3 text-sm whitespace-pre-wrap font-mono"
                data-testid="reward-explanation-text"
              >
                {estimate.explanation}
              </div>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

// ─── Exports ────────────────────────────────────────────────────────

export { formatTokenBalance, groupTokens };
