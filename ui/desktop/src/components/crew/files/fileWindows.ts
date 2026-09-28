/**
 * The native file windows this renderer has open: the secure picker, a Save sheet, the share
 * confirmation a drop or paste opened. The main process shows one at a time and refuses a second
 * with "Finish the open … first." (FILES2-N5): a card that said so kept saying it after that window
 * had closed, and after later saves and shares had worked, until the card was used again. A card
 * showing such a note hears here when every window has closed, and lets it go.
 *
 * Module scope, since the windows are opened from many surfaces (every attachment card, the Files
 * tab, the composer) and one of them names another's. Counts only; nothing about the file.
 */
let open = 0;
const listeners = new Set<() => void>();

/** A native file window is opening. Returns what closes it, which is safe to call twice. */
export function openFileWindow(): () => void {
  open += 1;
  let closed = false;
  return () => {
    if (closed) return;
    closed = true;
    open = Math.max(0, open - 1);
    if (open === 0) for (const listener of [...listeners]) listener();
  };
}

/** Run `task`, which shows a native file window, counting the window open until it settles. */
export async function withFileWindow<T>(task: () => Promise<T>): Promise<T> {
  const close = openFileWindow();
  try {
    return await task();
  } finally {
    close();
  }
}

/** Call `listener` each time the last open file window closes. Returns the unsubscribe. */
export function subscribeFileWindowsClosed(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** The main process's "Finish the open … first." for a window already open: this note's words. */
export function isFileWindowBusyNote(text: string): boolean {
  return /^Finish\b.*\bfirst\.$/.test(text.trim());
}
