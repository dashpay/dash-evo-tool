import { useCallback, useState } from "react";
import { Eye, EyeOff, ShieldAlert } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/shared/CopyButton";

interface PrivateKeyDialogProps {
  /** Whether the dialog is open */
  open: boolean;
  /** Called when the dialog should close */
  onOpenChange: (open: boolean) => void;
  /** The address whose key is being displayed */
  address: string;
  /** The private key in WIF format */
  privateKeyWif: string;
}

/**
 * Dialog for displaying a wallet address's private key in WIF format.
 *
 * Matches egui PrivateKeyDialogState behavior:
 * - Address display with copy button
 * - Private key display (masked by default, revealed on toggle)
 * - Copy Address and Copy Key buttons
 * - Security warning about keeping the key safe
 * - Key hidden again when dialog closes
 *
 * The caller is responsible for wallet unlock before opening this dialog.
 *
 * @see egui: src/ui/wallets/wallets_screen/dialogs.rs lines 82-96, 763-873
 */
export function PrivateKeyDialog({
  open,
  onOpenChange,
  address,
  privateKeyWif,
}: PrivateKeyDialogProps) {
  // Use a key to reset inner state when dialog reopens
  const [openCount, setOpenCount] = useState(0);
  const handleOpenChange = useCallback(
    (value: boolean) => {
      if (value) setOpenCount((c) => c + 1);
      onOpenChange(value);
    },
    [onOpenChange],
  );

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-md" showCloseButton={false}>
        <DialogHeader>
          <DialogTitle>Private Key</DialogTitle>
          <DialogDescription className="sr-only">
            View the private key for this address
          </DialogDescription>
        </DialogHeader>
        <PrivateKeyContent
          key={openCount}
          address={address}
          privateKeyWif={privateKeyWif}
        />
        <DialogFooter>
          <Button variant="outline" onClick={() => handleOpenChange(false)}>
            Close
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** Inner content that resets state when remounted via key. */
function PrivateKeyContent({
  address,
  privateKeyWif,
}: {
  address: string;
  privateKeyWif: string;
}) {
  const [showKey, setShowKey] = useState(false);

  const maskedKey = "••••••••••••••••••••••••••••••••••••••••••••••••••••";

  return (
    <div className="space-y-4">
      {/* Address Section */}
      <div className="space-y-1.5">
        <span className="text-xs text-muted-foreground">Address</span>
        <div className="flex items-center gap-2">
          <code className="flex-1 break-all rounded bg-muted px-3 py-2 text-xs font-mono">
            {address}
          </code>
          <CopyButton value={address} label="Copy" size="sm" />
        </div>
      </div>

      <hr className="border-border" />

      {/* Private Key Section */}
      <div className="space-y-1.5">
        <span className="text-xs text-muted-foreground">
          Private Key (WIF)
        </span>
        <div className="flex items-center gap-2">
          <code
            className="flex-1 break-all rounded bg-muted px-3 py-2 text-xs font-mono"
            aria-label={showKey ? "Private key revealed" : "Private key hidden"}
          >
            {showKey ? privateKeyWif : maskedKey}
          </code>
          {showKey && (
            <CopyButton value={privateKeyWif} label="Copy" size="sm" />
          )}
        </div>

        {/* Show/Hide toggle */}
        <Button
          variant="outline"
          size="sm"
          onClick={() => setShowKey(!showKey)}
          className="mt-1"
          aria-label={showKey ? "Hide private key" : "Show private key"}
        >
          {showKey ? (
            <EyeOff className="mr-1.5 size-4" />
          ) : (
            <Eye className="mr-1.5 size-4" />
          )}
          {showKey ? "Hide Key" : "Show Key"}
        </Button>
      </div>

      {/* Security Warning */}
      <div className="flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2.5">
        <ShieldAlert className="mt-0.5 size-4 shrink-0 text-destructive" />
        <p className="text-xs text-destructive" role="alert">
          Keep your private key secure. Never share it with anyone.
        </p>
      </div>
    </div>
  );
}
