import { locateEdit, mapPosition } from './dictationLedger';

/**
 * Placing a transcript in a Biorouter composer (DD-63, "Inserting text" in
 * `docs/dictation/surfaces.md`).
 *
 * One function decides where the text goes and how, for every composer: the
 * undoable path when the field still has focus, the composer's own state
 * otherwise, and the stored draft when the composer is gone. Nothing here sends
 * anything; a dictated message is never sent by itself (DD-70).
 */

/** The field's selection when recording started, moved through later edits. */
export interface Anchor {
  start: number;
  end: number;
}

const OPENERS = new Set(['(', '[', '{', '"', "'", '“', '‘', '«', '<']);
const STARTS_WITH_WORD = /^[\p{L}\p{N}]/u;

/**
 * The transcript as it goes into `value` at `anchor`, and the value after it.
 *
 * - Runs of whitespace collapse to one space, and the ends are trimmed.
 * - Leading slashes are dropped, so a spoken "slash bug" never becomes a
 *   command.
 * - A space goes before the text unless it starts the field or follows
 *   whitespace or an opening bracket or quote.
 * - A space goes after it only in the middle of text, before a word.
 * - Letter case is never changed.
 *
 * `caret` is where the caret belongs afterwards: right after the inserted text.
 * An empty transcript inserts nothing and leaves `value` as it was.
 */
export function spliceDictation(
  value: string,
  anchor: Anchor,
  text: string
): { value: string; inserted: string; caret: number } {
  const start = clamp(Math.min(anchor.start, anchor.end), 0, value.length);
  const end = clamp(Math.max(anchor.start, anchor.end), start, value.length);
  const words = text
    .replace(/\s+/g, ' ')
    .trim()
    .replace(/^\/+\s*/, '');
  if (!words) return { value, inserted: '', caret: start };

  const before = value.slice(0, start);
  const after = value.slice(end);
  const last = before.slice(-1);
  const lead = before.length === 0 || /\s/.test(last) || OPENERS.has(last) ? '' : ' ';
  const trail = after.length > 0 && STARTS_WITH_WORD.test(after) ? ' ' : '';
  const inserted = `${lead}${words}${trail}`;
  return {
    value: `${before}${inserted}${after}`,
    inserted,
    caret: start + inserted.length,
  };
}

function clamp(n: number, low: number, high: number): number {
  return Math.min(high, Math.max(low, Number.isFinite(n) ? Math.trunc(n) : low));
}

/**
 * Move a recording's anchor through one edit the person made while it
 * recorded. An untouched selection keeps covering the same text; a selection
 * the person typed into collapses to its mapped end; a caret moves with text
 * typed at it, so speech lands after what was typed during the recording.
 */
export function mapAnchor(
  anchor: Anchor,
  before: string,
  after: string,
  caretAfter?: number
): Anchor {
  const edit = locateEdit(before, after, caretAfter);
  if (!edit) return anchor;
  const { start, end } = anchor;
  if (start === end) {
    const at = mapPosition(start, edit, 'after');
    return { start: at, end: at };
  }
  const typedInto =
    (edit.start < end && edit.endBefore > start) ||
    (edit.start === edit.endBefore && edit.start > start && edit.start < end);
  if (typedInto) {
    const at = mapPosition(end, edit, 'after');
    return { start: at, end: at };
  }
  return { start: mapPosition(start, edit, 'after'), end: mapPosition(end, edit, 'before') };
}

/**
 * Whether the field may take an `execCommand` insert: it has focus, or focus is
 * on the page body or on another element of the same composer card (the
 * microphone itself). `execCommand` writes into whatever has focus, so anything
 * else, another field or another pane, sends the text down the state path.
 */
export function composerOwnsFocus(field: HTMLTextAreaElement): boolean {
  const active = field.ownerDocument.activeElement;
  if (!active || active === field || active === field.ownerDocument.body) return true;
  const card = field.closest('.biorouter-composer-card');
  return !!card && card.contains(active);
}

export type InsertPath = 'undoable' | 'state' | 'draft';

/**
 * Put a transcript into a composer's field at `anchor`.
 *
 * - `'undoable'`: the field owns focus and no IME composition runs, so the text
 *   goes in with `setSelectionRange` then `execCommand('insertText')`. That is
 *   one undo step and fires React's `onChange` (measured on Electron 39.8.10).
 *   The result is read back; if the text is not where it should be, the state
 *   path below writes the intended value over whatever happened.
 * - `'state'`: focus is elsewhere, a composition is running, `execCommand`
 *   refused, or the read back failed. `setByState` receives the full new value
 *   and the caret. This path is not undoable.
 * - `'draft'`: there is no field any more (the composer unmounted). Nothing is
 *   written here; the caller splices the text into the stored draft with
 *   `insertIntoComposerDraft`.
 *
 * A caller that knows a composition is running should wait for its
 * `compositionend` before calling, so the text can still take the undoable path.
 */
export function insertDictatedText(
  field: HTMLTextAreaElement | null,
  anchor: Anchor,
  text: string,
  setByState: (value: string, caret: number) => void,
  options: { composing?: boolean } = {}
): InsertPath {
  if (!field || !field.isConnected) return 'draft';
  const planned = spliceDictation(field.value, anchor, text);
  if (!planned.inserted) return 'state';

  if (!options.composing && !field.disabled && !field.readOnly && composerOwnsFocus(field)) {
    const start = Math.min(planned.caret - planned.inserted.length, field.value.length);
    const replaceEnd = clamp(Math.max(anchor.start, anchor.end), start, field.value.length);
    try {
      if (field.ownerDocument.activeElement !== field) field.focus({ preventScroll: true });
      field.setSelectionRange(start, replaceEnd);
      const ok = field.ownerDocument.execCommand('insertText', false, planned.inserted);
      if (ok && field.value.slice(start, start + planned.inserted.length) === planned.inserted) {
        return 'undoable';
      }
    } catch {
      // Fall through to the state path with the value planned before the attempt.
    }
  }
  setByState(planned.value, planned.caret);
  return 'state';
}
