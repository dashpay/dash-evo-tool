import { useCallback, useEffect, useRef, useState } from "react";
import { ToolPageLayout } from "@/components/tools/ToolPageLayout";
import { CopyButton } from "@/components/shared/CopyButton";
import { commands, events } from "@/bindings";
import type { TaskResultEvent, TaskErrorEvent } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { toastError } from "@/lib/toastError";
import { Loader2, Search } from "lucide-react";

/** Validated address balance result from the backend. */
interface AddressBalanceResult {
  address: string;
  balance: number;
  nonce: number;
}

/** Validate a platform address. Returns an error string or null if valid. */
function validateAddress(address: string): string | null {
  if (address.length === 0) return null;
  if (!address.startsWith("evo1") && !address.startsWith("tevo1")) {
    return 'Address must start with "evo1" (mainnet) or "tevo1" (testnet/devnet)';
  }
  return null;
}

/** Format a credit balance as both credits and Dash. */
function formatBalance(credits: number): string {
  const dash = credits / 100_000_000_000;
  return `${credits.toLocaleString()} credits (${dash.toFixed(8)} Dash)`;
}

/**
 * Address Balance screen — look up the balance and nonce of a Platform address.
 *
 * Single-card form with address input, validation, and result display.
 * Uses `platformFetchAddressBalance` IPC command.
 */
export function AddressBalanceScreen() {
  const [address, setAddress] = useState("");
  const [validationError, setValidationError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [result, setResult] = useState<AddressBalanceResult | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  const activeTaskIdRef = useRef<string | null>(null);

  // Subscribe to task result / error events
  useEffect(() => {
    let cleanupResult: (() => void) | undefined;
    let cleanupError: (() => void) | undefined;

    const subscribe = async () => {
      cleanupResult = await events.taskResultEvent.listen(
        (event: { payload: TaskResultEvent }) => {
          const { taskId, result } = event.payload;

          if (result.type !== "platformAddressBalance") return;
          if (activeTaskIdRef.current !== taskId) return;

          setResult({
            address: result.address,
            balance: result.balance,
            nonce: result.nonce,
          });

          setIsLoading(false);
          setErrorMessage(null);
          activeTaskIdRef.current = null;
        },
      );

      cleanupError = await events.taskErrorEvent.listen(
        (event: { payload: TaskErrorEvent }) => {
          const { taskId, message } = event.payload;
          if (activeTaskIdRef.current !== taskId) return;

          setErrorMessage(message);
          setIsLoading(false);
          activeTaskIdRef.current = null;
          toastError(message);
        },
      );
    };

    subscribe().catch(() => {});

    return () => {
      cleanupResult?.();
      cleanupError?.();
    };
  }, []);

  const handleAddressChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const value = e.target.value;
      setAddress(value);
      setValidationError(validateAddress(value.trim()));
    },
    [],
  );

  const canFetch =
    !isLoading &&
    address.trim().length > 0 &&
    validationError === null;

  const handleFetch = useCallback(async () => {
    const trimmed = address.trim();
    if (!trimmed || isLoading) return;

    const error = validateAddress(trimmed);
    if (error) {
      setValidationError(error);
      return;
    }

    setIsLoading(true);
    setErrorMessage(null);

    try {
      const response = await commands.platformFetchAddressBalance({
        address: trimmed,
      });
      activeTaskIdRef.current = response.taskId;
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setErrorMessage(msg);
      setIsLoading(false);
      toastError(msg);
    }
  }, [address, isLoading]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLInputElement>) => {
      if (e.key === "Enter" && canFetch) {
        handleFetch();
      }
    },
    [canFetch, handleFetch],
  );

  return (
    <ToolPageLayout
      title="Platform Address Balance Lookup"
      subtitle="Look up the balance and nonce of a Platform address"
    >
      <div className="flex flex-col gap-6 max-w-2xl">
        {/* Input section */}
        <div className="space-y-3">
          <label
            htmlFor="address-input"
            className="text-sm font-medium leading-none"
          >
            Enter a Platform address (evo1... or tevo1...):
          </label>
          <div className="flex gap-2">
            <div className="flex-1">
              <Input
                id="address-input"
                type="text"
                value={address}
                onChange={handleAddressChange}
                onKeyDown={handleKeyDown}
                placeholder="evo1... or tevo1..."
                disabled={isLoading}
                aria-invalid={validationError !== null}
                aria-describedby={
                  validationError ? "address-validation-error" : undefined
                }
                className={cn(
                  "font-mono",
                  validationError && "border-destructive focus-visible:ring-destructive",
                )}
              />
              {validationError && (
                <p
                  id="address-validation-error"
                  className="mt-1.5 text-xs text-destructive"
                  role="alert"
                >
                  {validationError}
                </p>
              )}
            </div>
            <Button
              onClick={handleFetch}
              disabled={!canFetch}
              className="flex-shrink-0"
            >
              {isLoading ? (
                <>
                  <Loader2 className="size-4 animate-spin mr-2" />
                  Loading...
                </>
              ) : (
                <>
                  <Search className="size-4 mr-2" />
                  Fetch Balance
                </>
              )}
            </Button>
          </div>
        </div>

        {/* Error banner */}
        {errorMessage && (
          <div className="flex items-start gap-3 rounded-lg border border-destructive/30 bg-destructive/5 p-4">
            <div className="flex-1 min-w-0">
              <p className="text-sm font-medium text-destructive">Error</p>
              <p className="mt-1 text-sm text-destructive/80">
                {errorMessage}
              </p>
            </div>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setErrorMessage(null)}
              className="flex-shrink-0 text-destructive hover:text-destructive"
            >
              Dismiss
            </Button>
          </div>
        )}

        {/* Loading state (only when no result yet) */}
        {isLoading && !result && (
          <div className="flex items-center justify-center py-12">
            <div className="flex flex-col items-center gap-3 text-muted-foreground">
              <Loader2 className="size-8 animate-spin" />
              <p className="text-sm">Fetching balance...</p>
            </div>
          </div>
        )}

        {/* Result display */}
        {result && (
          <div className="rounded-lg border bg-card p-6 space-y-4">
            <h2 className="text-lg font-semibold">Result</h2>
            <div className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-3 items-baseline">
              <span className="text-sm text-muted-foreground">Address:</span>
              <div className="flex items-center gap-2 min-w-0">
                <code className="text-sm font-mono truncate">
                  {result.address}
                </code>
                <CopyButton value={result.address} />
              </div>

              <span className="text-sm text-muted-foreground">Balance:</span>
              <div className="flex items-center gap-2 min-w-0">
                <code className="text-sm font-mono">
                  {formatBalance(result.balance)}
                </code>
                <CopyButton value={String(result.balance)} />
              </div>

              <span className="text-sm text-muted-foreground">Nonce:</span>
              <div className="flex items-center gap-2 min-w-0">
                <code className="text-sm font-mono">{result.nonce}</code>
                <CopyButton value={String(result.nonce)} />
              </div>
            </div>
          </div>
        )}

        {/* Empty state */}
        {!isLoading && !result && !errorMessage && (
          <div className="flex items-center justify-center py-12">
            <p className="text-sm text-muted-foreground">
              Enter a platform address above and click &quot;Fetch Balance&quot;
              to look up its balance and nonce.
            </p>
          </div>
        )}
      </div>
    </ToolPageLayout>
  );
}
