import * as React from 'react';
import { ModalShell } from '../ModalShell';
import { Button } from './button';
import { uiCopy } from './copy';
import { Disclosure } from './disclosure';

/** The words on a confirmation's detail disclosure, when the caller names none. */
export const CONFIRMATION_DETAILS_LABEL = 'Details';

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
 * It is ONE sentence: what pressing the button will do. The consequence of an irreversible action
 * always stays there. Anything a person may want after it (the figures behind a warning, what a
 * term means on this machine) goes in `details`, behind a closed disclosure under the message
 * (owner, 2026-11-13: one line up front, the rest on a click).
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
  details,
  detailsLabel = CONFIRMATION_DETAILS_LABEL,
  children,
  onOpenAutoFocus,
  onCloseAutoFocus,
}: {
  isOpen: boolean;
  title: string;
  /** One sentence: what the action will do. An irreversible action's consequence stays here. */
  message: string;
  onConfirm: () => void;
  onCancel: () => void;
  confirmLabel?: string;
  cancelLabel?: string;
  isSubmitting?: boolean; // To handle debounce state
  confirmVariant?:
    | 'default'
    | 'destructive'
    | 'critical'
    | 'outline'
    | 'secondary'
    | 'ghost'
    | 'link';
  /**
   * What a person may want after the message, folded behind a closed disclosure under it: the
   * reason, the figures, a second sentence. A string is set as muted body text.
   */
  details?: React.ReactNode;
  /** The disclosure's words: "Details", or a better name for this detail ("Why?"). */
  detailsLabel?: string;
  /**
   * Rendered between the message and the buttons — for the one line a caller must show in place,
   * such as the refusal of the action being confirmed, so it lands in the dialog rather than
   * behind it.
   */
  children?: React.ReactNode;
  /**
   * Radix's open-focus and close-focus hooks, passed through to the shell. A confirmation opened
   * from a menu item returns focus to the menu's trigger by itself (`useReturnFocusToOpener`); a
   * caller whose opener is neither the trigger nor the focused control (a ContextMenu row that
   * re-rendered, a confirmation chained from another dialog) says where focus goes instead:
   * `event.preventDefault()` and focus it yourself.
   */
  onOpenAutoFocus?: (event: Event) => void;
  onCloseAutoFocus?: (event: Event) => void;
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
      onOpenAutoFocus={onOpenAutoFocus}
      onCloseAutoFocus={onCloseAutoFocus}
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
      {details !== undefined && details !== null && details !== '' ? (
        <Disclosure label={detailsLabel}>
          {typeof details === 'string' ? (
            <p className="text-body text-text-muted">{details}</p>
          ) : (
            details
          )}
        </Disclosure>
      ) : null}
      {children}
    </ModalShell>
  );
}
