/**
 * Event Simulation Helpers for Tauri IPC Events
 *
 * Provides typed helpers for emitting mock Tauri events in tests.
 * Works in conjunction with `createMockBindings()` from `./mock-ipc`.
 *
 * Usage:
 * ```ts
 * const mocks = createMockBindings();
 * // ... render component that calls events.taskResultEvent.listen(cb) ...
 *
 * // Emit a task result to all registered listeners:
 * emitTaskResult(mocks, {
 *   taskId: "abc",
 *   resultType: "Wallet",
 *   payload: { ... },
 * });
 * ```
 */

import type { MockBindingsResult, EventName } from "./mock-ipc";

// ---------------------------------------------------------------------------
// Event payload types (matching bindings.ts generated types)
// ---------------------------------------------------------------------------

export interface TaskResultPayload {
  taskId: string;
  resultType: string;
  payload: unknown;
}

export interface TaskErrorPayload {
  taskId: string;
  message: string;
  details: string;
  retryable?: boolean;
}

export interface WalletUpdatedPayload {
  walletSeedHash: string;
  network: string;
}

export interface SpvStatusPayload {
  network: string;
  status: string;
  syncProgressPct: number | null;
  headerHeight: number | null;
}

export interface ZmqChainLockedBlockPayload {
  network: string;
  blockHeight: number;
  blockHash: string;
  txCount: number;
}

export interface ZmqConnectionStatusPayload {
  network: string;
  connected: boolean;
}

export interface ZmqIsLockedTransactionPayload {
  network: string;
  txid: string;
  rawTx: string;
  utxoCount: number;
}

export interface ScheduledVoteExecutedPayload {
  contestedName: string;
  voterId: string;
  success: boolean;
  error: string | null;
}

// ---------------------------------------------------------------------------
// Core emit function
// ---------------------------------------------------------------------------

/**
 * Emit a mock event to all listeners registered via `events.*.listen()`.
 * Calls every listener synchronously with `{ payload }`.
 */
export function emitMockEvent(
  mocks: MockBindingsResult,
  eventName: EventName,
  payload: unknown,
): void {
  mocks.emitMockEvent(eventName, payload);
}

// ---------------------------------------------------------------------------
// Typed event emitters
// ---------------------------------------------------------------------------

/**
 * Emit a `taskResultEvent` to all listeners.
 *
 * This is the most commonly needed event — it delivers backend task results
 * to the frontend (stores subscribe to this for identity, wallet, token, etc. updates).
 */
export function emitTaskResult(
  mocks: MockBindingsResult,
  payload: TaskResultPayload,
): void {
  mocks.emitMockEvent("taskResultEvent", payload);
}

/**
 * Emit a `taskErrorEvent` to all listeners.
 */
export function emitTaskError(
  mocks: MockBindingsResult,
  payload: TaskErrorPayload,
): void {
  mocks.emitMockEvent("taskErrorEvent", payload);
}

/**
 * Emit a `walletUpdatedEvent` to all listeners.
 */
export function emitWalletUpdated(
  mocks: MockBindingsResult,
  payload: WalletUpdatedPayload,
): void {
  mocks.emitMockEvent("walletUpdatedEvent", payload);
}

/**
 * Emit a `spvStatusEvent` to all listeners.
 */
export function emitSpvStatus(
  mocks: MockBindingsResult,
  payload: SpvStatusPayload,
): void {
  mocks.emitMockEvent("spvStatusEvent", payload);
}

/**
 * Emit a `zmqChainLockedBlockEvent` to all listeners.
 */
export function emitZmqChainLockedBlock(
  mocks: MockBindingsResult,
  payload: ZmqChainLockedBlockPayload,
): void {
  mocks.emitMockEvent("zmqChainLockedBlockEvent", payload);
}

/**
 * Emit a `zmqConnectionStatusEvent` to all listeners.
 */
export function emitZmqConnectionStatus(
  mocks: MockBindingsResult,
  payload: ZmqConnectionStatusPayload,
): void {
  mocks.emitMockEvent("zmqConnectionStatusEvent", payload);
}

/**
 * Emit a `zmqIsLockedTransactionEvent` to all listeners.
 */
export function emitZmqIsLockedTransaction(
  mocks: MockBindingsResult,
  payload: ZmqIsLockedTransactionPayload,
): void {
  mocks.emitMockEvent("zmqIsLockedTransactionEvent", payload);
}

/**
 * Emit a `scheduledVoteExecutedEvent` to all listeners.
 */
export function emitScheduledVoteExecuted(
  mocks: MockBindingsResult,
  payload: ScheduledVoteExecutedPayload,
): void {
  mocks.emitMockEvent("scheduledVoteExecutedEvent", payload);
}

// ---------------------------------------------------------------------------
// Assertion helpers
// ---------------------------------------------------------------------------

/**
 * Returns the number of listeners currently registered for the given event.
 */
export function getListenerCount(
  mocks: MockBindingsResult,
  eventName: EventName,
): number {
  return (mocks.eventListeners.get(eventName) ?? []).length;
}

/**
 * Returns all listeners currently registered for the given event.
 */
export function getEventListeners(
  mocks: MockBindingsResult,
  eventName: EventName,
): Array<(event: { payload: unknown }) => void> {
  return [...(mocks.eventListeners.get(eventName) ?? [])];
}
