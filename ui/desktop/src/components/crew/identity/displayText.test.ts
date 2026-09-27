import { describe, expect, it } from 'vitest';
import { stripHiddenCharacters } from '../../../utils/untrustedText';
import { sanitizeDisplayText } from './displayText';

/**
 * `sanitizeDisplayText` is a security sanitizer: a legacy nickname reaches it
 * verbatim and may carry a bidi override, a zero-width joiner or a control.
 * It used to remove every rejected class with ONE character class; it now
 * removes the classes beyond the hidden-character drop set itself and calls the
 * one shared definition of that set (`utils/untrustedText.ts`) for the rest.
 *
 * Every expected value below was captured from the single-class implementation
 * before the split, so a row that changes is a behaviour change, not a
 * refactor. Escapes are written in the braced form throughout so the source
 * file itself never holds an invisible character.
 */

describe('sanitizeDisplayText', () => {
  it.each([
    ['a right-to-left override', 'abc\u{202E}def', 'abcdef'],
    ['bidi isolates and an embedding', '\u{2066}Alice\u{2069} \u{202A}gone\u{202C}', 'Alice gone'],
    ['a zero-width space and a BOM', 'Al\u{200B}i\u{FEFF}ce', 'Alice'],
    ['a zero-width joiner and non-joiner', 'a\u{200D}b\u{200C}c', 'abc'],
    ['C0 controls (NUL, BEL, ESC)', 'a\u{0}b\u{7}c\u{1B}d', 'abcd'],
    ['C1 controls', 'a\u{80}b\u{9B}c\u{9F}d', 'abcd'],
    ['DEL', 'a\u{7F}b', 'ab'],
    ['a private-use character', 'x\u{E000}y', 'xy'],
    ['a lone high surrogate', 'x\u{D800}y', 'xy'],
    ['a lone low surrogate', 'x\u{DC00}y', 'xy'],
    ['a variation selector', 'x\u{FE0F}y', 'xy'],
    ['a supplementary variation selector', 'x\u{E0100}y', 'xy'],
    ['the Hangul filler', 'x\u{3164}y', 'xy'],
    ['the Hangul choseong filler', 'x\u{115F}y', 'xy'],
    ['a tag character', 'a\u{E0041}b', 'ab'],
    ['an unassigned code point', 'a\u{378}b', 'ab'],
    ['a soft hyphen', 'a\u{AD}b', 'ab'],
    ['the Arabic letter mark', 'a\u{61C}b', 'ab'],
    ['hidden characters at the edges', '  \u{200B} Alice \u{200B}  ', 'Alice'],
    ['a hidden character between spaces', 'a \u{200B} b', 'a b'],
  ])('removes %s', (_label, raw, expected) => {
    expect(sanitizeDisplayText(raw)).toBe(expected);
  });

  it.each([
    ['a run holding a newline and a tab', 'Alice \n\t  Smith', 'Alice Smith'],
    ['a lone newline', 'a\nb', 'a b'],
    ['CRLF', 'a\r\nb', 'a b'],
    ['no-break spaces', 'a\u{A0}\u{A0}b', 'a b'],
    ['the line separator', 'a\u{2028}b', 'a b'],
    ['the paragraph separator', 'a\u{2029}b', 'a b'],
  ])('collapses %s to one space rather than dropping it', (_label, raw, expected) => {
    expect(sanitizeDisplayText(raw)).toBe(expected);
  });

  it('normalizes to NFC', () => {
    expect(sanitizeDisplayText('e\u{301}')).toBe('\u{E9}');
  });

  it('keeps a real astral character and an emoji sequence minus its joiner', () => {
    expect(sanitizeDisplayText('\u{10000}')).toBe('\u{10000}');
    expect(sanitizeDisplayText('\u{1F469}\u{200D}\u{1F4BB}')).toBe('\u{1F469}\u{1F4BB}');
  });

  it('turns a non-string into an empty string', () => {
    expect(sanitizeDisplayText(null)).toBe('');
    expect(sanitizeDisplayText(undefined)).toBe('');
    expect(sanitizeDisplayText(7)).toBe('');
  });

  /*
   * The ordering the split depends on. Deleting the format character first
   * would join the two lone surrogates around it into U+10000, a visible
   * character no later pass removes; the single class dropped all three.
   */
  it('never joins two lone surrogates by deleting what stood between them', () => {
    expect(sanitizeDisplayText('\u{D800}\u{2066}\u{DC00}')).toBe('');
    expect(sanitizeDisplayText('x\u{D800}\u{202E}\u{DC00}y')).toBe('xy');
    expect(sanitizeDisplayText('x\u{D800}\u{0}\u{DC00}y')).toBe('xy');

    // For every control and format character: the lone surrogates around it
    // contribute nothing. (A white-space control such as `\n` still becomes a
    // space, with or without them.)
    const joined: string[] = [];
    for (let cp = 0; cp <= 0xffff; cp += 1) {
      const char = String.fromCharCode(cp);
      if (stripHiddenCharacters(char) !== '') continue;
      const around = sanitizeDisplayText(`x\u{D800}${char}\u{DC00}y`);
      if (around !== sanitizeDisplayText(`x${char}y`)) joined.push(cp.toString(16));
    }
    expect(joined).toEqual([]);
  });

  it('leaves no rejected character behind, for every code point', () => {
    const beyondHidden = /[\p{Co}\p{Cs}\p{Cn}\p{Zl}\p{Zp}\p{Default_Ignorable_Code_Point}]/u;
    const unexpected: string[] = [];
    for (let cp = 0; cp <= 0x10ffff; cp += 1) {
      const raw = `a${String.fromCodePoint(cp)}b`;
      const out = sanitizeDisplayText(raw);
      const clean = stripHiddenCharacters(out) === out && !beyondHidden.test(out);
      const shape = out === 'ab' || out === 'a b' || out === raw.normalize('NFC');
      if (!clean || !shape) unexpected.push(cp.toString(16));
    }
    expect(unexpected).toEqual([]);
  });
});
