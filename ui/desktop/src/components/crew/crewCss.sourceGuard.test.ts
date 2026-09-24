import { readdirSync, readFileSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { describe, expect, it } from 'vitest';
import { PREVIEW_MIN_WIDTH, PREVIEW_SIDE_WIDTH } from '../Layout/yieldLadder';

/**
 * The Crew stylesheet contract, enforced at the source.
 *
 * `crew/crew-app.css` states the contract in its header; every
 * `crew/<area>/*.css` an area package adds is bound by it too. The legacy
 * `crew/crew.css` (and anything under `crew/legacy/`) is exempt: it is the old
 * layout's stylesheet, and it goes when the legacy layout does.
 *
 * Why at the source: jsdom runs no Tailwind, evaluates no container query and
 * never loads these files, so a component test cannot see any of this. The
 * guarded failures are all silent in a real window too — an element selector
 * that re-skins a primitive (`.crew-view input` did, and beat the global focus
 * surface), a hex that ignores the theme family, a loop with no reduced-motion
 * rest, a pane width that drifted from the yield ladder, and a newly written
 * arbitrary utility that never generates under `BIOROUTER_NO_HMR`.
 *
 * Each detector is exercised on a bad fixture below, so the guard cannot pass
 * by failing to look.
 */

const CREW_DIR = __dirname;
const LEGACY_CSS = join(CREW_DIR, 'crew.css');
const LEGACY_DIR = join(CREW_DIR, 'legacy');
const CREW_APP_CSS = join(CREW_DIR, 'crew-app.css');
const THIS_FILE = join(CREW_DIR, 'crewCss.sourceGuard.test.ts');

function walk(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return entry.name === 'node_modules' ? [] : walk(path);
    return [path];
  });
}

const isLegacy = (path: string) => path === LEGACY_CSS || path.startsWith(LEGACY_DIR + sep);
const rel = (path: string) => relative(CREW_DIR, path).split(sep).join('/');

const CSS_FILES = walk(CREW_DIR).filter((path) => path.endsWith('.css') && !isLegacy(path));
const SOURCE_FILES = walk(CREW_DIR).filter(
  (path) =>
    /\.(ts|tsx)$/.test(path) && !path.endsWith('.d.ts') && !isLegacy(path) && path !== THIS_FILE
);

// ── A small CSS reader ──────────────────────────────────────────────────────
// Enough of CSS for this contract: comments, strings, nested blocks (at-rules
// and CSS nesting), declarations. Not a validator — the build is that.

interface Declaration {
  property: string;
  value: string;
}

interface StyleRule {
  selector: string;
  declarations: Declaration[];
  /** Enclosing at-rule preludes, outermost first. */
  atRules: string[];
}

interface ParsedCss {
  rules: StyleRule[];
  keyframes: string[];
  atPreludes: string[];
}

function parseDeclarations(body: string): Declaration[] {
  return body
    .split(';')
    .map((part) => part.trim())
    .filter((part) => part.includes(':'))
    .map((part) => {
      const colon = part.indexOf(':');
      return {
        property: part.slice(0, colon).trim().toLowerCase(),
        value: part.slice(colon + 1).trim(),
      };
    });
}

function parseCss(source: string): ParsedCss {
  const text = source.replace(/\/\*[\s\S]*?\*\//g, ' ');
  const rules: StyleRule[] = [];
  const keyframes: string[] = [];
  const atPreludes: string[] = [];
  const stack: { prelude: string; body: string }[] = [];
  let buffer = '';
  let quote: string | null = null;

  for (let index = 0; index < text.length; index += 1) {
    const char = text[index];
    if (quote) {
      buffer += char;
      if (char === '\\') {
        buffer += text[index + 1] ?? '';
        index += 1;
      } else if (char === quote) {
        quote = null;
      }
      continue;
    }
    if (char === '"' || char === "'") {
      quote = char;
      buffer += char;
      continue;
    }
    if (char === '{') {
      const lastSemicolon = buffer.lastIndexOf(';');
      if (stack.length > 0) stack[stack.length - 1].body += buffer.slice(0, lastSemicolon + 1);
      stack.push({ prelude: buffer.slice(lastSemicolon + 1).trim(), body: '' });
      buffer = '';
    } else if (char === '}') {
      const frame = stack.pop();
      if (!frame) throw new Error('unbalanced "}" in CSS');
      frame.body += buffer;
      buffer = '';
      const ancestors = stack.map((open) => open.prelude);
      if (frame.prelude.startsWith('@')) {
        atPreludes.push(frame.prelude);
        const name = /^@keyframes\s+([^\s{]+)/.exec(frame.prelude)?.[1];
        if (name) keyframes.push(name);
      } else if (!ancestors.some((prelude) => prelude.startsWith('@keyframes'))) {
        rules.push({
          selector: frame.prelude,
          declarations: parseDeclarations(frame.body),
          atRules: ancestors.filter((prelude) => prelude.startsWith('@')),
        });
      }
    } else {
      buffer += char;
    }
  }
  if (stack.length > 0) throw new Error('unbalanced "{" in CSS');
  return { rules, keyframes, atPreludes };
}

// ── Detectors ───────────────────────────────────────────────────────────────

/** Everything in a selector that is not a `.crew-*` class or a data/aria hook. */
function selectorViolations(selector: string): string[] {
  const problems: string[] = [];
  let rest = selector.replace(/"[^"]*"|'[^']*'/g, "''");
  // An+B arguments are not selectors; an `of S` clause is.
  rest = rest.replace(
    /:nth-(?:last-)?(?:child|of-type)\(([^)]*)\)/g,
    (_match, argument: string) => {
      const of = argument.search(/\bof\b/);
      return of >= 0 ? ` ${argument.slice(of + 2)} ` : ' ';
    }
  );
  rest = rest.replace(/:(?:dir|lang)\([^)]*\)/g, ' ');
  rest = rest.replace(/\[\s*([^\]\s=~|^$*]+)[^\]]*\]/g, (_match, name: string) => {
    if (!/^(?:data|aria)-/.test(name)) problems.push(`attribute [${name}]`);
    return ' ';
  });
  rest = rest.replace(/\.(-?[_a-zA-Z][\w-]*)/g, (_match, name: string) => {
    if (!name.startsWith('crew-')) problems.push(`class .${name}`);
    return ' ';
  });
  rest = rest.replace(/#(-?[_a-zA-Z][\w-]*)/g, (_match, name: string) => {
    problems.push(`id #${name}`);
    return ' ';
  });
  rest = rest.replace(/::?[a-zA-Z-]+/g, ' ');
  if (rest.includes('*')) problems.push('universal selector *');
  for (const name of rest.match(/-?[_a-zA-Z][\w-]*/g) ?? []) {
    problems.push(`element selector ${name}`);
  }
  return problems;
}

const COLOR_LITERAL = /#[0-9a-fA-F]{3,8}\b|\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\(/;

/** Contract violations in one stylesheet, as readable lines. */
function stylesheetViolations(source: string): string[] {
  const problems: string[] = [];
  const parsed = parseCss(source);
  if (/@apply\b/.test(source.replace(/\/\*[\s\S]*?\*\//g, ' '))) problems.push('uses @apply');

  for (const rule of parsed.rules) {
    for (const problem of selectorViolations(rule.selector)) {
      problems.push(`${rule.selector}: ${problem}`);
    }
    for (const { property, value } of rule.declarations) {
      if (COLOR_LITERAL.test(value)) {
        problems.push(`${rule.selector}: colour literal in ${property}: ${value}`);
      }
      if ((property === 'font-size' || property === 'font') && /\d(?:\.\d+)?px\b/.test(value)) {
        problems.push(`${rule.selector}: pixel font size in ${property}: ${value}`);
      }
    }
  }
  for (const name of parsed.keyframes) {
    if (!name.startsWith('crew-')) problems.push(`@keyframes ${name} is not named crew-*`);
  }
  for (const problem of reducedMotionViolations(parsed)) problems.push(problem);
  return problems;
}

const isReduced = (rule: StyleRule) =>
  rule.atRules.some((prelude) => /prefers-reduced-motion\s*:\s*reduce/.test(prelude));

const normalizeSelector = (selector: string) =>
  selector
    .replace(/"/g, "'")
    .replace(/\s*([>+~,])\s*/g, '$1')
    .replace(/\s+/g, ' ')
    .trim();

/** The `crew-*` keyframes an animation declaration names. */
function animationNames(rule: StyleRule): string[] {
  return rule.declarations
    .filter(({ property }) => property === 'animation' || property === 'animation-name')
    .flatMap(({ value }) => value.match(/(?<![\w-])crew-[\w-]+/g) ?? []);
}

/**
 * Every rule that animates with a `crew-*` keyframes must restate its resting
 * state, under the same selector, inside a `prefers-reduced-motion: reduce`
 * block in the same file.
 */
function reducedMotionViolations(parsed: ParsedCss): string[] {
  const rests = new Set(
    parsed.rules
      .filter(
        (rule) =>
          isReduced(rule) &&
          rule.declarations.some(({ property }) => property.startsWith('animation'))
      )
      .map((rule) => normalizeSelector(rule.selector))
  );
  return parsed.rules
    .filter((rule) => !isReduced(rule) && animationNames(rule).length > 0)
    .filter((rule) => !rests.has(normalizeSelector(rule.selector)))
    .map(
      (rule) =>
        `${rule.selector}: animates with ${animationNames(rule).join(', ')} but declares no ` +
        'resting state under @media (prefers-reduced-motion: reduce)'
    );
}

const ARBITRARY_VALUE =
  /-\[(?:[a-z-]+:)?(?:var\(|calc\(|-?\d*\.?\d+(?:px|rem|em|ch|vh|vw|dvh|svh|lvh|ms|s|deg|%)(?=[\]_,)\s/]))/;

/** Newly written arbitrary-value utilities in a source file, as `line: match`. */
function arbitraryValueViolations(source: string): string[] {
  const withoutComments = source
    .replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\n]/g, ' '))
    .replace(/(^|[^:'"`\\])\/\/[^\n]*/g, '$1');
  return withoutComments.split('\n').flatMap((line, index) => {
    const match = ARBITRARY_VALUE.exec(line);
    return match ? [`${index + 1}: ${line.trim()}`] : [];
  });
}

// ── The guard ───────────────────────────────────────────────────────────────

describe('the Crew stylesheet contract', () => {
  it('reads the redesigned stylesheets and skips the legacy one', () => {
    expect(CSS_FILES).toContain(CREW_APP_CSS);
    expect(CSS_FILES).not.toContain(LEGACY_CSS);
    expect(SOURCE_FILES.length).toBeGreaterThan(0);
  });

  it.each(CSS_FILES.map((path) => [rel(path), path]))(
    '%s uses only crew hooks, tokens and reduced-motion rests',
    (_name, path) => {
      expect(stylesheetViolations(readFileSync(path, 'utf8'))).toEqual([]);
    }
  );

  it('names every keyframes once across the crew stylesheets, and uses each one', () => {
    const declared = CSS_FILES.flatMap((path) =>
      parseCss(readFileSync(path, 'utf8')).keyframes.map((name) => ({ name, file: rel(path) }))
    );
    const seen = new Map<string, string>();
    const duplicates: string[] = [];
    for (const { name, file } of declared) {
      const first = seen.get(name);
      if (first) duplicates.push(`${name} in ${first} and ${file}`);
      else seen.set(name, file);
    }
    expect(duplicates).toEqual([]);

    // A keyframes used only through an inline style or an `animate-[…]` utility
    // would escape the reduced-motion rest this file enforces.
    const used = new Set(
      CSS_FILES.flatMap((path) =>
        parseCss(readFileSync(path, 'utf8')).rules.flatMap((rule) => animationNames(rule))
      )
    );
    expect(declared.map(({ name }) => name).filter((name) => !used.has(name))).toEqual([]);
  });

  it('pins the pane geometry to the yield ladder, which CSS cannot import', () => {
    const parsed = parseCss(readFileSync(CREW_APP_CSS, 'utf8'));
    const root = parsed.rules.find((rule) => normalizeSelector(rule.selector) === '.crew-app');
    expect(root, '.crew-app is missing from crew-app.css').toBeDefined();
    const value = (property: string) =>
      root?.declarations.find((declaration) => declaration.property === property)?.value;
    expect(value('--crew-pane-width')).toBe(`${PREVIEW_MIN_WIDTH}px`);
    expect(value('--crew-push-min')).toBe(`${PREVIEW_SIDE_WIDTH}px`);

    // A container query cannot read `--crew-push-min`, so each threshold is a
    // literal — and each must be the same number.
    const thresholds = CSS_FILES.flatMap((path) =>
      parseCss(readFileSync(path, 'utf8'))
        .atPreludes.filter((prelude) => /^@container\s+crew-main\b/.test(prelude))
        .map((prelude) => ({ file: rel(path), prelude }))
    );
    expect(thresholds.filter(({ file }) => file === 'crew-app.css').length).toBeGreaterThan(0);
    for (const { file, prelude } of thresholds) {
      const numbers = (prelude.match(/\d+(?:\.\d+)?px/g) ?? []).map((px) => parseFloat(px));
      expect(numbers.length, `${file}: ${prelude}`).toBeGreaterThan(0);
      for (const number of numbers) expect(number, `${file}: ${prelude}`).toBe(PREVIEW_SIDE_WIDTH);
    }
  });

  /**
   * Measured in a real browser, not reasoned: an absolutely positioned grid item
   * takes its GRID AREA as its containing block, so a covering pane left in the
   * `auto` column 2 rendered 0px wide. jsdom evaluates neither the container
   * query nor grid, so only the source can hold this.
   */
  it('lets a covering pane span the whole stage below the channel header', () => {
    const cover = parseCss(readFileSync(CREW_APP_CSS, 'utf8')).rules.find(
      (rule) =>
        normalizeSelector(rule.selector) === '.crew-pane' &&
        rule.atRules.some((prelude) => /^@container\s+crew-main\s*\(width\s*</.test(prelude)) &&
        !isReduced(rule)
    );
    expect(cover, 'no .crew-pane rule inside the cover container query').toBeDefined();
    const value = (property: string) =>
      cover?.declarations.find((declaration) => declaration.property === property)?.value;
    expect(value('position')).toBe('absolute');
    expect(value('grid-column')).toBe('1 / -1');
    expect(value('grid-row')).toBe('1 / -1');
    expect(value('inset')).toBe('var(--chrome-height) 0 0 0');
  });

  it('keeps load-bearing styles out of newly written arbitrary-value utilities', () => {
    const problems = SOURCE_FILES.flatMap((path) =>
      arbitraryValueViolations(readFileSync(path, 'utf8')).map((line) => `${rel(path)}:${line}`)
    );
    expect(problems).toEqual([]);
  });
});

describe('the guard itself', () => {
  it('finds element, universal, id and foreign-class selectors', () => {
    expect(selectorViolations('.crew-view input')).toEqual(['element selector input']);
    expect(selectorViolations('.crew-stack > *')).toEqual(['universal selector *']);
    expect(selectorViolations('#crew')).toEqual(['id #crew']);
    expect(selectorViolations('.crew-row .biorouter-copy-field')).toEqual([
      'class .biorouter-copy-field',
    ]);
    expect(selectorViolations('.crew-row[role=button]')).toEqual(['attribute [role]']);
    expect(selectorViolations('.crew-x:has(textarea:focus)')).toEqual([
      'element selector textarea',
    ]);
    expect(selectorViolations('.crew-list:nth-child(2n + 1 of li)')).toEqual([
      'element selector li',
    ]);
  });

  it('accepts crew classes, data and aria hooks, pseudo-classes and :has()', () => {
    for (const selector of [
      '.crew-app',
      ".crew-pane[data-state='open'] > .crew-pane-content",
      '.crew-stage:has(> .crew-pane[data-state="open"]) .crew-channel-body',
      '.crew-row[aria-current=page]:focus-visible',
      '.crew-row:nth-child(2n + 1)',
      '.crew-dot::after',
      '.crew-row:not(.crew-row-muted):hover',
    ]) {
      expect(selectorViolations(selector), selector).toEqual([]);
    }
  });

  it('finds colour literals, pixel font sizes, @apply and foreign keyframes', () => {
    const problems = stylesheetViolations(`
      .crew-a { color: #b86b46; }
      .crew-b { background: rgb(0 0 0 / 40%); }
      .crew-c { font-size: 11px; }
      .crew-d { font: 500 12px/16px var(--font-sans); }
      .crew-e { @apply text-label; }
      .crew-f { background: color-mix(in oklab, var(--text-default) 5%, transparent); }
      @keyframes fade { from { opacity: 0; } to { opacity: 1; } }
    `);
    expect(problems).toEqual([
      'uses @apply',
      '.crew-a: colour literal in color: #b86b46',
      '.crew-b: colour literal in background: rgb(0 0 0 / 40%)',
      '.crew-c: pixel font size in font-size: 11px',
      '.crew-d: pixel font size in font: 500 12px/16px var(--font-sans)',
      '@keyframes fade is not named crew-*',
    ]);
  });

  it('requires a reduced-motion rest for every rule that animates with a crew keyframes', () => {
    const missing = stylesheetViolations(`
      .crew-dot { animation: crew-pulse 2s var(--ease-out) infinite; }
      @keyframes crew-pulse { from { opacity: 0; } to { opacity: 1; } }
    `);
    expect(missing).toEqual([
      '.crew-dot: animates with crew-pulse but declares no resting state under ' +
        '@media (prefers-reduced-motion: reduce)',
    ]);

    const rested = stylesheetViolations(`
      .crew-dot { animation: crew-pulse 2s var(--ease-out) infinite; }
      @container crew-main (width < 800px) {
        .crew-pane[data-state="open"] { animation: crew-pulse 1s; }
      }
      @keyframes crew-pulse { from { opacity: 0; } to { opacity: 1; } }
      @media (prefers-reduced-motion: reduce) {
        .crew-dot { animation: none; }
        .crew-pane[data-state='open'] { animation-name: crew-fade-in; }
      }
    `);
    expect(rested).toEqual([]);

    // A custom property holding a crew name is not an animation.
    expect(stylesheetViolations('.crew-app { --crew-pane-width: 360px; }')).toEqual([]);
  });

  it('fails the real stylesheet once its reduced-motion rests are taken away', () => {
    const source = readFileSync(CREW_APP_CSS, 'utf8');
    const reduced = source.lastIndexOf('@media (prefers-reduced-motion: reduce)');
    expect(reduced).toBeGreaterThan(0);
    const withoutRests = stylesheetViolations(source.slice(0, reduced));
    // Every animated rule in crew-app.css depends on that block — the pane in
    // both modes, its content, the cross-fade and the highlight.
    for (const selector of [
      ".crew-pane[data-state='open']",
      ".crew-pane[data-state='closed']",
      ".crew-pane[data-state='open'] > .crew-pane-content",
      ".crew-crossfade-item[data-state='open']",
      ".crew-crossfade-item[data-state='closed']",
      '.crew-highlight',
    ]) {
      expect(withoutRests.some((problem) => problem.startsWith(`${selector}: animates`))).toBe(
        true
      );
    }
  });

  it('finds arbitrary-value utilities, and ignores comments and variants', () => {
    expect(
      arbitraryValueViolations(
        [
          '<div className="w-[var(--crew-pane-width)]" />',
          "cn('max-w-[760px] flex')",
          '<p className="text-[11px]" />',
          '<p className="bg-[color:var(--background-well)]" />',
          '<p className="grid-cols-[96px_minmax(0,1fr)]" />',
        ].join('\n')
      )
    ).toEqual([
      '1: <div className="w-[var(--crew-pane-width)]" />',
      "2: cn('max-w-[760px] flex')",
      '3: <p className="text-[11px]" />',
      '4: <p className="bg-[color:var(--background-well)]" />',
      '5: <p className="grid-cols-[96px_minmax(0,1fr)]" />',
    ]);
    expect(
      arbitraryValueViolations(
        [
          '// never write w-[var(--crew-pane-width)]',
          '/* nor max-w-[760px] */',
          '<div className="data-[state=open]:bg-overlay-selected [&_svg]:size-4 has-[>svg]:px-2" />',
          "const url = 'https://example.org/w-[1px]';",
        ].join('\n')
      )
    ).toEqual(["4: const url = 'https://example.org/w-[1px]';"]);
  });
});
