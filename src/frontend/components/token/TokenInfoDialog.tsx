import { useState } from "react";
import { Coins, ChevronDown, ChevronRight } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import { CopyButton } from "@/components/shared/CopyButton";
import { JsonViewer } from "@/components/shared/JsonViewer";

// ─── Types ──────────────────────────────────────────────────────────

export interface TokenInfoData {
  /** Token name. */
  name: string | null;
  /** Token ID (hex). */
  tokenId: string;
  /** Contract ID (hex). */
  contractId: string;
  /** Token position within the contract. */
  tokenPosition: number;
  /** Description (optional). */
  description?: string | null;
  /** Number of decimals for display. */
  decimals: number;
  /** Base supply (string, u128). */
  baseSupply?: string | null;
  /** Max supply (string, u128) — null means unlimited. */
  maxSupply?: string | null;
  /** Whether perpetual distribution is configured. */
  hasPerpetualDistribution?: boolean;
  /** Whether pre-programmed distribution is configured. */
  hasPreprogrammedDistribution?: boolean;
  /** Contract owner identity ID (hex). */
  ownerIdentityId?: string | null;
  /** Whether the token is currently paused. */
  paused?: boolean;
  /** Full token configuration as JSON (for "View Schema" section). */
  configurationJson?: unknown;
}

export interface TokenInfoDialogProps {
  /** Whether the dialog is open. */
  open: boolean;
  /** Called when the dialog should close. */
  onOpenChange: (open: boolean) => void;
  /** Token data to display. */
  token: TokenInfoData | null;
}

// ─── Helpers ────────────────────────────────────────────────────────

/** Truncate a hex string for display. */
function truncateHex(hex: string, chars = 12): string {
  if (hex.length <= chars * 2 + 3) return hex;
  return `${hex.slice(0, chars)}...${hex.slice(-chars)}`;
}

/** Format a supply value for display. */
function formatSupply(value: string | null | undefined, decimals: number): string {
  if (!value || value === "0") return "0";
  if (decimals === 0) return value;

  const padded = value.padStart(decimals + 1, "0");
  const intPart = padded.slice(0, padded.length - decimals);
  const fracPart = padded.slice(padded.length - decimals);
  const trimmedFrac = fracPart.replace(/0+$/, "");
  if (!trimmedFrac) return intPart;
  return `${intPart}.${trimmedFrac}`;
}

// ─── Collapsible section ────────────────────────────────────────────

function CollapsibleSection({
  title,
  defaultOpen = true,
  children,
}: {
  title: string;
  defaultOpen?: boolean;
  children: React.ReactNode;
}) {
  const [open, setOpen] = useState(defaultOpen);

  return (
    <div className="space-y-2">
      <button
        type="button"
        className="flex items-center gap-1.5 text-sm font-semibold text-foreground hover:text-foreground/80 transition-colors"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
      >
        {open ? (
          <ChevronDown className="size-4" />
        ) : (
          <ChevronRight className="size-4" />
        )}
        {title}
      </button>
      {open && <div className="pl-5.5">{children}</div>}
    </div>
  );
}

// ─── Info row ───────────────────────────────────────────────────────

function InfoRow({
  label,
  value,
  mono = false,
  copyable = false,
}: {
  label: string;
  value: string;
  mono?: boolean;
  copyable?: boolean;
}) {
  return (
    <div className="flex items-start gap-3 py-1">
      <span className="text-sm text-muted-foreground min-w-[160px] shrink-0">
        {label}
      </span>
      <div className="flex items-center gap-1 min-w-0">
        <span
          className={`text-sm break-all ${mono ? "font-mono" : ""}`}
          data-testid={`info-${label.toLowerCase().replace(/\s+/g, "-")}`}
        >
          {value}
        </span>
        {copyable && <CopyButton value={value} />}
      </div>
    </div>
  );
}

// ─── Component ──────────────────────────────────────────────────────

export function TokenInfoDialog({
  open,
  onOpenChange,
  token,
}: TokenInfoDialogProps) {
  const [showSchema, setShowSchema] = useState(false);

  if (!token) return null;

  const displayName = token.name ?? "Unnamed Token";

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <Coins className="size-5 text-dash-blue" />
            <DialogTitle>{displayName}</DialogTitle>
            {token.paused && (
              <Badge variant="secondary" className="text-xs">
                Paused
              </Badge>
            )}
          </div>
          <DialogDescription>
            Token configuration and metadata details
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          {/* Basic Information */}
          <CollapsibleSection title="Basic Information">
            <div className="space-y-0.5">
              <InfoRow
                label="Description"
                value={token.description || "No description"}
              />
              <InfoRow
                label="Base Supply"
                value={
                  token.baseSupply
                    ? formatSupply(token.baseSupply, token.decimals)
                    : "N/A"
                }
              />
              <InfoRow
                label="Max Supply"
                value={
                  token.maxSupply
                    ? formatSupply(token.maxSupply, token.decimals)
                    : "Unlimited"
                }
              />
              <InfoRow
                label="Decimals"
                value={String(token.decimals)}
              />
              <InfoRow
                label="Perpetual Distribution"
                value={token.hasPerpetualDistribution ? "Yes" : "No"}
              />
              <InfoRow
                label="Preprogrammed Distribution"
                value={token.hasPreprogrammedDistribution ? "Yes" : "No"}
              />

              <Separator className="my-2" />

              <InfoRow
                label="Token ID"
                value={truncateHex(token.tokenId)}
                mono
                copyable
              />
              <InfoRow
                label="Contract ID"
                value={truncateHex(token.contractId)}
                mono
                copyable
              />
              {token.ownerIdentityId && (
                <InfoRow
                  label="Contract Owner"
                  value={truncateHex(token.ownerIdentityId)}
                  mono
                  copyable
                />
              )}
              <InfoRow
                label="Token Position"
                value={String(token.tokenPosition)}
              />
            </div>
          </CollapsibleSection>

          {/* Token Configuration (JSON schema) */}
          {!!token.configurationJson && (
            <CollapsibleSection title="Token Configuration" defaultOpen={false}>
              <p className="text-sm text-muted-foreground mb-2">
                Full token configuration schema:
              </p>
              <JsonViewer
                data={token.configurationJson}
                defaultExpanded={false}
                expandDepth={2}
                showCopy
                className="max-h-[300px]"
              />
            </CollapsibleSection>
          )}

          {/* View Schema button (when configurationJson is available) */}
          {!!token.configurationJson && !showSchema && (
            <div className="flex justify-center">
              {/* The schema is shown inline in the collapsible above;
                  this button opens a full expanded view */}
            </div>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Close
          </Button>
        </DialogFooter>
      </DialogContent>

      {/* Full Schema Dialog (nested) */}
      <Dialog open={showSchema} onOpenChange={setShowSchema}>
        <DialogContent className="sm:max-w-4xl max-h-[90vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>
              {displayName} — Full Configuration Schema
            </DialogTitle>
            <DialogDescription>
              Complete token contract configuration as JSON
            </DialogDescription>
          </DialogHeader>
          <JsonViewer
            data={token.configurationJson}
            defaultExpanded
            expandDepth={-1}
            showCopy
            className="max-h-[70vh]"
          />
          <DialogFooter>
            <Button variant="outline" onClick={() => setShowSchema(false)}>
              Close
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </Dialog>
  );
}

// ─── Exports ────────────────────────────────────────────────────────

export { truncateHex, formatSupply };
