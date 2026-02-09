import { useCallback, useEffect, useRef, useState } from "react";
import { commands } from "@/bindings";

/** Interval in ms between wallet refresh polls. */
const DEFAULT_POLL_INTERVAL = 5000;

export interface UtxoMonitorOptions {
  /** The HD wallet seed hash to monitor. */
  seedHash: string | null;
  /** The address to watch for incoming funds. */
  address: string | null;
  /** Whether monitoring is active. */
  enabled: boolean;
  /** Polling interval in ms (default 5000). */
  pollIntervalMs?: number;
}

export interface UtxoMonitorResult {
  /** True once the monitored address has a balance > 0. */
  fundsReceived: boolean;
  /** The balance detected at the address, in duffs. */
  balance: number;
  /** Whether a refresh is currently in progress. */
  polling: boolean;
}

/**
 * Polls wallet data to detect incoming funds at a specific address.
 *
 * When `enabled` is true and both `seedHash` and `address` are set,
 * periodically fetches the wallet's address list and checks if the
 * target address has a non-zero balance. Triggers a Core wallet refresh
 * before each check so that newly received UTXOs are picked up.
 *
 * Once funds are detected, polling stops automatically.
 */
export function useUtxoMonitor({
  seedHash,
  address,
  enabled,
  pollIntervalMs = DEFAULT_POLL_INTERVAL,
}: UtxoMonitorOptions): UtxoMonitorResult {
  const [fundsReceived, setFundsReceived] = useState(false);
  const [balance, setBalance] = useState(0);
  const [polling, setPolling] = useState(false);

  // Track whether the component is still mounted
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // Reset state when address or wallet changes
  useEffect(() => {
    setFundsReceived(false);
    setBalance(0);
  }, [seedHash, address]);

  const checkForFunds = useCallback(async () => {
    if (!seedHash || !address) return;

    setPolling(true);
    try {
      // Trigger a Core refresh so the backend picks up new UTXOs
      await commands.coreRefreshWalletInfo({
        walletSeedHash: seedHash,
        platformSyncMode: null,
      });

      // Read updated wallet data
      const result = await commands.walletGetHd(seedHash);
      if (!mountedRef.current) return;

      if (result.status === "ok") {
        const addr = result.data.addresses.find(
          (a) => a.address === address,
        );
        if (addr && addr.balance > 0) {
          setBalance(addr.balance);
          setFundsReceived(true);
        }
      }
    } catch {
      // Swallow — polling will retry
    } finally {
      if (mountedRef.current) {
        setPolling(false);
      }
    }
  }, [seedHash, address]);

  useEffect(() => {
    if (!enabled || !seedHash || !address || fundsReceived) return;

    // Run an initial check immediately
    checkForFunds();

    const timer = setInterval(checkForFunds, pollIntervalMs);
    return () => clearInterval(timer);
  }, [enabled, seedHash, address, fundsReceived, pollIntervalMs, checkForFunds]);

  return { fundsReceived, balance, polling };
}
