import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  ArrowLeft,
  Check,
  Loader2,
  Plus,
  Search,
  Trash2,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Island, PageHeader } from "@/components/layout";
import { CopyButton } from "@/components/shared/CopyButton";
import { commands, events } from "@/bindings";
import type { TaskResultEvent, TaskErrorEvent } from "@/bindings";
import { useContractStore } from "@/stores/contractStore";
import { toastError } from "@/lib/toastError";
import { hexToBase58, formatElapsed } from "@/lib/utils";
import { toast } from "sonner";

const MAX_CONTRACTS = 10;

type ScreenStatus =
  | { type: "input" }
  | { type: "fetching"; startTime: number }
  | { type: "complete"; foundIds: string[]; notFoundInputs: string[] }
  | { type: "error"; message: string };

/**
 * Parse a contract ID string (hex or base58) into a normalized hex ID.
 * Returns the hex string or throws an error.
 */
function parseContractId(input: string): string {
  const trimmed = input.trim();
  if (!trimmed) throw new Error("Empty ID");

  // Try hex first: must be 64 hex characters (32 bytes)
  if (/^[0-9a-fA-F]{64}$/.test(trimmed)) {
    return trimmed.toLowerCase();
  }

  // Try base58: Dash identifiers are typically 43-44 chars in base58
  if (/^[123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz]+$/.test(trimmed)) {
    // The backend accepts both hex and base58 — pass through as-is
    // and let the backend handle the conversion
    return trimmed;
  }

  throw new Error(`Invalid format: expected 64-char hex or base58`);
}

/**
 * AddContractsScreen — Multi-field contract ID input with fetch, progress,
 * and alias editing on success.
 *
 * Supports up to 10 contract IDs in hex or base58 format.
 * Fetches contracts from Platform, saves to local DB, and allows alias editing.
 */
export function AddContractsScreen() {
  const navigate = useNavigate();

  // Input fields — start with one empty field
  const [contractInputs, setContractInputs] = useState<string[]>([""]);
  const [status, setStatus] = useState<ScreenStatus>({ type: "input" });
  const [elapsedMs, setElapsedMs] = useState(0);

  // Alias editing
  const [aliasInputs, setAliasInputs] = useState<Record<string, string>>({});
  const [aliasResults, setAliasResults] = useState<
    Record<string, { success: boolean; message: string }>
  >({});

  const activeTaskIdRef = useRef<string | null>(null);
  const elapsedTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const parsedIdsRef = useRef<string[]>([]);

  const { loadContracts } = useContractStore();

  // Elapsed time ticker
  useEffect(() => {
    if (status.type === "fetching") {
      elapsedTimerRef.current = setInterval(() => {
        setElapsedMs(Date.now() - status.startTime);
      }, 200);
    } else {
      if (elapsedTimerRef.current) {
        clearInterval(elapsedTimerRef.current);
        elapsedTimerRef.current = null;
      }
    }
    return () => {
      if (elapsedTimerRef.current) {
        clearInterval(elapsedTimerRef.current);
      }
    };
  }, [status]);

  // Subscribe to task events
  useEffect(() => {
    let cleanupResult: (() => void) | undefined;
    let cleanupError: (() => void) | undefined;

    const subscribe = async () => {
      cleanupResult = await events.taskResultEvent.listen(
        (event: { payload: TaskResultEvent }) => {
          const { taskId, result } = event.payload;
          if (result.type !== "contractCompleted") return;
          if (activeTaskIdRef.current !== taskId) return;

          // Contract fetch completed — reload contracts from DB to check which were found
          activeTaskIdRef.current = null;
          loadContracts().then(() => {
            const contracts = useContractStore.getState().contracts;
            const contractIdSet = new Set(
              contracts.map((c) => c.id.toLowerCase()),
            );

            const foundIds: string[] = [];
            const notFoundInputs: string[] = [];

            for (const inputId of parsedIdsRef.current) {
              if (contractIdSet.has(inputId.toLowerCase())) {
                foundIds.push(inputId.toLowerCase());
              } else {
                // Find the original input text for this ID
                notFoundInputs.push(inputId);
              }
            }

            setStatus({ type: "complete", foundIds, notFoundInputs });
          });
        },
      );

      cleanupError = await events.taskErrorEvent.listen(
        (event: { payload: TaskErrorEvent }) => {
          const { taskId, message } = event.payload;
          if (activeTaskIdRef.current !== taskId) return;

          activeTaskIdRef.current = null;
          setStatus({ type: "error", message });
          toastError(message);
        },
      );
    };

    subscribe().catch(() => {});

    return () => {
      cleanupResult?.();
      cleanupError?.();
    };
  }, [loadContracts]);

  // --- Input handlers ---

  const handleInputChange = useCallback((index: number, value: string) => {
    setContractInputs((prev) => {
      const next = [...prev];
      next[index] = value;
      return next;
    });
  }, []);

  const handleAddField = useCallback(() => {
    setContractInputs((prev) => {
      if (prev.length >= MAX_CONTRACTS) return prev;
      return [...prev, ""];
    });
  }, []);

  const handleRemoveField = useCallback((index: number) => {
    setContractInputs((prev) => {
      if (prev.length <= 1) return prev;
      return prev.filter((_, i) => i !== index);
    });
  }, []);

  // --- Fetch ---

  const handleFetch = useCallback(async () => {
    // Parse all non-empty inputs
    const parsedIds: string[] = [];
    const errors: string[] = [];

    contractInputs.forEach((input, index) => {
      const trimmed = input.trim();
      if (!trimmed) return; // skip empty
      try {
        parsedIds.push(parseContractId(trimmed));
      } catch (e) {
        errors.push(
          `Field ${index + 1}: ${e instanceof Error ? e.message : String(e)}`,
        );
      }
    });

    if (errors.length > 0) {
      setStatus({
        type: "error",
        message: `Invalid ID${errors.length > 1 ? "s" : ""}: ${errors.join("; ")}`,
      });
      return;
    }

    if (parsedIds.length === 0) {
      setStatus({
        type: "error",
        message: "Please enter at least one contract ID.",
      });
      return;
    }

    // Store parsed IDs for matching results later
    parsedIdsRef.current = parsedIds;

    setStatus({ type: "fetching", startTime: Date.now() });
    setElapsedMs(0);

    try {
      const result = await commands.contractFetch({
        contractIds: parsedIds,
      });
      if (result.status === "ok") {
        activeTaskIdRef.current = result.data.taskId;
        // Use backend-normalized hex IDs for result matching
        parsedIdsRef.current = result.data.normalizedIds;
      } else {
        setStatus({ type: "error", message: result.error });
        toastError(result.error);
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setStatus({ type: "error", message: msg });
      toastError(msg);
    }
  }, [contractInputs]);

  // --- Alias handlers ---

  const handleAliasChange = useCallback((contractId: string, value: string) => {
    setAliasInputs((prev) => ({ ...prev, [contractId]: value }));
  }, []);

  const handleSetAlias = useCallback(
    async (contractId: string) => {
      const alias = (aliasInputs[contractId] || "").trim();
      if (!alias) {
        setAliasResults((prev) => ({
          ...prev,
          [contractId]: { success: false, message: "Alias cannot be empty." },
        }));
        return;
      }

      try {
        const result = await commands.contractSetAlias({
          contractId,
          alias,
        });
        if (result.status === "ok") {
          setAliasResults((prev) => ({
            ...prev,
            [contractId]: {
              success: true,
              message: `Alias set successfully (${alias})`,
            },
          }));
          setAliasInputs((prev) => ({ ...prev, [contractId]: "" }));
          loadContracts();
          toast.success(`Alias set: ${alias}`);
        } else {
          setAliasResults((prev) => ({
            ...prev,
            [contractId]: {
              success: false,
              message: `Failed to set alias: ${result.error}`,
            },
          }));
        }
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        setAliasResults((prev) => ({
          ...prev,
          [contractId]: { success: false, message: `Failed to set alias: ${msg}` },
        }));
      }
    },
    [aliasInputs, loadContracts],
  );

  // --- Navigation ---

  const handleBack = useCallback(() => {
    navigate({ to: "/contracts" });
  }, [navigate]);

  const handleDismissError = useCallback(() => {
    setStatus({ type: "input" });
  }, []);

  // Can fetch: at least one non-empty input and not currently fetching
  const canFetch =
    status.type !== "fetching" &&
    contractInputs.some((input) => input.trim().length > 0);

  return (
    <div className="flex flex-1 flex-col gap-6 overflow-auto p-6">
      <PageHeader
        title="Add Contracts"
        breadcrumbs={[
          { label: "Contracts" },
          { label: "Add Contracts" },
        ]}
        actions={
          <Button variant="outline" size="sm" onClick={handleBack}>
            <ArrowLeft className="size-4 mr-2" />
            Back to Contracts
          </Button>
        }
      />

      <Island>
        <div className="flex flex-col gap-6 p-6 max-w-2xl">
          {/* --- INPUT PHASE --- */}
          {(status.type === "input" || status.type === "error") && (
            <>
              <div className="space-y-1">
                <h3 className="text-sm font-medium">
                  Enter Contract Identifiers
                </h3>
                <p className="text-xs text-muted-foreground">
                  Enter up to {MAX_CONTRACTS} contract IDs in hex or base58
                  format.
                </p>
              </div>

              <div className="space-y-3">
                {contractInputs.map((value, index) => (
                  <div key={index} className="flex items-center gap-2">
                    <label
                      htmlFor={`contract-input-${index}`}
                      className="text-sm text-muted-foreground w-24 shrink-0"
                    >
                      Contract {index + 1}:
                    </label>
                    <Input
                      id={`contract-input-${index}`}
                      type="text"
                      value={value}
                      onChange={(e) => handleInputChange(index, e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" && canFetch) handleFetch();
                      }}
                      placeholder="Hex or base58 identifier"
                      className="font-mono flex-1"
                      disabled={(status as ScreenStatus).type === "fetching"}
                    />
                    {contractInputs.length > 1 && (
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={() => handleRemoveField(index)}
                        aria-label={`Remove contract field ${index + 1}`}
                        className="shrink-0 text-muted-foreground hover:text-destructive"
                      >
                        <Trash2 className="size-4" />
                      </Button>
                    )}
                  </div>
                ))}
              </div>

              <div className="flex items-center gap-3">
                {contractInputs.length < MAX_CONTRACTS && (
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={handleAddField}
                  >
                    <Plus className="size-4 mr-2" />
                    Add Another Contract Field
                  </Button>
                )}
              </div>

              {/* Error banner */}
              {status.type === "error" && (
                <div className="flex items-start gap-3 rounded-lg border border-destructive/30 bg-destructive/5 p-4">
                  <div className="flex-1 min-w-0">
                    <p className="text-sm font-medium text-destructive">
                      Error
                    </p>
                    <p className="mt-1 text-sm text-destructive/80">
                      {status.message}
                    </p>
                  </div>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={handleDismissError}
                    className="shrink-0 text-destructive hover:text-destructive"
                  >
                    Dismiss
                  </Button>
                </div>
              )}

              <div>
                <Button onClick={handleFetch} disabled={!canFetch}>
                  <Search className="size-4 mr-2" />
                  Add Contracts
                </Button>
              </div>
            </>
          )}

          {/* --- FETCHING PHASE --- */}
          {status.type === "fetching" && (
            <div className="flex flex-col items-center gap-4 py-12">
              <Loader2 className="size-8 animate-spin text-dash-blue" />
              <p className="text-sm text-muted-foreground">
                Fetching contracts... Time taken so far:{" "}
                {formatElapsed(elapsedMs)}
              </p>
            </div>
          )}

          {/* --- COMPLETE PHASE --- */}
          {status.type === "complete" && (
            <div className="flex flex-col gap-6">
              <div className="text-center space-y-1">
                <h3 className="text-lg font-semibold">
                  Successfully queried contracts
                </h3>
              </div>

              {/* Found contracts */}
              {status.foundIds.length > 0 && (
                <div className="space-y-3">
                  <h4 className="text-sm font-medium">
                    Found and added the following contracts:
                  </h4>
                  <div className="space-y-3">
                    {status.foundIds.map((contractId) => {
                      const displayId = hexToBase58(contractId);
                      return (
                      <div
                        key={contractId}
                        className="rounded-lg border bg-card p-4"
                      >
                        <div className="flex items-center gap-3 mb-3">
                          <Check className="size-4 text-success shrink-0" />
                          <code className="text-sm font-mono text-success truncate">
                            {displayId}
                          </code>
                          <CopyButton value={displayId} />
                        </div>
                        <div className="flex items-center gap-2">
                          <Input
                            type="text"
                            value={aliasInputs[contractId] || ""}
                            onChange={(e) =>
                              handleAliasChange(contractId, e.target.value)
                            }
                            onKeyDown={(e) => {
                              if (e.key === "Enter")
                                handleSetAlias(contractId);
                            }}
                            placeholder="Enter alias..."
                            className="flex-1 h-8 text-sm"
                          />
                          <Button
                            variant="secondary"
                            size="sm"
                            onClick={() => handleSetAlias(contractId)}
                          >
                            Set Alias
                          </Button>
                        </div>
                        {aliasResults[contractId] && (
                          <p
                            className={`mt-2 text-xs ${aliasResults[contractId].success ? "text-success" : "text-destructive"}`}
                          >
                            {aliasResults[contractId].message}
                          </p>
                        )}
                      </div>
                      );
                    })}
                  </div>
                </div>
              )}

              {/* Not found contracts */}
              {status.notFoundInputs.length > 0 && (
                <div className="space-y-3">
                  <h4 className="text-sm font-medium text-destructive">
                    The following contracts were not found:
                  </h4>
                  <div className="space-y-2">
                    {status.notFoundInputs.map((id, index) => (
                      <div
                        key={index}
                        className="flex items-center gap-2 rounded-lg border border-destructive/30 bg-destructive/5 p-3"
                      >
                        <X className="size-4 text-destructive shrink-0" />
                        <code className="text-sm font-mono text-destructive truncate">
                          {id}
                        </code>
                      </div>
                    ))}
                  </div>
                </div>
              )}

              <div>
                <Button onClick={handleBack}>
                  <ArrowLeft className="size-4 mr-2" />
                  Back to Contracts
                </Button>
              </div>
            </div>
          )}
        </div>
      </Island>
    </div>
  );
}
