import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  revealHiddenCharacters,
  sanitizeArtifactTitle,
  sanitizeUntrustedLabel,
  stripHiddenCharacters,
  UNTRUSTED_LABEL_MAX_CHARS,
  type RevealedSegment,
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

/**
 * A message body another person wrote, drawn with what could make it read as something else made
 * visible (QA M3, SEC-9), and nothing that legitimate text needs taken away.
 */
describe('revealHiddenCharacters', () => {
  /** The segments as text, each hidden character as `[escape]`. */
  const shown = (segments: RevealedSegment[]) =>
    segments.map((part) => (part.kind === 'text' ? part.text : `[${part.escape}]`)).join('');
  /** What a copy of the segments gives back: the raw characters. */
  const raw = (segments: RevealedSegment[]) =>
    segments.map((part) => (part.kind === 'text' ? part.text : part.raw)).join('');

  it('shows a right-to-left override as its escape, as the command line prints it', () => {
    const segments = revealHiddenCharacters('invoice_\u{202E}gnp.exe and more');
    expect(segments).toEqual([
      { kind: 'text', text: 'invoice_' },
      { kind: 'hidden', raw: '\u{202E}', escape: '\\u{202e}', codePoint: 'U+202E' },
      { kind: 'text', text: 'gnp.exe and more' },
    ]);
  });

  it.each([
    ['every embedding, override and isolate', 'a\u{202A}b\u{202B}c\u{202C}d\u{202D}e', 4],
    ['the isolates', '\u{2066}x\u{2067}y\u{2068}z\u{2069}', 4],
    ['control characters other than a tab or a break', 'bell\u{7}back\u{8}esc\u{1B}c1\u{9B}', 4],
    ['a tag character outside a flag', 'a\u{E0041}\u{E0042}b', 2],
    ['a zero-width space inside a handle', 'hi @cre\u{200B}w_bob', 1],
    ['a zero-width space inside a bare username', 'cre\u{200B}w_bob said', 1],
    ['a word joiner inside an address', 'see https://www.ucsf\u{2060}.edu/login', 1],
    ['a byte-order mark on a domain', 'open ucsf.edu\u{FEFF} now', 1],
  ])('shows %s', (_label, value, count) => {
    const segments = revealHiddenCharacters(value);
    expect(segments.filter((part) => part.kind === 'hidden')).toHaveLength(count);
    expect(raw(segments)).toBe(value);
  });

  it.each([
    ['a Hebrew paragraph', 'שלום לכולם, הפגישה בשעה 3.'],
    ['right-to-left marks', 'x\u{200F}y\u{200E}z\u{61C}'],
    [
      'an emoji joiner sequence',
      'scientist \u{1F469}\u{1F3FD}\u{200D}\u{1F52C} and family \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}',
    ],
    ['a zero-width non-joiner in Persian', 'می\u{200C}خواهم'],
    ['a zero-width space between Thai words', 'สวัสดี\u{200B}ครับ'],
    ['a subdivision flag', 'go \u{1F3F4}\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}!'],
    ['tabs, newlines and a soft hyphen', 'a\tb\nc\r\nd\u{AD}e'],
  ])('leaves %s alone', (_label, value) => {
    expect(revealHiddenCharacters(value)).toEqual([{ kind: 'text', text: value }]);
  });

  it('keeps every raw character for a copy, whatever it shows', () => {
    const value = '\u{202E}a\u{200B}@b\u{FEFF}c\u{E0041}\u{2066}';
    const segments = revealHiddenCharacters(value);
    expect(raw(segments)).toBe(value);
    expect(shown(segments)).toBe('[\\u{202e}]a[\\u{200b}]@b[\\u{feff}]c[\\u{e0041}][\\u{2066}]');
  });

  it('never splits a surrogate pair', () => {
    expect(revealHiddenCharacters('\u{1F600}\u{202E}\u{1F600}')).toEqual([
      { kind: 'text', text: '\u{1F600}' },
      { kind: 'hidden', raw: '\u{202E}', escape: '\\u{202e}', codePoint: 'U+202E' },
      { kind: 'text', text: '\u{1F600}' },
    ]);
  });

  it('returns nothing for nothing', () => {
    expect(revealHiddenCharacters('')).toEqual([]);
  });
});

/**
 * No renderer source may carry a literal bidirectional control. An unmatched
 * embedding, override or isolate reorders how the lines after it DISPLAY in an
 * editor or a diff without changing what the compiler reads (the "Trojan
 * Source" pattern), and GitHub flags the file for it. A comment in
 * `untrustedText.ts` shipped one: a live U+2066 inside what was meant to be the
 * example `\uD800\u2066\uDC00`, so the module that exists to strip these
 * characters held an unterminated isolate, and the example read as an
 * already-valid pair, hiding the very hazard it described. Write the escape.
 *
 * Scope: every `.ts`, `.tsx`, `.mts` and `.css` file under `ui/desktop/src`,
 * not just the sanitizer modules — the walk costs well under a second and
 * found no other hit. It skips the top-level `web/` and `bin/` (ignored build
 * output and staged binaries, never source) and any `node_modules`.
 */
describe('source hygiene', () => {
  // Braced escapes only, so this file never holds the characters it hunts.
  const BIDI_CONTROL = /[\u{202A}-\u{202E}\u{2066}-\u{2069}\u{200E}\u{200F}\u{061C}]/u;

  it('recognises every character it guards against', () => {
    const guarded = [
      0x202a, 0x202b, 0x202c, 0x202d, 0x202e, 0x2066, 0x2067, 0x2068, 0x2069, 0x200e, 0x200f,
      0x061c,
    ];
    for (const codePoint of guarded) {
      expect(BIDI_CONTROL.test(`a${String.fromCodePoint(codePoint)}b`)).toBe(true);
    }
    // The escape spelling is what a source file should hold instead.
    expect(BIDI_CONTROL.test('`\\uD800\\u2066\\uDC00`')).toBe(false);
  });

  it('holds no literal bidirectional control character in any renderer source', () => {
    const root = join(__dirname, '..');
    const skipAtRoot = new Set(['web', 'bin']);
    const hits: string[] = [];
    let scanned = 0;

    const walk = (directory: string) => {
      for (const entry of readdirSync(directory, { withFileTypes: true })) {
        const path = join(directory, entry.name);
        if (entry.isDirectory()) {
          if (entry.name === 'node_modules') continue;
          if (directory === root && skipAtRoot.has(entry.name)) continue;
          walk(path);
        } else if (/\.(tsx?|mts|css)$/.test(entry.name)) {
          scanned += 1;
          readFileSync(path, 'utf8')
            .split('\n')
            .forEach((line, index) => {
              if (BIDI_CONTROL.test(line)) {
                hits.push(`${path.slice(root.length + 1)}:${index + 1}`);
              }
            });
        }
      }
    };
    walk(root);

    // A walk that reads nothing would agree with a walk that finds nothing.
    expect(scanned).toBeGreaterThan(200);
    expect(hits).toEqual([]);
  });
});
