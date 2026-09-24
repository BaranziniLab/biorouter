import { useCallback, useRef } from 'react';

/**
 * Where keyboard focus goes when a Crew dialog closes (WCAG 2.4.3; QA T-15).
 *
 * Crew dialogs are opened programmatically — `crew.openDialog(intent)` from a button, a menu item or
 * an effect — so Radix never sees a `Dialog.Trigger`, and its own close handler focuses a trigger
 * that does not exist. The element that had focus is removed with the dialog and the browser drops
 * focus to `<body>`: a keyboard or screen-reader user starts again from the top of the window.
 *
 * So the opener is recorded when the dialog OPENS, while it still has focus, and focus is put back
 * on it once the dialog is gone:
 * - A menu item stands for the trigger of its menu (a menu closes as it acts, taking the item with
 *   it). A submenu's item climbs to the root menu's trigger. A control in a popover stands for the
 *   popover's trigger the same way.
 * - A dialog opened from inside another dialog that it replaces keeps the first dialog's opener,
 *   because the control that opened the second one leaves with the first.
 * - Focus is only ever RESCUED: when something else deliberately took focus after the dialog closed
 *   (a pane that focuses its heading, a composer), it is left there.
 * - When the opener is gone too, focus lands on the first fallback that exists: the channel's
 *   heading button, then the composer, then the workspace switcher.
 */

const CHANNEL_HEADING = '.crew-app h1 button';
const COMPOSER = '.crew-app textarea[aria-label^="Message"]';
const WORKSPACE_SWITCHER = '.crew-sidebar-switcher';

/**
 * Where focus goes when a dialog's opener is gone: the channel heading, then the composer, then the
 * workspace switcher. The switcher is last, for a Crew area showing a status screen instead of a
 * channel (loading, checking, offline: `layout/MainScreen.tsx` draws neither heading nor composer),
 * where focus would otherwise stay on `<body>` (QA Q2-27).
 */
export const DIALOG_FOCUS_FALLBACKS: readonly string[] = [
  CHANNEL_HEADING,
  COMPOSER,
  WORKSPACE_SWITCHER,
];

/** Where focus goes when Sign in's opener is gone: the workspace switcher first. */
export const SIGN_IN_FOCUS_FALLBACKS: readonly string[] = [
  WORKSPACE_SWITCHER,
  CHANNEL_HEADING,
  COMPOSER,
];

/**
 * A surface that closes as it acts and stands for its trigger: a menu, and a popover (Radix renders
 * one as `role="dialog"` inside its popper wrapper, which a modal dialog never is). A dialog opened
 * from the privacy popover returned focus to the channel heading, because only menus climbed (QA
 * Q2-27).
 */
const MENU = '[role="menu"], [data-radix-popper-content-wrapper] > [role="dialog"]';
const DIALOG = '[role="dialog"], [role="alertdialog"]';

/**
 * The trigger a menu or popover belongs to: the control it `aria-controls` from, else the one
 * naming it.
 */
function menuTrigger(menu: Element): HTMLElement | null {
  if (menu.id) {
    const controller = document.querySelector(`[aria-controls="${cssEscape(menu.id)}"]`);
    if (controller instanceof HTMLElement && !menu.contains(controller)) return controller;
  }
  const labelledBy = menu.getAttribute('aria-labelledby');
  if (labelledBy) {
    const label = document.getElementById(labelledBy.split(/\s+/)[0] ?? '');
    if (label instanceof HTMLElement && !menu.contains(label)) return label;
  }
  return null;
}

function cssEscape(value: string): string {
  return typeof CSS !== 'undefined' && typeof CSS.escape === 'function'
    ? CSS.escape(value)
    : value.replace(/["\\]/g, '\\$&');
}

/**
 * The control that stands for `element` as the opener of a surface: the element itself, or — when
 * it sits in a menu — the trigger of that menu (a submenu climbs to the root menu's trigger).
 * Null for nothing, `<body>`, or a menu whose trigger cannot be found.
 */
export function openerOf(element: Element | null | undefined): HTMLElement | null {
  let current: Element | null | undefined = element;
  // A submenu nests at most a few levels; the bound only stops a malformed tree from looping.
  for (let depth = 0; depth < 8; depth += 1) {
    if (!(current instanceof HTMLElement)) return null;
    if (current === document.body || current === document.documentElement) return null;
    const menu = current.closest(MENU);
    if (!menu) return current;
    current = menuTrigger(menu);
  }
  return null;
}

/** Whether `element` can take focus now: still in the document, not disabled, hidden or inert. */
export function canFocus(element: Element | null | undefined): element is HTMLElement {
  return (
    element instanceof HTMLElement &&
    element.isConnected &&
    !element.hasAttribute('disabled') &&
    element.closest('[aria-hidden="true"], [inert], [hidden]') === null
  );
}

/**
 * Whether focus has been lost: it sits on nothing, on `<body>`, on a removed node, or on a dialog's
 * own frame (where a focus trap parks it when the control that had focus disappears under it).
 */
export function focusIsLost(): boolean {
  if (typeof document === 'undefined') return false;
  const active = document.activeElement;
  if (!active || active === document.body || active === document.documentElement) return true;
  if (!active.isConnected) return true;
  return active.matches(DIALOG);
}

/**
 * The opener to record for a surface that is opening now. When focus is inside a dialog that the
 * new surface replaces, the earlier opener (if it is still there) is kept, because the control
 * inside the dialog leaves with it.
 */
export function nextOpener(previous: HTMLElement | null): HTMLElement | null {
  if (typeof document === 'undefined') return previous;
  const active = document.activeElement;
  const insideDialog = active instanceof Element && active.closest(DIALOG) !== null;
  if (insideDialog && previous && previous.isConnected) return previous;
  return openerOf(active);
}

/**
 * Put focus back on `opener`, else on the first fallback that can take it — only when focus has
 * been lost. Returns whether it moved focus.
 */
export function restoreFocus(
  opener: HTMLElement | null,
  fallbacks: readonly string[] = DIALOG_FOCUS_FALLBACKS
): boolean {
  if (typeof document === 'undefined' || !focusIsLost()) return false;
  const targets: (Element | null)[] = [opener];
  for (const selector of fallbacks) targets.push(document.querySelector(selector));
  const target = targets.find(canFocus);
  if (!target) return false;
  target.focus();
  return document.activeElement === target;
}

/**
 * `restoreFocus` once the closing surface has left the document: on the next frame, after React
 * has unmounted it and Radix's own close handling has run.
 */
export function restoreFocusSoon(
  opener: HTMLElement | null,
  fallbacks: readonly string[] = DIALOG_FOCUS_FALLBACKS
): void {
  if (typeof window === 'undefined') return;
  const run = () => void restoreFocus(opener, fallbacks);
  if (typeof window.requestAnimationFrame === 'function') window.requestAnimationFrame(run);
  else window.setTimeout(run, 0);
}

/**
 * For a surface a component shows and hides itself (a confirmation over a dialog): `remember()`
 * when it opens — from the event that opens it, while the control still has focus — and
 * `restore()` when it closes. The restore uses no fallbacks: when the opener is gone, whatever
 * closed the outer dialog decides where focus goes.
 */
export function useFocusReturn(): { remember(): void; restore(): void } {
  const opener = useRef<HTMLElement | null>(null);
  const remember = useCallback(() => {
    opener.current = typeof document === 'undefined' ? null : openerOf(document.activeElement);
  }, []);
  const restore = useCallback(() => {
    const target = opener.current;
    opener.current = null;
    restoreFocusSoon(target, []);
  }, []);
  return { remember, restore };
}
