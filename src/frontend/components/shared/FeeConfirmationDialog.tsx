import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";

export type FeeConfirmationResult =
  | { status: "confirmed"; overrideFee: number }
  | { status: "canceled" };

interface FeeConfirmationDialogProps {
  /** Whether the dialog is open */
  open: boolean;
  /** Called when the dialog should close */
  onOpenChange: (open: boolean) => void;
  /** The initially estimated fee (in duffs or credits) */
  estimatedFee: number;
  /** The fee required by the network (in duffs or credits) */
  requiredFee: number;
  /** Unit label: "duffs" for Core, "credits" for Platform */
  unit?: string;
  /** Called with the user's decision */
  onResult?: (result: FeeConfirmationResult) => void;
}

function formatDash(duffs: number): string {
  return (duffs / 100_000_000).toFixed(8);
}

/**
 * Fee confirmation dialog for when the network requires a higher fee.
 *
 * Matches egui FeeConfirmationDialog behavior:
 * - Shows estimated fee, required fee (highlighted), and additional cost
 * - Confirm & Send or Cancel
 * - Returns override fee on confirmation
 */
export function FeeConfirmationDialog({
  open,
  onOpenChange,
  estimatedFee,
  requiredFee,
  unit = "duffs",
  onResult,
}: FeeConfirmationDialogProps) {
  const additionalCost = requiredFee - estimatedFee;

  const handleConfirm = () => {
    onResult?.({ status: "confirmed", overrideFee: requiredFee });
    onOpenChange(false);
  };

  const handleCancel = () => {
    onResult?.({ status: "canceled" });
    onOpenChange(false);
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(value) => {
        if (!value) handleCancel();
      }}
    >
      <DialogContent showCloseButton={false} className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Fee Confirmation Required</DialogTitle>
          <DialogDescription>
            The network requires a higher fee than estimated.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-2 rounded-md bg-muted/50 p-4 text-sm">
          <div className="flex justify-between">
            <span className="text-muted-foreground">Estimated fee</span>
            <span>
              {estimatedFee.toLocaleString()} {unit}{" "}
              <span className="text-muted-foreground">
                ({formatDash(estimatedFee)} DASH)
              </span>
            </span>
          </div>
          <div className="flex justify-between font-medium text-warning">
            <span>Required fee</span>
            <span>
              {requiredFee.toLocaleString()} {unit}{" "}
              <span className="text-warning/70">
                ({formatDash(requiredFee)} DASH)
              </span>
            </span>
          </div>
          <div className="border-t border-border pt-2 flex justify-between">
            <span className="text-muted-foreground">Additional cost</span>
            <span>
              +{additionalCost.toLocaleString()} {unit}{" "}
              <span className="text-muted-foreground">
                ({formatDash(additionalCost)} DASH)
              </span>
            </span>
          </div>
        </div>

        <p className="text-sm text-muted-foreground">
          Would you like to proceed with the higher fee?
        </p>

        <DialogFooter>
          <Button variant="outline" onClick={handleCancel}>
            Cancel
          </Button>
          <Button onClick={handleConfirm}>Confirm &amp; Send</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
