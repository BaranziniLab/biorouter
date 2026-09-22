import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * A menu option row is a 32px FLOOR, not a 32px box.
 *
 * The bug: `classNames.option` carried `h-8`, a fixed 32px height. That is only
 * correct while every option is one line. The `formatOptionLabel` call sites —
 * the model picker — render a title plus a wrapped detail line, ~65px of
 * content, and react-select's option is `overflow: visible`, so the surplus did
 * not clip: it SPILLED over the rows beneath. Spilled content still hit-tests,
 * so `document.elementFromPoint` at row N's centre returned row N-1, and the
 * user silently selected the model ABOVE the one they clicked. Nothing looked
 * wrong; the wrong model was configured.
 *
 * The fix is `min-h-8` plus `py-1`: a single-line row still sits on the shared
 * 32px menu-row rung (design.md §3.8), and a two-line row owns its real height,
 * so the rows below it are pushed down instead of painted over.
 *
 * ⚠ **This is asserted at the SOURCE, and it has to be.** jsdom has no layout
 * engine, and both halves of this bug are pure layout:
 *
 *   - `getBoundingClientRect()` returns all-zeroes for every node, so a rendered
 *     option is 0px tall whether its class says `h-8` or `min-h-8` — and nothing
 *     can overflow a box with no size, so nothing spills.
 *   - `document.elementFromPoint()`, the exact API whose answer the bug
 *     corrupts, is a hit-test against a layout that does not exist; jsdom has no
 *     meaningful answer to give for any coordinate.
 *
 * A component test that mounted the menu, measured a row and probed its centre
 * would therefore pass identically with the bug present and with it fixed. The
 * only thing assertable here is the declaration, so that is what is asserted.
 * Measured in a real browser when the fix landed: BEFORE, a two-line option
 * computed `height: 32px` around a 65px content box and `elementFromPoint` at
 * the next row's centre returned the option above it; AFTER, the row computes
 * its own height and each row answers for itself.
 */
const SELECT = readFileSync(join(__dirname, 'Select.tsx'), 'utf8');

/**
 * A STANDALONE `h-8` class token.
 *
 * ⚠ **This regex is the trap in this file, and it has two halves.** `min-h-8`
 * contains the substring `h-8`, so:
 *
 *   - `expect(base).not.toContain('h-8')` is unusable — it fails against the
 *     CORRECT string. The temptation is then to weaken it into something that no
 *     longer catches the bug at all.
 *   - `/\bh-8\b/` is no better, and is the more dangerous mistake because it
 *     LOOKS boundary-aware. `\b` is a *word* boundary and `-` is a non-word
 *     character, so `min-h-8` carries a word boundary immediately before `h`:
 *     the naive word-boundary regex matches the correct string too.
 *
 * A Tailwind token can be preceded by `-` (`min-h-8`, `max-h-8`) as well as by a
 * word character, so the guard has to exclude BOTH on both sides.
 */
const FIXED_H8_TOKEN = /(?<![\w-])h-8(?![\w-])/;

/** Any bare `h-*` token — `h-9`, `h-[65px]`, `h-full` reintroduce the same bug. */
const ANY_FIXED_HEIGHT_TOKEN = /(?<![\w-])h-[^\s'"]+/;

/**
 * The `classNames.option` renderer, sliced off the file so the assertions below
 * cannot read some other part of it.
 *
 * ⚠ The scoping is load-bearing: the TRIGGER legitimately keeps a fixed `h-8`
 * (`'flex h-8 w-full items-center …'`, pinned by Select.test.ts) — it is a
 * single-line control on the same rung. A file-wide ban on `h-8` would fail
 * against correct code. Only the option row has to be a floor.
 *
 * Keyed on the destructured state argument (`option: ({ isFocused, … })`), which
 * tells it apart from the `styles.option` entry further down (`option: (base) =>`)
 * without depending on which of the two appears first in the file.
 */
function optionRenderer(): string {
  const start = SELECT.search(/option:\s*\(\{/);
  if (start < 0) throw new Error('Select.tsx declares no classNames.option entry');
  return SELECT.slice(start);
}

/** The `const base = '…'` class string every option branch is built from. */
function optionBaseClasses(): string {
  const match = optionRenderer().match(/const base =\s*(['"])([^'"]*)\1/);
  if (!match) throw new Error("the option renderer declares no `const base = '…'` class string");
  return match[2];
}

describe('the shared Select option row geometry', () => {
  /**
   * The guard is a regex, so it is worth something only if it can tell the two
   * spellings apart. Asserting that directly means a future "simplification"
   * into `/h-8/` or `/\bh-8\b/` fails HERE, loudly, rather than quietly passing
   * every other test in this file for the rest of time.
   */
  it('has a guard that rejects a bare `h-8` and accepts `min-h-8`', () => {
    expect('flex h-8 items-center px-2').toMatch(FIXED_H8_TOKEN);
    expect('flex h-8 items-center px-2').toMatch(ANY_FIXED_HEIGHT_TOKEN);
    expect('flex min-h-8 items-center px-2 py-1').not.toMatch(FIXED_H8_TOKEN);
    expect('flex min-h-8 items-center px-2 py-1').not.toMatch(ANY_FIXED_HEIGHT_TOKEN);
    // The mistake this whole comment exists to prevent, made executable: the
    // naive checks cannot distinguish the fix from the bug.
    expect('min-h-8'.includes('h-8')).toBe(true);
    expect(/\bh-8\b/.test('min-h-8')).toBe(true);
  });

  it('keeps the 32px menu rung, as a floor', () => {
    expect(optionBaseClasses().split(/\s+/)).toContain('min-h-control-md');
  });

  it('pins no fixed height on a row that can wrap to two lines', () => {
    const base = optionBaseClasses();
    expect(base).not.toMatch(FIXED_H8_TOKEN);
    // Reported as the offending token rather than as a bare boolean, so a
    // failure names what it found.
    expect(base.match(ANY_FIXED_HEIGHT_TOKEN)?.[0] ?? null).toBeNull();
  });

  /**
   * `min-h-8` alone would let a two-line row's ink run edge to edge against the
   * rows above and below it. The padding is what makes the floor a floor: it is
   * inert on a single-line row (which the 32px minimum already governs) and is
   * the only vertical breathing room a taller row gets.
   */
  it('gives the row vertical padding', () => {
    expect(optionBaseClasses().split(/\s+/)).toContain('py-1');
  });
});
