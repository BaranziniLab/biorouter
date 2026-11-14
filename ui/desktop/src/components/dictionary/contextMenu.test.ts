import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  BIOROUTER_DICTIONARY_LABEL,
  DICTIONARY_ADD_TERM_CHANNEL,
  DICTIONARY_OFFERED_CHANNEL,
  SPELLING_DICTIONARY_LABEL,
  biorouterDictionaryTerm,
  dictionaryAddTermPayload,
  isDictionaryOffered,
  readDictionaryAddTermPayload,
  resetDictionaryOfferedForTests,
  setDictionaryOffered,
  spellingMenuEntries,
  type OfferContents,
} from './contextMenu';

/**
 * The context menu door (DD-68), from the main process's side. `main.ts` cannot be imported
 * under vitest (module level Electron side effects), so its menu is built from these pure
 * descriptions, and what is pinned about `main.ts` and `preload.ts` themselves is read from
 * their source, as `main.openPath.test.ts` does.
 */
const src = join(__dirname, '../..');
const main = readFileSync(join(src, 'main.ts'), 'utf8');
const preload = readFileSync(join(src, 'preload.ts'), 'utf8');

const at = { x: 120.4, y: 340.6 };

describe('the menu labels', () => {
  it('relabels Electron’s spelling item and names Biorouter’s with an ellipsis', () => {
    expect(SPELLING_DICTIONARY_LABEL).toBe('Add to spelling dictionary');
    expect(BIOROUTER_DICTIONARY_LABEL).toBe('Add to Biorouter dictionary…');
    expect(DICTIONARY_ADD_TERM_CHANNEL).toBe('dictionary:add-term');
  });

  it('lists the suggestions, the spelling item, then Biorouter’s, for a misspelled word', () => {
    expect(
      spellingMenuEntries({
        dictionarySuggestions: ['Epas', 'Pass'],
        misspelledWord: 'EPAS1',
        selectionText: '',
        ...at,
      })
    ).toEqual([
      { kind: 'replace', label: 'Epas' },
      { kind: 'replace', label: 'Pass' },
      { kind: 'spelling', label: SPELLING_DICTIONARY_LABEL, word: 'EPAS1' },
      {
        kind: 'biorouter',
        label: BIOROUTER_DICTIONARY_LABEL,
        payload: { text: 'EPAS1', x: 120, y: 341 },
      },
    ]);
  });

  it('offers Biorouter’s item for a selection, before a separator and Cut and Copy', () => {
    expect(
      spellingMenuEntries({ misspelledWord: '', selectionText: '  Zeta Lab ', ...at })
    ).toEqual([
      {
        kind: 'biorouter',
        label: BIOROUTER_DICTIONARY_LABEL,
        payload: { text: 'Zeta Lab', x: 120, y: 341 },
      },
      { kind: 'separator' },
    ]);
  });

  it('offers nothing of its own with no word and no selection', () => {
    expect(spellingMenuEntries({ misspelledWord: '', selectionText: '', ...at })).toEqual([]);
  });

  it('leaves Biorouter’s item out where the dictionary is turned off, and keeps Electron’s', () => {
    expect(
      spellingMenuEntries(
        { dictionarySuggestions: ['Epas'], misspelledWord: 'EPAS1', selectionText: '', ...at },
        false
      )
    ).toEqual([
      { kind: 'replace', label: 'Epas' },
      { kind: 'spelling', label: SPELLING_DICTIONARY_LABEL, word: 'EPAS1' },
    ]);
    // A selection keeps its separator before Cut and Copy.
    expect(
      spellingMenuEntries({ misspelledWord: '', selectionText: 'UCSF', ...at }, false)
    ).toEqual([]);
  });
});

describe('which windows offer the item', () => {
  function contents(id: number) {
    const listeners: Array<() => void> = [];
    const value: OfferContents & { destroy: () => void; watchers: () => number } = {
      id,
      once: (_event, listener) => listeners.push(listener),
      destroy: () => listeners.splice(0).forEach((listener) => listener()),
      watchers: () => listeners.length,
    };
    return value;
  }

  it('offers it until the window says the dictionary is off, and again when it says on', () => {
    resetDictionaryOfferedForTests();
    const one = contents(1);
    const two = contents(2);
    expect(isDictionaryOffered(1)).toBe(true);
    setDictionaryOffered(one, false);
    expect(isDictionaryOffered(1)).toBe(false);
    // Each window speaks for itself.
    expect(isDictionaryOffered(2)).toBe(true);
    setDictionaryOffered(two, true);
    expect(isDictionaryOffered(2)).toBe(true);
    setDictionaryOffered(one, true);
    expect(isDictionaryOffered(1)).toBe(true);
    expect(DICTIONARY_OFFERED_CHANNEL).toBe('dictionary:offered');
  });

  it('forgets a window when it is destroyed, watching it once however often it says off', () => {
    resetDictionaryOfferedForTests();
    const closing = contents(7);
    setDictionaryOffered(closing, false);
    setDictionaryOffered(closing, false);
    expect(closing.watchers()).toBe(1);
    closing.destroy();
    expect(isDictionaryOffered(7)).toBe(true);
  });
});

describe('which text the item adds', () => {
  it('takes a selection of one line and at most 64 characters, else the misspelled word', () => {
    expect(biorouterDictionaryTerm({ selectionText: 'HIF2A', misspelledWord: 'x', ...at })).toBe(
      'HIF2A'
    );
    expect(biorouterDictionaryTerm({ selectionText: '', misspelledWord: 'EPAS1', ...at })).toBe(
      'EPAS1'
    );
    expect(biorouterDictionaryTerm({ selectionText: 'a'.repeat(64), ...at })).toBe('a'.repeat(64));
    expect(biorouterDictionaryTerm({ selectionText: 'a'.repeat(65), ...at })).toBeNull();
    // Counted in characters, as the store counts a term: 64 emoji are 128 UTF-16 units.
    expect(biorouterDictionaryTerm({ selectionText: '🧬'.repeat(64), ...at })).toBe(
      '🧬'.repeat(64)
    );
    expect(biorouterDictionaryTerm({ selectionText: 'two\nlines', ...at })).toBeNull();
    // A long selection does not fall back to the word under the pointer.
    expect(
      biorouterDictionaryTerm({ selectionText: 'x'.repeat(80), misspelledWord: 'EPAS1', ...at })
    ).toBeNull();
  });

  it('sends {text, x, y}, the menu’s point rounded to whole pixels', () => {
    expect(dictionaryAddTermPayload({ misspelledWord: 'EPAS1', ...at })).toEqual({
      text: 'EPAS1',
      x: 120,
      y: 341,
    });
    expect(Object.keys(dictionaryAddTermPayload({ selectionText: 'UCSF', x: 1, y: 2 })!)).toEqual([
      'text',
      'x',
      'y',
    ]);
  });

  it('accepts on the renderer’s side only that shape, checked again', () => {
    expect(readDictionaryAddTermPayload({ text: ' EPAS1 ', x: 1, y: 2 })).toEqual({
      text: 'EPAS1',
      x: 1,
      y: 2,
    });
    for (const bad of [
      null,
      'EPAS1',
      { text: 'EPAS1', x: '1', y: 2 },
      { text: 'EPAS1', x: Number.NaN, y: 2 },
      { text: 'x'.repeat(65), x: 1, y: 2 },
      { text: 'a\nb', x: 1, y: 2 },
      { text: 42, x: 1, y: 2 },
    ]) {
      expect(readDictionaryAddTermPayload(bad), JSON.stringify(bad)).toBeNull();
    }
  });
});

describe('main.ts and preload.ts use them', () => {
  const handler = main.slice(
    main.indexOf("mainWindow.webContents.on('context-menu'"),
    main.indexOf('// Handle new window creation for links.')
  );

  it('builds the spelling part of the menu from spellingMenuEntries, asking whether to offer it', () => {
    expect(handler.length).toBeGreaterThan(100);
    expect(handler).toContain('isDictionaryOffered(mainWindow.webContents.id)');
    expect(handler).toContain('spellingMenuEntries(params, offerBiorouter)');
    // The old label is gone, so the spelling item cannot be read as Biorouter's feature.
    expect(main).not.toContain("label: 'Add to dictionary'");
  });

  it('records what a window says on the offered channel, from that window only', () => {
    const listener = main.slice(main.indexOf('ipcMain.on(DICTIONARY_OFFERED_CHANNEL'));
    const body = listener.slice(0, listener.indexOf('});'));
    expect(body).toContain('if (!isDictationSender(event)) return;');
    expect(body).toContain('setDictionaryOffered(event.sender, offered === true)');
    const bridge = preload.slice(preload.indexOf('setDictionaryOffered: ('));
    expect(bridge).toContain('ipcRenderer.send(DICTIONARY_OFFERED_CHANNEL, offered === true)');
  });

  it('sends the payload to the window the menu opened in, on the shared channel', () => {
    expect(handler).toContain(
      'mainWindow.webContents.send(DICTIONARY_ADD_TERM_CHANNEL, entry.payload)'
    );
    // Electron's spellchecker gets only its own item's word, never a Biorouter term.
    const biorouter = handler.slice(
      handler.indexOf("case 'biorouter'"),
      handler.indexOf("case 'separator'")
    );
    expect(biorouter).not.toContain('addWordToSpellCheckerDictionary');
  });

  it('checks every payload in the preload bridge before the page hears it', () => {
    const bridge = preload.slice(preload.indexOf('onDictionaryAddTerm: ('));
    expect(bridge).toContain('readDictionaryAddTermPayload(payload)');
    expect(bridge).toContain('ipcRenderer.on(DICTIONARY_ADD_TERM_CHANNEL, listener)');
    expect(bridge).toContain('ipcRenderer.removeListener(DICTIONARY_ADD_TERM_CHANNEL, listener)');
  });
});
