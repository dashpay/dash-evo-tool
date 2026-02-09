import { useMemo } from "react";
import {
  Key,
  Plus,
  Shield,
  ShieldAlert,
  ShieldCheck,
  Lock,
  Unlock,
  ArrowLeft,
  Eye,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { EmptyState } from "@/components/feedback/EmptyState";
import type { IdentityKeyDto, QualifiedIdentityDto } from "@/bindings";

// ─── Constants ─────────────────────────────────────────────────────

const PURPOSE_ORDER: Record<string, number> = {
  AUTHENTICATION: 0,
  TRANSFER: 1,
  VOTING: 2,
  OWNER: 3,
  ENCRYPTION: 4,
  DECRYPTION: 5,
};

const SECURITY_LEVEL_ORDER: Record<string, number> = {
  MASTER: 0,
  CRITICAL: 1,
  HIGH: 2,
  MEDIUM: 3,
};

// ─── Helpers ───────────────────────────────────────────────────────

function getPurposeLabel(purpose: string): string {
  return purpose.charAt(0) + purpose.slice(1).toLowerCase();
}

function getSecurityLevelColor(level: string): string {
  switch (level) {
    case "MASTER":
      return "text-red-600 dark:text-red-400";
    case "CRITICAL":
      return "text-orange-600 dark:text-orange-400";
    case "HIGH":
      return "text-yellow-600 dark:text-yellow-400";
    case "MEDIUM":
      return "text-green-600 dark:text-green-400";
    default:
      return "text-muted-foreground";
  }
}

function SecurityLevelIcon({ level, className }: { level: string; className?: string }) {
  switch (level) {
    case "MASTER":
      return <ShieldAlert className={className} />;
    case "CRITICAL":
      return <ShieldCheck className={className} />;
    default:
      return <Shield className={className} />;
  }
}

function getKeyTypeBadge(keyType: string): string {
  switch (keyType) {
    case "ECDSA_SECP256K1":
      return "ECDSA";
    case "BLS12_381":
      return "BLS";
    case "ECDSA_HASH160":
      return "ECDSA-H160";
    case "EDDSA_25519_HASH160":
      return "EdDSA";
    default:
      return keyType;
  }
}

function sortKeys(keys: IdentityKeyDto[]): IdentityKeyDto[] {
  return [...keys].sort((a, b) => {
    const purposeA = PURPOSE_ORDER[a.purpose] ?? 99;
    const purposeB = PURPOSE_ORDER[b.purpose] ?? 99;
    if (purposeA !== purposeB) return purposeA - purposeB;
    const levelA = SECURITY_LEVEL_ORDER[a.securityLevel] ?? 99;
    const levelB = SECURITY_LEVEL_ORDER[b.securityLevel] ?? 99;
    if (levelA !== levelB) return levelA - levelB;
    return a.keyId - b.keyId;
  });
}

// ─── Props ─────────────────────────────────────────────────────────

export interface KeyManagementScreenProps {
  /** The identity whose keys to manage. */
  identity: QualifiedIdentityDto;
  /** Called when user clicks on a key row to view details. */
  onViewKey?: (keyId: number) => void;
  /** Called when user clicks Add Key. */
  onAddKey?: () => void;
  /** Called when user clicks the back button. */
  onBack?: () => void;
  /** Additional CSS class. */
  className?: string;
}

// ─── Component ─────────────────────────────────────────────────────

/**
 * Key management screen showing all keys on an identity.
 *
 * Matches egui keys_screen.rs: table of keys grouped by Main vs Voter sections,
 * with Key ID, Purpose, Security Level, Type, Status, and Has Private Key columns.
 * Adds clickable rows for key info and an "Add Key" button.
 */
export function KeyManagementScreen({
  identity,
  onViewKey,
  onAddKey,
  onBack,
  className,
}: KeyManagementScreenProps) {
  const allKeys = identity.keys;

  const mainKeys = useMemo(
    () => sortKeys(allKeys.filter((k) => k.purpose !== "VOTING")),
    [allKeys],
  );

  const voterKeys = useMemo(
    () => sortKeys(allKeys.filter((k) => k.purpose === "VOTING")),
    [allKeys],
  );

  const displayName =
    identity.alias || `${identity.id.slice(0, 8)}…${identity.id.slice(-6)}`;

  return (
    <div className={cn("flex flex-col gap-4", className)} role="region" aria-label="Key management">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          {onBack && (
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={onBack}
              aria-label="Go back"
            >
              <ArrowLeft className="size-4" />
            </Button>
          )}
          <div>
            <h2 className="text-lg font-semibold">Identity Keys</h2>
            <p className="text-sm text-muted-foreground">{displayName}</p>
          </div>
        </div>
        {onAddKey && (
          <Button onClick={onAddKey} size="sm" aria-label="Add key">
            <Plus className="mr-1.5 size-4" />
            Add Key
          </Button>
        )}
      </div>

      <Separator />

      {allKeys.length === 0 ? (
        <EmptyState
          icon={Key}
          title="No keys"
          description="This identity has no public keys."
          actionLabel={onAddKey ? "Add Key" : undefined}
          onAction={onAddKey}
        />
      ) : (
        <div className="space-y-6">
          {/* Main Keys */}
          {mainKeys.length > 0 && (
            <KeySection
              title="Main Keys"
              keys={mainKeys}
              onViewKey={onViewKey}
            />
          )}

          {/* Voter Keys */}
          {voterKeys.length > 0 && (
            <KeySection
              title="Voter Keys"
              keys={voterKeys}
              onViewKey={onViewKey}
            />
          )}
        </div>
      )}
    </div>
  );
}

// ─── Key Section ───────────────────────────────────────────────────

function KeySection({
  title,
  keys,
  onViewKey,
}: {
  title: string;
  keys: IdentityKeyDto[];
  onViewKey?: (keyId: number) => void;
}) {
  return (
    <div>
      <h3 className="mb-2 text-sm font-medium text-muted-foreground">
        {title}
        <Badge variant="secondary" className="ml-2">
          {keys.length}
        </Badge>
      </h3>
      <div className="rounded-md border">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="w-[60px]">ID</TableHead>
              <TableHead>Purpose</TableHead>
              <TableHead>Security</TableHead>
              <TableHead>Type</TableHead>
              <TableHead className="w-[80px]">Status</TableHead>
              <TableHead className="w-[80px] text-center">
                <Tooltip>
                  <TooltipTrigger asChild>
                    <span className="cursor-help">Private</span>
                  </TooltipTrigger>
                  <TooltipContent>Has private key in local storage</TooltipContent>
                </Tooltip>
              </TableHead>
              {onViewKey && <TableHead className="w-[60px]" />}
            </TableRow>
          </TableHeader>
          <TableBody>
            {keys.map((key) => (
              <KeyRow key={key.keyId} keyData={key} onViewKey={onViewKey} />
            ))}
          </TableBody>
        </Table>
      </div>
    </div>
  );
}

// ─── Key Row ───────────────────────────────────────────────────────

function KeyRow({
  keyData,
  onViewKey,
}: {
  keyData: IdentityKeyDto;
  onViewKey?: (keyId: number) => void;
}) {
  return (
    <TableRow
      className={cn(
        onViewKey && "cursor-pointer hover:bg-muted/50",
        keyData.isDisabled && "opacity-60",
      )}
      onClick={() => onViewKey?.(keyData.keyId)}
      role={onViewKey ? "button" : undefined}
      tabIndex={onViewKey ? 0 : undefined}
      onKeyDown={
        onViewKey
          ? (e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                onViewKey(keyData.keyId);
              }
            }
          : undefined
      }
      aria-label={`Key ${keyData.keyId}, ${keyData.purpose}, ${keyData.securityLevel}`}
    >
      <TableCell className="font-mono text-sm">{keyData.keyId}</TableCell>
      <TableCell>
        <span className="text-sm">{getPurposeLabel(keyData.purpose)}</span>
      </TableCell>
      <TableCell>
        <div className="flex items-center gap-1.5">
          <SecurityLevelIcon
            level={keyData.securityLevel}
            className={cn("size-3.5", getSecurityLevelColor(keyData.securityLevel))}
          />
          <span
            className={cn(
              "text-sm font-medium",
              getSecurityLevelColor(keyData.securityLevel),
            )}
          >
            {keyData.securityLevel.charAt(0) +
              keyData.securityLevel.slice(1).toLowerCase()}
          </span>
        </div>
      </TableCell>
      <TableCell>
        <Badge variant="outline" className="font-mono text-xs">
          {getKeyTypeBadge(keyData.keyType)}
        </Badge>
      </TableCell>
      <TableCell>
        {keyData.isDisabled ? (
          <Badge variant="destructive" className="text-xs">
            Disabled
          </Badge>
        ) : (
          <Badge
            variant="secondary"
            className="bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-400 text-xs"
          >
            Active
          </Badge>
        )}
      </TableCell>
      <TableCell className="text-center">
        {keyData.hasPrivateKey ? (
          <Tooltip>
            <TooltipTrigger asChild>
              <Unlock className="mx-auto size-4 text-green-600 dark:text-green-400" aria-label="Private key available" />
            </TooltipTrigger>
            <TooltipContent>Private key available</TooltipContent>
          </Tooltip>
        ) : (
          <Tooltip>
            <TooltipTrigger asChild>
              <Lock className="mx-auto size-4 text-muted-foreground/50" aria-label="No private key" />
            </TooltipTrigger>
            <TooltipContent>No private key</TooltipContent>
          </Tooltip>
        )}
      </TableCell>
      {onViewKey && (
        <TableCell>
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label={`View key ${keyData.keyId}`}
            onClick={(e) => {
              e.stopPropagation();
              onViewKey(keyData.keyId);
            }}
          >
            <Eye className="size-3.5" />
          </Button>
        </TableCell>
      )}
    </TableRow>
  );
}
