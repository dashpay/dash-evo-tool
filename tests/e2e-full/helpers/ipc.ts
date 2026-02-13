/**
 * Tauri IPC helpers for WebdriverIO E2E tests.
 *
 * Wraps `window.__TAURI_INTERNALS__.invoke()` calls via `browser.executeAsync()`
 * so spec files can call Tauri commands directly.
 */

/**
 * Invoke a Tauri IPC command from the browser context.
 *
 * @param command - The snake_case Tauri command name (e.g. "get_network_info")
 * @param args    - Optional argument object passed to the command
 * @returns       The deserialized return value from the Rust backend
 */
interface IpcResult {
  ok: boolean;
  value?: unknown;
  error?: string;
}

export async function invoke<T = unknown>(
  command: string,
  args?: Record<string, unknown>
): Promise<T> {
  const result = await browser.executeAsync(
    (cmd: string, cmdArgs: Record<string, unknown> | undefined, done: (r: IpcResult) => void) => {
      const tauriInternals = (window as any).__TAURI_INTERNALS__;
      if (!tauriInternals || typeof tauriInternals.invoke !== "function") {
        done({ ok: false, error: "Tauri IPC bridge not available" });
        return;
      }
      tauriInternals
        .invoke(cmd, cmdArgs ?? {})
        .then((value: unknown) => done({ ok: true, value }))
        .catch((err: unknown) =>
          done({ ok: false, error: String(err) })
        );
    },
    command,
    args
  );

  const res = result as IpcResult;
  if (!res.ok) {
    throw new Error(`IPC "${command}" failed: ${res.error}`);
  }
  return res.value as T;
}

/**
 * Check whether the Tauri IPC bridge is available in the current page context.
 */
export async function isBridgeAvailable(): Promise<boolean> {
  return browser.execute(() => {
    const t = (window as any).__TAURI_INTERNALS__;
    return !!(t && typeof t.invoke === "function");
  });
}

/**
 * Poll `get_spv_status` until the given network shows "running" status.
 *
 * @param network   - Network name to watch (e.g. "testnet")
 * @param timeoutMs - Maximum wait time (default 5 minutes)
 */
export async function waitForSpvSync(
  network: string,
  timeoutMs = 300_000
): Promise<void> {
  const start = Date.now();
  const interval = 3_000;

  while (Date.now() - start < timeoutMs) {
    try {
      const statuses = await invoke<Array<{ network: string; status: string }>>(
        "get_spv_status"
      );
      const entry = statuses.find(
        (s) => s.network.toLowerCase() === network.toLowerCase()
      );
      if (entry && entry.status === "running") return;
    } catch {
      // SPV not ready yet — keep polling
    }
    await new Promise((r) => setTimeout(r, interval));
  }

  throw new Error(
    `SPV sync for "${network}" did not reach "running" within ${timeoutMs}ms`
  );
}

/**
 * Fetch all wallets and return the total balance (duffs) for a given seed hash.
 */
export async function getWalletBalance(seedHash: string): Promise<number> {
  const result = await invoke<{
    hdWallets: Array<{ seedHash: string; totalBalance: number }>;
    singleKeyWallets: Array<{ keyHash: string; totalBalance: number }>;
  }>("wallet_list_all");

  const hd = result.hdWallets.find((w) => w.seedHash === seedHash);
  if (hd) return hd.totalBalance;

  return 0;
}
