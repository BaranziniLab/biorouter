import { locateEdit, mapPosition } from './dictationLedger';

/**
 * Placing a transcript in a Biorouter field (DD-63, "Inserting text" in
 * `docs/dictation/surfaces.md`).
 *
 * One function decides where the text goes and how, for every composer and for
 * any other field fn dictates into (a rename box, a search field, a dialog's
 * field, a `contenteditable` region): the undoable path when the field still
 * has focus, the composer's own state otherwise (or, for a field with no
 * composer, the field's own value setter), and the stored draft when the
 * composer is gone. Every path reads the result back. Nothing here sends
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
 * A field dictation can write into: a textarea, a text-like `input`, or an
 * element whose content is editable (a `contenteditable` editing host).
 */
export type DictationField = HTMLTextAreaElement | HTMLInputElement | HTMLElement;

/** The `input` types whose text has a caret dictation can put words at. Never `password`. */
const TEXT_INPUT_TYPES = new Set(['text', 'search', 'url', 'tel']);

/** Whether `el`'s content is editable, read from the attribute where a DOM has no `isContentEditable`. */
function contentEditable(el: Element): boolean {
  const own = (el as HTMLElement).isContentEditable;
  if (typeof own === 'boolean') return own;
  for (let node: Element | null = el; node; node = node.parentElement) {
    const value = node.getAttribute('contenteditable');
    if (value === null) continue;
    const word = value.trim().toLowerCase();
    return word === '' || word === 'true' || word === 'plaintext-only';
  }
  return false;
}

/**
 * What kind of field `el` is, or null when dictation must not write there: a
 * password, checkbox, number or other non-text `input`, a disabled or read only
 * control, or anything that is not editable at all.
 */
export function dictationFieldKind(
  el: Element | null | undefined
): 'textarea' | 'input' | 'contenteditable' | null {
  if (!el) return null;
  const tag = el.localName;
  if (tag === 'textarea' || tag === 'input') {
    const control = el as HTMLTextAreaElement | HTMLInputElement;
    if (control.disabled || control.readOnly) return null;
    if (tag === 'textarea') return 'textarea';
    return TEXT_INPUT_TYPES.has((control as HTMLInputElement).type) ? 'input' : null;
  }
  return contentEditable(el) ? 'contenteditable' : null;
}

const isTextControl = (el: Element): el is HTMLTextAreaElement | HTMLInputElement =>
  el.localName === 'textarea' || el.localName === 'input';

/** The field's text: a control's value, or an editable region's text content. */
export function fieldText(el: DictationField): string {
  return isTextControl(el) ? el.value : (el.textContent ?? '');
}

/** How many characters of `root`'s text come before the DOM position `(node, offset)`. */
function textOffset(root: Element, node: Node, offset: number): number {
  const range = root.ownerDocument.createRange();
  range.setStart(root, 0);
  range.setEnd(node, offset);
  return range.toString().length;
}

/** The DOM position of the `at`th character of `root`'s text. */
function domPosition(root: Element, at: number): { node: Node; offset: number } {
  const walker = root.ownerDocument.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  let seen = 0;
  let last: Text | null = null;
  for (let node = walker.nextNode() as Text | null; node; node = walker.nextNode() as Text | null) {
    const length = node.data.length;
    if (at <= seen + length) return { node, offset: Math.max(0, at - seen) };
    seen += length;
    last = node;
  }
  if (last) return { node: last, offset: last.data.length };
  return { node: root, offset: at <= 0 ? 0 : root.childNodes.length };
}

/**
 * The field's selection in its text, or null when it has none: a control's
 * selection, or the document's selection when it lies inside the editable region.
 */
export function fieldSelection(el: DictationField): Anchor | null {
  if (isTextControl(el)) {
    try {
      const start = el.selectionStart;
      const end = el.selectionEnd;
      return start === null || end === null ? null : { start, end };
    } catch {
      return null;
    }
  }
  const selection = el.ownerDocument.getSelection?.();
  if (!selection || selection.rangeCount === 0) return null;
  const range = selection.getRangeAt(0);
  if (!el.contains(range.startContainer) || !el.contains(range.endContainer)) return null;
  return {
    start: textOffset(el, range.startContainer, range.startOffset),
    end: textOffset(el, range.endContainer, range.endOffset),
  };
}

/**
 * Whether the field may take an `execCommand` insert: it has focus, or focus is
 * on the page body or on another element of the same composer card (the
 * microphone itself). `execCommand` writes into whatever has focus, so anything
 * else, another field or another pane, sends the text down the state path.
 */
export function composerOwnsFocus(field: Element): boolean {
  const active = field.ownerDocument.activeElement;
  if (!active || active === field || active === field.ownerDocument.body) return true;
  if (field.contains(active)) return true;
  const card = field.closest('.biorouter-composer-card');
  return !!card && card.contains(active);
}

export type InsertPath = 'undoable' | 'state' | 'draft';

/**
 * What {@link insertDictatedText} did: an {@link InsertPath}, or `refused` when
 * the field did not keep the text (a controlled field that put its old value
 * back, a region that rejected the edit). The caller then keeps the text for the
 * person, because it went nowhere.
 */
export type InsertResult = InsertPath | 'refused';

/** Chromium writes a space at an edge of an editable region as a no-break space. */
const sameText = (a: string, b: string): boolean =>
  a.replace(/\u00a0/g, ' ') === b.replace(/\u00a0/g, ' ');

/** The text went in where it was planned. */
function landed(el: DictationField, start: number, inserted: string): boolean {
  return sameText(fieldText(el).slice(start, start + inserted.length), inserted);
}

/**
 * A field's own state path when no composer keeps its text in React state: the
 * new value through the element prototype's value setter (so React's value
 * tracker sees a change and its `onChange` runs), then an `input` event, then the
 * caret. True when the field kept the value, read back after the event, so a
 * controlled field that rejects the change is told apart from one that took it.
 */
export function setFieldTextByState(
  el: HTMLTextAreaElement | HTMLInputElement,
  value: string,
  caret: number
): boolean {
  const view = el.ownerDocument.defaultView;
  const proto =
    el.localName === 'textarea'
      ? view?.HTMLTextAreaElement?.prototype
      : view?.HTMLInputElement?.prototype;
  const setter = proto ? Object.getOwnPropertyDescriptor(proto, 'value')?.set : undefined;
  if (setter) setter.call(el, value);
  else el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  if (el.value !== value) return false;
  try {
    if (el.ownerDocument.activeElement === el) el.setSelectionRange(caret, caret);
  } catch {
    // A type without a selection keeps its own caret.
  }
  return true;
}

function inputEvent(el: Element, inserted: string): Event {
  const view = el.ownerDocument.defaultView;
  const Input = view?.InputEvent;
  return Input
    ? new Input('input', { bubbles: true, inputType: 'insertText', data: inserted })
    : new Event('input', { bubbles: true });
}

/** Put `inserted` over `[start, end)` of an editable region's text, as one undo step when it can. */
function insertIntoEditable(
  el: HTMLElement,
  start: number,
  end: number,
  inserted: string,
  undoable: boolean
): InsertResult {
  const doc = el.ownerDocument;
  const rangeFor = () => {
    const from = domPosition(el, start);
    const to = domPosition(el, end);
    const range = doc.createRange();
    range.setStart(from.node, from.offset);
    range.setEnd(to.node, to.offset);
    return range;
  };
  const before = fieldText(el);
  if (undoable) {
    try {
      if (doc.activeElement !== el && !el.contains(doc.activeElement)) {
        el.focus({ preventScroll: true });
      }
      const selection = doc.getSelection();
      if (selection) {
        selection.removeAllRanges();
        selection.addRange(rangeFor());
        const ok = doc.execCommand('insertText', false, inserted);
        if (ok && landed(el, start, inserted)) return 'undoable';
      }
    } catch {
      // Fall through to the state path below.
    }
    // A refused or misplaced command may have written something: start again from the text as it was.
    if (!sameText(fieldText(el), before))
      return landed(el, start, inserted) ? 'undoable' : 'refused';
  }
  try {
    const range = rangeFor();
    range.deleteContents();
    const node = doc.createTextNode(inserted);
    range.insertNode(node);
    if (doc.activeElement === el || el.contains(doc.activeElement)) {
      const selection = doc.getSelection();
      const caret = doc.createRange();
      caret.setStartAfter(node);
      caret.collapse(true);
      selection?.removeAllRanges();
      selection?.addRange(caret);
    }
    el.dispatchEvent(inputEvent(el, inserted));
  } catch {
    return 'refused';
  }
  return landed(el, start, inserted) ? 'state' : 'refused';
}

/**
 * Put a transcript into a field at `anchor`.
 *
 * - `'undoable'`: the field owns focus and no IME composition runs, so the text
 *   goes in with the selection set to the anchor, then `execCommand('insertText')`.
 *   That is one undo step and fires the field's `input` event (React's
 *   `onChange`; measured on Electron 39.8.10). The result is read back; if the
 *   text is not where it should be, the state path below writes the intended
 *   value over whatever happened.
 * - `'state'`: focus is elsewhere, a composition is running, `execCommand`
 *   refused, or the read back failed. A textarea or `input` hands `setByState`
 *   the full new value and the caret (a composer's React state, or
 *   {@link setFieldTextByState} for a field with no composer); an editable
 *   region gets the words in place of the anchor's range and an `input` event.
 *   This path is not undoable.
 * - `'draft'`: there is no field any more (the composer unmounted). Nothing is
 *   written here; the caller splices the text into the stored draft with
 *   `insertIntoComposerDraft`, or keeps it for the person.
 * - `'refused'`: the field did not keep the text (`setByState` answered false,
 *   or the region's text did not read back). Nothing went in.
 *
 * A caller that knows a composition is running should wait for its
 * `compositionend` before calling, so the text can still take the undoable path.
 */
export function insertDictatedText(
  field: DictationField | null,
  anchor: Anchor,
  text: string,
  setByState: (value: string, caret: number) => void | boolean,
  options: { composing?: boolean } = {}
): InsertResult {
  if (!field || !field.isConnected) return 'draft';
  const value = fieldText(field);
  const planned = spliceDictation(value, anchor, text);
  if (!planned.inserted) return 'state';
  const start = Math.min(planned.caret - planned.inserted.length, value.length);
  const replaceEnd = clamp(Math.max(anchor.start, anchor.end), start, value.length);
  const kind = dictationFieldKind(field);
  const undoable = !options.composing && kind !== null && composerOwnsFocus(field);

  if (!isTextControl(field)) {
    if (kind !== 'contenteditable') return 'refused';
    return insertIntoEditable(field, start, replaceEnd, planned.inserted, undoable);
  }

  if (undoable) {
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
  return setByState(planned.value, planned.caret) === false ? 'refused' : 'state';
}
