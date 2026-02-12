import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  ArrowLeft,
  Loader2,
  Play,
  Search,
  Users,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { EmptyState } from "@/components/feedback";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Island, PageHeader } from "@/components/layout";
import { IdentitySelector } from "@/components/shared/IdentitySelector";
import { commands, events } from "@/bindings";
import type {
  TaskResultEvent,
  ContractSummaryDto,
  QualifiedIdentityDto,
} from "@/bindings";
import { useIdentityStore } from "@/stores/identityStore";
import { useContractStore } from "@/stores/contractStore";
import { toastError } from "@/lib/toastError";
import { displayId } from "@/lib/utils";

// ─── Types ────────────────────────────────────────────────────────────

/** Group action as returned from the backend event payload. */
export interface GroupActionItem {
  groupPosition: number;
  actionId: string;
  actionType: string;
  signersCount: number;
  requiredSignatures: number;
  details: Record<string, unknown>;
}

type ScreenStatus =
  | { type: "idle" }
  | { type: "fetching"; startTime: number }
  | { type: "fetched"; actions: GroupActionItem[] }
  | { type: "error"; message: string };

// ─── Helpers ──────────────────────────────────────────────────────────

const SYSTEM_CONTRACTS = [
  "dpns",
  "dashpay",
  "keyword_search",
  "token_history",
  "withdrawals",
];

/** Filter contracts to only those that have tokens (group actions relate to tokens). */
function getContractsWithTokens(
  contracts: ContractSummaryDto[],
): ContractSummaryDto[] {
  return contracts.filter(
    (c) =>
      c.tokenCount > 0 &&
      !SYSTEM_CONTRACTS.includes(c.alias?.toLowerCase() ?? ""),
  );
}

/** Format action type for display (e.g. "TokenMint" → "Mint"). */
function formatActionType(actionType: string): string {
  return actionType.replace(/^Token/, "");
}

/** Extract a human-readable info string from group action details. */
function formatActionInfo(details: Record<string, unknown>): string {
  const parts: string[] = [];
  if (details.amount !== undefined) {
    parts.push(`Amount: ${String(details.amount)}`);
  }
  if (details.recipient !== undefined) {
    parts.push(`To: ${displayId(String(details.recipient))}`);
  }
  if (details.identity !== undefined) {
    parts.push(`Identity: ${displayId(String(details.identity))}`);
  }
  if (details.note !== undefined && details.note !== null) {
    parts.push(`Note: ${String(details.note)}`);
  }
  return parts.length > 0 ? parts.join(", ") : "—";
}

// ─── Component ────────────────────────────────────────────────────────

export function GroupActionsScreen() {
  const navigate = useNavigate();

  // ── Stores ──────────────────────────────────────────────────────────
  const identities = useIdentityStore((s) => s.identities);
  const identitiesLoading = useIdentityStore((s) => s.loading);
  const loadIdentities = useIdentityStore((s) => s.loadIdentities);
  const {
    contracts,
    loading: contractsLoading,
    loadContracts,
  } = useContractStore();

  // ── Local state ─────────────────────────────────────────────────────
  const [selectedContractId, setSelectedContractId] = useState("");
  const [selectedIdentityId, setSelectedIdentityId] = useState("");
  const [status, setStatus] = useState<ScreenStatus>({ type: "idle" });
  const [elapsedMs, setElapsedMs] = useState(0);
  const [searchFilter, setSearchFilter] = useState("");

  // Track the task ID for matching events
  const taskIdRef = useRef<string | null>(null);
  const elapsedTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // ── Derived values ──────────────────────────────────────────────────
  const filteredContracts = useMemo(
    () => getContractsWithTokens(contracts),
    [contracts],
  );

  const identityOptions = useMemo(
    () =>
      identities.map((i: QualifiedIdentityDto) => ({
        id: i.id,
        displayName:
          i.alias ?? `${i.id.slice(0, 8)}…${i.id.slice(-4)}`,
      })),
    [identities],
  );

  // Auto-select first identity if only one
  const effectiveIdentityId = useMemo(() => {
    if (selectedIdentityId) return selectedIdentityId;
    if (identities.length === 1) return identities[0]!.id;
    return "";
  }, [selectedIdentityId, identities]);

  const canFetch = selectedContractId !== "" && effectiveIdentityId !== "";

  // Filter displayed actions by search
  const displayedActions = useMemo(() => {
    if (status.type !== "fetched") return [];
    if (!searchFilter.trim()) return status.actions;
    const lower = searchFilter.toLowerCase();
    return status.actions.filter(
      (a) =>
        a.actionId.toLowerCase().includes(lower) ||
        a.actionType.toLowerCase().includes(lower) ||
        formatActionInfo(a.details).toLowerCase().includes(lower),
    );
  }, [status, searchFilter]);

  // ── Load data on mount ──────────────────────────────────────────────
  useEffect(() => {
    if (identities.length === 0 && !identitiesLoading) {
      loadIdentities();
    }
    if (contracts.length === 0 && !contractsLoading) {
      loadContracts();
    }
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // ── Elapsed timer ───────────────────────────────────────────────────
  useEffect(() => {
    if (status.type === "fetching") {
      elapsedTimerRef.current = setInterval(() => {
        setElapsedMs(Date.now() - status.startTime);
      }, 100);
    } else {
      if (elapsedTimerRef.current) {
        clearInterval(elapsedTimerRef.current);
        elapsedTimerRef.current = null;
      }
    }
    return () => {
      if (elapsedTimerRef.current) {
        clearInterval(elapsedTimerRef.current);
        elapsedTimerRef.current = null;
      }
    };
  }, [status]);

  // ── Event listener ──────────────────────────────────────────────────
  useEffect(() => {
    let unlistenResult: (() => void) | null = null;
    let unlistenError: (() => void) | null = null;

    const setup = async () => {
      unlistenResult = await events.taskResultEvent.listen(
        (event: { payload: TaskResultEvent }) => {
          const { taskId, result } = event.payload;

          // Only handle our task
          if (taskId !== taskIdRef.current) return;
          if (result.type !== "contractCompleted") return;

          // Parse group actions from result
          try {
            const actions = parseGroupActions(result);
            setStatus({ type: "fetched", actions });
          } catch {
            const msg = "Failed to parse group actions result";
            setStatus({ type: "error", message: msg });
            toastError(msg);
          }
        },
      );

      unlistenError = await events.taskErrorEvent.listen(
        (event: { payload: { taskId: string; message: string } }) => {
          if (event.payload.taskId !== taskIdRef.current) return;
          setStatus({ type: "error", message: event.payload.message });
          toastError(event.payload.message);
        },
      );
    };

    setup();
    return () => {
      unlistenResult?.();
      unlistenError?.();
    };
  }, []);

  // ── Fetch group actions ─────────────────────────────────────────────
  const handleFetch = useCallback(async () => {
    if (!canFetch) return;

    setStatus({ type: "fetching", startTime: Date.now() });
    setElapsedMs(0);
    setSearchFilter("");

    try {
      const result = await commands.contractFetchActiveGroupActions({
        contractId: selectedContractId,
        identityId: effectiveIdentityId,
      });

      if (result.status === "ok") {
        taskIdRef.current = result.data.taskId;
      } else {
        setStatus({ type: "error", message: result.error });
        toastError(result.error);
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setStatus({ type: "error", message: msg });
      toastError(msg);
    }
  }, [canFetch, selectedContractId, effectiveIdentityId]);

  // ── Take action on a group action → navigate to token screen ───────
  const handleTakeAction = useCallback(
    (action: GroupActionItem) => {
      // The token action screens are in /tokens/* routes.
      // We pass the group action details as search params so the token screen
      // can detect it's signing an existing group action.
      const actionRoute = getTokenRouteForAction(action.actionType);
      if (actionRoute) {
        navigate({
          to: actionRoute,
          search: {
            groupActionId: action.actionId,
            groupPosition: action.groupPosition,
            contractId: selectedContractId,
            details: JSON.stringify(action.details),
          },
        });
      }
    },
    [navigate, selectedContractId],
  );

  // ── Render ──────────────────────────────────────────────────────────
  return (
    <div className="flex flex-col gap-6 p-6">
      <PageHeader
        title="Group Actions"
        breadcrumbs={[
          { label: "Contracts" },
          { label: "Group Actions" },
        ]}
        actions={
          <Button
            variant="ghost"
            size="sm"
            onClick={() => navigate({ to: "/contracts" })}
          >
            <ArrowLeft className="mr-2 h-4 w-4" />
            Back to Contracts
          </Button>
        }
      />

      {/* Step 1: Select Contract */}
      <Island>
        <div className="flex flex-col gap-4 p-4">
          <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">
            Step 1 — Select Contract
          </h3>

          {contractsLoading ? (
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin" />
              Loading contracts…
            </div>
          ) : filteredContracts.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No contracts with tokens found. Add a contract with
              group-action-enabled tokens first.
            </p>
          ) : (
            <div>
              <Label htmlFor="contract-select">Contract</Label>
              <Select
                value={selectedContractId}
                onValueChange={(val) => {
                  setSelectedContractId(val);
                  setStatus({ type: "idle" });
                }}
              >
                <SelectTrigger id="contract-select" className="mt-1">
                  <SelectValue placeholder="Choose a contract…" />
                </SelectTrigger>
                <SelectContent>
                  {filteredContracts.map((c) => (
                    <SelectItem key={c.id} value={c.id}>
                      {c.alias
                        ? `${c.alias} (${displayId(c.id)})`
                        : displayId(c.id)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          )}
        </div>
      </Island>

      {/* Step 2: Select Identity */}
      <Island>
        <div className="flex flex-col gap-4 p-4">
          <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">
            Step 2 — Select Identity
          </h3>

          {identitiesLoading ? (
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin" />
              Loading identities…
            </div>
          ) : identities.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No identities loaded. Load an identity first to check for group
              membership.
            </p>
          ) : (
            <IdentitySelector
              value={effectiveIdentityId}
              onChange={(id) => {
                setSelectedIdentityId(id);
                setStatus({ type: "idle" });
              }}
              identities={identityOptions}
              showOther={false}
              label="Identity"
              placeholder="Choose an identity…"
            />
          )}
        </div>
      </Island>

      {/* Step 3: Fetch & Results */}
      <Island>
        <div className="flex flex-col gap-4 p-4">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">
              Step 3 — Active Group Actions
            </h3>
            <Button
              onClick={handleFetch}
              disabled={!canFetch || status.type === "fetching"}
              size="sm"
            >
              {status.type === "fetching" ? (
                <>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Fetching… ({(elapsedMs / 1000).toFixed(1)}s)
                </>
              ) : (
                <>
                  <Search className="mr-2 h-4 w-4" />
                  Fetch Group Actions
                </>
              )}
            </Button>
          </div>

          {/* Error banner */}
          {status.type === "error" && (
            <div className="rounded-md bg-destructive/10 border border-destructive/30 p-3 text-sm text-destructive flex items-center justify-between">
              <span>{status.message}</span>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setStatus({ type: "idle" })}
              >
                Dismiss
              </Button>
            </div>
          )}

          {/* Results */}
          {status.type === "fetched" && (
            <>
              {status.actions.length === 0 ? (
                <EmptyState
                  icon={Users}
                  title="No Group Actions"
                  description="No active group actions found for this contract and identity."
                />
              ) : (
                <>
                  {/* Search filter */}
                  <div className="flex items-center gap-2">
                    <Search className="h-4 w-4 text-muted-foreground" />
                    <Input
                      placeholder="Filter actions…"
                      value={searchFilter}
                      onChange={(e) => setSearchFilter(e.target.value)}
                      className="max-w-sm"
                    />
                    <span className="text-xs text-muted-foreground">
                      {displayedActions.length} of {status.actions.length}{" "}
                      action{status.actions.length !== 1 ? "s" : ""}
                    </span>
                  </div>

                  {/* Actions table */}
                  <div className="rounded-md border">
                    <Table>
                      <TableHeader>
                        <TableRow>
                          <TableHead className="w-[180px]">
                            Action ID
                          </TableHead>
                          <TableHead className="w-[100px]">Type</TableHead>
                          <TableHead>Info</TableHead>
                          <TableHead className="w-[120px]">
                            Signers
                          </TableHead>
                          <TableHead className="w-[100px] text-right">
                            Action
                          </TableHead>
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {displayedActions.map((action) => (
                          <TableRow key={action.actionId}>
                            <TableCell
                              className="font-mono text-xs"
                              title={action.actionId}
                            >
                              {displayId(action.actionId)}
                            </TableCell>
                            <TableCell>
                              <span className="inline-flex items-center rounded-md bg-primary/10 px-2 py-0.5 text-xs font-medium text-primary">
                                {formatActionType(action.actionType)}
                              </span>
                            </TableCell>
                            <TableCell className="text-sm">
                              {formatActionInfo(action.details)}
                            </TableCell>
                            <TableCell className="text-sm">
                              {action.signersCount}/
                              {action.requiredSignatures}
                            </TableCell>
                            <TableCell className="text-right">
                              <Button
                                variant="outline"
                                size="sm"
                                onClick={() => handleTakeAction(action)}
                              >
                                <Play className="mr-1 h-3 w-3" />
                                Take Action
                              </Button>
                            </TableCell>
                          </TableRow>
                        ))}
                      </TableBody>
                    </Table>
                  </div>
                </>
              )}
            </>
          )}

          {/* Idle hint */}
          {status.type === "idle" && canFetch && (
            <EmptyState
              icon={Search}
              title="Fetch Group Actions"
              description='Select a contract and identity above, then click "Fetch Group Actions" to see pending actions that need your signature.'
              className="py-8"
            />
          )}

          {status.type === "idle" && !canFetch && (
            <EmptyState
              icon={Users}
              title="Group Actions"
              description="Select a contract with tokens and an identity to get started."
              className="py-8"
            />
          )}
        </div>
      </Island>
    </div>
  );
}

// ─── Parse helpers ────────────────────────────────────────────────────

/** Parse group actions from the TaskResultEvent payload. */
function parseGroupActions(
  payload: unknown,
): GroupActionItem[] {
  if (!payload) return [];

  // The payload can be an array of GroupActionDto objects,
  // or an object with a "groupActions" key containing the array.
  if (Array.isArray(payload)) {
    return payload.map(normalizeGroupAction);
  }

  if (typeof payload === "object" && payload !== null) {
    const p = payload as Record<string, unknown>;
    // Check for "groupActions" key
    if (Array.isArray(p.groupActions)) {
      return p.groupActions.map(normalizeGroupAction);
    }
    // Check for "actions" key
    if (Array.isArray(p.actions)) {
      return p.actions.map(normalizeGroupAction);
    }
    // The payload might be a map of action_id -> action — convert to array
    const entries = Object.entries(p);
    if (
      entries.length > 0 &&
      typeof entries[0]![1] === "object" &&
      entries[0]![1] !== null
    ) {
      return entries.map(([, v]) => normalizeGroupAction(v));
    }
  }

  return [];
}

function normalizeGroupAction(raw: unknown): GroupActionItem {
  const o = (raw ?? {}) as Record<string, unknown>;
  return {
    groupPosition: Number(o.groupPosition ?? o.group_position ?? 0),
    actionId: String(o.actionId ?? o.action_id ?? ""),
    actionType: String(o.actionType ?? o.action_type ?? "Unknown"),
    signersCount: Number(o.signersCount ?? o.signers_count ?? 0),
    requiredSignatures: Number(
      o.requiredSignatures ?? o.required_signatures ?? 0,
    ),
    details: (typeof o.details === "object" && o.details !== null
      ? o.details
      : {}) as Record<string, unknown>,
  };
}

/** Map a group action type string to the corresponding token route. */
function getTokenRouteForAction(actionType: string): string | null {
  const type = actionType.replace(/^Token/, "").toLowerCase();
  const routeMap: Record<string, string> = {
    mint: "/tokens/mint",
    burn: "/tokens/burn",
    freeze: "/tokens/freeze",
    unfreeze: "/tokens/unfreeze",
    destroyfrozenfunds: "/tokens/destroy-frozen-funds",
    pause: "/tokens/pause",
    resume: "/tokens/resume",
    configupdate: "/tokens/update-config",
    changeprice: "/tokens/set-price",
    changepriceforbulkpurchase: "/tokens/set-price",
    changepricefordirectpurchase: "/tokens/set-price",
    transfer: "/tokens/transfer",
    claim: "/tokens/claim",
    emergencyaction: "/tokens/pause",
  };
  return routeMap[type] ?? null;
}
