/**
 * Show a row that has just moved: scrolled into view and, if the move took keyboard focus with
 * it, focused again (V1-R6-1, ruling 16).
 *
 * A person's own rename lifts a chat to the top of the list. If the list was scrolled down to
 * where the chat used to be, the row would leave the screen the moment it moved, and a row that
 * changes date bucket is remounted by React, which drops focus to the page. Both are put right
 * here, the way the sidebar does for its own rename (`RecentChats.scrollRowIntoView`).
 *
 * Plain DOM, no React: `viewport` is the scroll area's scrolling element.
 */

/**
 * The nested run (a `data-depth="1"` row in its parent's block) that holds keyboard focus, or
 * whose `⋯` menu does, by session id. Read while History renders, from what the last commit
 * drew, so a fold can keep that run drawn (`splitNestedRuns`'s `keep`). Unlike a focus listener
 * it needs no clearing: focus the person moved anywhere else is simply not on a run.
 */
export function focusedNestedRunId(doc: Document = document): string | null {
  let element: Element | null = doc.activeElement;
  // A Radix menu renders in a portal and names its trigger, which sits in the row.
  const menu = element?.closest('[data-radix-menu-content]');
  if (menu) {
    const trigger = menu.getAttribute('aria-labelledby');
    element = trigger ? doc.getElementById(trigger) : null;
  }
  const row = element?.closest('[data-session-id][data-depth="1"]');
  return row?.getAttribute('data-session-id') ?? null;
}

/** The History row for a chat, by the id its root carries, among the rows under `root`. */
export function findHistoryRow(root: HTMLElement, sessionId: string): HTMLElement | null {
  for (const row of root.querySelectorAll<HTMLElement>('[data-session-id]')) {
    if (row.getAttribute('data-session-id') === sessionId) return row;
  }
  return null;
}

/**
 * Scroll `row` into `viewport`. A row that opens its bucket brings the bucket's label with it, so
 * the heading stays above it; any other row clears the sticky label that pins to the viewport's
 * top edge. Moves only when part of what must show is out of view.
 */
export function scrollHistoryRowIntoView(viewport: HTMLElement, row: HTMLElement): void {
  const section = row.closest('section');
  const label = section?.querySelector<HTMLElement>('.br-history-bucket') ?? null;
  const opensBucket = section?.querySelector('[data-session-id]') === row;
  const box = viewport.getBoundingClientRect();
  const top = (opensBucket && section ? section : row).getBoundingClientRect().top;
  const covered = opensBucket ? 0 : (label?.offsetHeight ?? 0);
  const { bottom } = row.getBoundingClientRect();
  if (top - covered < box.top) {
    // The first row of the list: all the way up, which also brings back the list's own padding.
    if (opensBucket && section?.previousElementSibling === null) viewport.scrollTop = 0;
    else viewport.scrollTop -= box.top + covered - top;
  } else if (bottom > box.bottom) {
    viewport.scrollTop += bottom - box.bottom;
  }
}

/**
 * Give focus back to the row when the move took it: nothing is focused (the page, or an element
 * that has left the document). Focus the person put somewhere else stays there.
 */
export function refocusHistoryRow(row: HTMLElement): void {
  const focused = document.activeElement;
  if (focused && focused !== document.body && focused.isConnected) return;
  row.querySelector<HTMLElement>('.br-history-row-open')?.focus({ preventScroll: true });
}
