import { useEffect, useState } from "react";
import { events } from "@/bindings";
import type {
  ZmqChainLockedBlockEvent,
  ZmqIsLockedTransactionEvent,
} from "@/bindings";
import { ToolPageLayout } from "@/components/tools/ToolPageLayout";
import { CopyButton } from "@/components/shared/CopyButton";
import { Badge } from "@/components/ui/badge";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  CheckCircle2,
  XCircle,
  Blocks,
  ArrowRightLeft,
  Radio,
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Truncate a hex string for display: first 8…last 8 chars. */
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
// Main Screen
// ---------------------------------------------------------------------------

/**
 * Masternode List Diff screen — Core Items tab.
 *
 * Displays real-time chain-locked blocks and instant-send-locked transactions
 * received via ZMQ from Dash Core. Three-column layout: blocks list,
 * transactions list, and detail panel.
 *
 * Future tabs (Diffs, QR Info, Quorum Viewer) will be added in tasks 9.1m/9.1n.
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
          <TabsTrigger value="qr-info" disabled>
            QR Info
          </TabsTrigger>
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
      </Tabs>
    </ToolPageLayout>
  );
}
