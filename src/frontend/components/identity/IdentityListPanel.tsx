import React, { useState, useCallback, useRef, useEffect, useMemo } from "react";
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
  User,
  Shield,
  Server,
  GripVertical,
  MoreVertical,
  Pencil,
  Trash2,
  ArrowUpFromLine,
  ArrowDownToLine,
  ArrowLeftRight,
  ArrowUpDown,
  Tag,
  Key,
  RefreshCw,
  UserPlus,
  Loader2,
  Check,
  Wallet,
  Plus,
  Search,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
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
import { EmptyState } from "@/components/feedback/EmptyState";
import { ConfirmationDialog } from "@/components/shared/ConfirmationDialog";
import { CopyButton } from "@/components/shared/CopyButton";
import { formatAmount } from "@/components/shared/AmountInput";
import type {
  QualifiedIdentityDto,
  IdentityTypeDto,
  IdentityStatusDto,
} from "@/bindings";
import type { IdentitySortColumn, IdentitySortOrder } from "@/stores/identityStore";

// ─── Constants ─────────────────────────────────────────────────────

const MAX_ALIAS_LENGTH = 64;
const CREDITS_DECIMAL_PLACES = 8;
const MIN_WITHDRAW_BALANCE = 500_000_000;
const MIN_TRANSFER_BALANCE = 20_000_000;

// ─── Props ─────────────────────────────────────────────────────────

export interface IdentityListPanelProps {
  /** Identities to display (already sorted/ordered by the store). */
  identities: QualifiedIdentityDto[];
  /** Currently selected identity ID. */
  selectedIdentityId: string | null;
  /** Set of identity IDs currently being refreshed. */
  refreshingIds: Set<string>;
  /** Whether a bulk refresh is in progress. */
  refreshingAll: boolean;
  /** Called when an identity is selected. */
  onSelectIdentity: (identityId: string | null) => void;
  /** Called to set/clear an alias. */
  onSetAlias: (identityId: string, alias: string | null) => Promise<void>;
  /** Called when an identity is drag-and-dropped to a new position. */
  onReorder?: (activeId: string, overId: string) => Promise<void>;
  /** Called to remove an identity. */
  onRemoveIdentity: (identityId: string) => Promise<void>;
  /** Called to refresh a single identity. */
  onRefreshIdentity: (identityId: string) => Promise<void>;
  /** Called to refresh all identities. */
  onRefreshAll: () => Promise<void>;
  /** Called to navigate to identity keys. */
  onViewKeys?: (identityId: string) => void;
  /** Called to navigate to DPNS registration. */
  onRegisterDpns?: (identityId: string) => void;
  /** Called to navigate to top-up. */
  onTopUp?: (identityId: string) => void;
  /** Called to navigate to withdraw. */
  onWithdraw?: (identityId: string) => void;
  /** Called to navigate to transfer. */
  onTransfer?: (identityId: string) => void;
  /** Map of wallet seed/key hashes to display names. */
  walletNames?: Record<string, string>;
  /** Current sort column. */
  sortColumn?: IdentitySortColumn;
  /** Current sort order. */
  sortOrder?: IdentitySortOrder;
  /** Whether custom ordering is active (overrides column sort). */
  useCustomOrder?: boolean;
  /** Called when the user selects a sort column. */
  onSortChange?: (column: IdentitySortColumn) => void;
  /** Called to navigate to create identity. */
  onCreateIdentity?: () => void;
  /** Called to navigate to load identity. */
  onLoadIdentity?: () => void;
  /** Additional CSS class. */
  className?: string;
}

// ─── Helpers ───────────────────────────────────────────────────────

function formatCreditsBalance(credits: number): string {
  return formatAmount(credits, CREDITS_DECIMAL_PLACES);
}

function getIdentityDisplayName(identity: QualifiedIdentityDto): string {
  return identity.alias?.trim() || truncateId(identity.id);
}

function truncateId(id: string): string {
  if (id.length <= 12) return id;
  return `${id.slice(0, 6)}...${id.slice(-6)}`;
}

function IdentityTypeIcon({
  type,
  className,
}: {
  type: IdentityTypeDto;
  className?: string;
}) {
  switch (type) {
    case "user":
      return <User className={className} aria-hidden="true" />;
    case "masternode":
      return <Shield className={className} aria-hidden="true" />;
    case "evonode":
      return <Server className={className} aria-hidden="true" />;
  }
}

function getTypeLabel(type: IdentityTypeDto): string {
  switch (type) {
    case "user":
      return "User";
    case "masternode":
      return "Masternode";
    case "evonode":
      return "Evonode";
  }
}

function getStatusColor(status: IdentityStatusDto): string {
  switch (status) {
    case "active":
      return "text-green-600 dark:text-green-400";
    case "pendingCreation":
      return "text-yellow-600 dark:text-yellow-400";
    case "unknown":
      return "text-muted-foreground";
    case "notFound":
    case "failedCreation":
      return "text-destructive";
  }
}

function getStatusLabel(status: IdentityStatusDto): string {
  switch (status) {
    case "active":
      return "Active";
    case "pendingCreation":
      return "Pending";
    case "unknown":
      return "Unknown";
    case "notFound":
      return "Not Found";
    case "failedCreation":
      return "Failed";
  }
}

function isActive(identity: QualifiedIdentityDto): boolean {
  return identity.status === "active";
}

const SORT_COLUMNS: { value: IdentitySortColumn; label: string }[] = [
  { value: "alias", label: "Alias" },
  { value: "balance", label: "Balance" },
  { value: "type", label: "Type" },
  { value: "identityId", label: "Identity ID" },
  { value: "inWallet", label: "Wallet" },
];

// ─── Inline alias editor ──────────────────────────────────────────

interface InlineAliasEditProps {
  currentAlias: string | null;
  onSave: (alias: string | null) => void;
  onCancel: () => void;
}

function InlineAliasEdit({
  currentAlias,
  onSave,
  onCancel,
}: InlineAliasEditProps) {
  const [value, setValue] = useState(currentAlias ?? "");
  const inputRef = useRef<HTMLInputElement>(null);
  const mountedRef = useRef(false);

  useEffect(() => {
    const timer = setTimeout(() => {
      mountedRef.current = true;
      inputRef.current?.focus();
      inputRef.current?.select();
    }, 10);
    return () => clearTimeout(timer);
  }, []);

  const handleSave = () => {
    const trimmed = value.trim();
    onSave(trimmed || null);
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      handleSave();
    } else if (e.key === "Escape") {
      e.preventDefault();
      onCancel();
    }
  };

  return (
    <Input
      ref={inputRef}
      value={value}
      onChange={(e) => setValue(e.target.value.slice(0, MAX_ALIAS_LENGTH))}
      onKeyDown={handleKeyDown}
      onBlur={() => {
        if (mountedRef.current) {
          handleSave();
        }
      }}
      placeholder="Enter alias..."
      className="h-7 text-sm"
      aria-label="Identity alias"
    />
  );
}

// ─── Identity Card ────────────────────────────────────────────────

interface IdentityCardProps {
  identity: QualifiedIdentityDto;
  isSelected: boolean;
  isRefreshing: boolean;
  walletNames?: Record<string, string>;
  onSelect: () => void;
  onSetAlias: (alias: string | null) => void;
  onRemove: () => void;
  onRefresh: () => void;
  onViewKeys?: () => void;
  onRegisterDpns?: () => void;
  onTopUp?: () => void;
  onWithdraw?: () => void;
  onTransfer?: () => void;
}

function SortableIdentityCard(props: IdentityCardProps) {
  const {
    attributes,
    listeners,
    setNodeRef,
    setActivatorNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: props.identity.id });

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : undefined,
    zIndex: isDragging ? 10 : undefined,
    position: (isDragging ? "relative" : undefined) as "relative" | undefined,
  };

  return (
    <IdentityCard
      {...props}
      ref={setNodeRef}
      style={style}
      dragHandleRef={setActivatorNodeRef}
      dragHandleProps={{ ...attributes, ...listeners }}
      isDragging={isDragging}
    />
  );
}

interface IdentityCardInternalProps extends IdentityCardProps {
  style?: React.CSSProperties;
  dragHandleRef?: (node: HTMLElement | null) => void;
  dragHandleProps?: Record<string, unknown>;
  isDragging?: boolean;
}

const IdentityCard = React.memo(React.forwardRef<HTMLDivElement, IdentityCardInternalProps>(
  function IdentityCard(
    {
      identity,
      isSelected,
      isRefreshing,
      walletNames,
      onSelect,
      onSetAlias,
      onRemove,
      onRefresh,
      onViewKeys,
      onRegisterDpns,
      onTopUp,
      onWithdraw,
      onTransfer,
      style,
      dragHandleRef,
      dragHandleProps,
      isDragging,
    },
    ref,
  ) {
  const [isRenaming, setIsRenaming] = useState(false);
  const displayName = getIdentityDisplayName(identity);
  const balance = formatCreditsBalance(identity.balance);
  const active = isActive(identity);
  const typeLabel = getTypeLabel(identity.identityType);
  const hasKeys = identity.keys.length > 0;
  const hasPrivateKeys = identity.keys.some((k) => k.hasPrivateKey);
  const canWithdraw = active && identity.balance >= MIN_WITHDRAW_BALANCE;
  const canTransfer = active && identity.balance >= MIN_TRANSFER_BALANCE;

  // Resolve wallet display name from associatedWalletHashes
  const walletDisplayName = (() => {
    if (!walletNames || identity.associatedWalletHashes.length === 0) return null;
    for (const hash of identity.associatedWalletHashes) {
      const name = walletNames[hash];
      if (name) return name;
    }
    return null;
  })();

  const handleRename = (alias: string | null) => {
    setIsRenaming(false);
    onSetAlias(alias);
  };

  return (
    <div
      ref={ref}
      role="option"
      tabIndex={0}
      className={cn(
        "w-full text-left rounded-md px-3 py-2.5 transition-colors cursor-pointer",
        "hover:bg-accent/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        isSelected && "bg-accent border-l-2 border-l-primary",
        isDragging && "shadow-lg bg-accent/80",
      )}
      style={style}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (
          (e.key === "Enter" || e.key === " ") &&
          e.target === e.currentTarget
        ) {
          e.preventDefault();
          onSelect();
        }
      }}
      aria-selected={isSelected}
      data-identity-id={identity.id}
    >
      <div className="flex items-start justify-between gap-2">
        {/* Drag handle */}
        <div
          ref={dragHandleRef}
          {...(dragHandleProps as React.HTMLAttributes<HTMLDivElement>)}
          className="flex items-center shrink-0 cursor-grab active:cursor-grabbing pt-0.5 touch-none"
          aria-label="Drag to reorder"
          onClick={(e) => e.stopPropagation()}
        >
          <GripVertical className="h-4 w-4 text-muted-foreground/50 hover:text-muted-foreground" />
        </div>

        <div className="flex items-center gap-2 min-w-0 flex-1">
          <IdentityTypeIcon
            type={identity.identityType}
            className="h-4 w-4 shrink-0 text-muted-foreground"
          />
          <div className="min-w-0 flex-1">
            {isRenaming ? (
              <InlineAliasEdit
                currentAlias={identity.alias}
                onSave={handleRename}
                onCancel={() => setIsRenaming(false)}
              />
            ) : (
              <div className="flex items-center gap-1.5">
                <p className="text-sm font-medium truncate">{displayName}</p>
                {!identity.alias && (
                  <Button
                    variant="ghost"
                    size="icon-xs"
                    className="h-5 w-5 shrink-0 text-muted-foreground"
                    onClick={(e) => {
                      e.stopPropagation();
                      setIsRenaming(true);
                    }}
                    aria-label="Set alias"
                  >
                    <Pencil className="h-3 w-3" />
                  </Button>
                )}
              </div>
            )}
            <div className="flex items-center gap-1.5 mt-0.5">
              <span
                className={cn(
                  "text-xs",
                  active
                    ? "text-muted-foreground"
                    : "text-muted-foreground/60",
                )}
              >
                {balance} DASH
              </span>
              <Badge
                variant="outline"
                className="text-[10px] px-1 py-0 h-4"
              >
                {typeLabel}
              </Badge>
              {!active && (
                <Badge
                  variant="secondary"
                  className={cn(
                    "text-[10px] px-1 py-0 h-4",
                    getStatusColor(identity.status),
                  )}
                >
                  {getStatusLabel(identity.status)}
                </Badge>
              )}
              {hasKeys && hasPrivateKeys && (
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Key
                      className="h-3 w-3 text-primary"
                      aria-label="Has private keys"
                    />
                  </TooltipTrigger>
                  <TooltipContent>Has private keys</TooltipContent>
                </Tooltip>
              )}
              {isRefreshing && (
                <Loader2
                  className="h-3 w-3 animate-spin text-muted-foreground"
                  aria-label="Refreshing"
                />
              )}
            </div>
            {walletDisplayName && (
              <div className="flex items-center gap-1 mt-0.5">
                <Wallet className="h-3 w-3 text-muted-foreground/70 shrink-0" aria-hidden="true" />
                <span className="text-[10px] text-muted-foreground/70 truncate" data-testid="wallet-name">
                  {walletDisplayName}
                </span>
              </div>
            )}
            <div className="flex items-center gap-1 mt-0.5">
              <Tooltip>
                <TooltipTrigger asChild>
                  <span className="text-[10px] text-muted-foreground/70 font-mono truncate max-w-[140px]">
                    {identity.id}
                  </span>
                </TooltipTrigger>
                <TooltipContent>
                  <div className="text-xs">
                    <span className="text-muted-foreground">
                      {identity.identityType === "user" ? "UserId (Base58)" : "ProTxHash (Hex)"}:
                    </span>
                    <br />
                    {identity.id}
                  </div>
                </TooltipContent>
              </Tooltip>
              <CopyButton value={identity.id} size="icon-xs" />
            </div>
          </div>
        </div>

        {/* Action area: context menu */}
        <div className="flex items-center gap-0.5 shrink-0">
          {/* Context menu */}
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="h-6 w-6 shrink-0"
                onClick={(e) => e.stopPropagation()}
                aria-label="Identity actions"
              >
                <MoreVertical className="h-3.5 w-3.5" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent
              align="end"
              onCloseAutoFocus={(e) => e.preventDefault()}
            >
              <DropdownMenuItem onClick={() => setIsRenaming(true)}>
                <Pencil className="h-4 w-4 mr-2" />
                Update Alias
              </DropdownMenuItem>
              {hasKeys && onViewKeys && (
                <DropdownMenuItem onClick={onViewKeys}>
                  <Key className="h-4 w-4 mr-2" />
                  View Keys
                </DropdownMenuItem>
              )}
              <DropdownMenuItem onClick={onRefresh} disabled={isRefreshing}>
                <RefreshCw
                  className={cn(
                    "h-4 w-4 mr-2",
                    isRefreshing && "animate-spin",
                  )}
                />
                Refresh
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              {onTopUp && (
                <DropdownMenuItem onClick={onTopUp} disabled={!active}>
                  <ArrowDownToLine className="h-4 w-4 mr-2" />
                  Top Up
                </DropdownMenuItem>
              )}
              {onWithdraw && (
                <DropdownMenuItem
                  onClick={onWithdraw}
                  disabled={!canWithdraw}
                >
                  <ArrowUpFromLine className="h-4 w-4 mr-2" />
                  Withdraw
                </DropdownMenuItem>
              )}
              {onTransfer && (
                <DropdownMenuItem
                  onClick={onTransfer}
                  disabled={!canTransfer}
                >
                  <ArrowLeftRight className="h-4 w-4 mr-2" />
                  Transfer
                </DropdownMenuItem>
              )}
              {onRegisterDpns && (
                <DropdownMenuItem
                  onClick={onRegisterDpns}
                  disabled={!active}
                >
                  <Tag className="h-4 w-4 mr-2" />
                  Register DPNS Name
                </DropdownMenuItem>
              )}
              <DropdownMenuSeparator />
              <DropdownMenuItem
                onClick={onRemove}
                className="text-destructive focus:text-destructive"
              >
                <Trash2 className="h-4 w-4 mr-2" />
                Remove
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>
    </div>
  );
}));

// ─── IdentityListPanel ────────────────────────────────────────────

export function IdentityListPanel({
  identities,
  selectedIdentityId,
  refreshingIds,
  refreshingAll,
  onSelectIdentity,
  onSetAlias,
  onReorder,
  onRemoveIdentity,
  onRefreshIdentity,
  onRefreshAll,
  onViewKeys,
  onRegisterDpns,
  onTopUp,
  onWithdraw,
  onTransfer,
  walletNames,
  sortColumn = "alias",
  sortOrder = "ascending",
  useCustomOrder = true,
  onSortChange,
  onCreateIdentity,
  onLoadIdentity,
  className,
}: IdentityListPanelProps) {
  const [removeTarget, setRemoveTarget] = useState<{
    id: string;
    displayName: string;
    type: IdentityTypeDto;
  } | null>(null);

  const hasIdentities = identities.length > 0;

  const handleRemoveConfirm = useCallback(async () => {
    if (!removeTarget) return;
    await onRemoveIdentity(removeTarget.id);
    setRemoveTarget(null);
  }, [removeTarget, onRemoveIdentity]);

  // DnD sensors — require 8px movement to start drag (avoids conflicts with click)
  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: { distance: 8 },
    }),
    useSensor(KeyboardSensor),
  );

  const identityIds = useMemo(
    () => identities.map((i) => i.id),
    [identities],
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

  if (!hasIdentities) {
    return (
      <div
        className={cn("flex flex-col h-full", className)}
        role="region"
        aria-label="Identity list"
      >
        <EmptyState
          icon={User}
          title="No Identities Loaded"
          description="Load an existing identity or create a new one after setting up a wallet."
          actionLabel={onLoadIdentity ? "Load Identity" : undefined}
          onAction={onLoadIdentity}
          className="flex-1"
        />
        {onCreateIdentity && (
          <div className="px-4 pb-4">
            <Button
              variant="outline"
              className="w-full"
              onClick={onCreateIdentity}
            >
              <UserPlus className="h-4 w-4 mr-2" />
              Create Identity
            </Button>
          </div>
        )}
      </div>
    );
  }

  return (
    <div
      className={cn("flex flex-col h-full", className)}
      role="region"
      aria-label="Identity list"
    >
      {/* Header with count, sort, and refresh-all */}
      <div className="flex items-center justify-between px-3 py-2 border-b">
        <div className="flex items-center gap-2">
          <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
            Identities
          </h3>
          <Badge variant="secondary" className="text-[10px] px-1.5 py-0 h-4">
            {identities.length}
          </Badge>
        </div>
        <div className="flex items-center gap-0.5">
          {/* Create Identity button */}
          {onCreateIdentity && (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-xs"
                  onClick={onCreateIdentity}
                  aria-label="Create identity"
                  data-testid="create-identity-btn"
                >
                  <Plus className="h-3.5 w-3.5" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>Create identity</TooltipContent>
            </Tooltip>
          )}
          {/* Load Identity button */}
          {onLoadIdentity && (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-xs"
                  onClick={onLoadIdentity}
                  aria-label="Load identity"
                  data-testid="load-identity-btn"
                >
                  <Search className="h-3.5 w-3.5" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>Load identity</TooltipContent>
            </Tooltip>
          )}
          {/* Sort dropdown */}
          {onSortChange && (
            <DropdownMenu>
              <Tooltip>
                <TooltipTrigger asChild>
                  <DropdownMenuTrigger asChild>
                    <Button
                      variant="ghost"
                      size="icon-xs"
                      aria-label="Sort identities"
                    >
                      <ArrowUpDown className="h-3.5 w-3.5" />
                    </Button>
                  </DropdownMenuTrigger>
                </TooltipTrigger>
                <TooltipContent>Sort identities</TooltipContent>
              </Tooltip>
              <DropdownMenuContent align="end">
                {SORT_COLUMNS.map((col) => (
                  <DropdownMenuItem
                    key={col.value}
                    onClick={() => onSortChange(col.value)}
                  >
                    <span className="flex-1">{col.label}</span>
                    {!useCustomOrder && sortColumn === col.value && (
                      <span className="ml-2 text-muted-foreground text-xs">
                        {sortOrder === "ascending" ? "↑" : "↓"}
                      </span>
                    )}
                  </DropdownMenuItem>
                ))}
                <DropdownMenuSeparator />
                <DropdownMenuItem disabled={useCustomOrder}>
                  <Check
                    className={cn(
                      "h-3.5 w-3.5 mr-1.5",
                      useCustomOrder
                        ? "text-primary"
                        : "text-transparent",
                    )}
                  />
                  Custom Order
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          )}
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon-xs"
                onClick={onRefreshAll}
                disabled={refreshingAll}
                aria-label="Refresh all identities"
              >
                <RefreshCw
                  className={cn(
                    "h-3.5 w-3.5",
                    refreshingAll && "animate-spin",
                  )}
                />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Refresh all identities</TooltipContent>
          </Tooltip>
        </div>
      </div>

      {/* Identity list with drag-and-drop */}
      <DndContext
        sensors={sensors}
        collisionDetection={closestCenter}
        modifiers={[restrictToVerticalAxis, restrictToParentElement]}
        onDragEnd={handleDragEnd}
      >
        <SortableContext items={identityIds} strategy={verticalListSortingStrategy}>
          <div
            className="flex-1 overflow-y-auto px-2 py-2 space-y-0.5"
            role="listbox"
            aria-label="Identities"
          >
            {identities.map((identity) => (
              <SortableIdentityCard
                key={identity.id}
                identity={identity}
                isSelected={selectedIdentityId === identity.id}
                isRefreshing={refreshingIds.has(identity.id)}
                walletNames={walletNames}
                onSelect={() => onSelectIdentity(identity.id)}
                onSetAlias={(alias) => onSetAlias(identity.id, alias)}
                onRemove={() =>
                  setRemoveTarget({
                    id: identity.id,
                    displayName: getIdentityDisplayName(identity),
                    type: identity.identityType,
                  })
                }
                onRefresh={() => onRefreshIdentity(identity.id)}
                onViewKeys={
                  onViewKeys && identity.keys.length > 0
                    ? () => onViewKeys(identity.id)
                    : undefined
                }
                onRegisterDpns={
                  onRegisterDpns ? () => onRegisterDpns(identity.id) : undefined
                }
                onTopUp={onTopUp ? () => onTopUp(identity.id) : undefined}
                onWithdraw={
                  onWithdraw ? () => onWithdraw(identity.id) : undefined
                }
                onTransfer={
                  onTransfer ? () => onTransfer(identity.id) : undefined
                }
              />
            ))}
          </div>
        </SortableContext>
      </DndContext>

      {/* Remove Confirmation Dialog */}
      <ConfirmationDialog
        open={removeTarget !== null}
        onOpenChange={(open) => {
          if (!open) setRemoveTarget(null);
        }}
        title="Confirm Removal"
        message={
          removeTarget
            ? `Are you sure you want to no longer track this ${getTypeLabel(removeTarget.type)} identity?`
            : ""
        }
        confirmText="Remove"
        danger
        onResult={(status) => {
          if (status === "confirmed") {
            handleRemoveConfirm();
          } else {
            setRemoveTarget(null);
          }
        }}
      />
    </div>
  );
}
