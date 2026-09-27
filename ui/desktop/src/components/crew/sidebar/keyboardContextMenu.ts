/**
 * Opening a context menu from the keyboard (ui-redesign-spec, Accessibility → Keyboard: "A
 * channel row's context menu opens with Shift+F10 or the Menu key").
 *
 * ⚠ The page has to do this itself. It is tempting to rely on Chromium sending a `contextmenu`
 * event for those keys, and on Linux and Windows it does. **On macOS it never does**: Blink's
 * keyboard-to-context-menu path in `WebViewImpl::HandleKeyEvent` is compiled only when the
 * platform is not Mac. Measured in this app's Electron on darwin: Shift+F10 on a focused
 * button produced a `keydown` and no `contextmenu`, while a right-click in the same harness
 * produced `contextmenu`. Relying on the browser left the menu reachable by pointer alone on the
 * platform this app ships first.
 *
 * So the row handles the key and dispatches the same `contextmenu` event a pointer would, which
 * Radix's `ContextMenuTrigger` opens from. The key's `keydown` is default-prevented, and that is
 * load-bearing on the other platforms: Chromium only runs its own dispatch when the page leaves
 * the `keydown` unhandled, so the menu opens once, not twice. (Windows sends the Menu key's
 * `contextmenu` on key-UP instead; by then the menu has taken focus, the event lands inside it
 * rather than on the row, and nothing opens a second time.)
 */

/** The modifier state of a key event, which is all `isContextMenuKey` reads. */
export interface ContextMenuKeyEvent {
  key: string;
  shiftKey: boolean;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
}

/**
 * True for the two keys that ask for the focused element's context menu: **Shift+F10** (Shift
 * and no other modifier) and an **unmodified Menu key** (`ContextMenu`). The modifier rules are
 * Chromium's own, so a combination that means something else to the system is left alone.
 */
export function isContextMenuKey(event: ContextMenuKeyEvent): boolean {
  const otherModifier = event.altKey || event.ctrlKey || event.metaKey;
  if (otherModifier) return false;
  if (event.key === 'F10') return event.shiftKey;
  if (event.key === 'ContextMenu') return !event.shiftKey;
  return false;
}

/**
 * Dispatches a `contextmenu` event on `target`, as a right-click would, anchored just below
 * `anchor` (default: `target`) at its leading edge — so a keyboard-opened menu sits under the
 * row's name instead of wherever the pointer last was.
 *
 * @returns false when a listener cancelled the event (the trigger's handler does, to stop the
 *   native menu), true otherwise.
 */
export function openContextMenuFromKeyboard(
  target: HTMLElement,
  anchor: Element = target
): boolean {
  const rowRect = target.getBoundingClientRect();
  const anchorRect = anchor.getBoundingClientRect();
  return target.dispatchEvent(
    new MouseEvent('contextmenu', {
      bubbles: true,
      cancelable: true,
      composed: true,
      clientX: anchorRect.left,
      clientY: rowRect.bottom,
    })
  );
}
