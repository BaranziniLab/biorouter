import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * Red is rare in the decision UI (design.md 9.14; owner, 2026-11-12). A failed or
 * refused decision, a monitor that could not be saved, a gate that waits for the
 * person: each is said in words, in the primary ink, with the failure glyph
 * (`AlertCircle`) for a failure and a neutral chip for something that needs the
 * person. None of them reaches for the danger tone, a status-hued paragraph or a
 * colour of its own.
 *
 * Read from the source, because jsdom computes no colour from these stylesheets: a
 * component test that read `color` would pass with the red still there.
 *
 * Destructive actions (Remove a monitor, Remove a gate) keep their `destructive`
 * flag: the shared primitives draw it calm at the token layer (9.13 "how it is
 * built"), so it is not a call site's to undo.
 */
const DIR = __dirname;

const SOURCES = readdirSync(DIR)
  .filter((name) => /\.(tsx?|css)$/.test(name) && !/\.test\./.test(name))
  .map((name) => ({ name, text: readFileSync(join(DIR, name), 'utf8') }));

describe('the decision UI marks no failure, refusal or attention in red', () => {
  it('reads its sources', () => {
    expect(SOURCES.map(({ name }) => name)).toEqual(
      expect.arrayContaining([
        'DecisionCard.tsx',
        'DecisionHistory.tsx',
        'MonitorsDialog.tsx',
        'PhaseGateDialog.tsx',
        'decisionsChat.css',
        'decisionsPanel.css',
      ])
    );
  });

  it.each([
    ['the danger ink', /var\(--text-danger\)|text-text-danger/],
    [
      'the danger wash or fill',
      /--wash-danger|bg-wash-danger|--background-danger|bg-background-danger/,
    ],
    [
      'a danger tone',
      /tone=["']danger["']|tone:\s*'danger'|data-tone=["']danger["']|'danger'\s*:\s*undefined|\?\s*'danger'/,
    ],
    ['the invalid edge, which belongs to a field', /var\(--border-danger\)|border-border-danger/],
    ['a red utility', /--color-red-|\b(?:text|bg|border)-red-\d/],
    ['a paragraph in the warning hue', /<Note[^>]*tone=["']warning["']/],
  ])('uses no %s', (_what, pattern) => {
    const offenders = SOURCES.filter(({ text }) => pattern.test(text)).map(({ name }) => name);
    expect(offenders).toEqual([]);
  });

  it('declares no colour of its own in its stylesheets: tokens only', () => {
    const offenders = SOURCES.filter(
      ({ name, text }) =>
        name.endsWith('.css') && /#[0-9a-fA-F]{3,8}\b|\brgba?\(|\bhsla?\(/.test(text)
    ).map(({ name }) => name);
    expect(offenders).toEqual([]);
  });
});
