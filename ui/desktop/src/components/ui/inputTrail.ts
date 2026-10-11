/**
 * What the window last saw: whether the most recent input was a key or a pointer press, and which
 * control outside any menu last held focus.
 *
 * Both are capture listeners installed once when this module loads, and both answer a question a
 * primitive cannot answer from the DOM at the moment it needs to ask.
 *
 * - **The modality** is the question the browser asks itself for `:focus-visible`. A focus that a
 *   script moves after a POINTER press can still match `:focus-visible` (Chromium decides from the
 *   element that held focus before, and a menu item the pointer just picked counts), so a primitive
 *   that must not draw a keyboard ring after a mouse click, or must give focus back only to a
 *   keyboard user, asks here instead of guessing from the focus target.
 * - **The last control outside a menu** is the opener of a menu that names none. A DropdownMenu is
 *   labelled by its trigger; a ContextMenu is labelled by nothing, and by the time a dialog that a
 *   menu item opened closes, the item and the menu are gone. The control that held focus when the
 *   menu took it (the row a Shift+F10 was pressed on) is the one place left to go back to.
 *
 * - **Where focus came from** when it last moved (the `focusin`'s `relatedTarget`). A dialog whose
 *   field takes React's `autoFocus` has focus inside before Radix's focus scope mounts, so Radix
 *   never fires `onOpenAutoFocus` and the dialog cannot read its opener from the active element;
 *   the control focus left to reach that field is the opener (WS-SKILLS-2, V3-R3-1).
 *
 * `chats/menuFocusReturn.ts` re-exports `lastInputWasKey` from here, so the sidebar's menus and the
 * dialogs they open read one tracker.
 */
let lastInputWasKeyboard = false;
let lastOutsideMenu: HTMLElement | null = null;
let lastMove: { to: HTMLElement; from: HTMLElement | null } | null = null;

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
  window.addEventListener(
    'focusin',
    (event) => {
      const target = event.target;
      if (!(target instanceof HTMLElement) || target === document.body) return;
      const from = (event as FocusEvent).relatedTarget;
      lastMove = {
        to: target,
        from: from instanceof HTMLElement && from !== document.body ? from : null,
      };
      if (target.closest('[role="menu"]')) return;
      lastOutsideMenu = target;
    },
    true
  );
}

/** Was the most recent input a key press? A page nobody has touched reads as the pointer. */
export function lastInputWasKey(): boolean {
  return lastInputWasKeyboard;
}

/**
 * The element focus left when it last arrived on `element`, or null when the last focus move was
 * not onto `element` or came from nowhere (`<body>`, another window). It may have left the
 * document since (a menu item whose menu closed as it acted): `resolveDialogOpener` climbs from a
 * detached item to its menu's trigger.
 */
export function focusArrivedFrom(element: Element | null): HTMLElement | null {
  if (!element || !lastMove || lastMove.to !== element) return null;
  return lastMove.from;
}

/** The control outside any menu that held focus last, while it is still in the document. */
export function lastFocusOutsideMenu(): HTMLElement | null {
  return lastOutsideMenu && lastOutsideMenu.isConnected ? lastOutsideMenu : null;
}
