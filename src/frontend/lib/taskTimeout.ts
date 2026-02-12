/** Default timeout for async backend tasks (2 minutes). */
const DEFAULT_TASK_TIMEOUT_MS = 120_000;

/** Error message shown when a task times out. */
export const TIMEOUT_ERROR_MESSAGE =
  "Operation timed out \u2014 the backend may still be processing. Try refreshing.";

/** Handle returned by `startTaskTimeout` for clearing the timeout. */
export interface TaskTimeoutHandle {
  clear: () => void;
}

/**
 * Start a timeout that fires `onTimeout` if not cleared within `ms`.
 *
 * Usage:
 * ```ts
 * let timeout = startTaskTimeout(() => set({ loading: false, error: TIMEOUT_ERROR_MESSAGE }));
 * // ... later, in the event handler:
 * timeout.clear();
 * ```
 */
export function startTaskTimeout(
  onTimeout: () => void,
  ms = DEFAULT_TASK_TIMEOUT_MS,
): TaskTimeoutHandle {
  const id = window.setTimeout(onTimeout, ms);
  return { clear: () => window.clearTimeout(id) };
}

/**
 * Manages multiple concurrent task timeouts keyed by operation name.
 *
 * Replaces the single module-level `taskTimeoutHandle` pattern so that
 * concurrent operations don't cancel each other's timeouts.
 */
export class TaskTimeoutManager {
  private handles = new Map<string, TaskTimeoutHandle>();

  /** Start (or restart) a timeout for the given key. */
  start(key: string, onTimeout: () => void, ms?: number) {
    this.handles.get(key)?.clear();
    this.handles.set(key, startTaskTimeout(onTimeout, ms));
  }

  /** Clear a specific timeout by key. */
  clear(key: string) {
    this.handles.get(key)?.clear();
    this.handles.delete(key);
  }

  /** Clear all active timeouts. */
  clearAll() {
    this.handles.forEach((h) => h.clear());
    this.handles.clear();
  }
}
