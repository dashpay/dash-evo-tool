import {
  User,
  Shield,
  Server,
  Key,
  Tag,
  RefreshCw,
  ArrowUpFromLine,
  ArrowDownToLine,
  ArrowLeftRight,
  Wallet,
  Lock,
  Globe,
  Loader2,
} from "lucide-react";
import { cn, hexToBase58 } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { CopyButton } from "@/components/shared/CopyButton";
import { formatCreditsAsDash } from "@/components/shared/AmountInput";
import type {
  QualifiedIdentityDto,
  IdentityTypeDto,
  IdentityStatusDto,
  IdentityKeyDto,
} from "@/bindings";

// ─── Constants ─────────────────────────────────────────────────────

const MIN_WITHDRAW_BALANCE = 500_000_000;
const MIN_TRANSFER_BALANCE = 20_000_000;

// ─── Props ─────────────────────────────────────────────────────────

export interface IdentityDetailPanelProps {
  /** The identity to display details for. */
  identity: QualifiedIdentityDto;
  /** Whether this identity is currently being refreshed. */
  isRefreshing: boolean;
  /** Map of wallet seed hash → display name for resolving wallet associations. */
  walletNames: Record<string, string>;
  /** Called to refresh this identity from Platform. */
  onRefresh: (identityId: string) => void;
  /** Called to navigate to top-up. */
  onTopUp?: (identityId: string) => void;
  /** Called to navigate to withdraw. */
  onWithdraw?: (identityId: string) => void;
  /** Called to navigate to transfer. */
  onTransfer?: (identityId: string) => void;
  /** Called to navigate to DPNS registration. */
  onRegisterDpns?: (identityId: string) => void;
  /** Called to navigate to key management. */
  onViewKeys?: (identityId: string) => void;
  /** Called to view a specific key's details. */
  onViewKey?: (identityId: string, keyId: number) => void;
  /** Called to navigate to a wallet. */
  onNavigateToWallet?: (seedHash: string) => void;
  /** Additional CSS class. */
  className?: string;
}

// ─── Helpers ───────────────────────────────────────────────────────

function formatCreditsBalance(credits: number): string {
  return formatCreditsAsDash(credits);
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

function getStatusVariant(
  status: IdentityStatusDto,
): "default" | "secondary" | "destructive" | "outline" {
  switch (status) {
    case "active":
      return "default";
    case "pendingCreation":
      return "secondary";
    case "unknown":
      return "outline";
    case "notFound":
    case "failedCreation":
      return "destructive";
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

function getPurposeLetter(purpose: string): string {
  switch (purpose.toUpperCase()) {
    case "AUTHENTICATION":
      return "A";
    case "ENCRYPTION":
      return "E";
    case "DECRYPTION":
      return "D";
    case "TRANSFER":
      return "T";
    case "SYSTEM":
      return "S";
    case "VOTING":
      return "V";
    case "OWNER":
      return "O";
    default:
      return purpose.charAt(0).toUpperCase();
  }
}

function getSecurityLevelShort(level: string): string {
  switch (level.toUpperCase()) {
    case "MASTER":
      return "Master";
    case "CRITICAL":
      return "Critical";
    case "HIGH":
      return "High";
    case "MEDIUM":
      return "Medium";
    default:
      return level;
  }
}

function formatKeyLabel(key: IdentityKeyDto): string {
  return `#${key.keyId} — ${getPurposeLetter(key.purpose)} — ${getSecurityLevelShort(key.securityLevel)}`;
}

function formatTimestamp(ts: number): string {
  if (ts === 0) return "Unknown";
  const date = new Date(ts * 1000);
  return date.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

// ─── Component ─────────────────────────────────────────────────────

export function IdentityDetailPanel({
  identity,
  isRefreshing,
  walletNames,
  onRefresh,
  onTopUp,
  onWithdraw,
  onTransfer,
  onRegisterDpns,
  onViewKeys,
  onViewKey,
  onNavigateToWallet,
  className,
}: IdentityDetailPanelProps) {
  const active = identity.status === "active";
  const balance = formatCreditsBalance(identity.balance);
  const canWithdraw = active && identity.balance >= MIN_WITHDRAW_BALANCE;
  const canTransfer = active && identity.balance >= MIN_TRANSFER_BALANCE;
  const hasKeys = identity.keys.length > 0;
  const mainKeys = identity.keys.filter(
    (k) => k.purpose.toUpperCase() !== "VOTING",
  );
  const voterKeys = identity.keys.filter(
    (k) => k.purpose.toUpperCase() === "VOTING",
  );

  return (
    <div
      className={cn("flex flex-col h-full overflow-y-auto", className)}
      role="region"
      aria-label="Identity details"
    >
      {/* ─── Header ────────────────────────────────────── */}
      <div className="px-5 pt-5 pb-4">
        <div className="flex items-start gap-3">
          <div className="rounded-lg bg-muted p-2.5 shrink-0">
            <IdentityTypeIcon type={identity.identityType} className="h-5 w-5 text-muted-foreground" />
          </div>
          <div className="min-w-0 flex-1">
            <h2 className="text-lg font-semibold truncate">
              {identity.alias || "Unnamed Identity"}
            </h2>
            <div className="flex items-center gap-1.5 mt-1">
              <Tooltip>
                <TooltipTrigger asChild>
                  <code className="text-xs text-muted-foreground font-mono break-all select-all cursor-default">
                    {hexToBase58(identity.id)}
                  </code>
                </TooltipTrigger>
                <TooltipContent>
                  {identity.identityType === "user" ? "User ID" : "ProTxHash"}
                </TooltipContent>
              </Tooltip>
              <CopyButton value={hexToBase58(identity.id)} size="icon-xs" />
            </div>
            <div className="flex items-center gap-1.5 mt-2 flex-wrap">
              <Badge variant="outline">{getTypeLabel(identity.identityType)}</Badge>
              <Badge variant={getStatusVariant(identity.status)}>
                {getStatusLabel(identity.status)}
              </Badge>
              {identity.network !== "dash" && (
                <Badge variant="secondary" className="capitalize">
                  {identity.network}
                </Badge>
              )}
            </div>
          </div>
        </div>
      </div>

      <Separator />

      {/* ─── Balance ───────────────────────────────────── */}
      <div className="px-5 py-4">
        <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">
          Balance
        </h3>
        <Tooltip>
          <TooltipTrigger asChild>
            <p className="text-2xl font-bold tabular-nums" data-testid="identity-balance">
              {balance}
              <span className="text-base font-normal text-muted-foreground ml-1.5">
                DASH
              </span>
            </p>
          </TooltipTrigger>
          <TooltipContent>
            {identity.balance.toLocaleString()} credits
          </TooltipContent>
        </Tooltip>
      </div>

      <Separator />

      {/* ─── Action Bar ────────────────────────────────── */}
      <div className="px-5 py-3">
        <div className="flex items-center gap-2 flex-wrap">
          {onTopUp && (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={!active}
                  onClick={() => onTopUp(identity.id)}
                >
                  <ArrowDownToLine className="h-4 w-4 mr-1.5" />
                  Top Up
                </Button>
              </TooltipTrigger>
              {!active && (
                <TooltipContent>Identity must be active</TooltipContent>
              )}
            </Tooltip>
          )}
          {onWithdraw && (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={!canWithdraw}
                  onClick={() => onWithdraw(identity.id)}
                >
                  <ArrowUpFromLine className="h-4 w-4 mr-1.5" />
                  Withdraw
                </Button>
              </TooltipTrigger>
              {!canWithdraw && (
                <TooltipContent>
                  {!active
                    ? "Identity must be active"
                    : "Minimum balance: 0.005 DASH"}
                </TooltipContent>
              )}
            </Tooltip>
          )}
          {onTransfer && (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={!canTransfer}
                  onClick={() => onTransfer(identity.id)}
                >
                  <ArrowLeftRight className="h-4 w-4 mr-1.5" />
                  Transfer
                </Button>
              </TooltipTrigger>
              {!canTransfer && (
                <TooltipContent>
                  {!active
                    ? "Identity must be active"
                    : "Minimum balance: 0.0002 DASH"}
                </TooltipContent>
              )}
            </Tooltip>
          )}
          {onRegisterDpns && (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={!active}
                  onClick={() => onRegisterDpns(identity.id)}
                >
                  <Tag className="h-4 w-4 mr-1.5" />
                  Register DPNS
                </Button>
              </TooltipTrigger>
              {!active && (
                <TooltipContent>Identity must be active</TooltipContent>
              )}
            </Tooltip>
          )}
          <Button
            variant="outline"
            size="sm"
            disabled={isRefreshing}
            onClick={() => onRefresh(identity.id)}
          >
            <RefreshCw
              className={cn("h-4 w-4 mr-1.5", isRefreshing && "animate-spin")}
            />
            Refresh
          </Button>
        </div>
      </div>

      <Separator />

      {/* ─── DPNS Names ────────────────────────────────── */}
      {identity.dpnsNames.length > 0 && (
        <>
          <div className="px-5 py-4">
            <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">
              DPNS Names
            </h3>
            <ul className="space-y-1.5" data-testid="dpns-names-list">
              {identity.dpnsNames.map((dpns) => (
                <li
                  key={dpns.name}
                  className="flex items-center justify-between"
                >
                  <div className="flex items-center gap-2">
                    <Globe className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                    <span className="text-sm font-medium">{dpns.name}</span>
                  </div>
                  <span className="text-xs text-muted-foreground">
                    {formatTimestamp(dpns.acquiredAt)}
                  </span>
                </li>
              ))}
            </ul>
          </div>
          <Separator />
        </>
      )}

      {/* ─── Associated Wallets ─────────────────────────── */}
      {identity.associatedWalletHashes.length > 0 && (
        <>
          <div className="px-5 py-4">
            <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">
              Associated Wallets
            </h3>
            <ul className="space-y-1.5" data-testid="associated-wallets-list">
              {identity.associatedWalletHashes.map((hash) => {
                const name = walletNames[hash];
                return (
                  <li key={hash} className="flex items-center gap-2">
                    <Wallet className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                    {onNavigateToWallet ? (
                      <button
                        className="text-sm text-primary hover:underline cursor-pointer truncate text-left"
                        onClick={() => onNavigateToWallet(hash)}
                      >
                        {name || `Wallet ${hash.slice(0, 8)}...`}
                      </button>
                    ) : (
                      <span className="text-sm truncate">
                        {name || `Wallet ${hash.slice(0, 8)}...`}
                      </span>
                    )}
                  </li>
                );
              })}
            </ul>
          </div>
          <Separator />
        </>
      )}

      {/* ─── Associated Identities ──────────────────────── */}
      {(identity.voterIdentityId || identity.operatorIdentityId) && (
        <>
          <div className="px-5 py-4">
            <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">
              Associated Identities
            </h3>
            <dl className="space-y-2" data-testid="associated-identities">
              {identity.voterIdentityId && (
                <div className="flex items-center gap-2">
                  <dt className="text-xs text-muted-foreground shrink-0">Voter:</dt>
                  <dd className="flex items-center gap-1 min-w-0">
                    <code className="text-xs font-mono truncate">
                      {hexToBase58(identity.voterIdentityId)}
                    </code>
                    <CopyButton value={hexToBase58(identity.voterIdentityId)} size="icon-xs" />
                  </dd>
                </div>
              )}
              {identity.operatorIdentityId && (
                <div className="flex items-center gap-2">
                  <dt className="text-xs text-muted-foreground shrink-0">Operator:</dt>
                  <dd className="flex items-center gap-1 min-w-0">
                    <code className="text-xs font-mono truncate">
                      {hexToBase58(identity.operatorIdentityId)}
                    </code>
                    <CopyButton value={hexToBase58(identity.operatorIdentityId)} size="icon-xs" />
                  </dd>
                </div>
              )}
            </dl>
          </div>
          <Separator />
        </>
      )}

      {/* ─── Keys Quick-Access ─────────────────────────── */}
      <div className="px-5 py-4">
        <div className="flex items-center justify-between mb-2">
          <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
            Keys
            {hasKeys && (
              <span className="ml-1.5 text-muted-foreground/70">
                ({identity.keys.length})
              </span>
            )}
          </h3>
          {hasKeys && onViewKeys && (
            <Button
              variant="ghost"
              size="sm"
              className="h-7 text-xs"
              onClick={() => onViewKeys(identity.id)}
            >
              Manage Keys
            </Button>
          )}
        </div>

        {!hasKeys ? (
          <p className="text-sm text-muted-foreground">No keys loaded.</p>
        ) : (
          <div className="space-y-3">
            {/* Main keys */}
            {mainKeys.length > 0 && (
              <KeySection
                label="Main Keys"
                keys={mainKeys}
                identityId={identity.id}
                onViewKey={onViewKey}
              />
            )}

            {/* Voter keys */}
            {voterKeys.length > 0 && (
              <KeySection
                label="Voter Keys"
                keys={voterKeys}
                identityId={identity.id}
                onViewKey={onViewKey}
              />
            )}
          </div>
        )}
      </div>

      {/* ─── Wallet Index / Top-ups ────────────────────── */}
      {(identity.walletIndex !== null || identity.topUps.length > 0) && (
        <>
          <Separator />
          <div className="px-5 py-4">
            <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">
              Details
            </h3>
            <dl className="space-y-1.5 text-sm" data-testid="identity-details">
              {identity.walletIndex !== null && (
                <div className="flex items-center justify-between">
                  <dt className="text-muted-foreground">Wallet Index</dt>
                  <dd className="font-mono">{identity.walletIndex}</dd>
                </div>
              )}
              {identity.topUps.length > 0 && (
                <div>
                  <dt className="text-muted-foreground mb-1">Top-up History</dt>
                  <dd>
                    <ul className="space-y-0.5">
                      {identity.topUps.map((topUp) => (
                        <li
                          key={topUp.index}
                          className="flex items-center justify-between text-xs"
                        >
                          <span className="text-muted-foreground">
                            Index {topUp.index}
                          </span>
                          <span className="font-mono">
                            {formatCreditsAsDash(topUp.amount)} DASH
                          </span>
                        </li>
                      ))}
                    </ul>
                  </dd>
                </div>
              )}
            </dl>
          </div>
        </>
      )}

      {/* Refreshing overlay indicator */}
      {isRefreshing && (
        <div className="px-5 py-3 border-t bg-muted/30">
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            Refreshing identity from Platform...
          </div>
        </div>
      )}
    </div>
  );
}

// ─── Key Section Sub-component ─────────────────────────────────────

interface KeySectionProps {
  label: string;
  keys: IdentityKeyDto[];
  identityId: string;
  onViewKey?: (identityId: string, keyId: number) => void;
}

function KeySection({ label, keys, identityId, onViewKey }: KeySectionProps) {
  return (
    <div>
      <p className="text-xs text-muted-foreground mb-1">{label}</p>
      <div className="space-y-0.5">
        {keys.map((key) => (
          <KeyItem
            key={key.keyId}
            keyData={key}
            identityId={identityId}
            onViewKey={onViewKey}
          />
        ))}
      </div>
    </div>
  );
}

// ─── Key Item Sub-component ────────────────────────────────────────

interface KeyItemProps {
  keyData: IdentityKeyDto;
  identityId: string;
  onViewKey?: (identityId: string, keyId: number) => void;
}

function KeyItem({ keyData, identityId, onViewKey }: KeyItemProps) {
  const label = formatKeyLabel(keyData);

  return (
    <button
      className={cn(
        "w-full flex items-center gap-2 px-2 py-1.5 rounded text-sm text-left transition-colors",
        "hover:bg-accent/50",
        onViewKey && "cursor-pointer",
        !onViewKey && "cursor-default",
        keyData.isDisabled && "opacity-50",
      )}
      onClick={() => onViewKey?.(identityId, keyData.keyId)}
      disabled={!onViewKey}
      aria-label={`Key ${keyData.keyId}: ${keyData.purpose} ${keyData.securityLevel}`}
    >
      <Key
        className={cn(
          "h-3.5 w-3.5 shrink-0",
          keyData.hasPrivateKey ? "text-primary" : "text-muted-foreground",
        )}
      />
      <span className="font-mono text-xs flex-1 truncate">{label}</span>
      {keyData.hasPrivateKey && (
        <Tooltip>
          <TooltipTrigger asChild>
            <Lock className="h-3 w-3 text-primary shrink-0" />
          </TooltipTrigger>
          <TooltipContent>Private key available</TooltipContent>
        </Tooltip>
      )}
      {keyData.isDisabled && (
        <Badge
          variant="secondary"
          className="text-[10px] px-1 py-0 h-4 shrink-0"
        >
          Disabled
        </Badge>
      )}
    </button>
  );
}
