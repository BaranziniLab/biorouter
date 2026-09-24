import { describe, expect, it } from 'vitest';
import {
  sanitizeArtifactTitle,
  sanitizeUntrustedLabel,
  stripHiddenCharacters,
  UNTRUSTED_LABEL_MAX_CHARS,
} from './untrustedText';

/**
 * The one definition of the hidden-character drop set, and the label sanitizer
 * built on it. Every expected value was captured from the implementation
 * before `stripHiddenCharacters` was extracted, so a row that changes is a
 * behaviour change. Escapes use the braced form so this file never holds an
 * invisible character itself.
 */

describe('stripHiddenCharacters', () => {
  it.each([
    ['a right-to-left override', 'abc\u{202E}def', 'abcdef'],
    ['bidi isolates and an embedding', '\u{2066}A\u{2069}\u{202A}B\u{202C}', 'AB'],
    ['zero-width characters and a BOM', 'Al\u{200B}i\u{FEFF}c\u{200D}e', 'Alice'],
    ['C0 controls, including a newline and a tab', 'a\u{0}b\u{7}c\u{1B}d\ne\tf', 'abcdef'],
    ['C1 controls', 'a\u{80}b\u{9B}c\u{9F}d', 'abcd'],
    ['DEL', 'a\u{7F}b', 'ab'],
    ['a tag character', 'a\u{E0041}b', 'ab'],
    ['a soft hyphen', 'a\u{AD}b', 'ab'],
    ['the Arabic letter mark', 'a\u{61C}b', 'ab'],
  ])('removes %s', (_label, raw, expected) => {
    expect(stripHiddenCharacters(raw)).toBe(expected);
  });

  it.each([
    ['a private-use character', 'x\u{E000}y'],
    ['a lone surrogate', 'x\u{D800}y'],
    ['the line separator', 'a\u{2028}b'],
    ['the paragraph separator', 'a\u{2029}b'],
    ['a variation selector', 'x\u{FE0F}y'],
    ['the Hangul filler', 'x\u{3164}y'],
    ['no-break spaces', 'a\u{A0}\u{A0}b'],
    ['a combining mark, unnormalized', 'e\u{301}'],
  ])('leaves %s alone: it is not a control or format character', (_label, raw) => {
    expect(stripHiddenCharacters(raw)).toBe(raw);
  });

  it('neither trims nor caps', () => {
    expect(stripHiddenCharacters(' a\u{200B} ')).toBe(' a ');
    const long = 'x'.repeat(UNTRUSTED_LABEL_MAX_CHARS + 10);
    expect(stripHiddenCharacters(long)).toBe(long);
  });
});

describe('sanitizeUntrustedLabel', () => {
  it('drops a newline rather than turning it into a space', () => {
    expect(sanitizeUntrustedLabel('a\nb')).toBe('ab');
    expect(sanitizeUntrustedLabel('a\r\nb')).toBe('ab');
    expect(sanitizeUntrustedLabel('Alice \n\t  Smith')).toBe('Alice   Smith');
  });

  it('strips, then trims, then caps', () => {
    expect(sanitizeUntrustedLabel('  \u{200B} Alice \u{200B}  ')).toBe('Alice');
    expect(sanitizeUntrustedLabel('  ab  ', 1)).toBe('a');
    expect(sanitizeUntrustedLabel('\u{200B}abc', 2)).toBe('ab');
    const padded = sanitizeUntrustedLabel(`\u{202E}${'x'.repeat(300)}`);
    expect(padded).toBe('x'.repeat(UNTRUSTED_LABEL_MAX_CHARS));
  });

  it('keeps what is not hidden', () => {
    expect(sanitizeUntrustedLabel('x\u{E000}y')).toBe('x\u{E000}y');
    expect(sanitizeUntrustedLabel('a\u{2028}b')).toBe('a\u{2028}b');
    expect(sanitizeUntrustedLabel('x\u{FE0F}y')).toBe('x\u{FE0F}y');
  });
});

describe('sanitizeArtifactTitle', () => {
  it('always names something', () => {
    expect(sanitizeArtifactTitle('Report\u{202E}')).toBe('Report');
    expect(sanitizeArtifactTitle('\u{202E}\n')).toBe('Artifact');
    expect(sanitizeArtifactTitle('', '\u{200B}')).toBe('Artifact');
    expect(sanitizeArtifactTitle(' ', 'Figure')).toBe('Figure');
  });
});
