import { useState, useMemo } from "react";
import {
  ArrowUpDown,
  ArrowUp,
  ArrowDown,
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

export interface MyTokensTableProps {
  /** Token entries to display (sorted by the store). */
  tokens: TokenEntry[];
  /** Current sort column. */
  sortColumn: TokenSortColumn;
  /** Current sort direction. */
  sortOrder: TokenSortOrder;
  /** Called when a sort column header is clicked. */
  onSortChange: (column: TokenSortColumn) => void;
  /** Called when an action is triggered on a token. */
  onAction: (tokenId: string, action: TokenAction) => void;
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
  onRemove,
}: MyTokensTableProps) {
  const [removeDialogOpen, setRemoveDialogOpen] = useState(false);
  const [tokenToRemove, setTokenToRemove] = useState<TokenEntry | null>(null);

  const handleAction = (token: TokenEntry, action: TokenAction) => {
    if (action === "remove") {
      setTokenToRemove(token);
      setRemoveDialogOpen(true);
      return;
    }
    onAction(token.tokenId, action);
  };

  const handleConfirmRemove = () => {
    if (tokenToRemove) {
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
      <div className="w-full">
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
                  Owner Identity
                  <SortIndicator
                    column="ownerAlias"
                    activeColumn={sortColumn}
                    sortOrder={sortOrder}
                  />
                </Button>
              </TableHead>
              <TableHead>
                <Button
                  variant="ghost"
                  size="sm"
                  className="-ml-3 h-8 font-semibold"
                  onClick={() => onSortChange("name")}
                >
                  Token Name
                  <SortIndicator
                    column="name"
                    activeColumn={sortColumn}
                    sortOrder={sortOrder}
                  />
                </Button>
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
            {tokens.map((token) => (
              <TokenRow
                key={`${token.tokenId}-${token.identityId}`}
                token={token}
                onAction={handleAction}
              />
            ))}
          </TableBody>
        </Table>
      </div>

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

// ─── Token row ──────────────────────────────────────────────────────

function TokenRow({
  token,
  onAction,
}: {
  token: TokenEntry;
  onAction: (token: TokenEntry, action: TokenAction) => void;
}) {
  const formattedBalance = useMemo(
    () => formatTokenBalance(token.balance, token.decimals),
    [token.balance, token.decimals],
  );

  return (
    <TableRow>
      {/* Owner Identity / Alias */}
      <TableCell>
        <div className="flex flex-col gap-0.5">
          {token.ownerAlias ? (
            <>
              <span className="text-sm font-medium">{token.ownerAlias}</span>
              <Tooltip>
                <TooltipTrigger asChild>
                  <span className="text-xs text-muted-foreground font-mono cursor-default">
                    {truncateId(token.identityId)}
                  </span>
                </TooltipTrigger>
                <TooltipContent>
                  <p className="font-mono text-xs">{token.identityId}</p>
                </TooltipContent>
              </Tooltip>
            </>
          ) : (
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="text-sm font-mono cursor-default">
                  {truncateId(token.identityId)}
                </span>
              </TooltipTrigger>
              <TooltipContent>
                <p className="font-mono text-xs">{token.identityId}</p>
              </TooltipContent>
            </Tooltip>
          )}
        </div>
      </TableCell>

      {/* Token Name */}
      <TableCell>
        <span className="text-sm font-medium">
          {token.name ?? "Unnamed Token"}
        </span>
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
              aria-label={`Actions for ${token.name ?? "token"}`}
            >
              <MoreVertical className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-52">
            {ACTION_MENU_ITEMS.map((item) => (
              <ActionMenuEntry
                key={item.action}
                item={item}
                onClick={() => onAction(token, item.action)}
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

export { formatTokenBalance, truncateId };
