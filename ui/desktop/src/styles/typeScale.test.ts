import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * The type scale, asserted at the token (redesign 2026-10, spec §2.1).
 *
 * The owner asked for "very few" font sizes and weights. The app had thirteen
 * sizes in use; the redesign settles on five (12 · 13 · 14 · 17 · 24) and two
 * visible weights, because the face is Arial and Arial draws 400 and 700 only
 * (500 renders as 400, 600 as 700). This file pins the ROLES: what each type
 * token is allowed to be. The call-site guard (no `text-sm`, no `text-[11px]`,
 * no stray `tracking-*`, no `font-bold`) joins it in the redesign's wave 2, once
 * every owner has moved its own files onto the roles; adding it earlier would
 * fail on arrival and get switched off.
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
        'display',
        'heading',
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
    expect(tracked).toEqual(['caps', 'display', 'title']);
    expect(resolve('caps', 'letter-spacing')).toBe('0.04em');
    expect(resolve('title', 'letter-spacing')).toBe('-0.01em');
  });

  /** The two retired roles are aliases, not sizes of their own. */
  it('keeps text-heading and text-display as aliases until wave 2 deletes them', () => {
    expect(ROLES.get('heading')?.size).toBe('var(--text-subheading)');
    expect(ROLES.get('display')?.size).toBe('var(--text-title)');
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
