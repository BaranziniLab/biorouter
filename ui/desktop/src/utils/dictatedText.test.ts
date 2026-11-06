import { afterEach, describe, expect, it, vi } from 'vitest';
import { insertDictatedText, mapAnchor, spliceDictation, type Anchor } from './dictatedText';

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
