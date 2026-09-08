import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * The settings visual vocabulary, enforced at the SOURCE.
 *
 * **Why a source test and not a render test.** Every rule below is about a
 * class STRING, and jsdom never runs Tailwind: `bg-background-medium/70`
 * computes to nothing there, so a component test can mount a row, toggle its
 * switch, read `backgroundColor`, and see `''` in both states — passing
 * identically whether the fill exists or not. Two of these rules are worse than
 * invisible to a render test, because the defect only appears in the CASCADE:
 * `.biorouter-settings-row:hover` is unlayered and beats a `@layer utilities`
 * background, which is what made an "on" row visibly LIGHTEN under the pointer.
 * The same argument `styles/measures.test.ts` and `styles/composerFocus.test.ts`
 * make for the stylesheet, made here for the call sites.
 *
 * The vocabulary itself is written up in
 * `docs/desktop-ui/settings-visual-vocabulary.md`; this file is its teeth.
 *
 * ⚠ **The exclusions are the PR's stated scope, not an amnesty.** Extensions,
 * the provider-configuration page, the permission modals, dictation, the tunnel
 * and session sharing share these primitives and will inherit the rules; they
 * were left out because sweeping them triples the diff. Deleting a name from
 * this list is how that work gets finished — adding one is how the rules rot.
 */
const SETTINGS_DIR = __dirname;

/**
 * Every directory the vocabulary now governs. Settings is where it was written
 * down; each further entry is a surface that has since been swept onto it.
 * One root per line, so two PRs sweeping two different surfaces add two
 * different lines and merge without touching each other's.
 */
const ROOTS = [
  SETTINGS_DIR,
  join(SETTINGS_DIR, '../schedule'), // the Scheduler, 2026-09-07
];

const OUT_OF_SCOPE = [
  'extensions/',
  'providers/',
  'permission/',
  'dictation/',
  'tunnel/',
  'sessions/',
];

function sourceFiles(): { path: string; rel: string; text: string }[] {
  const found: { path: string; rel: string; text: string }[] = [];
  const walk = (root: string, dir: string) => {
    for (const entry of readdirSync(dir)) {
      const path = join(dir, entry);
      if (statSync(path).isDirectory()) {
        walk(root, path);
        continue;
      }
      if (!entry.endsWith('.tsx') || entry.includes('.test.')) continue;
      // ⚠ Two different relative paths, on purpose. `OUT_OF_SCOPE` names
      // subfolders of the root being walked, so the exclusion is tested
      // against the path relative to THAT root; the name a failure reports
      // stays relative to the settings directory, so one list can name files
      // from two roots without two `SchedulesView.tsx`-shaped ambiguities.
      if (OUT_OF_SCOPE.some((prefix) => relative(root, path).startsWith(prefix))) continue;
      found.push({ path, rel: relative(SETTINGS_DIR, path), text: readFileSync(path, 'utf8') });
    }
  };
  for (const root of ROOTS) walk(root, root);
  return found;
}

const FILES = sourceFiles();

/**
 * Extract the whole `className` expression a match sits inside — the string
 * literal, the template literal or the `cn(...)` call — so a rule can read a
 * conditional that spans several lines. A line-based regex cannot: prettier
 * reflows exactly these attributes.
 */
function classNameExpressions(text: string): string[] {
  const expressions: string[] = [];
  const attribute = /className=/g;
  let match: RegExpExecArray | null;
  while ((match = attribute.exec(text)) !== null) {
    let index = match.index + match[0].length;
    if (text[index] === '"' || text[index] === "'") {
      const quote = text[index];
      const end = text.indexOf(quote, index + 1);
      if (end === -1) continue;
      expressions.push(text.slice(index + 1, end));
      continue;
    }
    if (text[index] !== '{') continue;
    let depth = 0;
    const start = index;
    for (; index < text.length; index += 1) {
      if (text[index] === '{') depth += 1;
      else if (text[index] === '}') {
        depth -= 1;
        if (depth === 0) break;
      }
    }
    expressions.push(text.slice(start + 1, index));
  }
  return expressions;
}

/** Every `className` expression in the tree that paints a settings row. */
function rowExpressions(): { rel: string; expression: string }[] {
  return FILES.flatMap(({ rel, text }) =>
    classNameExpressions(text)
      .filter((expression) => expression.includes('biorouter-settings-row'))
      .map((expression) => ({ rel, expression }))
  );
}

/**
 * The opening tag of every `<Name …>` in a file.
 *
 * ⚠ A non-greedy `/<Button[\s\S]*?>/` does NOT work and passes silently: the
 * first `>` it finds is the one inside `onClick={() => …}`, so the match ends
 * before `className` is ever reached and the rule below reports nothing. Brace
 * depth is what separates a JSX expression's `>` from the tag's own.
 */
function openingTags(text: string, name: string): string[] {
  const tags: string[] = [];
  const opener = new RegExp(`<${name}\\b`, 'g');
  let match: RegExpExecArray | null;
  while ((match = opener.exec(text)) !== null) {
    let depth = 0;
    for (let index = match.index; index < text.length; index += 1) {
      const character = text[index];
      if (character === '{') depth += 1;
      else if (character === '}') depth -= 1;
      else if (character === '>' && depth === 0) {
        tags.push(text.slice(match.index, index + 1));
        break;
      }
    }
  }
  return tags;
}

function offenders(pattern: RegExp): string[] {
  return FILES.filter(({ text }) => pattern.test(text)).map(({ rel }) => rel);
}

describe('the settings vocabulary', () => {
  it('finds the settings sources at all', () => {
    // A walker that silently matched nothing would make every rule below vacuous.
    expect(FILES.length).toBeGreaterThan(15);
    expect(rowExpressions().length).toBeGreaterThan(8);
  });

  /**
   * Per-root, not just in total: `FILES.length` above is satisfied by the
   * settings directory alone, so a root that resolves to nothing — a typo, a
   * renamed folder — would add its name to the list and change no assertion.
   */
  it('finds sources under every root it claims to govern', () => {
    for (const root of ROOTS) {
      expect(FILES.filter(({ path }) => path.startsWith(root)).length).toBeGreaterThan(0);
    }
  });

  /**
   * V1, and the single most valuable assertion here: it is what stops the
   * reported defect coming back. A row's fill may not depend on its state — the
   * switch, the radio or the checkbox already says it, a second fill says it in
   * a weaker language, and the unlayered hover rule INVERTS it.
   */
  it('never makes a row’s fill depend on its state', () => {
    const shaded = rowExpressions().filter(
      ({ expression }) => expression.includes('?') && /\bbg-[a-z]/.test(expression)
    );
    expect(
      shaded.map(({ rel, expression }) => `${rel}: ${expression.replace(/\s+/g, ' ').trim()}`)
    ).toEqual([]);
  });

  /**
   * V2. One row: `px-3 py-2.5`, and the height comes from `--row-height` rather
   * than from a per-call-site `min-h-*`. A row that carries NO padding is
   * allowed and is not an oversight — `ResetPanel`'s row class sits on a
   * `Collapsible` root whose header and its expanded body are padded
   * separately.
   */
  it('gives every row the same padding, or none at all', () => {
    const wrong = rowExpressions().filter(({ expression }) => {
      if (/\bmin-h-/.test(expression)) return true;
      // `(?![\d.])` matters: a bare \b after the 2 is satisfied by the dot in
      // `py-2.5`, so the naive spelling of this rule fails every correct row.
      if (/\bpy-(?:1\.5|2|3|4)(?![\d.])/.test(expression)) return true;
      const padded = /\bp[xy]-/.test(expression);
      return padded && !(expression.includes('px-3') && expression.includes('py-2.5'));
    });
    expect(
      wrong.map(({ rel, expression }) => `${rel}: ${expression.replace(/\s+/g, ' ').trim()}`)
    ).toEqual([]);
  });

  /**
   * V4. `border-borderStandard` is not a token — it renders only because of the
   * `@layer base` `border-color` fallback — `rounded-lg` is a deprecated alias
   * of `--radius-element`, and `text-iconStandard` has no definition at all, so
   * every use of it was a no-op that read as intent.
   */
  it.each(['border-borderStandard', 'rounded-lg', 'text-iconStandard'])('has no `%s`', (banned) => {
    // Prose in a comment explaining why the class is gone is not a use of it.
    const uses = FILES.filter(({ text }) =>
      classNameExpressions(text).some((expression) => expression.includes(banned))
    ).map(({ rel }) => rel);
    expect(uses).toEqual([]);
  });

  /**
   * V8. The OS confirmation is theme-blind and unstyleable; it was the one
   * control in Settings that could not be read in dark mode.
   */
  it('confirms through the app’s own primitive, never `window.confirm`', () => {
    expect(offenders(/window\.confirm\(/)).toEqual([]);
  });

  /**
   * V7. `--control-compact`'s own comment: the 24px tier "exists for a
   * glyph-only control inside an already-dense cluster; a control carrying a
   * label never uses it". Nothing in these three tabs is that control, so the
   * narrow form of the rule — none at all — is the one that can be checked
   * without guessing at JSX structure.
   */
  it('puts no control on the 24px compact rung', () => {
    expect(offenders(/size="xs"/)).toEqual([]);
  });

  /**
   * V7. The cva base already emits `inline-flex items-center justify-center
   * gap-2`, and a bare `flex` FLIPS that `inline-flex` through tailwind-merge —
   * which is what rendered `ResetProviderSection`'s destructive button as a
   * full-width red bar.
   */
  it('never re-declares a Button’s own layout on the call site', () => {
    const uses: string[] = [];
    for (const { rel, text } of FILES) {
      for (const tag of openingTags(text, 'Button')) {
        if (/className="[^"]*\bflex\b[^"]*"/.test(tag)) uses.push(`${rel}: ${tag.trim()}`);
      }
    }
    expect(uses).toEqual([]);
  });
});
