import { useEffect, useLayoutEffect, useRef } from "react";
import { events } from "@/bindings";
import type { TaskResultEvent, TaskErrorEvent } from "@/bindings";

/**
 * Subscribe to task result/error events for a specific task ID.
 *
 * Uses the safe `async subscribe()` pattern with a `cancelled` flag
 * to avoid listener leaks when the component unmounts before `.listen()`
 * resolves.
 *
 * Callbacks are stored in refs so the effect doesn't re-subscribe when
 * they change.
 */
export function useTaskListener(
  taskId: string | null,
  onResult: (event: TaskResultEvent) => void,
  onError: (event: TaskErrorEvent) => void,
) {
  const onResultRef = useRef(onResult);
  const onErrorRef = useRef(onError);
  useLayoutEffect(() => {
    onResultRef.current = onResult;
    onErrorRef.current = onError;
  });

  useEffect(() => {
    if (!taskId) return;

    let cancelled = false;
    const cleanups: (() => void)[] = [];

    const subscribe = async () => {
      const unResult = await events.taskResultEvent.listen((event) => {
        if (cancelled || event.payload.taskId !== taskId) return;
        onResultRef.current(event.payload);
      });
      if (cancelled) { unResult(); return; }
      cleanups.push(unResult);

      const unError = await events.taskErrorEvent.listen((event) => {
        if (cancelled || event.payload.taskId !== taskId) return;
        onErrorRef.current(event.payload);
      });
      if (cancelled) { unError(); return; }
      cleanups.push(unError);
    };
    subscribe().catch(console.error);

    return () => {
      cancelled = true;
      cleanups.forEach((fn) => fn());
    };
  }, [taskId]);
}
