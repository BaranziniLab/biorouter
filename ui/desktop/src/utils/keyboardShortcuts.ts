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

/**
 * The same chord in `aria-keyshortcuts` syntax, so a control that the chord
 * also reaches can announce it to a screen reader without a visible hint.
 */
export function getSteerAriaKeyShortcuts(): string {
  return isMac() ? 'Meta+Enter' : 'Control+Enter';
}

/**
 * The dictation chord (DD-62), as a glyph: `⌘⇧D` on a Mac, `Ctrl+Shift+D`
 * elsewhere. A tap starts or stops dictation; a hold of 500 ms or more talks
 * while it is held. Ctrl+M was not used because it minimizes the window.
 */
export function getDictationShortcutText(): string {
  return isMac() ? '⌘⇧D' : 'Ctrl+Shift+D';
}

/** The same chord in `aria-keyshortcuts` syntax, for the microphone button. */
export function getDictationAriaKeyShortcuts(): string {
  return isMac() ? 'Meta+Shift+D' : 'Control+Shift+D';
}

/**
 * Whether a keydown is the dictation chord: matched on `code`, so the key is D
 * on every layout, with exactly the platform's modifier and Shift. A repeat
 * while the key is held, and a key typed into an IME composition, are not.
 */
export function isDictationChord(event: {
  code: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  repeat?: boolean;
  isComposing?: boolean;
}): boolean {
  if (event.code !== 'KeyD' || !event.shiftKey || event.altKey) return false;
  if (event.repeat || event.isComposing) return false;
  return isMac() ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
}
