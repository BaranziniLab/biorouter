import type { RefObject } from 'react';

/**
 * An `onOpenAutoFocus` for a dialog whose first stop is a text field that is not its first
 * tabbable (a search box, a URL, an editor, each after a help glyph or a close button).
 *
 * **Never React's `autoFocus` for this.** `useReturnFocusToOpener` (ui/dialog.tsx) records the
 * control that opened the dialog when Radix announces `onOpenAutoFocus`. Radix's focus scope
 * announces it only if focus is NOT already inside the content when it mounts, and an `autoFocus`
 * field takes focus as the content commits, a step earlier. So the announcement never came, no
 * opener was recorded, and after Escape focus fell to `<body>` for Browse skills, Add skill and
 * Write a skill (V3-R3-1). Here the field is left alone until Radix asks, the opener is recorded
 * first, and this handler then places focus on the field itself.
 *
 * With no field mounted the event is left alone and Radix places focus as it would.
 */
export function focusFieldOnOpen(field: RefObject<HTMLElement | null>) {
  return (event: Event) => {
    const target = field.current;
    if (!target) return;
    event.preventDefault();
    target.focus({ preventScroll: true });
  };
}
