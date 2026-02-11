import { useCallback, useEffect, useRef, useState } from "react";
import { commands, events } from "@/bindings";
import type { QualifiedIdentityDto } from "@/bindings";
import { useIdentityStore } from "@/stores/identityStore";

export interface FrozenIdentity {
  /** Identity ID (hex). */
  id: string;
  /** Display label — alias or truncated ID. */
  label: string;
}

export interface UseFrozenIdentitiesResult {
  /** Frozen identities for this token, filtered from local identities. */
  frozenIdentities: FrozenIdentity[];
  /** Whether the query is in progress. */
  loading: boolean;
  /** Error message if the query failed. */
  error: string | null;
}

/**
 * Fetches the list of frozen identities for a given token.
 *
 * On mount, dispatches `tokenQueryFrozenIdentities` with all local identity IDs.
 * When the result comes back via `taskResultEvent`, filters to only those that
 * are actually frozen. Returns the filtered list as dropdown-ready options.
 */
export function useFrozenIdentities(tokenId: string): UseFrozenIdentitiesResult {
  const identities = useIdentityStore((s) => s.identities);
  const [frozenIdentities, setFrozenIdentities] = useState<FrozenIdentity[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const taskIdRef = useRef<string | null>(null);

  // Dispatch the IPC query
  const dispatchQuery = useCallback(
    async (ids: QualifiedIdentityDto[]) => {
      if (!tokenId || ids.length === 0) {
        setLoading(false);
        setFrozenIdentities([]);
        return;
      }
      const identityIds = ids.map((i) => i.id);
      try {
        const result = await commands.tokenQueryFrozenIdentities({
          tokenId,
          identityIds,
        });
        if (result.status === "ok") {
          taskIdRef.current = result.data.taskId;
        } else {
          setError(
            typeof result.error === "string" ? result.error : "Query failed",
          );
          setLoading(false);
        }
      } catch {
        setError("Failed to query frozen identities");
        setLoading(false);
      }
    },
    [tokenId],
  );

  // Subscribe to events and dispatch query
  useEffect(() => {
    let resultCleanup: (() => void) | undefined;
    let errorCleanup: (() => void) | undefined;
    let cancelled = false;

    // Subscribe to task result events
    events.taskResultEvent
      .listen((event) => {
        if (cancelled) return;
        if (taskIdRef.current && event.payload.taskId === taskIdRef.current) {
          const frozenIds = event.payload.payload as string[] | null;
          if (frozenIds && Array.isArray(frozenIds)) {
            const frozenSet = new Set(frozenIds);
            const filtered = identities
              .filter((i) => frozenSet.has(i.id))
              .map((i) => ({
                id: i.id,
                label: i.alias || i.id.slice(0, 12) + "...",
              }));
            setFrozenIdentities(filtered);
          } else {
            setFrozenIdentities([]);
          }
          setLoading(false);
        }
      })
      .then((fn) => {
        resultCleanup = fn;
      });

    // Subscribe to task error events
    events.taskErrorEvent
      .listen((event) => {
        if (cancelled) return;
        const payload = event.payload as {
          taskId: string;
          message: string;
        };
        if (taskIdRef.current && payload.taskId === taskIdRef.current) {
          setError(payload.message || "Query failed");
          setLoading(false);
        }
      })
      .then((fn) => {
        errorCleanup = fn;
      });

    // Dispatch — async IPC call that updates state on completion
    // eslint-disable-next-line react-hooks/set-state-in-effect
    dispatchQuery(identities);

    return () => {
      cancelled = true;
      resultCleanup?.();
      errorCleanup?.();
    };
  }, [tokenId, identities, dispatchQuery]);

  return { frozenIdentities, loading, error };
}
