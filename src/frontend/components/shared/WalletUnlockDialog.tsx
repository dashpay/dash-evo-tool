import { useCallback, useEffect, useRef, useState } from "react";
import { Eye, EyeOff } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

export type WalletUnlockResult =
  | { status: "unlocked"; password: string }
  | { status: "cancelled" };

interface WalletUnlockDialogProps {
  /** Whether the dialog is open */
  open: boolean;
  /** Called when the dialog should close */
  onOpenChange: (open: boolean) => void;
  /** Wallet display name shown in the description */
  walletAlias: string;
  /** Called with the unlock result */
  onResult?: (result: WalletUnlockResult) => void;
  /** External error message to display (e.g. "Incorrect password") */
  error?: string | null;
  /** Password hint shown after a failed attempt */
  passwordHint?: string | null;
  /** Whether an unlock attempt is in progress */
  loading?: boolean;
}

/**
 * Password entry dialog for unlocking encrypted wallets.
 *
 * Matches egui WalletUnlockPopup behavior:
 * - Password input with show/hide toggle
 * - Auto-focus on password field when opened
 * - Enter key submits, Escape cancels
 * - Error message display with optional password hint
 * - Password cleared on close (security)
 */
export function WalletUnlockDialog({
  open,
  onOpenChange,
  walletAlias,
  onResult,
  error,
  passwordHint,
  loading = false,
}: WalletUnlockDialogProps) {
  const [password, setPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  // Auto-focus password field when dialog opens
  useEffect(() => {
    if (open) {
      const timer = setTimeout(() => inputRef.current?.focus(), 50);
      return () => clearTimeout(timer);
    }
  }, [open]);

  const clearState = useCallback(() => {
    setPassword("");
    setShowPassword(false);
  }, []);

  const handleUnlock = useCallback(() => {
    if (!password || loading) return;
    onResult?.({ status: "unlocked", password });
  }, [password, loading, onResult]);

  const handleCancel = useCallback(() => {
    onResult?.({ status: "cancelled" });
    clearState();
    onOpenChange(false);
  }, [onResult, clearState, onOpenChange]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        handleUnlock();
      }
    },
    [handleUnlock],
  );

  const errorMessage =
    error && passwordHint
      ? `${error}. Hint: ${passwordHint}`
      : error || undefined;

  return (
    <Dialog
      open={open}
      onOpenChange={(value) => {
        if (!value) handleCancel();
      }}
    >
      <DialogContent showCloseButton={false} className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Unlock Wallet</DialogTitle>
          <DialogDescription>
            Enter password to unlock &ldquo;{walletAlias}&rdquo;
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-3">
          <div className="relative">
            <Input
              ref={inputRef}
              type={showPassword ? "text" : "password"}
              placeholder="Enter password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              onKeyDown={handleKeyDown}
              aria-invalid={!!error}
              aria-describedby={errorMessage ? "unlock-error" : undefined}
              disabled={loading}
              className="pr-10"
            />
            <Button
              type="button"
              variant="ghost"
              size="icon-xs"
              className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground"
              onClick={() => setShowPassword(!showPassword)}
              aria-label={showPassword ? "Hide password" : "Show password"}
            >
              {showPassword ? (
                <EyeOff className="size-4" />
              ) : (
                <Eye className="size-4" />
              )}
            </Button>
          </div>

          {errorMessage && (
            <p
              id="unlock-error"
              className="text-sm text-destructive"
              role="alert"
            >
              {errorMessage}
            </p>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={handleCancel} disabled={loading}>
            Cancel
          </Button>
          <Button onClick={handleUnlock} disabled={!password || loading}>
            {loading ? "Unlocking…" : "Unlock"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
