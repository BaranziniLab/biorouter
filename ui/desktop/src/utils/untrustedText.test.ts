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
    [
      'direction marks in text with no right-to-left letter',
      'invoice\u{200F}.exe\u{200E}\u{61C}',
      3,
    ],
    // Every other character that draws nothing, taken by category, not only the three
    // zero-width characters the first version knew: each of these left `@crew_bob` looking whole.
    ['a zero-width non-joiner inside a handle', '@cre\u{200C}w_bob', 1],
    ['a zero-width joiner inside a handle', '@cre\u{200D}w_bob', 1],
    ['an invisible separator inside a handle', '@cre\u{2063}w_bob', 1],
    ['the invisible operators', 'a\u{2061}b\u{2062}c\u{2064}d', 3],
    ['the Mongolian vowel separator inside a handle', '@cre\u{180E}w_bob', 1],
    ['the deprecated format characters', 'a\u{206A}b\u{206F}c', 2],
    ['the combining grapheme joiner', 'cre\u{34F}w_bob', 1],
    ['a variation selector between ASCII letters', 'cre\u{FE0F}w_bob and cre\u{FE00}w', 2],
    ['an ideographic variation selector between ASCII letters', 'cre\u{E0100}w_bob', 1],
    ['a soft hyphen between ASCII letters', 'cre\u{AD}w_bob', 1],
    ['a Hangul filler beside a name', '@crew\u{3164}bob', 1],
    ['a run of invisible characters, each one', 'cre\u{200B}\u{200C}\u{200D}w_bob', 3],
    [
      'an invisible character between two visible escapes, looked through to the letters',
      'cre\u{202E}\u{200C}\u{202C}w',
      3,
    ],
    ['a joiner between a Thai word and a domain', 'ดู\u{200D}ucsf.edu', 1],
    ['a zero-width space inside a domain written in another script', 'пример\u{200B}сайт.рф', 1],
    ['a keycap selector with no keycap after it', 'cre1\u{FE0F}w', 1],
    [
      'a direction mark inside a handle, in a message with right-to-left words',
      'שלום @cre\u{200F}w_bob and cre\u{200E}w_\u{200F}bob',
      3,
    ],
    // Text hidden in text, for an agent to read where no person can.
    [
      'a run of zero-width characters between spaces',
      'hello \u{200B}\u{200C}\u{200B}\u{200C} there',
      4,
    ],
    [
      'variation selectors after an emoji, past the first',
      '\u{1F600}\u{FE00}\u{E0101}\u{E0102}!',
      2,
    ],
    [
      'a flag whose tags spell more than a subdivision',
      '\u{1F3F4}\u{E0069}\u{E0067}\u{E006E}\u{E006F}\u{E0072}\u{E0065}\u{E0020}\u{E0061}\u{E007F}',
      9,
    ],
    ['a flag spelled in capital tags', '\u{1F3F4}\u{E0047}\u{E0042}\u{E0053}\u{E007F}', 4],
  ])('shows %s', (_label, value, count) => {
    const segments = revealHiddenCharacters(value);
    expect(segments.filter((part) => part.kind === 'hidden')).toHaveLength(count);
    expect(raw(segments)).toBe(value);
  });

  it.each([
    ['a Hebrew paragraph', 'שלום לכולם, הפגישה בשעה 3.'],
    ['the direction marks of right-to-left text', 'שלום\u{200F} (C++)\u{200E} مرحبا\u{61C}'],
    ['a mark after a Latin word before its full stop', 'אני משתמש ב-Windows\u{200E}. תודה'],
    [
      'an emoji joiner sequence',
      'scientist \u{1F469}\u{1F3FD}\u{200D}\u{1F52C} and family \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}',
    ],
    ['a zero-width non-joiner in Persian', 'می\u{200C}خواهم'],
    ['a zero-width space between Thai words', 'สวัสดี\u{200B}ครับ'],
    ['a subdivision flag', 'go \u{1F3F4}\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}!'],
    ['a state flag', 'from \u{1F3F4}\u{E0075}\u{E0073}\u{E0074}\u{E0078}\u{E007F}'],
    ['the emoji smuggling test’s one selector', '\u{1F600}\u{FE0F} ok'],
    ['tabs and newlines', 'a\tb\nc\r\nd'],
    ['a soft hyphen between letters of another script', 'авто\u{AD}мобиль'],
    ['a joiner in an Indic conjunct', 'क्\u{200D}ष'],
    ['an emoji presentation selector', 'love \u{2764}\u{FE0F}it, \u{A9}\u{FE0F}2026'],
    [
      'a joiner sequence with a selector inside',
      'heart on fire \u{2764}\u{FE0F}\u{200D}\u{1F525}go',
    ],
    ['a keycap', 'press 1\u{FE0F}\u{20E3} or #\u{FE0F}\u{20E3} now'],
    ['an ideographic variation selector', '葛\u{E0100}城'],
    ['an emoji in a handle-like token', '@bob\u{1F469}\u{200D}\u{1F52C} \u{2764}\u{FE0F}@lab.org'],
    ['an Arabic number sign before ASCII digits', 'total \u{600}123'],
    ['a Mongolian free variation selector between Mongolian letters', 'ᠠ\u{180B}ᠢ'],
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

  /**
   * It runs on the renderer's main thread each time a row mounts, on up to 64 KB (the broker's
   * limit) that somebody else chose. The first version rescanned the whole token for every
   * zero-width character and the whole tag run for every tag, so one message stalled every viewer:
   * 21,800 zero-width spaces took 8 s, `@crew_bob` and 21,700 of them 20 s, a flag and 16,290 tags
   * 4 s. One pass takes a few milliseconds; the bound leaves room for a loaded machine and none
   * for a quadratic scan.
   */
  describe('within a bounded time, on a 64 KB body built to be slow', () => {
    const LIMIT_BYTES = 64 * 1024;
    const bytes = (value: string) => new TextEncoder().encode(value).length;
    /** `unit` repeated to just under the broker's limit. */
    const fill = (unit: string, lead = '') =>
      lead + unit.repeat(Math.floor((LIMIT_BYTES - bytes(lead)) / bytes(unit)));

    it.each([
      ['zero-width spaces alone', fill('\u{200B}'), 'hidden'],
      ['a letter and a zero-width space', fill('a\u{200B}'), 'hidden'],
      ['a letter and a word joiner', fill('a\u{2060}'), 'hidden'],
      ['a handle, then zero-width spaces', fill('\u{200B}', '@crew_bob'), 'hidden'],
      ['a domain, then zero-width non-joiners', fill('\u{200C}', 'ucsf.edu'), 'hidden'],
      ['tag characters alone', fill('\u{E0041}'), 'hidden'],
      ['a black flag, then tag characters', fill('\u{E0067}', '\u{1F3F4}'), 'hidden'],
      // Far more tags than a subdivision's code: shown, flag or not.
      [
        'a black flag, tag characters and a cancel tag',
        `${fill('\u{E0067}', '\u{1F3F4}').slice(0, -2)}\u{E007F}`,
        'hidden',
      ],
      ['an emoji, then variation selectors', fill('\u{E0100}', '\u{1F600}'), 'hidden'],
      ['Thai words and zero-width spaces', fill('สวัสดี\u{200B}'), 'text'],
      ['a plain line', fill('a'), 'text'],
    ])('%s', (_label, value, kind) => {
      expect(bytes(value)).toBeLessThanOrEqual(LIMIT_BYTES);
      expect(bytes(value)).toBeGreaterThan(LIMIT_BYTES - 64);
      const started = performance.now();
      const segments = revealHiddenCharacters(value);
      const elapsed = performance.now() - started;
      expect(segments.map((part) => (part.kind === 'text' ? part.text : part.raw)).join('')).toBe(
        value
      );
      expect(segments.some((part) => part.kind === kind)).toBe(true);
      expect(elapsed).toBeLessThan(750);
    });
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
