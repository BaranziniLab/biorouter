import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  composerOwnsFocus,
  dictationFieldKind,
  fieldSelection,
  fieldText,
  insertDictatedText,
  mapAnchor,
  setFieldTextByState,
  spliceDictation,
  type Anchor,
} from './dictatedText';

const at = (n: number): Anchor => ({ start: n, end: n });

describe('spliceDictation', () => {
  const cases: Array<[string, string, Anchor, string, string, number]> = [
    // [name, value, anchor, text, expected value, expected caret]
    ['empty field', '', at(0), 'Hello there', 'Hello there', 11],
    ['after a word gets a leading space', 'Check', at(5), 'EPAS1', 'Check EPAS1', 11],
    ['after a space gets none', 'Check ', at(6), 'EPAS1', 'Check EPAS1', 11],
    ['after an opening bracket gets none', 'gene (', at(6), 'EPAS1', 'gene (EPAS1', 11],
    ['after an opening quote gets none', 'say “', at(5), 'hello', 'say “hello', 10],
    ['before a word gets a trailing space', 'the gene', at(4), 'EPAS1', 'the EPAS1 gene', 10],
    ['before punctuation gets none', 'the gene.', at(8), 'EPAS1', 'the gene EPAS1.', 14],
    ['at the start before a word', 'gene', at(0), 'EPAS1', 'EPAS1 gene', 6],
    ['collapses runs of whitespace', '', at(0), '  two \n  words  ', 'two words', 9],
    ['strips a leading slash', '', at(0), '/bug in the parser', 'bug in the parser', 17],
    ['strips several leading slashes', 'note', at(4), '// comment', 'note comment', 12],
    ['keeps an inner slash', '', at(0), 'and/or', 'and/or', 6],
    ['never changes case', 'x', at(1), 'eGFR and HLA-B*57:01', 'x eGFR and HLA-B*57:01', 22],
    [
      'replaces a selection',
      'replace THIS now',
      { start: 8, end: 12 },
      'that',
      'replace that now',
      12,
    ],
    ['a reversed selection still replaces', 'ab XY cd', { start: 5, end: 3 }, 'z', 'ab z cd', 4],
    ['clamps an anchor past the end', 'abc', at(99), 'd', 'abc d', 5],
  ];
  it.each(cases)('%s', (_name, value, anchor, text, expected, caret) => {
    const result = spliceDictation(value, anchor, text);
    expect(result.value).toBe(expected);
    expect(result.caret).toBe(caret);
    expect(result.value.slice(result.caret - result.inserted.length, result.caret)).toBe(
      result.inserted
    );
  });

  it('inserts nothing for an empty or slash only transcript', () => {
    expect(spliceDictation('keep', at(4), '  ')).toEqual({ value: 'keep', inserted: '', caret: 4 });
    expect(spliceDictation('keep', at(4), '/')).toEqual({ value: 'keep', inserted: '', caret: 4 });
  });
});

describe('mapAnchor', () => {
  it('a caret moves with text typed before it', () => {
    expect(mapAnchor(at(4), 'abcd', 'XXabcd', 2)).toEqual(at(6));
  });
  it('a caret moves past text typed at it, so speech lands after what was typed', () => {
    expect(mapAnchor(at(2), 'ab', 'abcd', 4)).toEqual(at(4));
  });
  it('a caret stays when the edit is after it', () => {
    expect(mapAnchor(at(1), 'ab', 'abX', 3)).toEqual(at(1));
  });
  it('the caret hint places an insertion inside a run of repeats', () => {
    // "aa" to "aaa": typed at 0 (caret 1) moves an anchor at 1; typed at 2 does not.
    expect(mapAnchor(at(1), 'aa', 'aaa', 1)).toEqual(at(2));
    expect(mapAnchor(at(1), 'aa', 'aaa', 3)).toEqual(at(1));
  });
  it('a deletion before the anchor pulls it back', () => {
    expect(mapAnchor(at(6), 'hello world', 'o world', 0)).toEqual(at(2));
  });
  it('a deletion across the caret collapses it to the deletion point', () => {
    expect(mapAnchor(at(4), 'abcdef', 'abf', 2)).toEqual(at(2));
  });
  it('an untouched selection keeps covering the same text', () => {
    expect(mapAnchor({ start: 4, end: 8 }, 'say THIS now', 'please say THIS now', 7)).toEqual({
      start: 11,
      end: 15,
    });
    // Text typed right after the selection stays outside it.
    expect(mapAnchor({ start: 4, end: 8 }, 'say THIS now', 'say THISX now', 9)).toEqual({
      start: 4,
      end: 8,
    });
  });
  it('a selection the person typed into collapses to its mapped end', () => {
    expect(mapAnchor({ start: 4, end: 8 }, 'say THIS now', 'say THxxIS now', 8)).toEqual(at(10));
    expect(mapAnchor({ start: 4, end: 8 }, 'say THIS now', 'say T now', 5)).toEqual(at(5));
  });
  it('no edit, no change', () => {
    expect(mapAnchor({ start: 1, end: 2 }, 'abc', 'abc')).toEqual({ start: 1, end: 2 });
  });
});

describe('insertDictatedText', () => {
  afterEach(() => {
    document.body.innerHTML = '';
    vi.restoreAllMocks();
  });

  function mount(value: string): HTMLTextAreaElement {
    document.body.innerHTML = `<div class="biorouter-composer-card"><textarea></textarea><button>mic</button></div><input id="elsewhere" />`;
    const field = document.querySelector('textarea')!;
    field.value = value;
    return field;
  }

  /** jsdom has no execCommand; this one behaves as Chromium's does for a textarea. */
  function fakeExecCommand(behaviour: 'insert' | 'refuse' | 'misplace' = 'insert') {
    if (typeof document.execCommand !== 'function') {
      Object.assign(document, { execCommand: () => false });
    }
    return vi
      .spyOn(document, 'execCommand')
      .mockImplementation((command: string, _ui?: boolean, text?: string) => {
        const field = document.activeElement as HTMLTextAreaElement;
        if (
          command !== 'insertText' ||
          behaviour === 'refuse' ||
          !(field instanceof HTMLTextAreaElement)
        ) {
          return false;
        }
        const start = field.selectionStart;
        const end = field.selectionEnd;
        const insert = behaviour === 'misplace' ? `${text}!` : (text ?? '');
        field.value = field.value.slice(0, start) + insert + field.value.slice(end);
        field.setSelectionRange(start + insert.length, start + insert.length);
        field.dispatchEvent(new Event('input', { bubbles: true }));
        return true;
      });
  }

  it('takes the undoable path when the field has focus, and fires input', () => {
    const field = mount('Check the gene');
    field.focus();
    const exec = fakeExecCommand();
    const onInput = vi.fn();
    field.addEventListener('input', onInput);
    const setByState = vi.fn();
    expect(insertDictatedText(field, at(9), 'EPAS1', setByState)).toBe('undoable');
    expect(exec).toHaveBeenCalledWith('insertText', false, ' EPAS1');
    expect(field.value).toBe('Check the EPAS1 gene');
    expect(field.selectionStart).toBe(15);
    expect(onInput).toHaveBeenCalledTimes(1);
    expect(setByState).not.toHaveBeenCalled();
  });

  it('takes the undoable path when focus is on the microphone inside the same card', () => {
    const field = mount('');
    document.querySelector('button')!.focus();
    fakeExecCommand();
    expect(insertDictatedText(field, at(0), 'hello', vi.fn())).toBe('undoable');
    expect(document.activeElement).toBe(field);
    expect(field.value).toBe('hello');
  });

  it('falls back to the state path when focus is in another field', () => {
    const field = mount('abc');
    (document.getElementById('elsewhere') as HTMLInputElement).focus();
    const exec = fakeExecCommand();
    const setByState = vi.fn();
    expect(insertDictatedText(field, at(3), 'def', setByState)).toBe('state');
    expect(exec).not.toHaveBeenCalled();
    expect(setByState).toHaveBeenCalledWith('abc def', 7);
    expect(field.value).toBe('abc');
  });

  it('falls back to the state path when execCommand refuses', () => {
    const field = mount('abc');
    field.focus();
    fakeExecCommand('refuse');
    const setByState = vi.fn();
    expect(insertDictatedText(field, at(3), 'def', setByState)).toBe('state');
    expect(setByState).toHaveBeenCalledWith('abc def', 7);
  });

  it('writes the planned value over a misplaced insert', () => {
    const field = mount('abc');
    field.focus();
    fakeExecCommand('misplace');
    const setByState = vi.fn();
    // The read back still finds " def" where it belongs, so this one is accepted...
    expect(insertDictatedText(field, at(3), 'def', setByState)).toBe('undoable');
    // ...but one that did not land where planned goes down the state path.
    const other = mount('xyz');
    other.focus();
    vi.restoreAllMocks();
    fakeExecCommand().mockImplementation(() => {
      other.value = 'something else';
      return true;
    });
    const fallback = vi.fn();
    expect(insertDictatedText(other, at(3), 'def', fallback)).toBe('state');
    expect(fallback).toHaveBeenCalledWith('xyz def', 7);
  });

  it('takes the state path during an IME composition', () => {
    const field = mount('abc');
    field.focus();
    const exec = fakeExecCommand();
    const setByState = vi.fn();
    expect(insertDictatedText(field, at(3), 'def', setByState, { composing: true })).toBe('state');
    expect(exec).not.toHaveBeenCalled();
  });

  it('answers draft when the field is gone', () => {
    const field = mount('abc');
    field.remove();
    const setByState = vi.fn();
    expect(insertDictatedText(field, at(3), 'def', setByState)).toBe('draft');
    expect(insertDictatedText(null, at(3), 'def', setByState)).toBe('draft');
    expect(setByState).not.toHaveBeenCalled();
  });
});

describe('the fields dictation writes into', () => {
  afterEach(() => {
    document.body.innerHTML = '';
    vi.restoreAllMocks();
  });

  const make = (html: string) => {
    document.body.innerHTML = html;
    return document.body.firstElementChild as HTMLElement;
  };

  it('a textarea, a text-like input and an editable region; never a password or a locked control', () => {
    expect(dictationFieldKind(make('<textarea></textarea>'))).toBe('textarea');
    for (const type of ['text', 'search', 'url', 'tel']) {
      expect(dictationFieldKind(make(`<input type="${type}">`)), type).toBe('input');
    }
    expect(dictationFieldKind(make('<input>'))).toBe('input');
    expect(dictationFieldKind(make('<div contenteditable="true"></div>'))).toBe('contenteditable');
    expect(dictationFieldKind(make('<div contenteditable=""></div>'))).toBe('contenteditable');
    expect(dictationFieldKind(make('<div contenteditable="plaintext-only"></div>'))).toBe(
      'contenteditable'
    );
    const nested = make('<div contenteditable="true"><p>x</p></div>').querySelector('p');
    expect(dictationFieldKind(nested)).toBe('contenteditable');
    for (const html of [
      '<input type="password">',
      '<input type="checkbox">',
      '<input type="number">',
      '<input type="email">',
      '<input disabled>',
      '<input readonly>',
      '<textarea disabled></textarea>',
      '<textarea readonly></textarea>',
      '<div contenteditable="false"></div>',
      '<div></div>',
    ]) {
      expect(dictationFieldKind(make(html)), html).toBeNull();
    }
    expect(dictationFieldKind(null)).toBeNull();
  });

  it('reads an editable region’s text and the selection inside it', () => {
    const region = make('<div contenteditable="true">Check <b>the</b> gene</div>');
    expect(fieldText(region)).toBe('Check the gene');
    const bold = region.querySelector('b')!.firstChild!;
    const range = document.createRange();
    range.setStart(bold, 1);
    range.setEnd(bold, 3);
    const selection = document.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
    expect(fieldSelection(region)).toEqual({ start: 7, end: 9 });
    // A selection outside the region is none of its.
    const outside = document.createElement('p');
    outside.textContent = 'elsewhere';
    document.body.append(outside);
    selection.removeAllRanges();
    const away = document.createRange();
    away.setStart(outside.firstChild!, 2);
    selection.addRange(away);
    expect(fieldSelection(region)).toBeNull();
  });

  it('a field owns focus when focus is inside its editable region', () => {
    const region = make('<div contenteditable="true"><p tabindex="-1">x</p></div><input />');
    region.querySelector('p')!.focus();
    expect(composerOwnsFocus(region)).toBe(true);
    (document.querySelector('input') as HTMLInputElement).focus();
    expect(composerOwnsFocus(region)).toBe(false);
  });

  it('a field’s own state path sets the value, fires input, and reads it back', () => {
    const field = make('<input value="abc">') as HTMLInputElement;
    const onInput = vi.fn();
    field.addEventListener('input', onInput);
    expect(setFieldTextByState(field, 'abc def', 7)).toBe(true);
    expect(field.value).toBe('abc def');
    expect(onInput).toHaveBeenCalledTimes(1);
    // A controlled field that puts its old value back did not keep the text.
    field.addEventListener('input', () => {
      field.value = 'abc';
    });
    expect(setFieldTextByState(field, 'abc xyz', 7)).toBe(false);
  });

  it('a text input takes the undoable path like a textarea, and the state path elsewhere', () => {
    document.body.innerHTML = '<input id="field" value="Untitled"><input id="other">';
    const field = document.getElementById('field') as HTMLInputElement;
    if (typeof document.execCommand !== 'function') {
      Object.assign(document, { execCommand: () => false });
    }
    vi.spyOn(document, 'execCommand').mockImplementation((_c, _u, text) => {
      const target = document.activeElement as HTMLInputElement;
      const start = target.selectionStart ?? 0;
      target.value =
        target.value.slice(0, start) + (text ?? '') + target.value.slice(target.selectionEnd ?? 0);
      target.dispatchEvent(new Event('input', { bubbles: true }));
      return true;
    });
    field.focus();
    const state = (value: string, caret: number) => setFieldTextByState(field, value, caret);
    expect(insertDictatedText(field, at(8), 'notes', state)).toBe('undoable');
    expect(field.value).toBe('Untitled notes');
    (document.getElementById('other') as HTMLInputElement).focus();
    expect(insertDictatedText(field, at(14), 'today', state)).toBe('state');
    expect(field.value).toBe('Untitled notes today');
  });

  it('answers refused when the state path did not keep the text', () => {
    const field = make('<textarea>abc</textarea>') as HTMLTextAreaElement;
    expect(insertDictatedText(field, at(3), 'def', () => false)).toBe('refused');
    expect(insertDictatedText(field, at(3), 'def', () => undefined)).toBe('state');
  });

  it('an editable region without focus gets the words in place, with an input event', () => {
    document.body.innerHTML = '<div id="region" contenteditable="true">Check gene</div><input>';
    const region = document.getElementById('region')!;
    (document.querySelector('input') as HTMLInputElement).focus();
    const events: Event[] = [];
    region.addEventListener('input', (event) => events.push(event));
    const setByState = vi.fn();
    expect(insertDictatedText(region, at(5), 'EPAS1', setByState)).toBe('state');
    expect(region.textContent).toBe('Check EPAS1 gene');
    expect(events).toHaveLength(1);
    expect(events[0]).toBeInstanceOf(InputEvent);
    expect((events[0] as InputEvent).data).toBe(' EPAS1');
    // The composer's state path is for controls; a region writes itself.
    expect(setByState).not.toHaveBeenCalled();
  });

  it('an editable region with focus takes the undoable path when the command lands', () => {
    document.body.innerHTML = '<div id="region" contenteditable="true">Check</div>';
    const region = document.getElementById('region')!;
    region.focus();
    if (typeof document.execCommand !== 'function') {
      Object.assign(document, { execCommand: () => false });
    }
    const exec = vi.spyOn(document, 'execCommand').mockImplementation((_c, _u, text) => {
      const range = document.getSelection()!.getRangeAt(0);
      range.deleteContents();
      // Chromium writes a space at the region's edge as a no-break space.
      range.insertNode(document.createTextNode((text ?? '').replace(/^ /, '\u00a0')));
      return true;
    });
    expect(insertDictatedText(region, at(5), 'EPAS1', vi.fn())).toBe('undoable');
    expect(exec).toHaveBeenCalledWith('insertText', false, ' EPAS1');
    expect(region.textContent?.replace(/\u00a0/g, ' ')).toBe('Check EPAS1');
  });
});
