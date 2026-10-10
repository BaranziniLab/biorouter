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
 * follows a key press does. The modality is the last thing the window saw
 * (capture listeners, installed once when this module loads), which is the same
 * question the browser asks for `:focus-visible` on a click.
 */
let lastInputWasKeyboard = false;

if (typeof window !== 'undefined') {
  window.addEventListener(
    'keydown',
    () => {
      lastInputWasKeyboard = true;
    },
    true
  );
  window.addEventListener(
    'pointerdown',
    () => {
      lastInputWasKeyboard = false;
    },
    true
  );
}

/** Was the most recent input a key press? Exported for tests. */
export function lastInputWasKey(): boolean {
  return lastInputWasKeyboard;
}

/**
 * An `onCloseAutoFocus` handler for a menu: keeps Radix from returning focus to
 * the trigger when the menu was closed with the pointer.
 */
export function keepFocusOffAfterPointerClose(event: Event): void {
  if (!lastInputWasKeyboard) event.preventDefault();
}
