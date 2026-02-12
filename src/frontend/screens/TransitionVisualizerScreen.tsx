import { useCallback, useEffect, useRef, useState } from "react";
import { commands, events } from "@/bindings";
import type { TaskResultEvent, TaskErrorEvent } from "@/bindings";
import { ToolPageLayout } from "@/components/tools/ToolPageLayout";
import { HexInput, decodeToHex, detectFormat } from "@/components/tools/HexInput";
import { JsonViewer } from "@/components/shared/JsonViewer";
import { CopyButton } from "@/components/shared/CopyButton";
import {
  AlertCircle,
  X,
  Send,
  Link as LinkIcon,
  CheckCircle2,
  Loader2,
  ExternalLink,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import { toastError } from "@/lib/toastError";
import { useNavigate } from "@tanstack/react-router";

type ParseState =
  | { status: "idle" }
  | { status: "error"; message: string }
  | { status: "success"; json: string; contractIds: string[] };

type BroadcastStatus =
  | { type: "idle" }
  | { type: "submitting"; startTime: number; taskId: string }
  | { type: "success"; timestamp: number }
  | { type: "error"; message: string; timestamp: number };

/**
 * Transition Visualizer tool screen.
 *
 * Accepts hex, base64, or comma-separated byte data representing a serialized
 * state transition. Parses on every change and displays the deserialized JSON,
 * detected contract IDs, and a broadcast button.
 */
export function TransitionVisualizerScreen() {
  const [inputValue, setInputValue] = useState("");
  const [parseState, setParseState] = useState<ParseState>({ status: "idle" });
  const [isParsing, setIsParsing] = useState(false);
  const [broadcastStatus, setBroadcastStatus] = useState<BroadcastStatus>({ type: "idle" });
  const [elapsedSeconds, setElapsedSeconds] = useState(0);
  const [contractDialogOpen, setContractDialogOpen] = useState(false);
  const [selectedContractId, setSelectedContractId] = useState<string | null>(null);
  const [contractFetchMessage, setContractFetchMessage] = useState<{
    text: string;
    timestamp: number;
  } | null>(null);

  const navigate = useNavigate();
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const activeTaskIdRef = useRef<string | null>(null);
  const contractFetchTaskIdRef = useRef<string | null>(null);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Elapsed time counter for broadcast
  useEffect(() => {
    if (broadcastStatus.type === "submitting") {
      setElapsedSeconds(0);
      timerRef.current = setInterval(() => {
        setElapsedSeconds((prev) => prev + 1);
      }, 1000);
    } else {
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
    }
    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, [broadcastStatus.type]);

  // Auto-clear success/error messages after 8 seconds
  useEffect(() => {
    if (broadcastStatus.type === "success" || broadcastStatus.type === "error") {
      const timer = setTimeout(() => {
        setBroadcastStatus({ type: "idle" });
      }, 8000);
      return () => clearTimeout(timer);
    }
  }, [broadcastStatus]);

  // Auto-clear contract fetch message after 8 seconds
  useEffect(() => {
    if (contractFetchMessage) {
      const timer = setTimeout(() => {
        setContractFetchMessage(null);
      }, 8000);
      return () => clearTimeout(timer);
    }
  }, [contractFetchMessage]);

  // Subscribe to task result/error events for broadcast
  useEffect(() => {
    let cleanupResult: (() => void) | undefined;
    let cleanupError: (() => void) | undefined;

    const subscribe = async () => {
      cleanupResult = await events.taskResultEvent.listen(
        (event: { payload: TaskResultEvent }) => {
          const { taskId } = event.payload;
          // Handle broadcast task completion
          if (activeTaskIdRef.current && taskId === activeTaskIdRef.current) {
            activeTaskIdRef.current = null;
            setBroadcastStatus({ type: "success", timestamp: Date.now() });
            return;
          }
          // Handle contract fetch task completion
          if (contractFetchTaskIdRef.current && taskId === contractFetchTaskIdRef.current) {
            const contractId = contractFetchTaskIdRef.current;
            contractFetchTaskIdRef.current = null;
            setContractFetchMessage({
              text: `Contract ${selectedContractId?.slice(0, 8) ?? contractId.slice(0, 8)}... fetched successfully`,
              timestamp: Date.now(),
            });
            return;
          }
        },
      );

      cleanupError = await events.taskErrorEvent.listen(
        (event: { payload: TaskErrorEvent }) => {
          const { taskId, message } = event.payload;
          // Handle broadcast task error
          if (activeTaskIdRef.current && taskId === activeTaskIdRef.current) {
            activeTaskIdRef.current = null;
            setBroadcastStatus({
              type: "error",
              message,
              timestamp: Date.now(),
            });
            toastError(message);
            return;
          }
          // Handle contract fetch task error
          if (contractFetchTaskIdRef.current && taskId === contractFetchTaskIdRef.current) {
            contractFetchTaskIdRef.current = null;
            setContractFetchMessage({
              text: `Failed to fetch contract: ${message}`,
              timestamp: Date.now(),
            });
            toastError(message);
            return;
          }
        },
      );
    };

    subscribe();
    return () => {
      cleanupResult?.();
      cleanupError?.();
    };
  }, []);

  const parseInput = useCallback(async (value: string) => {
    const trimmed = value.trim();
    if (!trimmed) {
      setParseState({ status: "idle" });
      setBroadcastStatus({ type: "idle" });
      return;
    }

    const format = detectFormat(trimmed);
    const hexData = decodeToHex(trimmed, format);

    if (!hexData) {
      setParseState({
        status: "error",
        message: "Unable to decode input. Provide valid hex, base64, or comma-separated bytes (0-255).",
      });
      return;
    }

    setIsParsing(true);
    try {
      const result = await commands.parseStateTransition({ hexData });
      if (result.status === "ok") {
        setParseState({
          status: "success",
          json: result.data.json,
          contractIds: result.data.detectedContractIds,
        });
        // Reset broadcast status when new data is parsed
        setBroadcastStatus({ type: "idle" });
      } else {
        setParseState({ status: "error", message: result.error });
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setParseState({ status: "error", message: msg });
      toastError(msg);
    } finally {
      setIsParsing(false);
    }
  }, []);

  const handleInputChange = useCallback(
    (value: string) => {
      setInputValue(value);
      if (debounceRef.current) {
        clearTimeout(debounceRef.current);
      }
      debounceRef.current = setTimeout(() => {
        parseInput(value);
      }, 300);
    },
    [parseInput],
  );

  // Cleanup debounce timer on unmount
  useEffect(() => {
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, []);

  const dismissError = useCallback(() => {
    setParseState({ status: "idle" });
  }, []);

  const handleBroadcast = useCallback(async () => {
    if (parseState.status !== "success") return;

    const trimmed = inputValue.trim();
    const format = detectFormat(trimmed);
    const hexData = decodeToHex(trimmed, format);
    if (!hexData) return;

    try {
      const result = await commands.broadcastStateTransition({
        stateTransitionHex: hexData,
      });
      if (result.status === "ok") {
        activeTaskIdRef.current = result.data.taskId;
        setBroadcastStatus({
          type: "submitting",
          startTime: Date.now(),
          taskId: result.data.taskId,
        });
      } else {
        setBroadcastStatus({
          type: "error",
          message: result.error,
          timestamp: Date.now(),
        });
        toastError(result.error);
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setBroadcastStatus({
        type: "error",
        message: msg,
        timestamp: Date.now(),
      });
      toastError(msg);
    }
  }, [parseState, inputValue]);

  const handleContractClick = useCallback((contractId: string) => {
    setSelectedContractId(contractId);
    setContractDialogOpen(true);
  }, []);

  const handleFetchContract = useCallback(async () => {
    if (!selectedContractId) return;
    setContractDialogOpen(false);

    try {
      const result = await commands.contractFetch({
        contractIds: [selectedContractId],
      });
      if (result.status === "ok") {
        // Store task ID — success message shown when task completes via event
        contractFetchTaskIdRef.current = result.data.taskId;
        setContractFetchMessage({
          text: `Fetching contract ${selectedContractId.slice(0, 8)}...`,
          timestamp: Date.now(),
        });
      } else {
        setContractFetchMessage({
          text: `Failed to fetch contract: ${result.error}`,
          timestamp: Date.now(),
        });
        toastError(result.error);
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setContractFetchMessage({
        text: `Error fetching contract: ${msg}`,
        timestamp: Date.now(),
      });
      toastError(msg);
    }
  }, [selectedContractId]);

  const handleViewInContracts = useCallback(() => {
    navigate({ to: "/contracts" });
  }, [navigate]);

  const canBroadcast =
    parseState.status === "success" &&
    (broadcastStatus.type === "idle" || broadcastStatus.type === "error");

  const formatElapsed = (seconds: number): string => {
    if (seconds < 60) return `${seconds} second${seconds !== 1 ? "s" : ""}`;
    const mins = Math.floor(seconds / 60);
    const secs = seconds % 60;
    return `${mins} minute${mins !== 1 ? "s" : ""} ${secs} second${secs !== 1 ? "s" : ""}`;
  };

  return (
    <ToolPageLayout
      title="Transition Visualizer"
      subtitle="Deserialize, inspect, and broadcast state transitions"
    >
      <div className="flex flex-col gap-6">
        {/* Input section */}
        <HexInput
          value={inputValue}
          onChange={handleInputChange}
          label="Enter hex, base64, or comma-separated integers for State Transition"
          placeholder="Paste serialized state transition bytes here..."
          rows={6}
        />

        {/* Contract IDs section */}
        {parseState.status === "success" && parseState.contractIds.length > 0 && (
          <div className="space-y-2">
            <span className="text-sm font-medium text-foreground">
              Detected Contract IDs
            </span>
            <div className="flex flex-wrap gap-2">
              {parseState.contractIds.map((id) => (
                <button
                  key={id}
                  onClick={() => handleContractClick(id)}
                  className={cn(
                    "inline-flex items-center gap-1.5 rounded-md border px-2.5 py-1",
                    "bg-muted/50 text-sm font-mono text-foreground",
                    "hover:bg-accent hover:text-accent-foreground transition-colors",
                    "cursor-pointer",
                  )}
                >
                  <LinkIcon className="size-3.5" />
                  <span className="truncate max-w-[200px]">{id}</span>
                  <CopyButton value={id} size="icon-xs" className="ml-1" />
                </button>
              ))}
            </div>
          </div>
        )}

        {/* Contract fetch message */}
        {contractFetchMessage && (
          <div className="flex items-center gap-2">
            {contractFetchTaskIdRef.current ? (
              <Loader2 className="size-4 animate-spin text-muted-foreground" />
            ) : contractFetchMessage.text.startsWith("Failed") || contractFetchMessage.text.startsWith("Error") ? (
              <AlertCircle className="size-4 text-destructive" />
            ) : (
              <CheckCircle2 className="size-4 text-green-600 dark:text-green-400" />
            )}
            <span className={cn(
              "text-sm",
              contractFetchMessage.text.startsWith("Failed") || contractFetchMessage.text.startsWith("Error")
                ? "text-destructive"
                : contractFetchTaskIdRef.current
                  ? "text-muted-foreground"
                  : "text-green-700 dark:text-green-300",
            )}>
              {contractFetchMessage.text}
            </span>
            {!contractFetchTaskIdRef.current && !contractFetchMessage.text.startsWith("Failed") && !contractFetchMessage.text.startsWith("Error") && (
              <Button
                variant="outline"
                size="sm"
                onClick={handleViewInContracts}
                className="ml-2"
              >
                <ExternalLink className="mr-1.5 size-3.5" />
                View in Contracts
              </Button>
            )}
          </div>
        )}

        {/* Output section */}
        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <span className="text-sm font-medium text-foreground">Result</span>

            {/* Broadcast button */}
            {canBroadcast && (
              <Button size="sm" onClick={handleBroadcast}>
                <Send className="mr-1.5 size-3.5" />
                Broadcast Transition to Platform
              </Button>
            )}
          </div>

          {/* Broadcast status messages */}
          {broadcastStatus.type === "submitting" && (
            <div className="flex items-center gap-2 rounded-md border border-blue-200 bg-blue-50 p-3 dark:border-blue-800 dark:bg-blue-950">
              <Loader2 className="size-4 animate-spin text-blue-600 dark:text-blue-400" />
              <span className="text-sm text-blue-700 dark:text-blue-300">
                Broadcasting… Time taken so far: {formatElapsed(elapsedSeconds)}
              </span>
            </div>
          )}

          {broadcastStatus.type === "success" && (
            <div className="flex items-center gap-2 rounded-md border border-green-200 bg-green-50 p-3 dark:border-green-800 dark:bg-green-950">
              <CheckCircle2 className="size-4 text-green-600 dark:text-green-400" />
              <span className="text-sm text-green-700 dark:text-green-300">
                Successfully broadcasted state transition.
              </span>
            </div>
          )}

          {broadcastStatus.type === "error" && (
            <div className="flex items-center gap-2 rounded-md border border-destructive/50 bg-destructive/5 p-3" role="alert">
              <AlertCircle className="size-4 shrink-0 text-destructive" />
              <span className="text-sm text-destructive">
                Broadcast error: {broadcastStatus.message}
              </span>
            </div>
          )}

          {/* Parse result */}
          {parseState.status === "idle" && !isParsing && (
            <div className="rounded-md border bg-muted/30 p-4">
              <span className="text-sm italic text-muted-foreground">
                Awaiting input…
              </span>
            </div>
          )}

          {isParsing && (
            <div className="rounded-md border bg-muted/30 p-4">
              <span className="text-sm text-muted-foreground">Parsing…</span>
            </div>
          )}

          {parseState.status === "error" && !isParsing && (
            <div
              className="flex items-start gap-3 rounded-md border border-destructive/50 bg-destructive/5 p-4"
              role="alert"
            >
              <AlertCircle className="mt-0.5 size-4 shrink-0 text-destructive" />
              <div className="min-w-0 flex-1">
                <p className="text-sm text-destructive">{parseState.message}</p>
              </div>
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={dismissError}
                aria-label="Dismiss error"
                className="shrink-0 text-destructive hover:text-destructive"
              >
                <X className="size-3.5" />
              </Button>
            </div>
          )}

          {parseState.status === "success" && !isParsing && (
            <JsonViewer
              data={parseState.json}
              expandDepth={4}
              className="max-h-[600px]"
            />
          )}
        </div>
      </div>

      {/* Contract fetch dialog */}
      <Dialog open={contractDialogOpen} onOpenChange={setContractDialogOpen}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Fetch Contract</DialogTitle>
            <DialogDescription>
              Contract ID:
            </DialogDescription>
          </DialogHeader>
          <div className="flex items-center gap-2">
            <Badge variant="secondary" className="font-mono text-xs break-all">
              {selectedContractId}
            </Badge>
            {selectedContractId && (
              <CopyButton value={selectedContractId} size="icon-xs" />
            )}
          </div>
          <p className="text-sm text-muted-foreground">
            Would you like to fetch this contract from Platform?
          </p>
          <DialogFooter className="gap-2">
            <Button variant="outline" onClick={() => setContractDialogOpen(false)}>
              Cancel
            </Button>
            <Button onClick={handleFetchContract}>
              Yes, Fetch
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </ToolPageLayout>
  );
}
