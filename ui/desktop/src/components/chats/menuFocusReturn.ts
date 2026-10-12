/**
 * Where focus goes when a row's menu closes.
 *
 * Radix hands focus back to the control that opened the menu. After a KEYBOARD
 * close (Escape, Enter on an item) that is exactly right. After a POINTER choice
 * it is wrong in two visible ways, both measured in Chromium: the script-focused
 * trigger matches `:focus-visible`, so a ring appears on a button the person
 * clicked with a mouse, and `:focus-within` keeps a row's hover-revealed actions
 * on screen after the pointer has left the row.
 *
 * So a close that follows a pointer press does not return focus; one that
 * follows a key press does. The modality is the last thing the window saw,
 * which is the same question the browser asks for `:focus-visible` on a click.
 * It is read from the app's one input tracker, `ui/inputTrail.ts` (capture
 * listeners installed once when that module loads), which the dialogs a menu
 * item opens ask too, so a menu and its dialog can never disagree about how the
 * person got there.
 *
 * A dialog a menu item opened has the same question when it closes, and the
 * same answer: see `returnFocusAfterKeyClose`.
 */
import { lastInputWasKey } from '../ui/inputTrail';

/** Was the most recent input a key press? Re-exported for tests and callers. */
export { lastInputWasKey };

/**
 * An `onCloseAutoFocus` handler for a menu: keeps Radix from returning focus to
 * the trigger when the menu was closed with the pointer.
 */
export function keepFocusOffAfterPointerClose(event: Event): void {
  if (!lastInputWasKey()) event.preventDefault();
}

/**
 * An `onCloseAutoFocus` handler for a dialog that a row's menu opened (Delete chat…): a dialog
 * closed with a KEY (Escape, or Enter or Space on Cancel) gives focus to `target`, the control the
 * menu was opened from (V1-R8-1).
 *
 * The shared dialog layer (`ui/dialog.tsx`) gives focus back after a key close to a trigger the
 * menu names, but not to a ContextMenu's remembered focus after a pointer pick, which may be a
 * control the person was not on. So a Delete picked with the pointer from a row's right-click menu
 * and then dismissed with Escape would leave focus on `<body>`, and the next Tab would start again
 * at the top of the window. The menu item that held focus is gone by then, but the row still knows
 * which control opened its menu.
 *
 * A dialog closed with the pointer leaves focus alone, for the reason above: a focused row keeps
 * its hover-revealed ⋯ on screen (`:focus-within`) after the pointer has left it. The browser
 * decides the ring: after a key press the returned focus is keyboard focus. `preventScroll`: the row
 * is still where it was when its menu opened.
 *
 * The caller finds `target` when the dialog closes, so a row that re-rendered meanwhile is found
 * again. `null` (a deleted chat, a row no longer drawn) leaves the shared rule to decide. It runs
 * before that rule, which stands aside once this one has placed focus.
 */
export function returnFocusAfterKeyClose(event: Event, target: HTMLElement | null): void {
  if (event.defaultPrevented || !lastInputWasKey() || !target?.isConnected) return;
  event.preventDefault();
  target.focus({ preventScroll: true });
}
