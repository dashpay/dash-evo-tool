import { clsx, type ClassValue } from "clsx"
import { twMerge } from "tailwind-merge"
import { events } from "@/bindings"

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

/** Wait for a dispatched backend task to complete. Resolves on success, rejects on error. */
export async function waitForTask(taskId: string, timeoutMs = 30000): Promise<void> {
  let resolved = false;
  let resolveFn: () => void;
  let rejectFn: (err: Error) => void;

  const promise = new Promise<void>((resolve, reject) => {
    resolveFn = resolve;
    rejectFn = reject;
  });

  const timer = setTimeout(() => {
    if (resolved) return;
    resolved = true;
    unsubResult();
    unsubError();
    rejectFn(new Error("Task timed out"));
  }, timeoutMs);

  const done = (fn: () => void) => {
    if (resolved) return;
    resolved = true;
    clearTimeout(timer);
    unsubResult();
    unsubError();
    fn();
  };

  const unsubResult = await events.taskResultEvent.listen((event) => {
    if (event.payload.taskId !== taskId) return;
    done(() => resolveFn());
  });
  const unsubError = await events.taskErrorEvent.listen((event) => {
    if (event.payload.taskId !== taskId) return;
    done(() => rejectFn(new Error(event.payload.message)));
  });

  if (resolved) {
    unsubResult();
    unsubError();
  }

  return promise;
}

const BASE58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/** Convert a hex string to base58 (Bitcoin/Dash alphabet). */
export function hexToBase58(hex: string): string {
  if (!/^[0-9a-fA-F]*$/.test(hex)) return hex;

  const bytes: number[] = [];
  for (let i = 0; i < hex.length; i += 2) {
    bytes.push(parseInt(hex.substring(i, i + 2), 16));
  }

  // Count leading zero bytes → each becomes a '1' prefix
  let leadingZeros = 0;
  for (const b of bytes) {
    if (b !== 0) break;
    leadingZeros++;
  }

  // Convert byte array to bigint
  let num = 0n;
  for (const b of bytes) {
    num = num * 256n + BigInt(b);
  }

  // Convert to base58
  const chars: string[] = [];
  while (num > 0n) {
    const remainder = Number(num % 58n);
    chars.push(BASE58_ALPHABET[remainder]);
    num = num / 58n;
  }

  return "1".repeat(leadingZeros) + chars.reverse().join("");
}

/** Convert a hex ID to truncated base58 for display. */
export function displayId(hex: string, chars = 6): string {
  const b58 = hexToBase58(hex);
  if (b58.length <= chars * 2 + 3) return b58;
  return `${b58.slice(0, chars)}...${b58.slice(-chars)}`;
}
