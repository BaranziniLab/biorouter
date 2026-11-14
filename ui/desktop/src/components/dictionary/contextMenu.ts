/**
 * The context menu door (DD-68, "Context menu door" in
 * `docs/dictation/dictionary.md`): what `main.ts`'s `context-menu` handler
 * offers, and what it sends the window.
 *
 * Electron's spelling item used to read "Add to dictionary", which a person
 * would take for this feature, so it reads **Add to spelling dictionary** now.
 * Beside it, **Add to Biorouter dictionary…** is shown for a misspelled word or
 * a selection of one line and at most 64 characters, and sends
 * `dictionary:add-term` to that window with `{text, x, y}` (the menu's
 * `params.x` and `params.y`). Main knows the window, not the pane, so the
 * renderer resolves which chat the point sits in (`termOrigin.ts`). Terms are
 * never copied into the operating system's spellchecker.
 *
 * Where managed policy turns the dictionary off, the item is withheld, as the
 * sidebar row is: each window tells main on `dictionary:offered` what its
 * dictation status store says once that store has read the policy, and main
 * offers the item until a window has said `false` ({@link setDictionaryOffered}).
 *
 * Free of Electron and React, so the main process, the preload bridge and the
 * renderer share it; `contextMenu.test.ts` pins the labels, the rule and the
 * payload, and that `main.ts` and `preload.ts` use them.
 */

/** The IPC channel main sends to the window the menu opened in. */
export const DICTIONARY_ADD_TERM_CHANNEL = 'dictionary:add-term';

/** The IPC channel a window uses to tell main whether to offer the item. */
export const DICTIONARY_OFFERED_CHANNEL = 'dictionary:offered';

/** Electron's own spelling item, relabelled so it is not read as Biorouter's. */
export const SPELLING_DICTIONARY_LABEL = 'Add to spelling dictionary';

/** Opens the term dialog prefilled with the word or selection. */
export const BIOROUTER_DICTIONARY_LABEL = 'Add to Biorouter dictionary…';

/** A term is at most 64 characters (MSG-TERM-TOO-LONG), so a longer selection offers nothing. */
export const MAX_CONTEXT_TERM_CHARS = 64;

/** The parts of Electron's `ContextMenuParams` the door reads. */
export interface ContextMenuFacts {
  misspelledWord?: string;
  selectionText?: string;
  x: number;
  y: number;
}

/** What main sends with `dictionary:add-term`. */
export interface DictionaryAddTermPayload {
  text: string;
  /** The menu's position in the window, in the page's coordinates. */
  x: number;
  y: number;
}

/** Characters, not UTF-16 units, as the store counts a term's length. */
function characters(text: string): number {
  return Array.from(text).length;
}

/**
 * The text **Add to Biorouter dictionary…** would add, or `null` when the item
 * is not shown: a selection of one line and at most 64 characters, else the
 * misspelled word under the pointer.
 */
export function biorouterDictionaryTerm(facts: ContextMenuFacts): string | null {
  const selection = (facts.selectionText ?? '').trim();
  if (selection) {
    if (/[\r\n]/.test(selection)) return null;
    return characters(selection) <= MAX_CONTEXT_TERM_CHARS ? selection : null;
  }
  const word = (facts.misspelledWord ?? '').trim();
  if (!word || characters(word) > MAX_CONTEXT_TERM_CHARS) return null;
  return word;
}

/** The payload main sends, or `null` when the Biorouter item is not shown. */
export function dictionaryAddTermPayload(facts: ContextMenuFacts): DictionaryAddTermPayload | null {
  const text = biorouterDictionaryTerm(facts);
  if (text === null) return null;
  return { text, x: Math.round(facts.x), y: Math.round(facts.y) };
}

/**
 * A payload as the renderer receives it, checked again: the preload bridge and
 * the door accept nothing else, whatever arrives on the channel.
 */
export function readDictionaryAddTermPayload(value: unknown): DictionaryAddTermPayload | null {
  if (!value || typeof value !== 'object') return null;
  const { text, x, y } = value as { text?: unknown; x?: unknown; y?: unknown };
  if (typeof text !== 'string' || typeof x !== 'number' || typeof y !== 'number') return null;
  if (!Number.isFinite(x) || !Number.isFinite(y)) return null;
  const term = biorouterDictionaryTerm({ selectionText: text, x, y });
  return term === null ? null : { text: term, x, y };
}

/** One item of the menu main builds, as a plain description `main.ts` turns into `MenuItem`s. */
export type ContextMenuEntry =
  | { kind: 'replace'; label: string }
  | { kind: 'spelling'; label: string; word: string }
  | { kind: 'biorouter'; label: string; payload: DictionaryAddTermPayload }
  | { kind: 'separator' };

/**
 * The spelling part of the menu, in order: each suggestion, **Add to spelling
 * dictionary** for a misspelled word, then **Add to Biorouter dictionary…** when
 * it applies and `offerBiorouter` holds (false where managed policy turns the
 * dictionary off). `main.ts` appends Cut, Copy and Paste after these.
 */
export function spellingMenuEntries(
  facts: ContextMenuFacts & { dictionarySuggestions?: readonly string[] },
  offerBiorouter = true
): ContextMenuEntry[] {
  const entries: ContextMenuEntry[] = [];
  for (const suggestion of facts.dictionarySuggestions ?? []) {
    entries.push({ kind: 'replace', label: suggestion });
  }
  if (facts.misspelledWord) {
    entries.push({
      kind: 'spelling',
      label: SPELLING_DICTIONARY_LABEL,
      word: facts.misspelledWord,
    });
  }
  const payload = offerBiorouter ? dictionaryAddTermPayload(facts) : null;
  if (payload) entries.push({ kind: 'biorouter', label: BIOROUTER_DICTIONARY_LABEL, payload });
  if (entries.length > 0 && facts.selectionText) entries.push({ kind: 'separator' });
  return entries;
}

// ── which windows offer the item (main's side) ───────────────────────────────

/** The part of Electron's `WebContents` the record needs. */
export interface OfferContents {
  readonly id: number;
  once(event: 'destroyed', listener: () => void): unknown;
}

/** Windows whose status store read managed `dictionary: false`. */
const withheld = new Set<number>();
/** Contents whose `destroyed` is already watched, once each. */
const watched = new WeakSet<OfferContents>();

/**
 * Record what one window said on {@link DICTIONARY_OFFERED_CHANNEL}. A window
 * that has said nothing is offered the item, as the sidebar shows its row until
 * the status store has loaded; a reload keeps what the window last said until
 * the new document's store reads the policy again.
 */
export function setDictionaryOffered(contents: OfferContents, offered: boolean): void {
  if (offered) {
    withheld.delete(contents.id);
    return;
  }
  withheld.add(contents.id);
  if (watched.has(contents)) return;
  watched.add(contents);
  const id = contents.id;
  contents.once('destroyed', () => {
    withheld.delete(id);
    watched.delete(contents);
  });
}

/** Whether the menu of the window with this `WebContents` id offers **Add to Biorouter dictionary…**. */
export function isDictionaryOffered(contentsId: number): boolean {
  return !withheld.has(contentsId);
}

/** For tests: forget every window. */
export function resetDictionaryOfferedForTests(): void {
  withheld.clear();
}
