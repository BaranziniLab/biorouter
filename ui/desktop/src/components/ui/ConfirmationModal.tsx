import * as React from 'react';
import { ModalShell } from '../ModalShell';
import { Button } from './button';
import { uiCopy } from './copy';

/**
 * A yes-or-no confirmation: a title, one message, Cancel and the action.
 *
 * It is a `ModalShell` at size `sm`, so it has the one dialog anatomy every other dialog has
 * (vision round 1, V4-10 and V1-L5): pinned at the top edge, the header rule, 16px insets and the
 * footer row. It used to be a raw `DialogContent` (425px, centred, 24px insets), so "Delete
 * chat?" from the sidebar sat in the middle of the window while History's "Rename chat" from the
 * same row menu sat at the top.
 *
 * The message is the dialog's description (`describedBy`), in body text, not a 12px subtitle.
 * While `isSubmitting` the dialog must be answered (`purpose="required"`): no ×, no Escape, no
 * backdrop, so a dismissal cannot orphan the running action.
 */
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
  const messageId = React.useId();
  return (
    <ModalShell
      open={isOpen}
      onOpenChange={(open) => {
        if (!open && !isSubmitting) onCancel();
      }}
      size="sm"
      purpose={isSubmitting ? 'required' : 'info'}
      title={title}
      describedBy={messageId}
      bodyClassName="flex flex-col gap-3"
      footer={
        <>
          <Button variant="secondary" onClick={onCancel} disabled={isSubmitting}>
            {cancelLabel}
          </Button>
          <Button variant={confirmVariant} onClick={onConfirm} disabled={isSubmitting}>
            {isSubmitting ? uiCopy.processing : confirmLabel}
          </Button>
        </>
      }
    >
      <p id={messageId} className="text-body text-text-muted">
        {message}
      </p>
      {children}
    </ModalShell>
  );
}
