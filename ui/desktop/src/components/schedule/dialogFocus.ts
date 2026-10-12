/**
 * Where focus goes when a dialog made only of Selects opens (vision round 1, V3-20).
 *
 * Radix focuses the dialog's first tabbable, and in "Schedule" (and Edit schedule) that is the
 * first closed Select, "Every [Day]". Opened with the mouse, it then sat painted in the grey
 * focus surface like a control the person had just chosen. These dialogs have no text field to
 * take focus instead, so the right target is the dialog itself, which is what Radix does when
 * nothing inside is tabbable: a screen reader hears the dialog's name, and the first Tab lands on
 * the first Select.
 *
 * Pass it as `ModalShell`'s `onOpenAutoFocus`. `DialogContent` records the opener BEFORE calling
 * it, so Escape and Cancel still return focus to where the dialog was opened from. A dialog that
 * opens on a text field (New schedule: its Name) must not pass it.
 */
export function focusDialogOnOpen(event: Event): void {
  event.preventDefault();
  if (event.target instanceof HTMLElement) event.target.focus({ preventScroll: true });
}
