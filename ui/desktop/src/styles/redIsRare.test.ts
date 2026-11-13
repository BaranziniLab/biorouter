import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { describe, expect, it } from 'vitest';

import almaMater from '../../themes/alma-mater.theme.mjs';
import parchment from '../../themes/parchment.theme.mjs';
import rocheLimit from '../../themes/roche-limit.theme.mjs';

/**
 * Red is rare (design.md 9.14; owner, 2026-11-12). Red never marks a failure, a
 * risk, a permission or a destructive action in Biorouter: words and a glyph do,
 * in ink. The rule is built into the tokens so that UI written later follows it
 * by default, and this file is what keeps it there.
 *
 * Two halves, because the rule has two ways to drift:
 * 1. A family re-points its danger tone at a red again. Every `danger` Badge,
 *    Note, StatusDot, Progress fill, destructive Button and menu item would turn
 *    red at once, with no call site changing.
 * 2. Someone reaches for the one red left, `--border-danger`, to colour a card,
 *    a banner or a status. It is the edge of an invalid field or of a drop zone
 *    that refused a file, and nothing else; a new use has to be added to the
 *    list below, a deliberate act with a reason, not a quiet divergence.
 */

const FAMILIES = {
  parchment,
  'alma-mater': almaMater,
  'roche-limit': rocheLimit,
} as const;

type Tokens = Record<string, string>;

describe.each(['light', 'dark'] as const)('%s: the danger tone is ink, not red', (mode) => {
  it.each(Object.keys(FAMILIES) as (keyof typeof FAMILIES)[])('%s', (id) => {
    const tokens = FAMILIES[id][mode].tokens as Tokens;
    // The ink a failure, an error note or a "Delete" item is written in.
    expect(tokens['text-danger']).toBe(tokens['text-default']);
    // The fill a failed dot, a full gauge or a destructive button's tint mixes.
    expect(tokens['background-danger']).toBe(tokens['text-muted']);
    // The invalid edge keeps the family's red: it is one of the two reds left,
    // so it must not collapse into the ink either.
    expect(tokens['border-danger']).not.toBe(tokens['text-default']);
    expect(tokens['border-danger']).not.toBe(tokens['text-muted']);
    // The danger zone's button keeps the family red too (owner, 2026-11-12).
    expect(tokens['text-critical']).not.toBe(tokens['text-default']);
    expect(tokens['background-critical']).not.toBe(tokens['text-muted']);
  });
});

const STYLES = join(__dirname);
const SRC = join(__dirname, '..');
const mainCss = readFileSync(join(STYLES, 'main.css'), 'utf8');

/** The hand-authored base blocks (Parchment) come first: `:root`, then `.dark`. */
function baseDeclarations(token: string): string[] {
  return [...mainCss.matchAll(new RegExp(`^\\s*--${token}:\\s*([^;]+);`, 'gm'))]
    .map((m) => m[1].trim())
    .slice(0, 2);
}

describe('the hand-authored base blocks agree', () => {
  it('Parchment light and dark draw danger in their own ink', () => {
    expect(baseDeclarations('text-danger')).toEqual(baseDeclarations('text-default'));
    expect(baseDeclarations('background-danger')).toEqual(baseDeclarations('text-muted'));
  });

  it('the danger wash is a light neutral step, not a 22% slab', () => {
    // 22% of a near-black ink is a mid-grey box; 8% sits at about the strong
    // fill step. `check-contrast.mjs` reads N from this declaration.
    expect(mainCss).toMatch(
      /--wash-danger: color-mix\(in oklab, var\(--text-danger\) 8%, transparent\);/
    );
  });
});

function sourceFiles(dir: string): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) {
      // `web` is the built `biorouter serve` bundle and `bin` the staged binaries: output, not source.
      if (['node_modules', 'api', 'web', 'bin'].includes(name)) continue;
      out.push(...sourceFiles(path));
    } else if (/\.(tsx?|css)$/.test(name) && !/\.test\.|\.d\.ts$/.test(name)) {
      out.push(path);
    }
  }
  return out;
}

const FILES = sourceFiles(SRC).map((path) => ({
  path,
  rel: relative(SRC, path).split(sep).join('/'),
  text: readFileSync(path, 'utf8'),
}));

/**
 * Every place the invalid edge may be drawn. Each one marks a field whose value
 * is invalid or a drop zone (or attachment) that refused a file.
 */
const INVALID_EDGE_FILES = new Set([
  'styles/main.css', // the token itself, its mirror and `.br-field[aria-invalid]`
  'components/ui/input.tsx',
  'components/ui/textarea.tsx',
  'components/ui/JsonSchemaForm.tsx',
  'components/settings/app/ExternalBackendSection.tsx',
  'components/workflows/shared/WorkflowFormFields.tsx',
  'components/SecretRequestCard.tsx',
  'components/crew/pane/CrewModelPicker.tsx',
  'components/composer/composer.css', // an attachment thumbnail that failed to load
  'components/workflows/workflows.css', // drop zone that refused a file
  'components/sessions/history.css', // drop zone that refused a file
  'components/skills/skills.css', // drop zone that refused a file
  'components/brxt-install.css', // drop zone that refused a file
]);

describe('the one red left is the invalid edge', () => {
  it('only the listed files draw `--border-danger`', () => {
    const users = FILES.filter(({ text }) =>
      /border-border-danger|var\(--border-danger\)/.test(text)
    ).map(({ rel }) => rel);
    expect(users.filter((rel) => !INVALID_EDGE_FILES.has(rel))).toEqual([]);
  });

  it('only Button `critical`, the danger zone button, wears the critical red', () => {
    const users = FILES.filter(({ text }) => /background-critical|text-critical/.test(text)).map(
      ({ rel }) => rel
    );
    expect(
      users.filter((rel) => !['styles/main.css', 'components/ui/button.tsx'].includes(rel))
    ).toEqual([]);
  });

  it('no chrome reaches for a red palette utility or the raw red ramp', () => {
    const offenders = FILES.filter(
      ({ rel, text }) =>
        rel !== 'styles/main.css' &&
        (/\b(?:text|bg|border|ring|fill|stroke|outline|decoration|from|via|to)-red-\d{2,3}\b/.test(
          text
        ) ||
          /var\(--color-red-\d{3}\)/.test(text))
    ).map(({ rel }) => rel);
    expect(offenders).toEqual([]);
  });
});

describe('permission prompts ask for attention without alarm', () => {
  it('no risk grade is drawn in the danger tone', () => {
    const preview = FILES.find(({ rel }) => rel === 'components/ToolCallPreview.tsx');
    const block = preview?.text.match(/const RISK_TONES[^}]+}/)?.[0] ?? '';
    expect(block).toContain("high: 'strong'");
    expect(block).not.toContain("'danger'");
  });
});
