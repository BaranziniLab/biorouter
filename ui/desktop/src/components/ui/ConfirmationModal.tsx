import type * as React from 'react';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from './dialog';
import { Button } from './button';

export function ConfirmationModal({
  isOpen,
  title,
  message,
  onConfirm,
  onCancel,
  confirmLabel = 'Yes',
  cancelLabel = 'No',
  isSubmitting = false,
  confirmVariant = 'default',
  children,
}: {
  isOpen: boolean;
  title: string;
  message: string;
  onConfirm: () => void;
  onCancel: () => void;
  confirmLabel?: string;
  cancelLabel?: string;
  isSubmitting?: boolean; // To handle debounce state
  confirmVariant?: 'default' | 'destructive' | 'outline' | 'secondary' | 'ghost' | 'link';
  /**
   * Rendered between the message and the buttons — for the one line a caller must show in place,
   * such as the refusal of the action being confirmed, so it lands in the dialog rather than
   * behind it.
   */
  children?: React.ReactNode;
}) {
  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && !isSubmitting && onCancel()}>
      <DialogContent dismissible={!isSubmitting} className="sm:max-w-[425px]">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{message}</DialogDescription>
        </DialogHeader>

        {children}

        <DialogFooter className="pt-2">
          <Button variant="outline" onClick={onCancel} disabled={isSubmitting}>
            {cancelLabel}
          </Button>
          <Button variant={confirmVariant} onClick={onConfirm} disabled={isSubmitting}>
            {isSubmitting ? 'Processing...' : confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
