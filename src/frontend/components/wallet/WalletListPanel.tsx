import { useState, useCallback, useRef, useEffect, memo } from "react";
import {
  Wallet,
  KeyRound,
  Lock,
  Unlock,
  Trash2,
  Pencil,
  MoreVertical,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { EmptyState } from "@/components/feedback/EmptyState";
import { ConfirmationDialog } from "@/components/shared/ConfirmationDialog";
import { WalletUnlockDialog } from "@/components/shared/WalletUnlockDialog";
import type { WalletDto, SingleKeyWalletDto, WalletRefDto } from "@/bindings";
import { formatAmount } from "@/components/shared/AmountInput";

// ─── Constants ─────────────────────────────────────────────────────

const MAX_ALIAS_LENGTH = 64;
const DUFFS_DECIMAL_PLACES = 8;

// ─── Props ─────────────────────────────────────────────────────────

interface WalletListPanelProps {
  /** HD wallets to display */
  hdWallets: WalletDto[];
  /** Single-key wallets to display */
  singleKeyWallets: SingleKeyWalletDto[];
  /** Currently selected wallet ref */
  selectedWallet: WalletRefDto | null;
  /** Called when a wallet is selected (null to deselect) */
  onSelectWallet: (ref: WalletRefDto | null) => void;
  /** Called to rename an HD wallet */
  onRenameHdWallet: (seedHash: string, alias: string | null) => Promise<void>;
  /** Called to rename a single-key wallet */
  onRenameSingleKeyWallet: (
    keyHash: string,
    alias: string | null,
  ) => Promise<void>;
  /** Called to remove an HD wallet */
  onRemoveHdWallet: (seedHash: string) => Promise<void>;
  /** Called to remove a single-key wallet */
  onRemoveSingleKeyWallet: (keyHash: string) => Promise<void>;
  /** Called when an HD wallet is unlocked. Returns error string or null on success. */
  onUnlockWallet?: (seedHash: string, password: string) => Promise<string | null>;
  /** Called when an HD wallet is locked */
  onLockWallet?: (seedHash: string) => Promise<void>;
  /** Called when the "Create Wallet" empty-state action is clicked */
  onCreateWallet?: () => void;
  /** Called when the "Import Wallet" empty-state action is clicked */
  onImportWallet?: () => void;
  /** Additional CSS class */
  className?: string;
}

// ─── Helpers ───────────────────────────────────────────────────────

function formatDashBalance(duffs: number): string {
  return formatAmount(duffs, DUFFS_DECIMAL_PLACES);
}

function getWalletDisplayName(
  alias: string | null,
  fallback: string,
): string {
  return alias?.trim() || fallback;
}

function isHdWalletSelected(
  selected: WalletRefDto | null,
  seedHash: string,
): boolean {
  return selected?.type === "hd" && selected.seedHash === seedHash;
}

function isSingleKeyWalletSelected(
  selected: WalletRefDto | null,
  keyHash: string,
): boolean {
  return selected?.type === "singleKey" && selected.keyHash === keyHash;
}

// ─── Inline rename input ───────────────────────────────────────────

interface InlineRenameProps {
  currentAlias: string | null;
  onSave: (alias: string | null) => void;
  onCancel: () => void;
}

function InlineRename({ currentAlias, onSave, onCancel }: InlineRenameProps) {
  const [value, setValue] = useState(currentAlias ?? "");
  const inputRef = useRef<HTMLInputElement>(null);
  const mountedRef = useRef(false);

  useEffect(() => {
    // Delay focus slightly to let Radix dropdown close animation complete
    // This prevents immediate blur when the dropdown restores focus
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
        // Only save on blur if the component has been fully mounted.
        // This prevents save-on-blur from firing during Radix dropdown
        // close animation which can steal focus from the input.
        if (mountedRef.current) {
          handleSave();
        }
      }}
      placeholder="Enter wallet name"
      className="h-7 text-sm"
      aria-label="Wallet name"
    />
  );
}

// ─── HD Wallet Card ────────────────────────────────────────────────

interface HdWalletCardProps {
  wallet: WalletDto;
  isSelected: boolean;
  onSelect: () => void;
  onRename: (alias: string | null) => void;
  onRemove: () => void;
  onUnlock?: () => void;
  onLock?: () => void;
}

const HdWalletCard = memo(function HdWalletCard({
  wallet,
  isSelected,
  onSelect,
  onRename,
  onRemove,
  onUnlock,
  onLock,
}: HdWalletCardProps) {
  const [isRenaming, setIsRenaming] = useState(false);
  const displayName = getWalletDisplayName(wallet.alias, "Unnamed Wallet");
  const balance = formatDashBalance(wallet.totalBalance);
  const hasPending = wallet.unconfirmedBalance > 0;

  const handleRename = (alias: string | null) => {
    setIsRenaming(false);
    onRename(alias);
  };

  return (
    <button
      type="button"
      className={cn(
        "w-full text-left rounded-md px-3 py-2.5 transition-colors",
        "hover:bg-accent/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        isSelected && "bg-accent border-l-2 border-l-primary",
      )}
      onClick={onSelect}
      aria-current={isSelected ? "true" : undefined}
      data-wallet-type="hd"
      data-seed-hash={wallet.seedHash}
    >
      <div className="flex items-start justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0 flex-1">
          <Wallet className="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          <div className="min-w-0 flex-1">
            {isRenaming ? (
              <InlineRename
                currentAlias={wallet.alias}
                onSave={handleRename}
                onCancel={() => setIsRenaming(false)}
              />
            ) : (
              <p className="text-sm font-medium truncate">{displayName}</p>
            )}
            <div className="flex items-center gap-1.5 mt-0.5">
              <span className="text-xs text-muted-foreground">
                {balance} DASH
              </span>
              {hasPending && (
                <Badge variant="secondary" className="text-[10px] px-1 py-0 h-4">
                  pending
                </Badge>
              )}
              {wallet.usesPassword && (
                <Lock className="h-3 w-3 text-muted-foreground" aria-label="Password protected" />
              )}
            </div>
          </div>
        </div>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className="h-6 w-6 shrink-0"
              onClick={(e) => e.stopPropagation()}
              aria-label="Wallet actions"
            >
              <MoreVertical className="h-3.5 w-3.5" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" onCloseAutoFocus={(e) => e.preventDefault()}>
            <DropdownMenuItem onClick={() => setIsRenaming(true)}>
              <Pencil className="h-4 w-4 mr-2" />
              Rename
            </DropdownMenuItem>
            {wallet.usesPassword && onUnlock && (
              <DropdownMenuItem onClick={() => onUnlock()}>
                <Unlock className="h-4 w-4 mr-2" />
                Unlock
              </DropdownMenuItem>
            )}
            {wallet.usesPassword && onLock && (
              <DropdownMenuItem onClick={() => onLock()}>
                <Lock className="h-4 w-4 mr-2" />
                Lock
              </DropdownMenuItem>
            )}
            <DropdownMenuItem
              onClick={() => onRemove()}
              className="text-destructive focus:text-destructive"
            >
              <Trash2 className="h-4 w-4 mr-2" />
              Remove
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </button>
  );
});

// ─── Single-Key Wallet Card ────────────────────────────────────────

interface SingleKeyWalletCardProps {
  wallet: SingleKeyWalletDto;
  isSelected: boolean;
  onSelect: () => void;
  onRename: (alias: string | null) => void;
  onRemove: () => void;
}

const SingleKeyWalletCard = memo(function SingleKeyWalletCard({
  wallet,
  isSelected,
  onSelect,
  onRename,
  onRemove,
}: SingleKeyWalletCardProps) {
  const [isRenaming, setIsRenaming] = useState(false);
  const displayName = getWalletDisplayName(wallet.alias, "Unnamed Wallet");
  const balance = formatDashBalance(wallet.totalBalance);
  const hasPending = wallet.unconfirmedBalance > 0;

  const handleRename = (alias: string | null) => {
    setIsRenaming(false);
    onRename(alias);
  };

  return (
    <button
      type="button"
      className={cn(
        "w-full text-left rounded-md px-3 py-2.5 transition-colors",
        "hover:bg-accent/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        isSelected && "bg-accent border-l-2 border-l-primary",
      )}
      onClick={onSelect}
      aria-current={isSelected ? "true" : undefined}
      data-wallet-type="singleKey"
      data-key-hash={wallet.keyHash}
    >
      <div className="flex items-start justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0 flex-1">
          <KeyRound className="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          <div className="min-w-0 flex-1">
            {isRenaming ? (
              <InlineRename
                currentAlias={wallet.alias}
                onSave={handleRename}
                onCancel={() => setIsRenaming(false)}
              />
            ) : (
              <p className="text-sm font-medium truncate">{displayName}</p>
            )}
            <div className="flex items-center gap-1.5 mt-0.5">
              <span className="text-xs text-muted-foreground">
                {balance} DASH
              </span>
              {hasPending && (
                <Badge variant="secondary" className="text-[10px] px-1 py-0 h-4">
                  pending
                </Badge>
              )}
              {wallet.usesPassword && (
                <Lock className="h-3 w-3 text-muted-foreground" aria-label="Password protected" />
              )}
            </div>
          </div>
        </div>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className="h-6 w-6 shrink-0"
              onClick={(e) => e.stopPropagation()}
              aria-label="Wallet actions"
            >
              <MoreVertical className="h-3.5 w-3.5" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" onCloseAutoFocus={(e) => e.preventDefault()}>
            <DropdownMenuItem
              onClick={() => setIsRenaming(true)}
              onSelect={() => setIsRenaming(true)}
            >
              <Pencil className="h-4 w-4 mr-2" />
              Rename
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => onRemove()}
              className="text-destructive focus:text-destructive"
            >
              <Trash2 className="h-4 w-4 mr-2" />
              Remove
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </button>
  );
});

// ─── WalletListPanel ───────────────────────────────────────────────

export function WalletListPanel({
  hdWallets,
  singleKeyWallets,
  selectedWallet,
  onSelectWallet,
  onRenameHdWallet,
  onRenameSingleKeyWallet,
  onRemoveHdWallet,
  onRemoveSingleKeyWallet,
  onUnlockWallet,
  onLockWallet,
  onCreateWallet,
  onImportWallet,
  className,
}: WalletListPanelProps) {
  const [removeTarget, setRemoveTarget] = useState<
    | { type: "hd"; seedHash: string; name: string }
    | { type: "singleKey"; keyHash: string; name: string }
    | null
  >(null);
  const [unlockTarget, setUnlockTarget] = useState<{
    seedHash: string;
    alias: string | null;
    passwordHint: string | null;
  } | null>(null);
  const [unlockError, setUnlockError] = useState<string | null>(null);
  const [unlockLoading, setUnlockLoading] = useState(false);

  const hasWallets = hdWallets.length > 0 || singleKeyWallets.length > 0;

  const handleRemoveConfirm = useCallback(async () => {
    if (!removeTarget) return;
    if (removeTarget.type === "hd") {
      await onRemoveHdWallet(removeTarget.seedHash);
    } else {
      await onRemoveSingleKeyWallet(removeTarget.keyHash);
    }
    setRemoveTarget(null);
  }, [removeTarget, onRemoveHdWallet, onRemoveSingleKeyWallet]);

  const handleUnlockResult = useCallback(
    async (result: { status: "unlocked"; password: string } | { status: "cancelled" }) => {
      if (result.status === "unlocked" && unlockTarget && onUnlockWallet) {
        setUnlockError(null);
        setUnlockLoading(true);
        try {
          const error = await onUnlockWallet(unlockTarget.seedHash, result.password);
          if (error) {
            setUnlockError(error);
            setUnlockLoading(false);
            return; // Keep dialog open on error
          }
        } catch (e) {
          setUnlockError(e instanceof Error ? e.message : String(e));
          setUnlockLoading(false);
          return; // Keep dialog open on error
        }
        setUnlockLoading(false);
      }
      setUnlockError(null);
      setUnlockTarget(null);
    },
    [unlockTarget, onUnlockWallet],
  );

  if (!hasWallets) {
    return (
      <div className={cn("flex flex-col h-full", className)} role="region" aria-label="Wallet list">
        <EmptyState
          icon={Wallet}
          title="No Wallets Loaded"
          description="Create or import a wallet to get started."
          actionLabel={onCreateWallet ? "Create Wallet" : undefined}
          onAction={onCreateWallet}
          className="flex-1"
        />
        {onImportWallet && (
          <div className="px-4 pb-4">
            <Button
              variant="outline"
              className="w-full"
              onClick={onImportWallet}
            >
              Import Wallet
            </Button>
          </div>
        )}
      </div>
    );
  }

  return (
    <div className={cn("flex flex-col h-full", className)} role="region" aria-label="Wallet list">
      <div className="flex-1 overflow-y-auto px-2 py-2 space-y-4">
        {/* HD Wallets Section */}
        {hdWallets.length > 0 && (
          <div>
            <div className="flex items-center justify-between px-1 mb-1">
              <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                HD Wallets
              </h3>
              <Badge variant="secondary" className="text-[10px] px-1.5 py-0 h-4">
                {hdWallets.length}
              </Badge>
            </div>
            <div className="space-y-0.5" role="listbox" aria-label="HD wallets">
              {hdWallets.map((wallet) => (
                <HdWalletCard
                  key={wallet.seedHash}
                  wallet={wallet}
                  isSelected={isHdWalletSelected(selectedWallet, wallet.seedHash)}
                  onSelect={() =>
                    onSelectWallet({ type: "hd", seedHash: wallet.seedHash })
                  }
                  onRename={(alias) =>
                    onRenameHdWallet(wallet.seedHash, alias)
                  }
                  onRemove={() =>
                    setRemoveTarget({
                      type: "hd",
                      seedHash: wallet.seedHash,
                      name: getWalletDisplayName(wallet.alias, "Unnamed Wallet"),
                    })
                  }
                  onUnlock={
                    wallet.usesPassword
                      ? () =>
                          setUnlockTarget({
                            seedHash: wallet.seedHash,
                            alias: wallet.alias,
                            passwordHint: wallet.passwordHint,
                          })
                      : undefined
                  }
                  onLock={
                    wallet.usesPassword && onLockWallet
                      ? () => onLockWallet(wallet.seedHash)
                      : undefined
                  }
                />
              ))}
            </div>
          </div>
        )}

        {/* Single-Key Wallets Section */}
        {singleKeyWallets.length > 0 && (
          <div>
            <div className="flex items-center justify-between px-1 mb-1">
              <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                Single-Key Wallets
              </h3>
              <Badge variant="secondary" className="text-[10px] px-1.5 py-0 h-4">
                {singleKeyWallets.length}
              </Badge>
            </div>
            <div className="space-y-0.5" role="listbox" aria-label="Single-key wallets">
              {singleKeyWallets.map((wallet) => (
                <SingleKeyWalletCard
                  key={wallet.keyHash}
                  wallet={wallet}
                  isSelected={isSingleKeyWalletSelected(
                    selectedWallet,
                    wallet.keyHash,
                  )}
                  onSelect={() =>
                    onSelectWallet({
                      type: "singleKey",
                      keyHash: wallet.keyHash,
                    })
                  }
                  onRename={(alias) =>
                    onRenameSingleKeyWallet(wallet.keyHash, alias)
                  }
                  onRemove={() =>
                    setRemoveTarget({
                      type: "singleKey",
                      keyHash: wallet.keyHash,
                      name: getWalletDisplayName(wallet.alias, "Unnamed Wallet"),
                    })
                  }
                />
              ))}
            </div>
          </div>
        )}
      </div>

      {/* Action buttons */}
      {(onCreateWallet || onImportWallet) && (
        <div className="px-2 pb-2 pt-1 space-y-1 border-t border-border/50">
          {onCreateWallet && (
            <Button
              variant="default"
              className="w-full"
              size="sm"
              onClick={onCreateWallet}
            >
              Create Wallet
            </Button>
          )}
          {onImportWallet && (
            <Button
              variant="outline"
              className="w-full"
              size="sm"
              onClick={onImportWallet}
            >
              Import Wallet
            </Button>
          )}
        </div>
      )}

      {/* Remove Confirmation Dialog */}
      <ConfirmationDialog
        open={removeTarget !== null}
        onOpenChange={(open) => {
          if (!open) setRemoveTarget(null);
        }}
        title="Remove Wallet"
        message={
          removeTarget
            ? `Are you sure you want to remove "${removeTarget.name}"? This action cannot be undone.`
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

      {/* Wallet Unlock Dialog */}
      <WalletUnlockDialog
        open={unlockTarget !== null}
        onOpenChange={(open) => {
          if (!open) {
            setUnlockTarget(null);
            setUnlockError(null);
          }
        }}
        walletAlias={unlockTarget?.alias ?? "Wallet"}
        passwordHint={unlockTarget?.passwordHint}
        error={unlockError}
        loading={unlockLoading}
        onResult={handleUnlockResult}
      />
    </div>
  );
}
