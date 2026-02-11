import { useCallback, useState } from "react";
import { useEffect } from "react";
import { commands, events } from "@/bindings";
import type {
  QrInfoDto,
  QuorumSnapshotDto,
  MnListDiffDto,
  SimpleQuorumEntryDto,
  ZmqChainLockedBlockEvent,
  ZmqIsLockedTransactionEvent,
} from "@/bindings";
import { ToolPageLayout } from "@/components/tools/ToolPageLayout";
import { CopyButton } from "@/components/shared/CopyButton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  CheckCircle2,
  XCircle,
  Blocks,
  ArrowRightLeft,
  Radio,
  Upload,
  FolderOpen,
} from "lucide-react";
import { cn } from "@/lib/utils";

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

function QrInfoTab() {
  const [qrInfo, setQrInfo] = useState<QrInfoDto | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedField, setSelectedField] = useState<QrField | null>(null);
  const [selectedItemIndex, setSelectedItemIndex] = useState<number | null>(null);
  const [selectedDetail, setSelectedDetail] = useState<QrSelectedDetail | null>(null);

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
        }
      }
    } catch (e) {
      setError(String(e));
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
// Main Screen
// ---------------------------------------------------------------------------

/**
 * Masternode List Diff screen — Core Items and QR Info tabs.
 *
 * Core Items: Real-time chain-locked blocks and instant-send-locked transactions
 * received via ZMQ from Dash Core.
 *
 * QR Info: Load and inspect QRInfo .dat files containing quorum rotation data.
 *
 * Quorum Viewer tab will be added in task 9.1n.
 */
export function MasternodeListDiffScreen() {
  return (
    <ToolPageLayout
      title="Masternode List Diff"
      subtitle="Real-time chain locks, instant send transactions, and masternode list data"
    >
      <Tabs defaultValue="core-items" className="flex flex-col flex-1 min-h-0">
        <TabsList className="w-fit">
          <TabsTrigger value="core-items">Core Items</TabsTrigger>
          <TabsTrigger value="qr-info">QR Info</TabsTrigger>
          <TabsTrigger value="quorum-viewer" disabled>
            Quorum Viewer
          </TabsTrigger>
        </TabsList>
        <TabsContent
          value="core-items"
          className="flex-1 min-h-0 flex flex-col mt-4"
        >
          <CoreItemsTab />
        </TabsContent>
        <TabsContent
          value="qr-info"
          className="flex-1 min-h-0 flex flex-col mt-4"
        >
          <QrInfoTab />
        </TabsContent>
      </Tabs>
    </ToolPageLayout>
  );
}
