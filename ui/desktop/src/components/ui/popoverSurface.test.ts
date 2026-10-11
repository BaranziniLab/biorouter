import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

import * as themeTokens from '../../../scripts/lib/theme-tokens.mjs';

/**
 * A floating surface is visible against what it floats over, in dark too (vision round 1, V1-M5;
 * checklist 13 "elevation that cannot be seen").
 *
 * In dark the neutrals step UP from the page and a shadow is black on near-black, so a menu that
 * painted `--background-default` (#1b1b19) over the sidebar (#171716) or a card (#1b1b19) was told
 * apart by its 1px `--border-subtle` alone, and the heatmap tooltip sat on a card of the identical
 * ground. `.dark .biorouter-popover-surface` steps the surface up to `--background-medium` and
 * takes `--border-strong` as its edge, and re-points `--background-default` INSIDE the surface so
 * what a popover holds rides its ground. jsdom paints nothing, so the step is read from the
 * stylesheet's own tokens in every family, the way `check-contrast.mjs` reads them.
 */

type Scope = { decls: Record<string, string>; theme: Record<string, string> };
const tokens = themeTokens as unknown as {
  buildScopes(css: string): Record<string, Scope>;
  resolveHex(name: string, scope: Scope): string | null;
  contrast(a: string, b: string): number;
};

const css = readFileSync(join(__dirname, '../../styles/main.css'), 'utf8');
const scopes = tokens.buildScopes(css);
const darkScopes = Object.entries(scopes).filter(([name]) => name.endsWith(':dark'));

/** The declarations of `.dark .biorouter-popover-surface`. */
function darkRule(): Record<string, string> {
  const match = css.match(/\n\.dark \.biorouter-popover-surface \{([^}]*)\}/);
  expect(match, 'main.css has a .dark .biorouter-popover-surface rule').toBeTruthy();
  return Object.fromEntries(
    match![1]
      .split(';')
      .map((decl) => decl.trim())
      .filter(Boolean)
      .map((decl) => {
        const at = decl.indexOf(':');
        return [decl.slice(0, at).trim(), decl.slice(at + 1).trim()];
      })
  );
}

const tokenOf = (value: string) => value.match(/^var\((--[a-z-]+)\)$/)?.[1];

/** What a dark popover floats over: the sidebar, a card and the page. */
const GROUNDS = ['--sidebar', '--background-card', '--background-default'];

describe('the floating surface in dark (V1-M5)', () => {
  it('is a WS-PRIMITIVES rule that steps the ground up and firms the edge', () => {
    const rule = darkRule();
    expect(tokenOf(rule['--background-default'])).toBe('--background-medium');
    expect(rule['background-color']).toBe('var(--background-default)');
    expect(tokenOf(rule['border-color'])).toBe('--border-strong');
    const at = css.indexOf('\n.dark .biorouter-popover-surface {');
    const begin = css.lastIndexOf('/* @ws WS-PRIMITIVES begin: popover elevation */', at);
    const end = css.indexOf('/* @ws WS-PRIMITIVES end: popover elevation */', at);
    expect(begin).toBeGreaterThan(-1);
    expect(end).toBeGreaterThan(at);
    // It follows the base rule, so nothing later in the file paints the old ground back.
    expect(at).toBeGreaterThan(css.indexOf('\n.biorouter-popover-surface {'));
  });

  it('measures in all three families', () => {
    expect(darkScopes).toHaveLength(3);
  });

  it.each(darkScopes)(
    '%s: the ground is a step up and the edge a clear line on every ground',
    (name, scope) => {
      const rule = darkRule();
      const ground = tokens.resolveHex(tokenOf(rule['--background-default'])!, scope);
      const edge = tokens.resolveHex(tokenOf(rule['border-color'])!, scope);
      expect(ground, `${name}: the popover ground resolves`).toBeTruthy();
      expect(edge, `${name}: the popover edge resolves`).toBeTruthy();
      for (const token of GROUNDS) {
        const under = tokens.resolveHex(token, scope);
        expect(under, `${name}: ${token} resolves`).toBeTruthy();
        // Measured 1.23 to 1.28 (ground) and 1.59 to 1.65 (edge) on 2026-10-10.
        expect(tokens.contrast(ground!, under!), `${name}: ground over ${token}`).toBeGreaterThan(
          1.15
        );
        expect(
          tokens.contrast(edge!, under!),
          `${name}: edge over ${token}`
        ).toBeGreaterThanOrEqual(1.5);
      }
    }
  );

  it.each(darkScopes)('%s: default ink stays readable on the raised ground', (name, scope) => {
    const ground = tokens.resolveHex('--background-medium', scope)!;
    expect(
      tokens.contrast(tokens.resolveHex('--text-default', scope)!, ground),
      `${name}: --text-default`
    ).toBeGreaterThanOrEqual(4.5);
    expect(
      tokens.contrast(tokens.resolveHex('--text-muted', scope)!, ground),
      `${name}: --text-muted`
    ).toBeGreaterThanOrEqual(4.5);
  });

  it('leaves light alone: there the shadow and the hairline already lift it', () => {
    expect(css).not.toMatch(/\n:root \.biorouter-popover-surface \{/);
    expect(css).not.toMatch(/\n\.light \.biorouter-popover-surface \{/);
  });
});
