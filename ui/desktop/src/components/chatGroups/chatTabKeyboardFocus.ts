// Chat switches remount the strip inside BaseChat. Preserve arrow-key focus
// through that remount, while mouse selections still focus the composer.
let pendingTabId: string | null = null;

export function requestTabKeyboardFocus(tabId: string) {
  pendingTabId = tabId;
}

export function isTabKeyboardFocusPending(tabId: string): boolean {
  return pendingTabId === tabId;
}

export function consumeTabKeyboardFocus(tabId: string): boolean {
  if (!isTabKeyboardFocusPending(tabId)) return false;
  pendingTabId = null;
  return true;
}
