import { readdirSync, readFileSync } from 'node:fs';
import { basename, dirname, join, relative, sep } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * Two cascade hazards for the composer and files packages, held at the source because jsdom
 * loads no stylesheet and a component test passes whether or not either one bites.
 *
 * 1. **Names shared with the legacy stylesheet.** `crew/crew.css` stays global for as long as
 *    `CrewView.tsx` imports the legacy layout, and its rules are unlayered. It styled
 *    `.crew-composer` (and every `textarea` in one: a 75–200px resizable box that beat the
 *    composer's one-row auto-grow) and `.crew-attachment` (10px padding and a 10px margin on a
 *    40px row). So no class these packages write may be one it defines, no class these packages
 *    style may be one the legacy markup writes, and no class these packages style may be one
 *    another area's stylesheet also styles.
 * 2. **Unlayered paint beating the D-15 focus fill.** The fill lives in `@layer base`; a
 *    `color` or `background-color` set here on a raw focusable element wins over it whatever
 *    the specificity, and the base rule's `outline: none` still applies — focus disappears. Every
 *    such element restates the fill under `:focus-visible`.
 *
 * Each detector is run on a bad fixture below, so the guard cannot pass by failing to look.
 */

const FILES_DIR = __dirname;
const CREW_DIR = dirname(FILES_DIR);
const COMPOSER_DIR = join(CREW_DIR, 'composer');
const LEGACY_CSS = join(CREW_DIR, 'crew.css');
const LEGACY_DIR = join(CREW_DIR, 'legacy');
const OWN_DIRS = [COMPOSER_DIR, FILES_DIR];

function walk(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return entry.name === 'node_modules' ? [] : walk(path);
    return [path];
  });
}

const rel = (path: string) => relative(CREW_DIR, path).split(sep).join('/');
const isOwn = (path: string) => OWN_DIRS.some((dir) => path.startsWith(dir + sep));
const isTest = (path: string) => /\.test\.tsx?$/.test(path);
const isMarkup = (path: string) => /\.tsx?$/.test(path) && !path.endsWith('.d.ts');

const read = (path: string) => readFileSync(path, 'utf8');

/** Source with block and line comments blanked, so prose naming a class is not a use of it. */
function stripComments(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\n]/g, ' '))
    .replace(/(^|[^:'"`\\])\/\/[^\n]*/g, '$1');
}

/** Every `crew-*` token markup writes (a class, a test id): not part of `data-crew-*` or `--crew-*`. */
function markupTokens(source: string): Set<string> {
  return new Set(stripComments(source).match(/(?<![\w-])crew-[\w-]+/g) ?? []);
}

/** Every `.crew-*` class a stylesheet's selectors name. */
function stylesheetClasses(source: string): Set<string> {
  const names = new Set<string>();
  for (const rule of leafRules(source)) {
    for (const match of rule.selector.matchAll(/\.(crew-[\w-]+)/g)) names.add(match[1]);
  }
  return names;
}

interface LeafRule {
  selector: string;
  declarations: Map<string, string>;
}

/** The innermost `selector { declarations }` blocks of a stylesheet. */
function leafRules(source: string): LeafRule[] {
  const text = stripComments(source);
  return [...text.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map(([, selector, body]) => ({
    selector: selector.replace(/\s+/g, ' ').trim(),
    declarations: new Map(
      body
        .split(';')
        .map((part) => part.trim())
        .filter((part) => part.includes(':'))
        .map((part) => {
          const colon = part.indexOf(':');
          return [part.slice(0, colon).trim().toLowerCase(), part.slice(colon + 1).trim()];
        })
    ),
  }));
}

const intersect = (left: Set<string>, right: Set<string>) =>
  [...left].filter((name) => right.has(name)).sort();

/** The crew classes on raw focusable elements: a `<button>` or anything with a tab stop. */
function focusableClasses(source: string): Set<string> {
  const names = new Set<string>();
  const text = stripComments(source);
  const tags = [
    ...(text.match(/<button\b[^<>]*>/g) ?? []),
    ...(text.match(/<[a-z][\w-]*\b[^<>]*\btabIndex=\{0\}[^<>]*>/g) ?? []),
  ];
  for (const tag of tags) {
    const className = /\bclassName="([^"]*)"/.exec(tag)?.[1] ?? '';
    for (const name of className.split(/\s+/)) if (name.startsWith('crew-')) names.add(name);
  }
  return names;
}

const PAINT = ['color', 'background-color', 'background'];

/** Focusable classes whose resting paint here would beat the D-15 fill, with no restatement. */
function focusFillViolations(css: string, focusable: Set<string>): string[] {
  const rules = leafRules(css);
  const problems: string[] = [];
  for (const name of [...focusable].sort()) {
    const paints = rules.some(
      (rule) =>
        rule.selector.split(',').some((part) => part.trim() === `.${name}`) &&
        PAINT.some((property) => rule.declarations.has(property))
    );
    if (!paints) continue;
    const focus = rules.find((rule) =>
      rule.selector.split(',').some((part) => part.trim() === `.${name}:focus-visible`)
    );
    if (
      focus?.declarations.get('background-color') !== 'var(--background-focus)' ||
      focus?.declarations.get('color') !== 'var(--text-default)'
    ) {
      problems.push(`.${name} paints at rest but does not restate the focus fill`);
    }
  }
  return problems;
}

// ── The real files ──────────────────────────────────────────────────────────

const ALL = walk(CREW_DIR);
const OWN_MARKUP = ALL.filter((path) => isOwn(path) && isMarkup(path) && !isTest(path));
const OWN_CSS = ALL.filter((path) => isOwn(path) && path.endsWith('.css'));
const LEGACY_MARKUP = ALL.filter(
  (path) =>
    isMarkup(path) &&
    !isTest(path) &&
    (path.startsWith(LEGACY_DIR + sep) || (dirname(path) === CREW_DIR && path.endsWith('.tsx')))
);
const OTHER_AREA_CSS = ALL.filter(
  (path) => path.endsWith('.css') && !isOwn(path) && path !== LEGACY_CSS
);

const union = (sets: Set<string>[]) => new Set(sets.flatMap((set) => [...set]));

describe('the composer and files stylesheets', () => {
  const legacyCss = stylesheetClasses(read(LEGACY_CSS));
  const legacyMarkup = union(LEGACY_MARKUP.map((path) => markupTokens(read(path))));
  const ownMarkup = union(OWN_MARKUP.map((path) => markupTokens(read(path))));
  const ownCss = union(OWN_CSS.map((path) => stylesheetClasses(read(path))));

  it('reads the files it guards', () => {
    expect(OWN_CSS.map(rel).sort()).toEqual(['composer/composer.css', 'files/files.css']);
    expect(OWN_MARKUP.map((path) => basename(path))).toContain('Composer.tsx');
    expect(OWN_MARKUP.map((path) => basename(path))).toContain('AttachmentCard.tsx');
    // The two names that collided, still styled by the legacy sheet and written by its markup.
    expect(legacyCss.has('crew-composer') && legacyCss.has('crew-attachment')).toBe(true);
    expect(legacyMarkup.has('crew-composer') && legacyMarkup.has('crew-attachment')).toBe(true);
    expect(ownMarkup.has('crew-compose-input')).toBe(true);
    expect(ownCss.has('crew-attachment-card')).toBe(true);
  });

  it('writes no class the legacy stylesheet styles', () => {
    expect(intersect(ownMarkup, legacyCss)).toEqual([]);
  });

  it('styles no class the legacy markup writes', () => {
    expect(intersect(ownCss, legacyMarkup)).toEqual([]);
  });

  it('styles no class another area stylesheet also styles', () => {
    const others = OTHER_AREA_CSS.map((path) => ({
      file: rel(path),
      shared: intersect(ownCss, stylesheetClasses(read(path))),
    })).filter(({ shared }) => shared.length > 0);
    expect(others).toEqual([]);
  });

  it('restates the focus fill on every raw focusable element it paints', () => {
    const focusable = union(OWN_MARKUP.map((path) => focusableClasses(read(path))));
    // The two raw focusables today; a new one joins the check without an edit here.
    expect(focusable.has('crew-chip-action')).toBe(true);
    expect(focusable.has('crew-server-path-note')).toBe(true);
    const css = OWN_CSS.map(read).join('\n');
    expect(focusFillViolations(css, focusable)).toEqual([]);
  });
});

describe('the guard itself', () => {
  it('finds a shared class in markup, and ignores comments and data/custom-property hooks', () => {
    const markup = markupTokens(`
      // <div className="crew-ignored-line" />
      /* className="crew-ignored-block" */
      <div className="crew-composer crew-compose-card" data-crew-menu="x" />
      <span style={{ height: 'var(--crew-rest)' }} />
    `);
    expect([...markup].sort()).toEqual(['crew-compose-card', 'crew-composer']);
    const legacy = stylesheetClasses(`
      /* .crew-in-a-comment { } */
      .crew-composer textarea { resize: vertical; }
      @media (max-width: 900px) { .crew-composer-footer { flex-wrap: wrap; } }
    `);
    expect([...legacy].sort()).toEqual(['crew-composer', 'crew-composer-footer']);
    expect(intersect(markup, legacy)).toEqual(['crew-composer']);
  });

  it('finds focusable elements that paint without restating the fill', () => {
    const focusable = focusableClasses(`
      <button type="button" className="crew-a" onClick={onClick}>x</button>
      <span className="crew-b other" tabIndex={0}>y</span>
      <span className="crew-c" tabIndex={-1}>z</span>
      <div className="crew-d">w</div>
    `);
    expect([...focusable].sort()).toEqual(['crew-a', 'crew-b']);

    expect(
      focusFillViolations(
        `
        .crew-a { background-color: transparent; color: var(--text-muted); }
        .crew-b { margin-left: auto; }
      `,
        focusable
      )
    ).toEqual(['.crew-a paints at rest but does not restate the focus fill']);

    expect(
      focusFillViolations(
        `
        .crew-a { color: var(--text-muted); }
        .crew-a:focus-visible { background-color: var(--background-focus); color: var(--text-default); }
      `,
        focusable
      )
    ).toEqual([]);
  });
});
