import type React from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";

export type ConfirmationStatus = "confirmed" | "canceled";

interface ConfirmationDialogProps {
  /** Whether the dialog is open */
  open: boolean;
  /** Called when the dialog should close */
  onOpenChange: (open: boolean) => void;
  /** Dialog title */
  title: string;
  /** Dialog message / description */
  message: React.ReactNode;
  /** Confirm button label (default: "Confirm"). Pass null to hide. */
  confirmText?: string | null;
  /** Cancel button label (default: "Cancel"). Pass null to hide. */
  cancelText?: string | null;
  /** Enable destructive (red) styling for confirm button */
  danger?: boolean;
  /** Called with the user's decision */
  onResult?: (status: ConfirmationStatus) => void;
}

/**
 * Reusable confirmation dialog.
 *
 * Matches egui ConfirmationDialog behavior:
 * - Centered modal with overlay backdrop
 * - Confirm + Cancel buttons (either can be hidden)
 * - Danger mode for destructive actions (red confirm button)
 * - Escape key / overlay click dismisses (cancels)
 */
export function ConfirmationDialog({
  open,
  onOpenChange,
  title,
  message,
  confirmText = "Confirm",
  cancelText = "Cancel",
  danger = false,
  onResult,
}: ConfirmationDialogProps) {
  const handleConfirm = () => {
    onResult?.("confirmed");
    onOpenChange(false);
  };

  const handleCancel = () => {
    onResult?.("canceled");
    onOpenChange(false);
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(value) => {
        if (!value) {
          handleCancel();
        }
      }}
    >
      <DialogContent showCloseButton={false}>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{message}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          {cancelText !== null && (
            <Button variant="outline" onClick={handleCancel}>
              {cancelText}
            </Button>
          )}
          {confirmText !== null && (
            <Button
              variant={danger ? "destructive" : "default"}
              onClick={handleConfirm}
            >
              {confirmText}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
