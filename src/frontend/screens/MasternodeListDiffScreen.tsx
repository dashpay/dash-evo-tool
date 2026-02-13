import { useCallback, useState, useRef, useMemo } from "react";
import { useEffect } from "react";
import { commands, events } from "@/bindings";
import type {
  QrInfoDto,
  QuorumSnapshotDto,
  MnListDiffDto,
  SimpleQuorumEntryDto,
  ZmqChainLockedBlockEvent,
  ZmqIsLockedTransactionEvent,
  TaskResultEvent,
} from "@/bindings";
import { ToolPageLayout } from "@/components/tools/ToolPageLayout";
import { CopyButton } from "@/components/shared/CopyButton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  CheckCircle2,
  XCircle,
  Blocks,
  ArrowRightLeft,
  Radio,
  Upload,
  FolderOpen,
  Loader2,
  AlertCircle,
  X,
  Shield,
  Search,
  GitCompare,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { toastError } from "@/lib/toastError";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface ChainLockedBlock {
  blockHeight: number;
  blockHash: string;
  txCount: number;
  txIds: string[];
  rawBlock: string;
  rawChainLock: string;
  signature: string;
  isValid: boolean;
}

interface InstantSendTransaction {
  txid: string;
  rawTx: string;
  rawIsLock: string;
  isValid: boolean;
}

type SelectedItem =
  | { type: "block"; block: ChainLockedBlock }
  | { type: "tx"; tx: InstantSendTransaction };

// QR Info types
type QrField =
  | "Quorum Snapshots"
  | "Masternode List Diffs"
  | "Rotated Quorums"
  | "Quorum Snapshot List"
  | "MN List Diff List";

type QrSelectedDetail =
  | { type: "snapshot"; label: string; data: QuorumSnapshotDto }
  | { type: "diff"; label: string; data: MnListDiffDto }
  | { type: "quorum"; label: string; data: SimpleQuorumEntryDto };

interface ChainLockSigEntry {
  height: number;
  blockHash: string;
  signature: string | null;
}

// Quorum viewer: aggregated quorum data by LLMQ type
interface QuorumViewerEntry {
  quorumHash: string;
  llmqType: number;
  signerCount: number;
  validMemberCount: number;
  totalMembers: number;
  quorumPublicKey: string;
  quorumIndex: number | null;
  sourceHeight: number;
}

// Pending operation type for the input area
type PendingOp =
  | "fetchDiff"
  | "fetchQrInfo"
  | "fetchDmlsNoRotation"
  | "fetchDmlsWithRotation"
  | "fetchChainLocks"
  | null;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Truncate a hex string for display: first 8...last 8 chars. */
function truncateHex(hex: string, chars = 8): string {
  if (hex.length <= chars * 2 + 1) return hex;
  return `${hex.slice(0, chars)}…${hex.slice(-chars)}`;
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function ValidationBadge({ isValid }: { isValid: boolean }) {
  return isValid ? (
    <CheckCircle2
      className="size-4 text-green-500 shrink-0"
      aria-label="Valid"
    />
  ) : (
    <XCircle className="size-4 text-red-500 shrink-0" aria-label="Invalid" />
  );
}

function ChainLockDetails({ block }: { block: ChainLockedBlock }) {
  return (
    <div className="flex flex-col gap-4" data-testid="chain-lock-details">
      <h3 className="text-lg font-semibold">ChainLock Details</h3>

      {/* Summary */}
      <div className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
        <span className="text-muted-foreground">Block Height</span>
        <span className="font-mono">{block.blockHeight}</span>

        <span className="text-muted-foreground">Block Hash</span>
        <span className="font-mono break-all">{block.blockHash}</span>

        <span className="text-muted-foreground">Valid</span>
        <span className="flex items-center gap-1">
          <ValidationBadge isValid={block.isValid} />
          {block.isValid ? "Yes" : "No"}
        </span>
      </div>

      {/* Block Transactions */}
      <div>
        <h4 className="text-sm font-semibold mb-2">
          Block Transactions ({block.txIds.length})
        </h4>
        {block.txIds.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No transactions in this block.
          </p>
        ) : (
          <div className="max-h-40 overflow-y-auto rounded border bg-muted/30 p-2">
            {block.txIds.map((txid, i) => (
              <div
                key={txid}
                className="flex items-center gap-2 py-0.5 text-xs font-mono"
              >
                <span className="text-muted-foreground w-6 text-right shrink-0">
                  {i + 1}.
                </span>
                <span className="break-all">{txid}</span>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Quorum Signature */}
      <div>
        <h4 className="text-sm font-semibold mb-1">Quorum Signature</h4>
        <div className="flex items-start gap-2">
          <p className="text-xs font-mono break-all bg-muted/30 p-2 rounded border flex-1">
            {block.signature}
          </p>
          <CopyButton value={block.signature} size="sm" />
        </div>
      </div>

      {/* Raw Data */}
      <div>
        <h4 className="text-sm font-semibold mb-1">Raw Data</h4>
        <div className="space-y-2">
          <div>
            <span className="text-xs text-muted-foreground">Block Data</span>
            <div className="flex items-start gap-2">
              <p className="text-xs font-mono break-all bg-muted/30 p-2 rounded border flex-1 max-h-32 overflow-y-auto">
                {block.rawBlock}
              </p>
              <CopyButton value={block.rawBlock} size="sm" />
            </div>
          </div>
          <div>
            <span className="text-xs text-muted-foreground">Lock Data</span>
            <div className="flex items-start gap-2">
              <p className="text-xs font-mono break-all bg-muted/30 p-2 rounded border flex-1 max-h-24 overflow-y-auto">
                {block.rawChainLock}
              </p>
              <CopyButton value={block.rawChainLock} size="sm" />
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function InstantSendDetails({ tx }: { tx: InstantSendTransaction }) {
  return (
    <div className="flex flex-col gap-4" data-testid="instant-send-details">
      <h3 className="text-lg font-semibold">Instant Send Details</h3>

      {/* Summary */}
      <div className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
        <span className="text-muted-foreground">TxID</span>
        <span className="font-mono break-all">{tx.txid}</span>

        <span className="text-muted-foreground">Valid</span>
        <span className="flex items-center gap-1">
          <ValidationBadge isValid={tx.isValid} />
          {tx.isValid ? "Yes" : "No"}
        </span>
      </div>

      {/* Raw Data */}
      <div>
        <h4 className="text-sm font-semibold mb-1">Raw Data</h4>
        <div className="space-y-2">
          <div>
            <span className="text-xs text-muted-foreground">
              Transaction Data
            </span>
            <div className="flex items-start gap-2">
              <p className="text-xs font-mono break-all bg-muted/30 p-2 rounded border flex-1 max-h-32 overflow-y-auto">
                {tx.rawTx}
              </p>
              <CopyButton value={tx.rawTx} size="sm" />
            </div>
          </div>
          <div>
            <span className="text-xs text-muted-foreground">Lock Data</span>
            <div className="flex items-start gap-2">
              <p className="text-xs font-mono break-all bg-muted/30 p-2 rounded border flex-1 max-h-24 overflow-y-auto">
                {tx.rawIsLock}
              </p>
              <CopyButton value={tx.rawIsLock} size="sm" />
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Core Items Tab
// ---------------------------------------------------------------------------

function CoreItemsTab() {
  const [blocks, setBlocks] = useState<Map<number, ChainLockedBlock>>(
    new Map(),
  );
  const [transactions, setTransactions] = useState<InstantSendTransaction[]>(
    [],
  );
  const [selected, setSelected] = useState<SelectedItem | null>(null);

  // Subscribe to ZMQ events
  useEffect(() => {
    let cleanupBlock: (() => void) | undefined;
    let cleanupTx: (() => void) | undefined;

    const subscribe = async () => {
      cleanupBlock = await events.zmqChainLockedBlockEvent.listen(
        (event: { payload: ZmqChainLockedBlockEvent }) => {
          const data = event.payload;
          const block: ChainLockedBlock = {
            blockHeight: data.blockHeight,
            blockHash: data.blockHash,
            txCount: data.txCount,
            txIds: data.txIds,
            rawBlock: data.rawBlock,
            rawChainLock: data.rawChainLock,
            signature: data.signature,
            isValid: data.isValid,
          };
          setBlocks((prev) => {
            const next = new Map(prev);
            next.set(block.blockHeight, block);
            return next;
          });
        },
      );

      cleanupTx = await events.zmqIsLockedTransactionEvent.listen(
        (event: { payload: ZmqIsLockedTransactionEvent }) => {
          const data = event.payload;
          const tx: InstantSendTransaction = {
            txid: data.txid,
            rawTx: data.rawTx,
            rawIsLock: data.rawIsLock,
            isValid: data.isValid,
          };
          setTransactions((prev) => [...prev, tx]);
        },
      );
    };

    subscribe().catch(() => {});
    return () => {
      cleanupBlock?.();
      cleanupTx?.();
    };
  }, []);

  // Sort blocks descending by height for display
  const sortedBlocks = Array.from(blocks.values()).sort(
    (a, b) => b.blockHeight - a.blockHeight,
  );

  return (
    <div className="flex gap-4 min-h-0 flex-1" data-testid="core-items-tab">
      {/* Left Column: ChainLocked Blocks */}
      <div className="w-64 shrink-0 flex flex-col min-h-0">
        <h3 className="text-sm font-semibold mb-2 flex items-center gap-1.5">
          <Blocks className="size-4" />
          ChainLocked Blocks
          {sortedBlocks.length > 0 && (
            <Badge variant="secondary" className="text-xs">
              {sortedBlocks.length}
            </Badge>
          )}
        </h3>
        <div
          className="flex-1 overflow-y-auto rounded border bg-card"
          role="listbox"
          aria-label="ChainLocked Blocks"
        >
          {sortedBlocks.length === 0 ? (
            <div className="p-4 text-sm text-muted-foreground text-center">
              <Radio className="size-5 mx-auto mb-2 opacity-40" />
              Waiting for chain-locked blocks…
            </div>
          ) : (
            sortedBlocks.map((block) => {
              const isSelected =
                selected?.type === "block" &&
                selected.block.blockHeight === block.blockHeight;
              return (
                <button
                  key={block.blockHeight}
                  onClick={() => setSelected({ type: "block", block })}
                  className={cn(
                    "w-full text-left px-3 py-2 text-xs border-b last:border-b-0 transition-colors",
                    "hover:bg-accent/50 focus:bg-accent/50 focus:outline-none",
                    isSelected && "bg-accent text-accent-foreground",
                  )}
                  aria-selected={isSelected}
                  role="option"
                >
                  <div className="flex items-center gap-1.5">
                    <ValidationBadge isValid={block.isValid} />
                    <span className="font-medium">{block.blockHeight}</span>
                  </div>
                  <div className="font-mono text-muted-foreground mt-0.5 truncate">
                    {truncateHex(block.blockHash)}
                  </div>
                </button>
              );
            })
          )}
        </div>
      </div>

      {/* Middle Column: Instant Send Transactions */}
      <div className="w-72 shrink-0 flex flex-col min-h-0">
        <h3 className="text-sm font-semibold mb-2 flex items-center gap-1.5">
          <ArrowRightLeft className="size-4" />
          Instant Send Transactions
          {transactions.length > 0 && (
            <Badge variant="secondary" className="text-xs">
              {transactions.length}
            </Badge>
          )}
        </h3>
        <div
          className="flex-1 overflow-y-auto rounded border bg-card"
          role="listbox"
          aria-label="Instant Send Transactions"
        >
          {transactions.length === 0 ? (
            <div className="p-4 text-sm text-muted-foreground text-center">
              <Radio className="size-5 mx-auto mb-2 opacity-40" />
              Waiting for IS-locked transactions…
            </div>
          ) : (
            transactions.map((tx, idx) => {
              const isSelected =
                selected?.type === "tx" && selected.tx.txid === tx.txid;
              return (
                <button
                  key={`${tx.txid}-${idx}`}
                  onClick={() => setSelected({ type: "tx", tx })}
                  className={cn(
                    "w-full text-left px-3 py-2 text-xs border-b last:border-b-0 transition-colors",
                    "hover:bg-accent/50 focus:bg-accent/50 focus:outline-none",
                    isSelected && "bg-accent text-accent-foreground",
                  )}
                  aria-selected={isSelected}
                  role="option"
                >
                  <div className="flex items-center gap-1.5">
                    <ValidationBadge isValid={tx.isValid} />
                    <span className="font-mono truncate">
                      TxID: {truncateHex(tx.txid)}
                    </span>
                  </div>
                </button>
              );
            })
          )}
        </div>
      </div>

      {/* Right Column: Details */}
      <div className="flex-1 min-h-0 overflow-y-auto rounded border bg-card p-4">
        {selected === null ? (
          <div className="h-full flex items-center justify-center text-sm text-muted-foreground">
            Select an item to view details.
          </div>
        ) : selected.type === "block" ? (
          <ChainLockDetails block={selected.block} />
        ) : (
          <InstantSendDetails tx={selected.tx} />
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// QR Info Detail Views
// ---------------------------------------------------------------------------

function SnapshotDetail({ snapshot }: { snapshot: QuorumSnapshotDto }) {
  const activeCount = snapshot.activeQuorumMembers.filter(Boolean).length;
  return (
    <div className="flex flex-col gap-3" data-testid="snapshot-detail">
      <h3 className="text-lg font-semibold">Quorum Snapshot</h3>
      <div className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
        <span className="text-muted-foreground">Skip List Mode</span>
        <span className="font-mono">{snapshot.skipListMode}</span>
      </div>

      <div>
        <h4 className="text-sm font-semibold mb-1">
          Active Quorum Members ({activeCount}/{snapshot.activeQuorumMembers.length})
        </h4>
        <div className="max-h-48 overflow-y-auto rounded border bg-muted/30 p-2 text-xs font-mono">
          {snapshot.activeQuorumMembers.map((active, i) => (
            <div key={i} className="py-0.5">
              Member {i}: {active ? "Active" : "Inactive"}
            </div>
          ))}
        </div>
      </div>

      <div>
        <h4 className="text-sm font-semibold mb-1">
          Skip List ({snapshot.skipList.length} entries)
        </h4>
        {snapshot.skipList.length === 0 ? (
          <p className="text-sm text-muted-foreground">Empty skip list.</p>
        ) : (
          <div className="max-h-32 overflow-y-auto rounded border bg-muted/30 p-2 text-xs font-mono">
            {snapshot.skipList.map((entry, i) => (
              <div key={i} className="py-0.5">
                Entry {i}: {entry}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function MnListDiffDetail({ diff }: { diff: MnListDiffDto }) {
  return (
    <div className="flex flex-col gap-3" data-testid="mnlistdiff-detail">
      <h3 className="text-lg font-semibold">MnListDiff</h3>

      <div className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
        <span className="text-muted-foreground">Version</span>
        <span className="font-mono">{diff.version}</span>

        <span className="text-muted-foreground">Base Block Hash</span>
        <span className="font-mono break-all">{diff.baseBlockHash}</span>

        <span className="text-muted-foreground">Block Hash</span>
        <span className="font-mono break-all">{diff.blockHash}</span>

        <span className="text-muted-foreground">Total Transactions</span>
        <span className="font-mono">{diff.totalTransactions}</span>
      </div>

      {/* Merkle Tree */}
      <div>
        <h4 className="text-sm font-semibold mb-1">
          Merkle Hashes ({diff.merkleHashes.length})
        </h4>
        {diff.merkleHashes.length > 0 && (
          <div className="max-h-32 overflow-y-auto rounded border bg-muted/30 p-2 text-xs font-mono">
            {diff.merkleHashes.map((h, i) => (
              <div key={i} className="py-0.5 break-all">
                {h}
              </div>
            ))}
          </div>
        )}
        <p className="text-xs text-muted-foreground mt-1">
          Merkle flags: {diff.merkleFlagsLen} bytes
        </p>
      </div>

      {/* Coinbase */}
      <div className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
        <span className="text-muted-foreground">Coinbase TxID</span>
        <span className="font-mono break-all">{diff.coinbaseTxid}</span>
        <span className="text-muted-foreground">Coinbase Size</span>
        <span className="font-mono">{diff.coinbaseSize} bytes</span>
      </div>

      {/* Masternode Changes */}
      <div>
        <h4 className="text-sm font-semibold mb-1">
          New Masternodes ({diff.newMasternodes.length}) / Deleted ({diff.deletedMasternodes.length})
        </h4>
        {diff.newMasternodes.length > 0 && (
          <div className="max-h-32 overflow-y-auto rounded border bg-muted/30 p-2 text-xs font-mono mb-1">
            {diff.newMasternodes.map((mn, i) => (
              <div key={i} className="py-0.5 break-all">
                {truncateHex(mn.proRegTxHash)} {mn.address}
              </div>
            ))}
          </div>
        )}
        {diff.deletedMasternodes.length > 0 && (
          <div className="max-h-24 overflow-y-auto rounded border bg-muted/30 p-2 text-xs font-mono">
            {diff.deletedMasternodes.map((h, i) => (
              <div key={i} className="py-0.5 break-all">
                {h}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Quorum Changes */}
      <div>
        <h4 className="text-sm font-semibold mb-1">
          New Quorums ({diff.newQuorums.length}) / Deleted ({diff.deletedQuorums.length})
        </h4>
        {diff.newQuorums.length > 0 && (
          <div className="max-h-32 overflow-y-auto rounded border bg-muted/30 p-2 text-xs font-mono mb-1">
            {diff.newQuorums.map((q, i) => (
              <div key={i} className="py-0.5 break-all">
                Quorum {truncateHex(q.quorumHash)} Type: {q.llmqType}
              </div>
            ))}
          </div>
        )}
        {diff.deletedQuorums.length > 0 && (
          <div className="max-h-24 overflow-y-auto rounded border bg-muted/30 p-2 text-xs font-mono">
            {diff.deletedQuorums.map((q, i) => (
              <div key={i} className="py-0.5 break-all">
                Quorum {truncateHex(q.quorumHash)} Type: {q.llmqType}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* ChainLock Signatures */}
      {diff.chainlockSigCount > 0 && (
        <div>
          <h4 className="text-sm font-semibold mb-1">
            ChainLock Signatures ({diff.chainlockSigCount})
          </h4>
          <div className="max-h-32 overflow-y-auto rounded border bg-muted/30 p-2 text-xs font-mono">
            {diff.chainlockSignatures.map((sig, i) => (
              <div key={i} className="py-0.5 break-all">
                Sig {i}: {truncateHex(sig.signature)} for [{sig.indexSet.join(", ")}]
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function QuorumEntryDetail({ entry }: { entry: SimpleQuorumEntryDto }) {
  const signerCount = entry.signers.filter(Boolean).length;
  const validCount = entry.validMembers.filter(Boolean).length;
  const memberCount = Math.max(entry.signers.length, entry.validMembers.length);

  return (
    <div className="flex flex-col gap-3" data-testid="quorum-entry-detail">
      <h3 className="text-lg font-semibold">Quorum Entry</h3>

      <div className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
        <span className="text-muted-foreground">Version</span>
        <span className="font-mono">{entry.version}</span>

        <span className="text-muted-foreground">Quorum Type</span>
        <span className="font-mono">{entry.llmqType}</span>

        <span className="text-muted-foreground">Quorum Hash</span>
        <span className="font-mono break-all">{entry.quorumHash}</span>

        {entry.quorumIndex !== null && (
          <>
            <span className="text-muted-foreground">Quorum Index</span>
            <span className="font-mono">{entry.quorumIndex}</span>
          </>
        )}
      </div>

      {/* Members Summary */}
      <div className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
        <span className="text-muted-foreground">Total Signers</span>
        <span className="font-mono">
          {signerCount} / {memberCount}
        </span>
        <span className="text-muted-foreground">Valid Members</span>
        <span className="font-mono">
          {validCount} / {memberCount}
        </span>
      </div>

      {/* Members Grid (8 columns) */}
      <div>
        <h4 className="text-sm font-semibold mb-1">
          Signers & Valid Members
        </h4>
        <div
          className="grid gap-0.5 text-xs font-mono"
          style={{ gridTemplateColumns: "repeat(8, 1fr)" }}
          data-testid="members-grid"
        >
          {Array.from({ length: memberCount }).map((_, i) => {
            const isSigner = entry.signers[i] ?? false;
            const isValid = entry.validMembers[i] ?? false;
            const label = `${isSigner ? "\u2714" : "\u2718"}${isValid ? "\u2714" : "\u2718"}`;
            return (
              <span
                key={i}
                title={`Member ${i}`}
                className={cn(
                  "text-center p-0.5 rounded text-[10px]",
                  isSigner && isValid && "bg-green-500/20 text-green-700 dark:text-green-400",
                  isSigner && !isValid && "bg-yellow-500/20 text-yellow-700 dark:text-yellow-400",
                  !isSigner && isValid && "bg-blue-500/20 text-blue-700 dark:text-blue-400",
                  !isSigner && !isValid && "bg-muted text-muted-foreground",
                )}
              >
                {label}
              </span>
            );
          })}
        </div>
      </div>

      {/* Keys & Signatures */}
      <div>
        <h4 className="text-sm font-semibold mb-1">Quorum Public Key</h4>
        <div className="flex items-start gap-2">
          <p className="text-xs font-mono break-all bg-muted/30 p-2 rounded border flex-1">
            {entry.quorumPublicKey}
          </p>
          <CopyButton value={entry.quorumPublicKey} size="sm" />
        </div>
      </div>

      <div>
        <h4 className="text-sm font-semibold mb-1">Verification Vector Hash</h4>
        <div className="flex items-start gap-2">
          <p className="text-xs font-mono break-all bg-muted/30 p-2 rounded border flex-1">
            {entry.quorumVvecHash}
          </p>
          <CopyButton value={entry.quorumVvecHash} size="sm" />
        </div>
      </div>

      <div>
        <h4 className="text-sm font-semibold mb-1">Threshold Signature</h4>
        <div className="flex items-start gap-2">
          <p className="text-xs font-mono break-all bg-muted/30 p-2 rounded border flex-1">
            {entry.thresholdSig}
          </p>
          <CopyButton value={entry.thresholdSig} size="sm" />
        </div>
      </div>

      <div>
        <h4 className="text-sm font-semibold mb-1">All Commitment Aggregated Signature</h4>
        <div className="flex items-start gap-2">
          <p className="text-xs font-mono break-all bg-muted/30 p-2 rounded border flex-1">
            {entry.allCommitmentAggregatedSignature}
          </p>
          <CopyButton value={entry.allCommitmentAggregatedSignature} size="sm" />
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// QR Info Tab
// ---------------------------------------------------------------------------

const QR_FIELDS: QrField[] = [
  "Quorum Snapshots",
  "Masternode List Diffs",
  "Rotated Quorums",
  "Quorum Snapshot List",
  "MN List Diff List",
];

/** Build the list items for the center panel based on the selected field. */
function getFieldItems(
  qrInfo: QrInfoDto,
  field: QrField,
): { label: string; detail: QrSelectedDetail }[] {
  switch (field) {
    case "Quorum Snapshots": {
      const items: { label: string; detail: QrSelectedDetail }[] = [
        { label: "Snapshot h-c", detail: { type: "snapshot", label: "Snapshot h-c", data: qrInfo.snapshotHMinusC } },
        { label: "Snapshot h-2c", detail: { type: "snapshot", label: "Snapshot h-2c", data: qrInfo.snapshotHMinus2C } },
        { label: "Snapshot h-3c", detail: { type: "snapshot", label: "Snapshot h-3c", data: qrInfo.snapshotHMinus3C } },
      ];
      if (qrInfo.snapshotHMinus4C) {
        items.push({ label: "Snapshot h-4c", detail: { type: "snapshot", label: "Snapshot h-4c", data: qrInfo.snapshotHMinus4C } });
      }
      return items;
    }
    case "Masternode List Diffs": {
      const items: { label: string; detail: QrSelectedDetail }[] = [
        { label: "MNListDiff h-3c", detail: { type: "diff", label: "MNListDiff h-3c", data: qrInfo.diffHMinus3C } },
        { label: "MNListDiff h-2c", detail: { type: "diff", label: "MNListDiff h-2c", data: qrInfo.diffHMinus2C } },
        { label: "MNListDiff h-c", detail: { type: "diff", label: "MNListDiff h-c", data: qrInfo.diffHMinusC } },
        { label: "MNListDiff h", detail: { type: "diff", label: "MNListDiff h", data: qrInfo.diffH } },
        { label: "MNListDiff Tip", detail: { type: "diff", label: "MNListDiff Tip", data: qrInfo.diffTip } },
      ];
      if (qrInfo.diffHMinus4C) {
        items.push({ label: "MNListDiff h-4c", detail: { type: "diff", label: "MNListDiff h-4c", data: qrInfo.diffHMinus4C } });
      }
      return items;
    }
    case "Rotated Quorums":
      return qrInfo.lastCommitments.map((q, i) => ({
        label: `Quorum at Index ${i}`,
        detail: { type: "quorum" as const, label: `Quorum at Index ${i}`, data: q },
      }));
    case "Quorum Snapshot List":
      return qrInfo.quorumSnapshotList.map((s, i) => ({
        label: `Snapshot ${i}`,
        detail: { type: "snapshot" as const, label: `Snapshot ${i}`, data: s },
      }));
    case "MN List Diff List":
      return qrInfo.mnListDiffList.map((d, i) => ({
        label: `MNListDiff ${i}`,
        detail: { type: "diff" as const, label: `MNListDiff ${i}`, data: d },
      }));
  }
}

function QrInfoTab({ externalQrInfo }: { externalQrInfo?: QrInfoDto | null }) {
  const [qrInfo, setQrInfo] = useState<QrInfoDto | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedField, setSelectedField] = useState<QrField | null>(null);
  const [selectedItemIndex, setSelectedItemIndex] = useState<number | null>(null);
  const [selectedDetail, setSelectedDetail] = useState<QrSelectedDetail | null>(null);

  // Accept externally fetched QR info (from "Get single end QR info" button)
  useEffect(() => {
    if (externalQrInfo) {
      setQrInfo(externalQrInfo);
      setSelectedField(null);
      setSelectedItemIndex(null);
      setSelectedDetail(null);
    }
  }, [externalQrInfo]);

  const handleLoadFile = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await commands.qrinfoLoadFile();
      if (result.status === "ok") {
        setQrInfo(result.data);
        setSelectedField(null);
        setSelectedItemIndex(null);
        setSelectedDetail(null);
      } else {
        if (result.error !== "File selection cancelled") {
          setError(result.error);
          toastError(result.error);
        }
      }
    } catch (e) {
      const msg = String(e);
      setError(msg);
      toastError(msg);
    } finally {
      setLoading(false);
    }
  }, []);

  const handleSelectField = useCallback((field: QrField) => {
    setSelectedField(field);
    setSelectedItemIndex(null);
    setSelectedDetail(null);
  }, []);

  const handleSelectItem = useCallback(
    (index: number, detail: QrSelectedDetail) => {
      setSelectedItemIndex(index);
      setSelectedDetail(detail);
    },
    [],
  );

  const fieldItems = qrInfo && selectedField ? getFieldItems(qrInfo, selectedField) : [];

  return (
    <div className="flex flex-col gap-3 min-h-0 flex-1" data-testid="qr-info-tab">
      {/* Top bar: Load / Save buttons */}
      <div className="flex items-center gap-2">
        <Button
          variant="outline"
          size="sm"
          onClick={handleLoadFile}
          disabled={loading}
          data-testid="load-qrinfo-button"
        >
          <FolderOpen className="size-4 mr-1.5" />
          {loading ? "Loading…" : "Load QR Info"}
        </Button>
        {qrInfo && (
          <span className="text-xs text-muted-foreground font-mono">
            Tip: {truncateHex(qrInfo.tipBlockHash)}
          </span>
        )}
      </div>

      {error && (
        <div className="text-sm text-destructive bg-destructive/10 p-2 rounded border border-destructive/20">
          {error}
        </div>
      )}

      {!qrInfo ? (
        <div className="flex-1 flex items-center justify-center text-sm text-muted-foreground">
          <div className="text-center">
            <Upload className="size-8 mx-auto mb-3 opacity-30" />
            <p>Load a QRInfo .dat file to inspect quorum rotation data.</p>
            <p className="text-xs mt-1">Supports consensus and bincode formats.</p>
          </div>
        </div>
      ) : (
        <div className="flex gap-3 min-h-0 flex-1">
          {/* Left Panel: Field List */}
          <div className="w-48 shrink-0 flex flex-col min-h-0">
            <h4 className="text-xs font-semibold text-muted-foreground mb-1.5">
              QRInfo Fields:
            </h4>
            <div
              className="flex-1 overflow-y-auto rounded border bg-card"
              role="listbox"
              aria-label="QRInfo Fields"
            >
              {QR_FIELDS.map((field) => {
                const isSelected = selectedField === field;
                return (
                  <button
                    key={field}
                    onClick={() => handleSelectField(field)}
                    className={cn(
                      "w-full text-left px-3 py-2 text-xs border-b last:border-b-0 transition-colors",
                      "hover:bg-accent/50 focus:bg-accent/50 focus:outline-none",
                      isSelected && "bg-accent text-accent-foreground font-medium",
                    )}
                    aria-selected={isSelected}
                    role="option"
                  >
                    {field}
                  </button>
                );
              })}
            </div>
          </div>

          {/* Center Panel: Items for selected field */}
          <div className="w-56 shrink-0 flex flex-col min-h-0">
            <h4 className="text-xs font-semibold text-muted-foreground mb-1.5">
              {selectedField ? `${selectedField}:` : "Select a field"}
            </h4>
            <div
              className="flex-1 overflow-y-auto rounded border bg-card"
              role="listbox"
              aria-label="Field Items"
            >
              {!selectedField ? (
                <div className="p-4 text-xs text-muted-foreground text-center">
                  Select a field from the left panel.
                </div>
              ) : fieldItems.length === 0 ? (
                <div className="p-4 text-xs text-muted-foreground text-center">
                  No items available.
                </div>
              ) : (
                fieldItems.map((item, i) => {
                  const isSelected = selectedItemIndex === i;
                  return (
                    <button
                      key={i}
                      onClick={() => handleSelectItem(i, item.detail)}
                      className={cn(
                        "w-full text-left px-3 py-2 text-xs border-b last:border-b-0 transition-colors",
                        "hover:bg-accent/50 focus:bg-accent/50 focus:outline-none",
                        isSelected && "bg-accent text-accent-foreground font-medium",
                      )}
                      aria-selected={isSelected}
                      role="option"
                    >
                      {item.label}
                    </button>
                  );
                })
              )}
            </div>
          </div>

          {/* Right Panel: Detail View */}
          <div className="flex-1 min-h-0 overflow-y-auto rounded border bg-card p-4">
            {!selectedDetail ? (
              <div className="h-full flex items-center justify-center text-sm text-muted-foreground">
                Select an item to view details.
              </div>
            ) : selectedDetail.type === "snapshot" ? (
              <SnapshotDetail snapshot={selectedDetail.data} />
            ) : selectedDetail.type === "diff" ? (
              <MnListDiffDetail diff={selectedDetail.data} />
            ) : (
              <QuorumEntryDetail entry={selectedDetail.data} />
            )}
          </div>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Diffs Tab
// ---------------------------------------------------------------------------

type DiffSubView = "newQuorums" | "masternodeChanges" | "chainLocks";

function DiffsTab({
  fetchedDiffs,
}: {
  fetchedDiffs: Array<{ baseHeight: number; height: number; diff: MnListDiffDto }>;
}) {
  const [selectedDiffIndex, setSelectedDiffIndex] = useState<number | null>(null);
  const [subView, setSubView] = useState<DiffSubView>("newQuorums");
  const [selectedItemIndex, setSelectedItemIndex] = useState<number | null>(null);
  const [mnSearch, setMnSearch] = useState("");

  // Reset selection when diffs array shrinks (clamp out-of-bounds index)
  useEffect(() => {
    if (selectedDiffIndex !== null && selectedDiffIndex >= fetchedDiffs.length) {
      // eslint-disable-next-line react-hooks/set-state-in-effect -- resetting stale index when external data shrinks
      setSelectedDiffIndex(null);
      setSelectedItemIndex(null);
    }
  }, [fetchedDiffs.length, selectedDiffIndex]);

  const selectedDiff = selectedDiffIndex !== null ? fetchedDiffs[selectedDiffIndex] : null;

  // Filtered masternodes based on search
  const filteredNewMasternodes = useMemo(() => {
    if (!selectedDiff) return [];
    const mns = selectedDiff.diff.newMasternodes;
    if (mnSearch.length < 3) return mns;
    const lower = mnSearch.toLowerCase();
    return mns.filter(
      (mn) =>
        mn.proRegTxHash.toLowerCase().includes(lower) ||
        mn.address.toLowerCase().includes(lower),
    );
  }, [selectedDiff, mnSearch]);

  const filteredDeletedMasternodes = useMemo(() => {
    if (!selectedDiff) return [];
    const dels = selectedDiff.diff.deletedMasternodes;
    if (mnSearch.length < 3) return dels;
    const lower = mnSearch.toLowerCase();
    return dels.filter((h) => h.toLowerCase().includes(lower));
  }, [selectedDiff, mnSearch]);

  if (fetchedDiffs.length === 0) {
    return (
      <div
        className="flex-1 flex items-center justify-center text-sm text-muted-foreground"
        data-testid="diffs-tab"
      >
        <div className="text-center">
          <GitCompare className="size-8 mx-auto mb-3 opacity-30" />
          <p>No diff data available.</p>
          <p className="text-xs mt-1">
            Fetch MnList diffs using the controls above to browse them here.
          </p>
        </div>
      </div>
    );
  }

  const handleSelectDiff = (index: number) => {
    setSelectedDiffIndex(index);
    setSelectedItemIndex(null);
    setMnSearch("");
  };

  const handleSelectSubView = (view: DiffSubView) => {
    setSubView(view);
    setSelectedItemIndex(null);
    setMnSearch("");
  };

  // Build middle column items based on sub-view
  const renderMiddleContent = () => {
    if (!selectedDiff) {
      return (
        <div className="p-4 text-xs text-muted-foreground text-center">
          Select a diff from the left panel.
        </div>
      );
    }

    const diff = selectedDiff.diff;

    switch (subView) {
      case "newQuorums":
        return diff.newQuorums.length === 0 ? (
          <div className="p-4 text-xs text-muted-foreground text-center">
            No new quorums in this diff.
          </div>
        ) : (
          diff.newQuorums.map((q, i) => {
            const isSelected = selectedItemIndex === i;
            return (
              <button
                key={i}
                onClick={() => setSelectedItemIndex(i)}
                className={cn(
                  "w-full text-left px-3 py-2 text-xs border-b last:border-b-0 transition-colors",
                  "hover:bg-accent/50 focus:bg-accent/50 focus:outline-none",
                  isSelected && "bg-accent text-accent-foreground font-medium",
                )}
                aria-selected={isSelected}
                role="option"
              >
                <div className="font-mono truncate">{truncateHex(q.quorumHash)}</div>
                <div className="text-muted-foreground">{llmqTypeLabel(q.llmqType)}</div>
              </button>
            );
          })
        );

      case "masternodeChanges":
        return (
          <>
            {/* Search input */}
            <div className="px-2 py-1.5 border-b">
              <div className="relative">
                <Search className="absolute left-2 top-1/2 -translate-y-1/2 size-3 text-muted-foreground" />
                <Input
                  type="text"
                  value={mnSearch}
                  onChange={(e) => { setMnSearch(e.target.value); setSelectedItemIndex(null); }}
                  placeholder="Search masternodes (3+ chars)"
                  className="h-7 text-xs pl-7"
                  data-testid="mn-search-input"
                />
              </div>
            </div>
            {/* New masternodes */}
            {filteredNewMasternodes.length > 0 && (
              <div className="px-2 py-1 text-[10px] font-semibold text-muted-foreground bg-muted/30 border-b">
                New Masternodes ({filteredNewMasternodes.length})
              </div>
            )}
            {filteredNewMasternodes.map((mn, i) => {
              const isSelected = selectedItemIndex === i;
              return (
                <button
                  key={`new-${i}`}
                  onClick={() => setSelectedItemIndex(i)}
                  className={cn(
                    "w-full text-left px-3 py-1.5 text-xs border-b last:border-b-0 transition-colors",
                    "hover:bg-accent/50 focus:bg-accent/50 focus:outline-none",
                    isSelected && "bg-accent text-accent-foreground font-medium",
                  )}
                  aria-selected={isSelected}
                  role="option"
                >
                  <div className="font-mono truncate">{truncateHex(mn.proRegTxHash)}</div>
                  <div className="text-muted-foreground">{mn.address}</div>
                </button>
              );
            })}
            {/* Deleted masternodes */}
            {filteredDeletedMasternodes.length > 0 && (
              <div className="px-2 py-1 text-[10px] font-semibold text-muted-foreground bg-muted/30 border-b">
                Deleted Masternodes ({filteredDeletedMasternodes.length})
              </div>
            )}
            {filteredDeletedMasternodes.map((h, i) => {
              const idx = filteredNewMasternodes.length + i;
              const isSelected = selectedItemIndex === idx;
              return (
                <button
                  key={`del-${i}`}
                  onClick={() => setSelectedItemIndex(idx)}
                  className={cn(
                    "w-full text-left px-3 py-1.5 text-xs border-b last:border-b-0 transition-colors text-red-600 dark:text-red-400",
                    "hover:bg-accent/50 focus:bg-accent/50 focus:outline-none",
                    isSelected && "bg-accent text-accent-foreground font-medium",
                  )}
                  aria-selected={isSelected}
                  role="option"
                >
                  <div className="font-mono truncate">{truncateHex(h)}</div>
                  <div className="text-muted-foreground">Deleted</div>
                </button>
              );
            })}
            {filteredNewMasternodes.length === 0 && filteredDeletedMasternodes.length === 0 && (
              <div className="p-4 text-xs text-muted-foreground text-center">
                {mnSearch.length >= 3
                  ? "No masternodes match your search."
                  : "No masternode changes in this diff."}
              </div>
            )}
          </>
        );

      case "chainLocks":
        return diff.chainlockSignatures.length === 0 ? (
          <div className="p-4 text-xs text-muted-foreground text-center">
            No chain lock signatures in this diff.
          </div>
        ) : (
          diff.chainlockSignatures.map((sig, i) => {
            const isSelected = selectedItemIndex === i;
            return (
              <button
                key={i}
                onClick={() => setSelectedItemIndex(i)}
                className={cn(
                  "w-full text-left px-3 py-2 text-xs border-b last:border-b-0 transition-colors",
                  "hover:bg-accent/50 focus:bg-accent/50 focus:outline-none",
                  isSelected && "bg-accent text-accent-foreground font-medium",
                )}
                aria-selected={isSelected}
                role="option"
              >
                <div className="font-mono truncate">{truncateHex(sig.signature)}</div>
                <div className="text-muted-foreground">
                  Index set: [{sig.indexSet.join(", ")}]
                </div>
              </button>
            );
          })
        );
    }
  };

  // Build right column detail based on selection
  const renderDetailContent = () => {
    if (!selectedDiff || selectedItemIndex === null) {
      return (
        <div className="h-full flex items-center justify-center text-sm text-muted-foreground">
          Select an item to view details.
        </div>
      );
    }

    const diff = selectedDiff.diff;

    switch (subView) {
      case "newQuorums": {
        const entry = diff.newQuorums[selectedItemIndex];
        if (!entry) return null;
        return <QuorumEntryDetail entry={entry} />;
      }
      case "masternodeChanges": {
        if (selectedItemIndex < filteredNewMasternodes.length) {
          const mn = filteredNewMasternodes[selectedItemIndex]!;
          return (
            <div className="flex flex-col gap-3" data-testid="masternode-detail">
              <h3 className="text-lg font-semibold">New Masternode</h3>
              <div className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
                <span className="text-muted-foreground">ProRegTx Hash</span>
                <span className="font-mono break-all">{mn.proRegTxHash}</span>
                <span className="text-muted-foreground">Address</span>
                <span className="font-mono">{mn.address}</span>
              </div>
            </div>
          );
        }
        const delIdx = selectedItemIndex - filteredNewMasternodes.length;
        const hash = filteredDeletedMasternodes[delIdx];
        if (!hash) return null;
        return (
          <div className="flex flex-col gap-3" data-testid="masternode-detail">
            <h3 className="text-lg font-semibold">Deleted Masternode</h3>
            <div className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
              <span className="text-muted-foreground">ProRegTx Hash</span>
              <span className="font-mono break-all">{hash}</span>
            </div>
          </div>
        );
      }
      case "chainLocks": {
        const sig = diff.chainlockSignatures[selectedItemIndex];
        if (!sig) return null;
        return (
          <div className="flex flex-col gap-3" data-testid="chainlock-sig-detail">
            <h3 className="text-lg font-semibold">Chain Lock Signature</h3>
            <div className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
              <span className="text-muted-foreground">Signature</span>
              <div className="flex items-start gap-2">
                <span className="font-mono break-all">{sig.signature}</span>
                <CopyButton value={sig.signature} size="sm" />
              </div>
              <span className="text-muted-foreground">Index Set</span>
              <span className="font-mono">[{sig.indexSet.join(", ")}]</span>
            </div>
          </div>
        );
      }
    }
  };

  return (
    <div className="flex gap-3 min-h-0 flex-1" data-testid="diffs-tab">
      {/* Left column: Diff list */}
      <div className="w-[150px] shrink-0 flex flex-col min-h-0">
        <h4 className="text-xs font-semibold text-muted-foreground mb-1.5">
          Fetched Diffs ({fetchedDiffs.length})
        </h4>
        <div
          className="flex-1 overflow-y-auto rounded border bg-card"
          role="listbox"
          aria-label="Fetched Diffs"
        >
          {fetchedDiffs.map((item, i) => {
            const isSelected = selectedDiffIndex === i;
            return (
              <button
                key={`${item.baseHeight}-${item.height}-${i}`}
                onClick={() => handleSelectDiff(i)}
                className={cn(
                  "w-full text-left px-3 py-2 text-xs border-b last:border-b-0 transition-colors",
                  "hover:bg-accent/50 focus:bg-accent/50 focus:outline-none",
                  isSelected && "bg-accent text-accent-foreground font-medium",
                )}
                aria-selected={isSelected}
                role="option"
              >
                {item.baseHeight} → {item.height}
              </button>
            );
          })}
        </div>
      </div>

      {/* Middle column: Sub-view */}
      <div className="w-[280px] shrink-0 flex flex-col min-h-0">
        {/* Sub-view toggle buttons */}
        <div className="flex items-center gap-1 mb-1.5">
          {(
            [
              ["newQuorums", "New Quorums"],
              ["masternodeChanges", "MN Changes"],
              ["chainLocks", "Chain Locks"],
            ] as [DiffSubView, string][]
          ).map(([view, label]) => (
            <Button
              key={view}
              variant={subView === view ? "default" : "outline"}
              size="sm"
              className="h-6 text-[10px] px-2"
              onClick={() => handleSelectSubView(view)}
            >
              {label}
            </Button>
          ))}
        </div>
        <div
          className="flex-1 overflow-y-auto rounded border bg-card"
          role="listbox"
          aria-label="Diff Items"
        >
          {renderMiddleContent()}
        </div>
      </div>

      {/* Right column: Detail view */}
      <div className="flex-1 min-h-0 overflow-y-auto rounded border bg-card p-4">
        {renderDetailContent()}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Chain Lock Sigs Tab
// ---------------------------------------------------------------------------

function ChainLockSigsTab({ entries }: { entries: ChainLockSigEntry[] }) {
  const sorted = useMemo(
    () => [...entries].sort((a, b) => a.height - b.height),
    [entries],
  );

  if (sorted.length === 0) {
    return (
      <div
        className="flex-1 flex items-center justify-center text-sm text-muted-foreground"
        data-testid="chain-lock-sigs-tab"
      >
        <div className="text-center">
          <Blocks className="size-8 mx-auto mb-3 opacity-30" />
          <p>No chain lock signature data available.</p>
          <p className="text-xs mt-1">
            Fetch chain locks using the controls above to populate data.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-3 min-h-0 flex-1" data-testid="chain-lock-sigs-tab">
      <div className="flex-1 overflow-y-auto rounded border bg-card">
        {/* Header */}
        <div className="flex items-center gap-2 px-3 py-1.5 text-[10px] text-muted-foreground font-semibold border-b bg-muted/30 sticky top-0">
          <span className="w-20 shrink-0">Height</span>
          <span className="flex-1">Block Hash</span>
          <span className="flex-1">Signature</span>
        </div>
        {sorted.map((entry, i) => (
          <div
            key={`${entry.height}-${i}`}
            className="flex items-center gap-2 px-3 py-1.5 text-xs border-b last:border-b-0"
          >
            <span className="w-20 shrink-0 font-mono font-medium">
              {entry.height}
            </span>
            <span className="flex-1 font-mono text-muted-foreground truncate">
              {truncateHex(entry.blockHash)}
            </span>
            <span className="flex-1 font-mono text-muted-foreground truncate">
              {entry.signature ?? "None"}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// LLMQ type labels
// ---------------------------------------------------------------------------

const LLMQ_TYPE_LABELS: Record<number, string> = {
  1: "LLMQ_50_60",
  2: "LLMQ_400_60",
  3: "LLMQ_400_85",
  4: "LLMQ_100_67",
  5: "LLMQ_60_75",
  6: "LLMQ_25_67",
  100: "LLMQ_TEST",
  101: "LLMQ_DEVNET",
  102: "LLMQ_TEST_V17",
  103: "LLMQ_TEST_DIP0024",
  104: "LLMQ_TEST_INSTANTSEND",
  105: "LLMQ_DEVNET_DIP0024",
  106: "LLMQ_TEST_PLATFORM",
  107: "LLMQ_DEVNET_PLATFORM",
};

function llmqTypeLabel(llmqType: number): string {
  return LLMQ_TYPE_LABELS[llmqType] ?? `Type ${llmqType}`;
}

// ---------------------------------------------------------------------------
// Quorum Viewer Tab
// ---------------------------------------------------------------------------

function QuorumViewerTab({
  quorumEntries,
}: {
  quorumEntries: QuorumViewerEntry[];
}) {
  const [selectedType, setSelectedType] = useState<number | null>(null);
  const [selectedHash, setSelectedHash] = useState<string | null>(null);

  // Group entries by LLMQ type
  const typeMap = new Map<number, QuorumViewerEntry[]>();
  for (const entry of quorumEntries) {
    const existing = typeMap.get(entry.llmqType) ?? [];
    existing.push(entry);
    typeMap.set(entry.llmqType, existing);
  }
  const sortedTypes = Array.from(typeMap.keys()).sort((a, b) => a - b);

  // Auto-select first type if none selected
  const activeType = selectedType ?? sortedTypes[0] ?? null;
  const activeEntries = activeType !== null ? typeMap.get(activeType) ?? [] : [];

  // Deduplicate quorum hashes (same quorum may appear in multiple diffs)
  const seenHashes = new Map<string, QuorumViewerEntry>();
  for (const e of activeEntries) {
    if (!seenHashes.has(e.quorumHash)) {
      seenHashes.set(e.quorumHash, e);
    }
  }
  const uniqueEntries = Array.from(seenHashes.values());

  const selectedEntry = selectedHash ? seenHashes.get(selectedHash) : null;

  // Find all heights where the selected quorum appears
  const selectedHeights = selectedHash
    ? activeEntries
        .filter((e) => e.quorumHash === selectedHash)
        .map((e) => e.sourceHeight)
        .sort((a, b) => a - b)
    : [];

  if (quorumEntries.length === 0) {
    return (
      <div
        className="flex-1 flex items-center justify-center text-sm text-muted-foreground"
        data-testid="quorum-viewer-tab"
      >
        <div className="text-center">
          <Shield className="size-8 mx-auto mb-3 opacity-30" />
          <p>No quorum data available.</p>
          <p className="text-xs mt-1">
            Fetch MnList diffs using the controls above to populate quorum data.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-3 min-h-0 flex-1" data-testid="quorum-viewer-tab">
      {/* Quorum Type selector bar */}
      <div className="flex items-center gap-1 flex-wrap">
        {sortedTypes.map((llmqType) => (
          <Button
            key={llmqType}
            variant={activeType === llmqType ? "default" : "outline"}
            size="sm"
            onClick={() => {
              setSelectedType(llmqType);
              setSelectedHash(null);
            }}
            className="text-xs h-7"
            data-testid={`quorum-type-${llmqType}`}
          >
            {llmqTypeLabel(llmqType)}
            <Badge variant="secondary" className="ml-1 text-[10px] h-4 px-1">
              {(typeMap.get(llmqType) ?? []).length}
            </Badge>
          </Button>
        ))}
      </div>

      {activeType === null ? (
        <div className="text-sm text-muted-foreground">No quorum types available.</div>
      ) : (
        <div className="flex gap-3 min-h-0 flex-1">
          {/* Left column: Quorum Hashes */}
          <div className="w-[400px] shrink-0 flex flex-col min-h-0">
            <h4 className="text-xs font-semibold text-muted-foreground mb-1.5">
              Quorums of Type: {llmqTypeLabel(activeType)} ({uniqueEntries.length})
            </h4>
            <div
              className="flex-1 overflow-y-auto rounded border bg-card"
              role="listbox"
              aria-label="Quorum Hashes"
            >
              {/* Header */}
              <div className="flex items-center gap-2 px-3 py-1.5 text-[10px] text-muted-foreground font-semibold border-b bg-muted/30">
                <span className="flex-1">Quorum Hash</span>
                <span className="w-16 text-right">Signers</span>
                <span className="w-16 text-right">Valid</span>
              </div>
              {uniqueEntries.map((entry) => {
                const isSelected = selectedHash === entry.quorumHash;
                return (
                  <button
                    key={entry.quorumHash}
                    onClick={() => setSelectedHash(entry.quorumHash)}
                    className={cn(
                      "w-full text-left px-3 py-1.5 text-xs border-b last:border-b-0 transition-colors flex items-center gap-2",
                      "hover:bg-accent/50 focus:bg-accent/50 focus:outline-none",
                      isSelected && "bg-accent text-accent-foreground font-medium",
                    )}
                    aria-selected={isSelected}
                    role="option"
                  >
                    <span className="font-mono truncate flex-1">
                      {truncateHex(entry.quorumHash, 10)}
                    </span>
                    <span className="w-16 text-right font-mono text-muted-foreground">
                      {entry.signerCount}/{entry.totalMembers}
                    </span>
                    <span className="w-16 text-right font-mono text-muted-foreground">
                      {entry.validMemberCount}/{entry.totalMembers}
                    </span>
                  </button>
                );
              })}
            </div>
          </div>

          {/* Right column: Quorum Details */}
          <div className="flex-1 min-h-0 overflow-y-auto rounded border bg-card p-4">
            {!selectedEntry ? (
              <div className="h-full flex items-center justify-center text-sm text-muted-foreground">
                Select a quorum to see its details.
              </div>
            ) : (
              <div className="flex flex-col gap-3">
                <h3 className="text-lg font-semibold">Quorum Details</h3>
                <div className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
                  <span className="text-muted-foreground">Quorum Type</span>
                  <span className="font-mono">
                    {llmqTypeLabel(selectedEntry.llmqType)}
                  </span>

                  <span className="text-muted-foreground">Quorum Hash</span>
                  <span className="font-mono break-all">
                    {selectedEntry.quorumHash}
                    <CopyButton value={selectedEntry.quorumHash} size="sm" className="ml-1 inline-flex" />
                  </span>

                  {selectedEntry.quorumIndex !== null && (
                    <>
                      <span className="text-muted-foreground">Quorum Index</span>
                      <span className="font-mono">{selectedEntry.quorumIndex}</span>
                    </>
                  )}

                  <span className="text-muted-foreground">Signers</span>
                  <span className="font-mono">
                    {selectedEntry.signerCount} / {selectedEntry.totalMembers}
                  </span>

                  <span className="text-muted-foreground">Valid Members</span>
                  <span className="font-mono">
                    {selectedEntry.validMemberCount} / {selectedEntry.totalMembers}
                  </span>
                </div>

                <div>
                  <h4 className="text-sm font-semibold mb-1">Public Key</h4>
                  <div className="flex items-start gap-2">
                    <p className="text-xs font-mono break-all bg-muted/30 p-2 rounded border flex-1">
                      {selectedEntry.quorumPublicKey}
                    </p>
                    <CopyButton value={selectedEntry.quorumPublicKey} size="sm" />
                  </div>
                </div>

                {selectedHeights.length > 0 && (
                  <div>
                    <h4 className="text-sm font-semibold mb-1">
                      Found in Diff Heights ({selectedHeights.length})
                    </h4>
                    <div className="max-h-48 overflow-y-auto rounded border bg-muted/30 p-2 text-xs font-mono">
                      {selectedHeights.map((h) => (
                        <div key={h} className="py-0.5">
                          Height: {h}
                        </div>
                      ))}
                    </div>
                  </div>
                )}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Input Area (base/end block heights + action buttons)
// ---------------------------------------------------------------------------

function InputArea({
  pendingOp,
  message,
  error,
  onDismissMessage,
  onDismissError,
  onAction,
}: {
  pendingOp: PendingOp;
  message: { text: string; type: "success" | "info" } | null;
  error: string | null;
  onDismissMessage: () => void;
  onDismissError: () => void;
  onAction: (
    action: string,
    baseHeight: string,
    endHeight: string,
  ) => void;
}) {
  const [baseHeight, setBaseHeight] = useState("");
  const [endHeight, setEndHeight] = useState("");

  const handleAction = useCallback(
    (action: string) => {
      onAction(action, baseHeight, endHeight);
    },
    [baseHeight, endHeight, onAction],
  );

  const pendingLabel: Record<string, string> = {
    fetchDiff: "Fetching DML diff…",
    fetchQrInfo: "Fetching QR info…",
    fetchDmlsNoRotation: "Fetching DMLs (no rotation)…",
    fetchDmlsWithRotation: "Fetching QR info + DMLs…",
    fetchChainLocks: "Fetching chain locks…",
  };

  return (
    <div className="flex flex-col gap-2 mb-3" data-testid="input-area">
      {/* Input row */}
      <div className="flex items-center gap-2 flex-wrap">
        <div className="flex items-center gap-1.5">
          <label className="text-xs text-muted-foreground whitespace-nowrap">
            Base Height:
          </label>
          <Input
            type="text"
            value={baseHeight}
            onChange={(e) => setBaseHeight(e.target.value)}
            placeholder="0"
            className="w-24 h-7 text-xs font-mono"
            data-testid="base-height-input"
          />
        </div>
        <div className="flex items-center gap-1.5">
          <label className="text-xs text-muted-foreground whitespace-nowrap">
            End Height:
          </label>
          <Input
            type="text"
            value={endHeight}
            onChange={(e) => setEndHeight(e.target.value)}
            placeholder="tip"
            className="w-24 h-7 text-xs font-mono"
            data-testid="end-height-input"
          />
        </div>
      </div>

      {/* Action buttons row */}
      <div className="flex items-center gap-1.5 flex-wrap">
        <Button
          variant="outline"
          size="sm"
          className="h-7 text-xs"
          onClick={() => handleAction("fetchDiff")}
          disabled={pendingOp !== null}
          data-testid="fetch-diff-button"
        >
          Get single end DML diff
        </Button>
        <Button
          variant="outline"
          size="sm"
          className="h-7 text-xs"
          onClick={() => handleAction("fetchQrInfo")}
          disabled={pendingOp !== null}
          data-testid="fetch-qrinfo-button"
        >
          Get single end QR info
        </Button>
        <Button
          variant="outline"
          size="sm"
          className="h-7 text-xs"
          onClick={() => handleAction("fetchDmlsNoRotation")}
          disabled={pendingOp !== null}
          data-testid="fetch-dmls-no-rotation-button"
        >
          Get DMLs w/o rotation
        </Button>
        <Button
          variant="outline"
          size="sm"
          className="h-7 text-xs"
          onClick={() => handleAction("fetchDmlsWithRotation")}
          disabled={pendingOp !== null}
          data-testid="fetch-dmls-with-rotation-button"
        >
          Get DMLs w/ rotation
        </Button>
        <Button
          variant="outline"
          size="sm"
          className="h-7 text-xs"
          onClick={() => handleAction("fetchChainLocks")}
          disabled={pendingOp !== null}
          data-testid="fetch-chain-locks-button"
        >
          Get chain locks
        </Button>
        <Button
          variant="outline"
          size="sm"
          className="h-7 text-xs"
          onClick={() => handleAction("clear")}
          disabled={pendingOp !== null}
          data-testid="clear-button"
        >
          Clear
        </Button>
        <Button
          variant="outline"
          size="sm"
          className="h-7 text-xs"
          onClick={() => handleAction("clearKeepBase")}
          disabled={pendingOp !== null}
          data-testid="clear-keep-base-button"
        >
          Clear Keep Base
        </Button>
      </div>

      {/* Pending spinner */}
      {pendingOp && (
        <div className="flex items-center gap-2 text-sm" data-testid="pending-indicator">
          <Loader2 className="size-4 animate-spin text-dash-blue" />
          <span>{pendingLabel[pendingOp] ?? "Working…"}</span>
        </div>
      )}

      {/* Message banner */}
      {message && (
        <div
          className={cn(
            "flex items-center gap-2 text-sm px-3 py-2 rounded border",
            message.type === "success" && "bg-green-500/10 border-green-500/20 text-green-700 dark:text-green-400",
            message.type === "info" && "bg-blue-500/10 border-blue-500/20 text-blue-700 dark:text-blue-400",
          )}
          data-testid="message-banner"
        >
          <span className="flex-1">{message.text}</span>
          <button onClick={onDismissMessage} className="shrink-0" aria-label="Dismiss message">
            <X className="size-3.5" />
          </button>
        </div>
      )}

      {/* Error banner */}
      {error && (
        <div
          className="flex items-center gap-2 text-sm px-3 py-2 rounded border bg-destructive/10 border-destructive/20 text-destructive"
          data-testid="error-banner"
        >
          <AlertCircle className="size-4 shrink-0" />
          <span className="flex-1 break-all">{error}</span>
          <button onClick={onDismissError} className="shrink-0" aria-label="Dismiss error">
            <X className="size-3.5" />
          </button>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main Screen
// ---------------------------------------------------------------------------

/**
 * Masternode List Diff screen — Core Items, QR Info, and Quorum Viewer tabs.
 *
 * Core Items: Real-time chain-locked blocks and instant-send-locked transactions
 * received via ZMQ from Dash Core.
 *
 * QR Info: Load and inspect QRInfo .dat files containing quorum rotation data.
 *
 * Quorum Viewer: View quorum data extracted from fetched MnList diffs.
 */
export function MasternodeListDiffScreen() {
  const [pendingOp, setPendingOp] = useState<PendingOp>(null);
  const [message, setMessage] = useState<{ text: string; type: "success" | "info" } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [fetchedDiffs, setFetchedDiffs] = useState<
    Array<{ baseHeight: number; height: number; diff: MnListDiffDto }>
  >([]);
  const [chainLockSigs, setChainLockSigs] = useState<ChainLockSigEntry[]>([]);
  const [fetchedQrInfo, setFetchedQrInfo] = useState<QrInfoDto | null>(null);

  // Track pending task IDs to match results
  const pendingTaskIds = useRef<Set<string>>(new Set());

  // Build quorum entries from fetched diffs
  const quorumEntries: QuorumViewerEntry[] = [];
  for (const item of fetchedDiffs) {
    for (const q of item.diff.newQuorums) {
      quorumEntries.push({
        quorumHash: q.quorumHash,
        llmqType: q.llmqType,
        signerCount: q.signers.filter(Boolean).length,
        validMemberCount: q.validMembers.filter(Boolean).length,
        totalMembers: Math.max(q.signers.length, q.validMembers.length),
        quorumPublicKey: q.quorumPublicKey,
        quorumIndex: q.quorumIndex,
        sourceHeight: item.height,
      });
    }
  }

  // Listen for MnList task results
  useEffect(() => {
    let cleanup: (() => void) | undefined;
    const subscribe = async () => {
      cleanup = await events.taskResultEvent.listen(
        (event: { payload: TaskResultEvent }) => {
          const { taskId, result } = event.payload;
          if (!pendingTaskIds.current.has(taskId)) return;
          pendingTaskIds.current.delete(taskId);

          switch (result.type) {
            case "mnListFetchedDiff":
              setFetchedDiffs((prev) => [
                ...prev,
                { baseHeight: result.baseHeight, height: result.height, diff: result.diff as unknown as MnListDiffDto },
              ]);
              setMessage({ text: `Fetched diff: ${result.baseHeight} → ${result.height}`, type: "success" });
              break;
            case "mnListFetchedQrInfo":
              // Cast needed: backend serializes QrInfoDto via serde_json::Value (typed as `any` in bindings)
              setFetchedQrInfo(result.qrInfo as unknown as QrInfoDto);
              setMessage({ text: "Fetched QR info successfully", type: "success" });
              break;
            case "mnListChainLockSigs":
              setChainLockSigs((prev) => [...prev, ...result.entries]);
              setMessage({ text: `Fetched ${result.entries.length} chain lock signatures`, type: "success" });
              break;
            case "mnListFetchedDiffs": {
              const diffs = result.diffs as unknown as Array<{ baseHeight: number; height: number; diff: MnListDiffDto }>;
              setFetchedDiffs((prev) => [
                ...prev,
                ...diffs.map((d) => ({
                  baseHeight: d.baseHeight,
                  height: d.height,
                  diff: d.diff,
                })),
              ]);
              setMessage({ text: `Fetched ${diffs.length} diffs`, type: "success" });
              break;
            }
            default:
              return; // Not an MnList result, ignore
          }
          setPendingOp(null);
        },
      );
    };
    subscribe().catch(() => {});
    return () => cleanup?.();
  }, []);

  // Listen for task errors
  useEffect(() => {
    let cleanup: (() => void) | undefined;
    const subscribe = async () => {
      cleanup = await events.taskErrorEvent.listen(
        (event: { payload: { taskId: string; message: string } }) => {
          const { taskId, message: errMsg } = event.payload;
          if (pendingTaskIds.current.has(taskId)) {
            pendingTaskIds.current.delete(taskId);
            setError(errMsg);
            toastError(errMsg);
            setPendingOp(null);
          }
        },
      );
    };
    subscribe().catch(() => {});
    return () => cleanup?.();
  }, []);

  const handleAction = useCallback(
    async (action: string, baseHeight: string, endHeight: string) => {
      setError(null);
      setMessage(null);

      if (action === "clear") {
        setFetchedDiffs([]);
        setChainLockSigs([]);
        setFetchedQrInfo(null);
        setMessage({ text: "Cleared all data", type: "success" });
        return;
      }

      if (action === "clearKeepBase") {
        setFetchedDiffs((prev) => {
          const baseDiff = prev.find((d) => d.baseHeight === 0);
          return baseDiff ? [baseDiff] : [];
        });
        setChainLockSigs([]);
        setFetchedQrInfo(null);
        setMessage({ text: "Cleared data, kept base diff", type: "success" });
        return;
      }

      // Parse heights — use "0" as default base, empty end means "tip"
      const baseH = baseHeight.trim() === "" ? 0 : parseInt(baseHeight.trim(), 10);
      const endH = endHeight.trim() === "" ? 0 : parseInt(endHeight.trim(), 10);

      if (isNaN(baseH)) {
        setError("Invalid base block height");
        return;
      }
      if (endHeight.trim() !== "" && isNaN(endH)) {
        setError("Invalid end block height");
        return;
      }

      // For hash-based commands, we use zero-hashes as placeholders.
      // The backend will resolve them via Core RPC.
      const zeroHash = "0".repeat(64);
      const baseHash = zeroHash;
      const endHash = zeroHash;

      try {
        let result;
        switch (action) {
          case "fetchDiff":
            setPendingOp("fetchDiff");
            result = await commands.mnlistFetchDiff({
              baseBlockHeight: baseH,
              baseBlockHash: baseHash,
              blockHeight: endH,
              blockHash: endHash,
              validateQuorums: false,
            });
            break;
          case "fetchQrInfo":
            setPendingOp("fetchQrInfo");
            result = await commands.mnlistFetchQrInfo({
              knownBlockHashes: [],
              blockHash: endHash,
            });
            break;
          case "fetchDmlsNoRotation":
            setPendingOp("fetchDmlsNoRotation");
            result = await commands.mnlistFetchDiff({
              baseBlockHeight: baseH,
              baseBlockHash: baseHash,
              blockHeight: endH,
              blockHash: endHash,
              validateQuorums: true,
            });
            break;
          case "fetchDmlsWithRotation":
            setPendingOp("fetchDmlsWithRotation");
            result = await commands.mnlistFetchQrInfoWithDmls({
              knownBlockHashes: [],
              blockHash: endHash,
            });
            break;
          case "fetchChainLocks":
            setPendingOp("fetchChainLocks");
            result = await commands.mnlistFetchChainLocks({
              baseBlockHeight: baseH,
              blockHeight: endH,
            });
            break;
          default:
            return;
        }

        // Track the task ID for result matching
        if (result) {
          const taskId =
            "status" in result
              ? result.status === "ok"
                ? result.data.taskId
                : null
              : result.taskId;
          if (taskId) {
            pendingTaskIds.current.add(taskId);
          } else if ("status" in result && result.status === "error") {
            setError(result.error);
            toastError(result.error);
            setPendingOp(null);
          }
        }
      } catch (e) {
        const msg = String(e);
        setError(msg);
        toastError(msg);
        setPendingOp(null);
      }
    },
    [],
  );

  return (
    <ToolPageLayout
      title="Masternode List Diff"
      subtitle="Real-time chain locks, instant send transactions, and masternode list data"
    >
      {/* Input area with action buttons */}
      <InputArea
        pendingOp={pendingOp}
        message={message}
        error={error}
        onDismissMessage={() => setMessage(null)}
        onDismissError={() => setError(null)}
        onAction={handleAction}
      />

      <Tabs defaultValue="core-items" className="flex flex-col flex-1 min-h-0">
        <TabsList className="w-fit">
          <TabsTrigger value="core-items">Core Items</TabsTrigger>
          <TabsTrigger value="diffs">
            Diffs
            {fetchedDiffs.length > 0 && (
              <Badge variant="secondary" className="ml-1 text-[10px] h-4 px-1">
                {fetchedDiffs.length}
              </Badge>
            )}
          </TabsTrigger>
          <TabsTrigger value="chain-lock-sigs">
            Chain Lock Sigs
            {chainLockSigs.length > 0 && (
              <Badge variant="secondary" className="ml-1 text-[10px] h-4 px-1">
                {chainLockSigs.length}
              </Badge>
            )}
          </TabsTrigger>
          <TabsTrigger value="qr-info">QR Info</TabsTrigger>
          <TabsTrigger value="quorum-viewer">
            Quorum Viewer
            {quorumEntries.length > 0 && (
              <Badge variant="secondary" className="ml-1 text-[10px] h-4 px-1">
                {quorumEntries.length}
              </Badge>
            )}
          </TabsTrigger>
        </TabsList>
        <TabsContent
          value="core-items"
          className="flex-1 min-h-0 flex flex-col mt-4"
        >
          <CoreItemsTab />
        </TabsContent>
        <TabsContent
          value="diffs"
          className="flex-1 min-h-0 flex flex-col mt-4"
        >
          <DiffsTab fetchedDiffs={fetchedDiffs} />
        </TabsContent>
        <TabsContent
          value="chain-lock-sigs"
          className="flex-1 min-h-0 flex flex-col mt-4"
        >
          <ChainLockSigsTab entries={chainLockSigs} />
        </TabsContent>
        <TabsContent
          value="qr-info"
          className="flex-1 min-h-0 flex flex-col mt-4"
        >
          <QrInfoTab externalQrInfo={fetchedQrInfo} />
        </TabsContent>
        <TabsContent
          value="quorum-viewer"
          className="flex-1 min-h-0 flex flex-col mt-4"
        >
          <QuorumViewerTab quorumEntries={quorumEntries} />
        </TabsContent>
      </Tabs>
    </ToolPageLayout>
  );
}
