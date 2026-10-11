import React, { useEffect, useId, useRef, useState } from 'react';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from './dialog';
import { Button } from './button';
import { Input } from './input';
import { lastInputWasKey } from './inputTrail';

/**
 * Issue #56. The one typed-confirmation primitive in the app, shared by every
 * irreversible privacy action so their phrases and their keyboard behaviour
 * cannot drift apart (§12.4's declassification and §14.6's `DISABLE
 * PROTECTION`).
 *
 * There was no precedent to copy: a repo-wide search for a typed confirmation
 * (`toUpperCase() ===`, `=== 'DELETE'`, `confirmationText`, `confirmPhrase`)
 * returned zero product hits before this. The nearest relative is
 * `ResetPanel`'s bespoke dialog, which previews exactly what will be destroyed
 * before confirming — a shape this keeps, via {@link children}.
 *
 * Three behaviours are the reason it is a component rather than a pattern:
 *
 *  1. **The phrase gates the button.** A field that is rendered but never
 *     consulted is the likely wrong implementation when there is nothing to
 *     copy, and it looks identical in a screenshot.
 *  2. **Nothing with an action holds initial focus by accident.** Radix's
 *     default `onOpenAutoFocus` takes the first tabbable node, which here is the
 *     phrase field or a control in the preview, chosen by layout rather than on
 *     purpose. This overrides it. A dialog opened with a KEY puts focus on
 *     Cancel, ringed, because there the ring is the point. Any other open (a
 *     click, a pointer pick from a menu, a dialog the app raised itself) rests
 *     on the dialog box: after a pointer pick in a menu Chromium rings every
 *     focus a script moves, so Cancel wore the keyboard ring after a mouse
 *     click (vision round 6, V3-R6-2). The box draws no ring
 *     (`.biorouter-modal-surface:focus`), a screen reader reads the title and
 *     the message from it, and the next Tab reaches the field or Cancel, both
 *     before the destructive button in the DOM. Either way the destructive
 *     button is never the first stop.
 *  3. **No key confirms.** Nothing here is a `<form>`, and Return inside the
 *     field is explicitly swallowed, so the muscle-memory "type, Enter" that
 *     ends every other dialog in this app cannot end this one.
 */
export interface DangerousConfirmDialogProps {
  open: boolean;
  title: string;
  /** One sentence saying what will happen and that it is not reversible. */
  description?: React.ReactNode;
  /**
   * The exact string the user must retype. Omit for a dangerous action that is
   * confirmed by a single click — the component still refuses to confirm on a
   * keypress, which is most of what it is for.
   */
  phrase?: string;
  /** The field's visible label. Must name the phrase, not merely ask for "the text". */
  fieldLabel?: string;
  confirmLabel: string;
  cancelLabel?: string;
  /** Rendered between the description and the field: the ResetPanel-style preview. */
  children?: React.ReactNode;
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
  /**
   * Hears every edit of the typed field. For a caller that must check the phrase more strictly
   * than this component's trimmed, case-folded comparison — a Unix username is case-sensitive —
   * at confirm time, on top of the gate here, never instead of it.
   */
  onPhraseChange?: (typed: string) => void;
}

/**
 * Trim and case-fold. The phrase is a "are you certain, and are you looking at
 * the right thing" check rather than a secret, so a trailing space costs the
 * user a retry and buys nothing.
 */
const normalize = (value: string) => value.trim().toLowerCase();

export function DangerousConfirmDialog({
  open,
  title,
  description,
  phrase,
  fieldLabel,
  confirmLabel,
  cancelLabel = 'Cancel',
  children,
  busy = false,
  onConfirm,
  onCancel,
  onPhraseChange,
}: DangerousConfirmDialogProps) {
  const fieldId = useId();
  const cancelRef = useRef<HTMLButtonElement>(null);
  const boxRef = useRef<HTMLDivElement>(null);
  const [typed, setTyped] = useState('');

  // A dialog reopened for a different subject must not arrive pre-satisfied by
  // the previous subject's phrase.
  useEffect(() => {
    if (!open) setTyped('');
  }, [open]);
  useEffect(() => {
    setTyped('');
  }, [phrase]);

  const satisfied = phrase === undefined || normalize(typed) === normalize(phrase);

  return (
    <Dialog open={open} onOpenChange={(next) => !next && !busy && onCancel()}>
      <DialogContent
        dismissible={!busy}
        {...(description ? {} : { 'aria-describedby': undefined })}
        ref={boxRef}
        className="sm:max-w-[480px]"
        // (2) above. A key open lands on Cancel; any other open rests on the
        // box, the same quiet open the shared dialog gives every pointer open.
        // This dialog places focus itself, so `dialog.tsx` leaves it alone.
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          if (lastInputWasKey()) cancelRef.current?.focus();
          else boxRef.current?.focus({ preventScroll: true });
        }}
      >
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          {description ? <DialogDescription>{description}</DialogDescription> : null}
        </DialogHeader>

        {children}

        {phrase !== undefined && (
          <div className="flex flex-col gap-1.5">
            <label htmlFor={fieldId} className="text-label text-text-default">
              {fieldLabel}
            </label>
            <div className="flex min-w-0 items-center gap-2">
              <Input
                id={fieldId}
                value={typed}
                autoComplete="off"
                spellCheck={false}
                disabled={busy}
                onChange={(event) => {
                  setTyped(event.target.value);
                  onPhraseChange?.(event.target.value);
                }}
                // (3) above. With no surrounding `<form>` this is already inert,
                // and it is written down anyway: the day someone wraps this in a
                // form for the layout, the destructive action must not acquire a
                // keyboard shortcut as a side effect.
                onKeyDown={(event) => {
                  if (event.key === 'Enter') event.preventDefault();
                }}
                className="min-w-0 flex-1 font-mono"
              />
              {/* Beside the field, so the user is comparing against the thing
                  itself rather than remembering it from the prose above. */}
              <code className="shrink-0 rounded-inner bg-background-muted px-2 py-1 font-mono text-code text-text-muted select-all">
                {phrase}
              </code>
            </div>
          </div>
        )}

        <DialogFooter className="pt-2">
          <Button ref={cancelRef} type="button" variant="secondary" onClick={onCancel}>
            {cancelLabel}
          </Button>
          <Button
            type="button"
            variant="destructive"
            disabled={!satisfied || busy}
            onClick={onConfirm}
          >
            {confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
