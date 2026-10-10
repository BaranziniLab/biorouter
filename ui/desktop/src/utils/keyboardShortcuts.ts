function isMac(): boolean {
  return window.electron?.platform === 'darwin';
}

export function getSearchShortcutText(): string {
  return isMac() ? '⌘F' : 'Ctrl+F';
}

/**
 * The BR-61 steer chord, as a glyph: `⌘↵` on a Mac, `Ctrl+↵` elsewhere.
 *
 * The binding has existed since BR-61 (`ChatInput`'s Enter handler: Cmd/Ctrl+Enter
 * while a turn is running hands the composer's text to that turn instead of
 * queueing it) but nothing in the interface ever said so, so it was reachable
 * only by someone who had read the source. Callers put it in hover text.
 */
export function getSteerShortcutText(): string {
  return isMac() ? '⌘↵' : 'Ctrl+↵';
}
