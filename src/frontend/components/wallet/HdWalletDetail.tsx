import { useState, useMemo, useCallback } from "react";
import {
  Send,
  Download,
  RefreshCw,
  Loader2,
  Eye,
  Plus,
  Search,
  ArrowUpDown,
  ArrowUp,
  ArrowDown,
  ExternalLink,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { CopyButton } from "@/components/shared/CopyButton";
import { EmptyState } from "@/components/feedback/EmptyState";
import { formatAmount } from "@/components/shared/AmountInput";
import type {
  WalletDto,
  WalletAddressDto,
  WalletTransactionDto,
  AssetLockDto,
  PlatformAddressDto,
} from "@/bindings";
import type { WalletRefreshMode } from "@/stores/walletStore";

// ─── Constants ─────────────────────────────────────────────────────

const DUFFS_DECIMAL_PLACES = 8;
const CREDITS_PER_DUFF = 1000;

// ─── Types ─────────────────────────────────────────────────────────

export type AddressSortColumn =
  | "address"
  | "balance"
  | "totalReceived"
  | "type"
  | "index"
  | "path";

export type SortOrder = "asc" | "desc";

/** Categorize addresses from their derivation path */
export type AddressCategory =
  | "funds"
  | "change"
  | "identityCreation"
  | "platform"
  | "system";

// ─── Props ─────────────────────────────────────────────────────────

interface HdWalletDetailProps {
  /** The HD wallet to display */
  wallet: WalletDto;
  /** Whether the wallet is currently being refreshed */
  refreshing?: boolean;
  /** Whether developer mode is enabled */
  isDeveloperMode?: boolean;
  /** Current refresh mode */
  refreshMode?: WalletRefreshMode;
  /** Called when the refresh mode changes */
  onRefreshModeChange?: (mode: WalletRefreshMode) => void;
  /** Called when the user clicks Send */
  onSend?: () => void;
  /** Called when the user clicks Receive */
  onReceive?: () => void;
  /** Called when the user clicks Refresh */
  onRefresh?: () => void;
  /** Called when the user clicks "Add Receiving Address" */
  onAddAddress?: () => void;
  /** Called when the user clicks "View Key" for an address */
  onViewKey?: (address: string, derivationPath: string) => void;
  /** Called when the user clicks "Create Asset Lock" */
  onCreateAssetLock?: () => void;
  /** Called when the user clicks "Search for Unused" asset locks */
  onSearchAssetLocks?: () => void;
  /** Called when the user clicks "View" on an asset lock */
  onViewAssetLock?: (txid: string) => void;
  /** Called when the user clicks "Fund" on an asset lock */
  onFundAssetLock?: (assetLockIndex: number) => void;
  /** Additional CSS class */
  className?: string;
}

// ─── Helpers ───────────────────────────────────────────────────────

function formatDash(duffs: number): string {
  return formatAmount(duffs, DUFFS_DECIMAL_PLACES);
}

function formatCreditsAsDash(credits: number): string {
  const duffs = credits / CREDITS_PER_DUFF;
  return formatAmount(duffs, DUFFS_DECIMAL_PLACES);
}

function categorizeAddress(derivationPath: string): AddressCategory {
  // BIP44 external chain: m/44'/5'/0'/0/...
  if (/m\/44'\/5'\/\d+'\/0\//.test(derivationPath)) return "funds";
  // BIP44 change chain: m/44'/5'/0'/1/...
  if (/m\/44'\/5'\/\d+'\/1\//.test(derivationPath)) return "change";
  // Asset lock / identity funding paths
  if (/m\/9'\/5'\//.test(derivationPath)) return "identityCreation";
  // Platform payment path
  if (/m\/2049'\/5'\//.test(derivationPath)) return "platform";
  return "system";
}

function categoryLabel(category: AddressCategory): string {
  switch (category) {
    case "funds":
      return "Funds";
    case "change":
      return "Change";
    case "identityCreation":
      return "Identity Creation";
    case "platform":
      return "Platform";
    case "system":
      return "System";
  }
}

function extractIndex(derivationPath: string): number {
  const parts = derivationPath.split("/");
  const last = parts[parts.length - 1];
  if (!last) return 0;
  return parseInt(last.replace("'", ""), 10) || 0;
}

function getAccountLabel(derivationPath: string): string {
  // Extract the account category from the path
  const category = categorizeAddress(derivationPath);
  if (category === "funds" || category === "change") {
    const match = derivationPath.match(/m\/44'\/5'\/(\d+)'/);
    if (match) {
      const accountIndex = parseInt(match[1] ?? "", 10);
      return accountIndex === 0
        ? "Main Account"
        : `BIP44 Account #${accountIndex}`;
    }
  }
  if (category === "platform") return "Platform Account";
  if (category === "identityCreation") return "Identity Registration";
  return categoryLabel(category);
}

/** Get unique accounts from addresses */
function getAccountOptions(
  addresses: WalletAddressDto[],
): { value: string; label: string }[] {
  const seen = new Map<string, string>();
  for (const addr of addresses) {
    const key = getAccountKey(addr.derivationPath);
    if (!seen.has(key)) {
      seen.set(key, getAccountLabel(addr.derivationPath));
    }
  }
  // Sort: Main Account first, then alphabetically
  const entries = Array.from(seen.entries());
  entries.sort((a, b) => {
    if (a[1] === "Main Account") return -1;
    if (b[1] === "Main Account") return 1;
    return a[1].localeCompare(b[1]);
  });
  return entries.map(([value, label]) => ({ value, label }));
}

/** Get a key that groups addresses by account */
function getAccountKey(derivationPath: string): string {
  const category = categorizeAddress(derivationPath);
  if (category === "funds" || category === "change") {
    const match = derivationPath.match(/m\/44'\/5'\/(\d+)'/);
    if (match) return `bip44-${match[1]}`;
  }
  return category;
}

function addressMatchesAccount(
  address: WalletAddressDto,
  accountKey: string,
): boolean {
  return getAccountKey(address.derivationPath) === accountKey;
}

function platformBalanceDuffs(platformAddresses: PlatformAddressDto[]): number {
  return platformAddresses.reduce(
    (sum, addr) => sum + addr.balance / CREDITS_PER_DUFF,
    0,
  );
}

// Refresh mode options
const REFRESH_MODES: { value: WalletRefreshMode; label: string }[] = [
  { value: "coreAndPlatformAuto", label: "All (Auto)" },
  { value: "coreOnly", label: "Core Only" },
  { value: "coreAndPlatformFull", label: "Core + Platform (Full)" },
  { value: "coreAndPlatformTerminal", label: "Core + Platform (Terminal)" },
  { value: "combined", label: "Combined" },
];

// ─── Address Table Sub-Component ──────────────────────────────────

interface AddressTableProps {
  addresses: WalletAddressDto[];
  platformAddresses: PlatformAddressDto[];
  hideZeroBalances: boolean;
  sortColumn: AddressSortColumn;
  sortOrder: SortOrder;
  onSort: (column: AddressSortColumn) => void;
  onViewKey?: (address: string, derivationPath: string) => void;
}

function SortableHeader({
  column,
  currentColumn,
  currentOrder,
  onSort,
  children,
  className,
}: {
  column: AddressSortColumn;
  currentColumn: AddressSortColumn;
  currentOrder: SortOrder;
  onSort: (col: AddressSortColumn) => void;
  children: React.ReactNode;
  className?: string;
}) {
  const isActive = column === currentColumn;
  return (
    <TableHead className={className}>
      <Button
        variant="ghost"
        size="xs"
        className="h-auto py-0 px-0 font-semibold hover:bg-transparent"
        onClick={() => onSort(column)}
      >
        {children}
        {isActive ? (
          currentOrder === "asc" ? (
            <ArrowUp className="ml-1 size-3" />
          ) : (
            <ArrowDown className="ml-1 size-3" />
          )
        ) : (
          <ArrowUpDown className="ml-1 size-3 opacity-40" />
        )}
      </Button>
    </TableHead>
  );
}

function AddressTable({
  addresses,
  platformAddresses,
  hideZeroBalances,
  sortColumn,
  sortOrder,
  onSort,
  onViewKey,
}: AddressTableProps) {
  // Build a lookup for platform address balances
  const platformBalanceMap = useMemo(() => {
    const map = new Map<string, number>();
    for (const pa of platformAddresses) {
      map.set(pa.address, pa.balance);
    }
    return map;
  }, [platformAddresses]);

  // Filter and sort addresses
  const sortedAddresses = useMemo(() => {
    let filtered = addresses;
    if (hideZeroBalances) {
      filtered = addresses.filter((a) => {
        const category = categorizeAddress(a.derivationPath);
        if (category === "platform") {
          // For platform addresses, check platform balance
          const credits = platformBalanceMap.get(a.address) ?? 0;
          return credits > 0;
        }
        return a.balance > 0;
      });
    }

    const sorted = [...filtered];
    sorted.sort((a, b) => {
      let cmp = 0;
      switch (sortColumn) {
        case "address":
          cmp = a.address.localeCompare(b.address);
          break;
        case "balance":
          cmp = a.balance - b.balance;
          break;
        case "totalReceived":
          cmp = a.totalReceived - b.totalReceived;
          break;
        case "type":
          cmp = categoryLabel(categorizeAddress(a.derivationPath)).localeCompare(
            categoryLabel(categorizeAddress(b.derivationPath)),
          );
          break;
        case "index":
          cmp = extractIndex(a.derivationPath) - extractIndex(b.derivationPath);
          break;
        case "path":
          cmp = a.derivationPath.localeCompare(b.derivationPath);
          break;
      }
      return sortOrder === "asc" ? cmp : -cmp;
    });
    return sorted;
  }, [
    addresses,
    hideZeroBalances,
    sortColumn,
    sortOrder,
    platformBalanceMap,
  ]);

  if (sortedAddresses.length === 0) {
    return (
      <EmptyState
        title="No addresses"
        description={
          hideZeroBalances
            ? "All addresses have zero balance. Uncheck 'Hide zero balances' to see them."
            : "No addresses found for this account."
        }
      />
    );
  }

  return (
    <div className="overflow-auto">
      <Table>
        <TableHeader>
          <TableRow>
            <SortableHeader
              column="address"
              currentColumn={sortColumn}
              currentOrder={sortOrder}
              onSort={onSort}
            >
              Address
            </SortableHeader>
            <SortableHeader
              column="balance"
              currentColumn={sortColumn}
              currentOrder={sortOrder}
              onSort={onSort}
              className="text-right"
            >
              Balance (DASH)
            </SortableHeader>
            <SortableHeader
              column="totalReceived"
              currentColumn={sortColumn}
              currentOrder={sortOrder}
              onSort={onSort}
              className="text-right"
            >
              Total Received
            </SortableHeader>
            <SortableHeader
              column="type"
              currentColumn={sortColumn}
              currentOrder={sortOrder}
              onSort={onSort}
            >
              Type
            </SortableHeader>
            <SortableHeader
              column="index"
              currentColumn={sortColumn}
              currentOrder={sortOrder}
              onSort={onSort}
              className="text-right"
            >
              Index
            </SortableHeader>
            <SortableHeader
              column="path"
              currentColumn={sortColumn}
              currentOrder={sortOrder}
              onSort={onSort}
            >
              Path
            </SortableHeader>
            <TableHead className="w-[80px]">Actions</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {sortedAddresses.map((addr) => {
            const category = categorizeAddress(addr.derivationPath);
            const isPlatform = category === "platform";
            const platformCredits = platformBalanceMap.get(addr.address);
            const displayBalance = isPlatform && platformCredits != null
              ? formatCreditsAsDash(platformCredits)
              : formatDash(addr.balance);
            const displayReceived = isPlatform ? "N/A" : formatDash(addr.totalReceived);

            return (
              <TableRow key={`${addr.address}-${addr.derivationPath}`}>
                <TableCell className="font-mono text-xs max-w-[200px]">
                  <div className="flex items-center gap-1">
                    <span className="truncate" title={addr.address}>
                      {addr.address}
                    </span>
                    <CopyButton value={addr.address} />
                  </div>
                </TableCell>
                <TableCell className="text-right font-mono text-xs">
                  {displayBalance}
                </TableCell>
                <TableCell className="text-right font-mono text-xs">
                  {displayReceived}
                </TableCell>
                <TableCell>
                  <Badge variant="outline" className="text-xs">
                    {categoryLabel(category)}
                  </Badge>
                </TableCell>
                <TableCell className="text-right text-xs">
                  {extractIndex(addr.derivationPath)}
                </TableCell>
                <TableCell className="font-mono text-xs max-w-[160px]">
                  <span className="truncate block" title={addr.derivationPath}>
                    {addr.derivationPath}
                  </span>
                </TableCell>
                <TableCell>
                  {onViewKey && (
                    <Button
                      variant="ghost"
                      size="xs"
                      onClick={() =>
                        onViewKey(addr.address, addr.derivationPath)
                      }
                      aria-label={`View key for ${addr.address}`}
                    >
                      <Eye className="size-3" />
                      Key
                    </Button>
                  )}
                </TableCell>
              </TableRow>
            );
          })}
        </TableBody>
      </Table>
    </div>
  );
}

// ─── Transaction Table Sub-Component ──────────────────────────────

interface TransactionTableProps {
  transactions: WalletTransactionDto[];
}

function transactionDirection(tx: WalletTransactionDto): string {
  if (tx.netAmount > 0) return "Received";
  if (tx.netAmount < 0) return "Sent";
  return "Internal";
}

function transactionAmountClass(tx: WalletTransactionDto): string {
  if (tx.netAmount > 0) return "text-success";
  if (tx.netAmount < 0) return "text-destructive";
  return "";
}

function formatTimestamp(timestamp: number): string {
  if (timestamp === 0) return "Unknown";
  const date = new Date(timestamp * 1000);
  return date.toISOString().replace("T", " ").slice(0, 19);
}

function TransactionTable({ transactions }: TransactionTableProps) {
  const sorted = useMemo(() => {
    return [...transactions].sort((a, b) => {
      return b.timestamp - a.timestamp || a.txid.localeCompare(b.txid);
    });
  }, [transactions]);

  if (sorted.length === 0) {
    return (
      <EmptyState
        title="No transactions"
        description="No transactions yet from SPV. Keep your wallet online to sync history."
      />
    );
  }

  return (
    <div className="overflow-auto">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Date</TableHead>
            <TableHead>Type</TableHead>
            <TableHead className="text-right">Amount</TableHead>
            <TableHead>Status</TableHead>
            <TableHead>TxID</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {sorted.map((tx) => {
            const direction = transactionDirection(tx);
            const amountAbs = Math.abs(tx.netAmount);
            const amountPrefix = tx.netAmount > 0 ? "+" : tx.netAmount < 0 ? "-" : "";
            const status = tx.height
              ? `Confirmed @${tx.height}`
              : "Pending";

            return (
              <TableRow key={tx.txid}>
                <TableCell className="text-xs">
                  {formatTimestamp(tx.timestamp)}
                </TableCell>
                <TableCell className="text-xs">{direction}</TableCell>
                <TableCell
                  className={cn(
                    "text-right font-mono text-xs font-semibold",
                    transactionAmountClass(tx),
                  )}
                >
                  {amountPrefix}
                  {formatDash(amountAbs)}
                </TableCell>
                <TableCell>
                  <Badge
                    variant={tx.height ? "secondary" : "outline"}
                    className="text-xs"
                  >
                    {status}
                  </Badge>
                </TableCell>
                <TableCell className="font-mono text-xs max-w-[200px]">
                  <div className="flex items-center gap-1">
                    <span className="truncate" title={tx.txid}>
                      {tx.txid}
                    </span>
                    <CopyButton value={tx.txid} />
                  </div>
                </TableCell>
              </TableRow>
            );
          })}
        </TableBody>
      </Table>
    </div>
  );
}

// ─── Asset Locks Table Sub-Component ──────────────────────────────

interface AssetLocksTableProps {
  assetLocks: AssetLockDto[];
  onCreateAssetLock?: () => void;
  onSearchAssetLocks?: () => void;
  onViewAssetLock?: (txid: string) => void;
  onFundAssetLock?: (index: number) => void;
}

function AssetLocksTable({
  assetLocks,
  onCreateAssetLock,
  onSearchAssetLocks,
  onViewAssetLock,
  onFundAssetLock,
}: AssetLocksTableProps) {
  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        {onCreateAssetLock && (
          <Button variant="outline" size="sm" onClick={onCreateAssetLock}>
            <Plus className="size-3.5" />
            Create Asset Lock
          </Button>
        )}
        {onSearchAssetLocks && (
          <Button variant="outline" size="sm" onClick={onSearchAssetLocks}>
            <Search className="size-3.5" />
            Search for Unused
          </Button>
        )}
      </div>

      {assetLocks.length === 0 ? (
        <EmptyState
          title="No asset locks"
          description="Asset locks are used to fund identity registration and top-ups."
        />
      ) : (
        <div className="overflow-auto">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Transaction ID</TableHead>
                <TableHead>Address</TableHead>
                <TableHead className="text-right">Amount (Duffs)</TableHead>
                <TableHead>InstantLock</TableHead>
                <TableHead>Usable</TableHead>
                <TableHead className="w-[120px]">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {assetLocks.map((lock, index) => (
                <TableRow key={lock.txid}>
                  <TableCell className="font-mono text-xs max-w-[180px]">
                    <div className="flex items-center gap-1">
                      <span className="truncate" title={lock.txid}>
                        {lock.txid}
                      </span>
                      <CopyButton value={lock.txid} />
                    </div>
                  </TableCell>
                  <TableCell className="font-mono text-xs max-w-[150px]">
                    <span className="truncate block" title={lock.address}>
                      {lock.address}
                    </span>
                  </TableCell>
                  <TableCell className="text-right font-mono text-xs">
                    {lock.amount.toLocaleString()}
                  </TableCell>
                  <TableCell>
                    <Badge
                      variant={lock.hasInstantLock ? "secondary" : "outline"}
                      className="text-xs"
                    >
                      {lock.hasInstantLock ? "Yes" : "No"}
                    </Badge>
                  </TableCell>
                  <TableCell>
                    <Badge
                      variant={lock.hasAssetLockProof ? "secondary" : "outline"}
                      className="text-xs"
                    >
                      {lock.hasAssetLockProof ? "Yes" : "No"}
                    </Badge>
                  </TableCell>
                  <TableCell>
                    <div className="flex items-center gap-1">
                      {onViewAssetLock && (
                        <Button
                          variant="ghost"
                          size="xs"
                          onClick={() => onViewAssetLock(lock.txid)}
                          aria-label={`View asset lock ${lock.txid}`}
                        >
                          <ExternalLink className="size-3" />
                          View
                        </Button>
                      )}
                      {onFundAssetLock && lock.hasAssetLockProof && (
                        <Button
                          variant="ghost"
                          size="xs"
                          onClick={() => onFundAssetLock(index)}
                          aria-label={`Fund from asset lock ${lock.txid}`}
                        >
                          Fund
                        </Button>
                      )}
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}
    </div>
  );
}

// ─── HdWalletDetail ───────────────────────────────────────────────

export function HdWalletDetail({
  wallet,
  refreshing = false,
  isDeveloperMode = false,
  refreshMode = "coreAndPlatformAuto",
  onRefreshModeChange,
  onSend,
  onReceive,
  onRefresh,
  onAddAddress,
  onViewKey,
  onCreateAssetLock,
  onSearchAssetLocks,
  onViewAssetLock,
  onFundAssetLock,
  className,
}: HdWalletDetailProps) {
  // Address table state
  const [sortColumn, setSortColumn] = useState<AddressSortColumn>("index");
  const [sortOrder, setSortOrder] = useState<SortOrder>("asc");
  const [hideZeroBalances, setHideZeroBalances] = useState(true);
  const [selectedAccount, setSelectedAccount] = useState<string | undefined>(
    undefined,
  );

  // Computed values
  const displayName = wallet.alias?.trim() || "Unnamed Wallet";
  const platformBalance = platformBalanceDuffs(wallet.platformAddresses);

  // Account options
  const accountOptions = useMemo(
    () => getAccountOptions(wallet.addresses),
    [wallet.addresses],
  );

  // Ensure selected account is valid (or default to first)
  const effectiveAccount = useMemo(() => {
    if (selectedAccount && accountOptions.some((o) => o.value === selectedAccount)) {
      return selectedAccount;
    }
    return accountOptions[0]?.value;
  }, [selectedAccount, accountOptions]);

  // Filter addresses by selected account
  const filteredAddresses = useMemo(() => {
    if (!effectiveAccount) return wallet.addresses;
    return wallet.addresses.filter((a) =>
      addressMatchesAccount(a, effectiveAccount),
    );
  }, [wallet.addresses, effectiveAccount]);

  // Check if selected account is the main BIP44 account
  const isMainAccount = effectiveAccount === "bip44-0";

  // Handle sort
  const handleSort = useCallback(
    (column: AddressSortColumn) => {
      if (column === sortColumn) {
        setSortOrder((prev) => (prev === "asc" ? "desc" : "asc"));
      } else {
        setSortColumn(column);
        setSortOrder("asc");
      }
    },
    [sortColumn],
  );

  // Determine default tab
  const defaultTab = "addresses";

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

        {/* Balance Summary */}
        <div className="flex flex-wrap items-center gap-x-6 gap-y-1 text-sm">
          <div>
            <span className="text-muted-foreground">Core balance: </span>
            <span className="font-mono font-medium">
              {formatDash(wallet.totalBalance)} DASH
            </span>
            {wallet.unconfirmedBalance > 0 && (
              <span className="text-muted-foreground ml-1">
                (+{formatDash(wallet.unconfirmedBalance)} pending)
              </span>
            )}
          </div>
          <div>
            <span className="text-muted-foreground">Platform balance: </span>
            <span className="font-mono font-medium">
              {formatDash(platformBalance)} DASH
            </span>
          </div>
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

        {/* Dev mode: refresh mode selector */}
        {isDeveloperMode && onRefreshModeChange && (
          <div className="flex items-center gap-2 ml-auto">
            <span className="text-xs text-muted-foreground">Refresh Mode:</span>
            <Select
              value={refreshMode}
              onValueChange={(value) =>
                onRefreshModeChange(value as WalletRefreshMode)
              }
            >
              <SelectTrigger className="h-7 w-[180px] text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {REFRESH_MODES.map((mode) => (
                  <SelectItem key={mode.value} value={mode.value}>
                    {mode.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        )}
      </div>

      {/* Tabbed Content */}
      <Tabs defaultValue={defaultTab} className="flex-1">
        <TabsList>
          <TabsTrigger value="addresses">Addresses</TabsTrigger>
          {isDeveloperMode && (
            <TabsTrigger value="transactions">Transactions</TabsTrigger>
          )}
          <TabsTrigger value="assetLocks">Asset Locks</TabsTrigger>
        </TabsList>

        {/* Addresses Tab */}
        <TabsContent value="addresses" className="space-y-3 mt-3">
          {/* Account selector + hide zero toggle */}
          <div className="flex items-center gap-3 flex-wrap">
            {accountOptions.length > 1 && (
              <Select
                value={effectiveAccount}
                onValueChange={setSelectedAccount}
              >
                <SelectTrigger className="h-8 w-[220px] text-sm">
                  <SelectValue placeholder="Select account" />
                </SelectTrigger>
                <SelectContent>
                  {accountOptions.map((opt) => (
                    <SelectItem key={opt.value} value={opt.value}>
                      {opt.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            )}

            <label className="flex items-center gap-2 text-sm cursor-pointer">
              <input
                type="checkbox"
                checked={hideZeroBalances}
                onChange={(e) => setHideZeroBalances(e.target.checked)}
                className="rounded border-border"
              />
              <span className="text-muted-foreground">
                Hide zero balances
              </span>
            </label>
          </div>

          <AddressTable
            addresses={filteredAddresses}
            platformAddresses={wallet.platformAddresses}
            hideZeroBalances={hideZeroBalances}
            sortColumn={sortColumn}
            sortOrder={sortOrder}
            onSort={handleSort}
            onViewKey={onViewKey}
          />

          {/* Add Receiving Address button (only for main account) */}
          {onAddAddress && isMainAccount && (
            <div className="pt-2">
              <Button
                variant="outline"
                size="sm"
                onClick={onAddAddress}
              >
                <Plus className="size-3.5" />
                Add Receiving Address
              </Button>
            </div>
          )}
        </TabsContent>

        {/* Transactions Tab (dev mode only) */}
        {isDeveloperMode && (
          <TabsContent value="transactions" className="mt-3">
            <TransactionTable transactions={wallet.transactions} />
          </TabsContent>
        )}

        {/* Asset Locks Tab */}
        <TabsContent value="assetLocks" className="mt-3">
          <AssetLocksTable
            assetLocks={wallet.unusedAssetLocks}
            onCreateAssetLock={onCreateAssetLock}
            onSearchAssetLocks={onSearchAssetLocks}
            onViewAssetLock={onViewAssetLock}
            onFundAssetLock={onFundAssetLock}
          />
        </TabsContent>
      </Tabs>
    </div>
  );
}
