import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * The type scale, asserted at the token (redesign 2026-10, spec §2.1).
 *
 * The owner asked for "very few" font sizes and weights. The app had thirteen
 * sizes in use; the redesign settles on five (12 · 13 · 14 · 17 · 24) and two
 * visible weights, because the face is Arial and Arial draws 400 and 700 only
 * (500 renders as 400, 600 as 700). This file pins the ROLES: what each type
 * token is allowed to be, and then holds the CALL SITES to them (no `text-sm`, no
 * `text-[11px]`, no stray `tracking-*`, no bare `uppercase`, no `font-bold`).
 *
 * The call-site guard is STRICT: no guarded file may carry a legacy form. It began
 * as a ratchet (a list of the files that still had classes to sweep, each with the
 * count it carried, so the guard could land while eight owners were mid-sweep
 * without failing on arrival and being switched off). The sweep finished in wave 2,
 * the list was deleted, and a file that regresses now fails with its own name.
 */
const CSS = readFileSync(join(__dirname, 'main.css'), 'utf8');
const UTILS = readFileSync(join(__dirname, '../utils.ts'), 'utf8');
const CODE = CSS.replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\n]/g, ' '));

const SCALE = [12, 13, 14, 17, 24];

/** Every `--text-<role>` declared in a plain `@theme` block, with its parts. */
function roles(): Map<string, Record<string, string>> {
  const out = new Map<string, Record<string, string>>();
  const re = /(^|\n)@theme\s*\{/g;
  while (re.exec(CODE)) {
    let depth = 1;
    let i = re.lastIndex;
    for (; i < CODE.length && depth > 0; i++) {
      if (CODE[i] === '{') depth++;
      else if (CODE[i] === '}') depth--;
    }
    const body = CODE.slice(re.lastIndex, i - 1);
    for (const d of body.matchAll(/--text-([a-z]+)(--[a-z-]+)?:\s*([^;]+);/g)) {
      const [, role, part, value] = d;
      const entry = out.get(role) ?? {};
      entry[part ? part.slice(2) : 'size'] = value.trim();
      out.set(role, entry);
    }
  }
  return out;
}

const ROLES = roles();
const px = (v: string) => Number(/^(\d+)px$/.exec(v)?.[1]);

/** Follow `var(--text-x…)` to the role it aliases. */
function resolve(role: string, part: string): string {
  let value = ROLES.get(role)?.[part] ?? '';
  for (let i = 0; i < 4 && value.startsWith('var('); i++) {
    const m = /^var\(--text-([a-z]+)(?:--([a-z-]+))?\)$/.exec(value);
    if (!m) break;
    value = ROLES.get(m[1])?.[m[2] ?? 'size'] ?? '';
  }
  return value;
}

describe('the type roles', () => {
  it('declares the roles the redesign names', () => {
    expect([...ROLES.keys()].sort()).toEqual(
      [
        'body',
        'caps',
        'chip',
        'code',
        'label',
        'prose',
        'secondary',
        'subheading',
        'supporting',
        'title',
      ].sort()
    );
  });

  it.each([...ROLES.keys()])('%s renders at one of the five sizes', (role) => {
    expect(SCALE).toContain(px(resolve(role, 'size')));
  });

  it.each([...ROLES.keys()])('%s uses a weight of 400, 500 or 600', (role) => {
    expect(['400', '500', '600']).toContain(resolve(role, 'font-weight'));
  });

  it.each([
    ['supporting', '12px', '16px', '400'],
    ['chip', '12px', '16px', '500'],
    ['caps', '12px', '16px', '500'],
    ['secondary', '13px', '18px', '400'],
    ['code', '13px', '20px', '400'],
    ['body', '14px', '20px', '400'],
    ['label', '14px', '20px', '500'],
    ['prose', '14px', '21px', '400'],
    ['subheading', '17px', '24px', '600'],
    ['title', '24px', '32px', '400'],
  ])('%s is %s / %s at %s', (role, size, lineHeight, weight) => {
    expect(resolve(role, 'size')).toBe(size);
    expect(resolve(role, 'line-height')).toBe(lineHeight);
    expect(resolve(role, 'font-weight')).toBe(weight);
  });

  /** One caps style, 12px so it is not a sixth size, tracked +0.04em. */
  it('keeps tracking only on the caps label and the title', () => {
    const tracked = [...ROLES.entries()]
      .filter(([, parts]) => parts['letter-spacing'])
      .map(([role]) => role)
      .sort();
    expect(tracked).toEqual(['caps', 'title']);
    expect(resolve('caps', 'letter-spacing')).toBe('0.04em');
    expect(resolve('title', 'letter-spacing')).toBe('-0.01em');
  });

  /** The two retired roles stay gone: `heading` (20/28) and `display` (30/36). */
  it('declares neither text-heading nor text-display', () => {
    expect(ROLES.has('heading')).toBe(false);
    expect(ROLES.has('display')).toBe(false);
  });

  /** `text-caps` carries its casing; the generated utility cannot. */
  it('folds the uppercase transform into text-caps', () => {
    expect(CODE).toMatch(/\.text-caps\s*\{\s*text-transform:\s*uppercase;\s*\}/);
  });
});

/**
 * tailwind-merge has to be taught every role, or `cn()` reads `text-prose` as a
 * text COLOUR and drops the real colour beside it (typography audit R7). The
 * list lives in `src/utils.ts`; this keeps it in step with the stylesheet.
 */
describe('tailwind-merge knows every role', () => {
  const list = /text:\s*\[([^\]]*)\]/.exec(UTILS)?.[1] ?? '';
  const registered = [...list.matchAll(/'([a-z]+)'/g)].map((m) => m[1]).sort();

  it('registers exactly the declared roles', () => {
    expect(registered).toEqual([...ROLES.keys()].sort());
  });
});

/**
 * ── The call sites ────────────────────────────────────────────────────────────
 *
 * What is counted, per file (comments are stripped first, so a note explaining
 * the rule cannot trip it):
 *  - `.ts` / `.tsx`: a legacy size class (`text-xs|sm|base|lg|xl|2xl…`,
 *    `text-[Npx]`, or the retired `text-heading` / `text-display`), a `tracking-*` class, a bare `uppercase` class (`text-caps`
 *    carries its own casing) and `font-bold` (Arial draws 400 and 700, so the
 *    strong weight is `font-semibold`, which the face renders as bold anyway).
 *  - `.css`: a `font-size` in px (a role is a `--text-*` token) and a
 *    `font-weight` of 700 or heavier.
 *
 * Out of scope, each for a reason: `components/crew/` (its own guard,
 * `crewCss.sourceGuard.test.ts`), the frozen files no workstream may restyle
 * (`components/privacy/**`, `context_management/**` except
 * `SystemNotificationInline.tsx`, `LauncherView`, `Pair`, the unmounted
 * `settings/{dictation,tunnel,sessions}/**`, `ModelAndProviderContext.tsx`), the
 * generated `api/`, the gitignored build output under `bin/` and `web/`, and tests.
 * `onboarding/**` and `ProviderGuard` were out of scope until their P2 sweep landed
 * (df9186abf); they are guarded now.
 */
const SRC = join(__dirname, '..');

const OUT_OF_SCOPE = [
  /^components\/crew\//,
  /^components\/privacy\//,
  /^components\/context_management\/(?!SystemNotificationInline)/,
  /^components\/LauncherView\./,
  /^components\/Pair\./,
  /^components\/settings\/(dictation|tunnel|sessions)\//,
  /^components\/ModelAndProviderContext\./,
  /^api\//,
  // Build output that is gitignored but sits under src/: the staged binaries and the
  // root-base browser bundle (`npm run build:web`). Compiled CSS is full of px sizes.
  /^bin\//,
  /^web\//,
];

const CLASS_RULES: Record<string, RegExp> = {
  'legacy size class':
    /(?<![\w-])text-(?:xs|sm|base|lg|[2-9]?xl|heading|display|\[\d+(?:\.\d+)?px\])(?![\w-])/g,
  'tracking class': /(?<![\w-])tracking-[\w[]/g,
  // `uppercase` the CLASS. The lookahead skips the identifier: a prop declaration
  // (`uppercase?: boolean`), its default (`uppercase = false`) and its use
  // (`uppercase && 'text-caps'`), which `ui/badge.tsx` has and which set no style.
  'bare uppercase': /(?<![\w-])uppercase(?![\w-])(?!\s*(?:\??:|=(?!=)|&&))/g,
  'font-bold': /(?<![\w-])font-bold(?![\w-])/g,
};

const CSS_RULES: Record<string, RegExp> = {
  'px font-size': /font-size:\s*\d*\.?\d+px/g,
  'weight 700 or heavier': /font-weight:\s*(?:700|bold|800|900)\b/g,
};

/** Comments blanked (newlines kept, so line numbers hold). `//` skips `://` in URLs. */
function withoutComments(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\n]/g, ' '))
    .replace(/(^|[^:'"`\\])\/\/[^\n]*/g, '$1');
}

/** Every legacy type form in a source file, as `rule @ line`. */
function legacyTypeHits(file: string, source: string): string[] {
  const rules = file.endsWith('.css') ? CSS_RULES : CLASS_RULES;
  const code = withoutComments(source);
  return Object.entries(rules).flatMap(([name, pattern]) =>
    [...code.matchAll(pattern)].map(
      (m) => `${name} @ line ${code.slice(0, m.index).split('\n').length}`
    )
  );
}

function walk(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === 'node_modules' ? [] : walk(path);
    return [path];
  });
}

function guardedFiles(): string[] {
  return walk(SRC)
    .map((path) => relative(SRC, path).split(sep).join('/'))
    .filter((file) => /\.(?:tsx?|css)$/.test(file))
    .filter((file) => !/\.test\.|\.browser\.|\.d\.ts$/.test(file))
    .filter((file) => !OUT_OF_SCOPE.some((pattern) => pattern.test(file)));
}

describe('the call sites use the type roles', () => {
  const files = guardedFiles();

  it('is looking at the whole renderer', () => {
    expect(files.length).toBeGreaterThan(500);
    expect(files).toContain('components/ui/button.tsx');
    expect(files).toContain('styles/main.css');
    expect(files.some((file) => file.startsWith('components/crew/'))).toBe(false);
  });

  it('lets no guarded file carry a legacy type form', () => {
    const over: string[] = [];
    for (const file of files) {
      const hits = legacyTypeHits(file, readFileSync(join(SRC, file), 'utf8'));
      if (hits.length > 0) over.push(`${file}: ${hits.length}: ${hits.slice(0, 3).join('; ')}`);
    }
    expect(over, 'use a text-<role> from main.css; see implementation-spec 2.1').toEqual([]);
  });

  it('guards the sweep that finished: onboarding and the provider guard are in scope', () => {
    expect(files.some((file) => file.startsWith('components/onboarding/'))).toBe(true);
    expect(files).toContain('components/ProviderGuard.tsx');
    expect(files).toContain('components/ui/badge.tsx');
  });

  /** Guards the guard: each form must be seen, and prose about it must not be. */
  it('recognises each legacy form and ignores comments about them', () => {
    const tsx = [
      '<p className="text-sm font-medium">a</p>',
      '<p className="md:text-[11px]">b</p>',
      '<p className="uppercase tracking-wide">c</p>',
      '<p className="font-bold">d</p>',
    ].join('\n');
    expect(legacyTypeHits('a.tsx', tsx)).toEqual([
      'legacy size class @ line 1',
      'legacy size class @ line 2',
      'tracking class @ line 3',
      'bare uppercase @ line 3',
      'font-bold @ line 4',
    ]);
    const prose = [
      '// text-sm is retired, so is tracking-wide and uppercase',
      '/* font-bold and text-[11px] */',
      '{/* uppercase */}',
      '<p className="text-supporting text-caps text-secondary text-text-muted font-semibold">e</p>',
      '<p className="context-lg normal-case">f</p>',
    ].join('\n');
    expect(legacyTypeHits('b.tsx', prose)).toEqual([]);
    // An identifier named `uppercase` is not the class: a prop, its default, its use.
    const identifier = [
      '  uppercase?: boolean;',
      '  uppercase = false,',
      "  uppercase && 'text-caps',",
    ].join('\n');
    expect(legacyTypeHits('d.tsx', identifier)).toEqual([]);
    expect(legacyTypeHits('e.tsx', "<p className='uppercase'>x</p>")).toEqual([
      'bare uppercase @ line 1',
    ]);
    const css =
      '.a { font-size: 13px; }\n.b { font-weight: 700; }\n.c { font-size: var(--text-body); }';
    expect(legacyTypeHits('c.css', css)).toEqual([
      'px font-size @ line 1',
      'weight 700 or heavier @ line 2',
    ]);
  });
});
