import { readdirSync, readFileSync } from 'node:fs';
import { basename, dirname, join, relative, sep } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * Two cascade hazards for the composer and files packages, held at the source because jsdom
 * loads no stylesheet and a component test passes whether or not either one bites.
 *
 * 1. **Names shared with another area's stylesheet.** Every Crew stylesheet is global once the
 *    route loads it, and its rules are unlayered, so a class two areas both style is decided by
 *    specificity and load order rather than by either area. No class these packages style may be
 *    one another area's stylesheet also styles. (The old layout's `crew/crew.css` is how this
 *    bit: it styled `.crew-composer` and `.crew-attachment` under the new composer. It is
 *    deleted, and `integration/legacyStylesheet.test.ts` keeps it deleted.)
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
const OTHER_AREA_CSS = ALL.filter((path) => path.endsWith('.css') && !isOwn(path));

const union = (sets: Set<string>[]) => new Set(sets.flatMap((set) => [...set]));

describe('the composer and files stylesheets', () => {
  const ownCss = union(OWN_CSS.map((path) => stylesheetClasses(read(path))));

  it('reads the files it guards', () => {
    expect(OWN_CSS.map(rel).sort()).toEqual(['composer/composer.css', 'files/files.css']);
    expect(OWN_MARKUP.map((path) => basename(path))).toContain('Composer.tsx');
    expect(OWN_MARKUP.map((path) => basename(path))).toContain('AttachmentCard.tsx');
    expect(ownCss.has('crew-attachment-card')).toBe(true);
    // The other areas' stylesheets are really compared against.
    expect(OTHER_AREA_CSS.map(rel)).toEqual(
      expect.arrayContaining(['crew-app.css', 'layout/layout.css'])
    );
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

/**
 * What jsdom cannot lay out, asserted at the source like `styles/composerFocus.test.ts`: a
 * component test renders the card whether or not the name can be read.
 */
describe('file names win the row (Q4-03) and the Files tab sits on the panel inset (Q4-29)', () => {
  const rules = leafRules(read(join(FILES_DIR, 'files.css')));
  const rule = (selector: string) => {
    const found = rules.find((item) => item.selector === selector);
    if (!found) throw new Error(`no rule for ${selector}`);
    return found.declarations;
  };

  it('lets the card’s meta give way wholly before its name loses a letter', () => {
    // The name gave way first — 28 of the 125px it needed — while the meta kept 152px. Now the
    // two share one box that takes the row's room; inside it the name does not shrink (but never
    // outgrows the box) and the meta shrinks into what is left, with an ellipsis.
    const label = rule('.crew-attachment-label, .crew-file-row-label');
    expect(label.get('display')).toBe('flex');
    expect(label.get('flex')).toBe('1 1 auto');
    expect(label.get('min-width')).toBe('3ch');
    expect(label.get('overflow')).toBe('hidden');
    const name = rule('.crew-attachment-name');
    expect(name.get('flex')).toBe('0 0 auto');
    expect(name.get('max-width')).toBe('100%');
    expect(name.get('text-overflow')).toBe('ellipsis');
    const meta = rule('.crew-attachment-meta');
    expect(meta.get('flex')).toBe('0 1 auto');
    expect(meta.get('min-width')).toBe('0');
    expect(meta.get('overflow')).toBe('hidden');
    expect(meta.get('text-overflow')).toBe('ellipsis');
    // The shared ink rule no longer pins the meta at its full width.
    expect(rule('.crew-attachment-meta, .crew-server-path-note').has('flex-shrink')).toBe(false);
    // …and the markup puts the two in that box (a sibling of the name would still take a share
    // of the shrink, and a share is an ellipsis).
    const card = read(join(FILES_DIR, 'AttachmentCard.tsx'));
    expect(card).toMatch(
      /<span className="crew-attachment-label">\s*(<Tooltip>\s*<TooltipTrigger asChild>\s*)?<span className="crew-attachment-name">/
    );
  });

  it('does the same for a Files row’s name and size, and lets a transfer row’s state give way', () => {
    expect(rule('.crew-file-row-label > .crew-file-row-name').get('flex')).toBe('0 0 auto');
    expect(rule('.crew-file-row-label > .crew-file-row-name').get('max-width')).toBe('100%');
    const name = rule('.crew-file-row-name');
    expect(name.get('min-width')).toBe('3ch');
    const meta = rule('.crew-file-row-meta');
    expect(meta.get('flex')).toBe('0 1 auto');
    expect(meta.get('min-width')).toBe('0');
    expect(meta.get('overflow')).toBe('hidden');
    expect(meta.get('text-overflow')).toBe('ellipsis');
  });

  it('gives the Files tab no padding of its own: the panel’s 8px is the inset', () => {
    const tab = rule('.crew-files-tab');
    expect([...tab.keys()].filter((property) => property.startsWith('padding'))).toEqual([]);
  });
});

describe('the guard itself', () => {
  it('finds a class two stylesheets share, and ignores comments and custom properties', () => {
    const own = stylesheetClasses(`
      /* .crew-in-a-comment { } */
      .crew-compose-card { --crew-rest: 1px; }
      .crew-composer textarea { resize: vertical; }
    `);
    expect([...own].sort()).toEqual(['crew-compose-card', 'crew-composer']);
    const other = stylesheetClasses(`
      /* .crew-compose-card { } */
      .crew-composer { margin: 0; }
      @media (max-width: 900px) { .crew-composer-footer { flex-wrap: wrap; } }
    `);
    expect([...other].sort()).toEqual(['crew-composer', 'crew-composer-footer']);
    expect(intersect(own, other)).toEqual(['crew-composer']);
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
