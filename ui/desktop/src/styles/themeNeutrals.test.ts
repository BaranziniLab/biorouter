import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

import almaMater from '../../themes/alma-mater.theme.mjs';
import parchment from '../../themes/parchment.theme.mjs';
import rocheLimit from '../../themes/roche-limit.theme.mjs';

/**
 * The three families share ONE set of neutrals and differ only in ink and
 * accent. This is the guard for that, and it exists because **nothing else
 * checks it**.
 *
 * `check-contrast.mjs` audits each family independently — it asks whether a
 * family's own ink is legible on that family's own ground. A family that
 * quietly re-tuned its greys would pass all 330 of those assertions, because
 * each one would still be internally consistent. The property that actually
 * matters here is a property BETWEEN families, and nothing was measuring it.
 *
 * That is not a hypothetical: three hand-tuned neutral sets is where this
 * started. Alma Mater's greys were cool blue, Roche Limit's warm, Parchment's
 * cream, and of the background/border/sidebar keys in the light block Alma and
 * Roche agreed on only four. Every new surface had to be checked three times
 * and still drifted. The fix was to make neutrals shared infrastructure; this
 * test is what keeps them that way.
 *
 * ⚠ **An allowlist, not a blocklist, and that direction is the point.** A new
 * token defaults to "must be identical across all three". If it genuinely
 * belongs to a family, it has to be added below — a deliberate act with a
 * reason attached, rather than a divergence nobody noticed. Getting a failure
 * here is the system working; the fix is usually to make the token match, not
 * to add it to the list.
 */

/** Tokens a family legitimately owns. Everything else is shared infrastructure. */
const PER_FAMILY = new Set([
  // The one accent, and the surfaces that carry it.
  'background-accent',
  'background-accent-hover',
  'border-accent',
  'text-accent',
  'text-on-accent',
  'accent-bar',
  'sidebar-icon',

  // Ink. The other axis a family is allowed to be itself on.
  'text-default',
  'text-muted',
  'text-subtle',
  'text-inverse',
  'sidebar-foreground',
  'sidebar-accent-foreground',

  // Status hues. Neither neutral scaffolding nor the family accent — a third
  // thing. Alma Mater's in particular are UCSF institutional brand values
  // rather than palette choices, which is why they were left per-family when
  // the neutrals were unified.
  'background-danger',
  'background-success',
  'background-warning',
  'background-info',
  'border-danger',
  'border-success',
  'border-warning',
  'border-info',
  'text-danger',
  'text-success',
  'text-warning',
  'text-info',
  'text-on-status',

  // The Home heatmap's ramp, which is built from the family's accent.
  'heat-1',
  'heat-2',
  'heat-3',
  'heat-4',
]);

const FAMILIES = { parchment, 'alma-mater': almaMater, 'roche-limit': rocheLimit } as const;
const REFERENCE = 'roche-limit' as const;

describe.each(['light', 'dark'] as const)('%s: the neutral scaffolding is shared', (mode) => {
  const reference = FAMILIES[REFERENCE][mode].tokens as Record<string, string>;
  const sharedKeys = Object.keys(reference).filter((key) => !PER_FAMILY.has(key));

  it.each((Object.keys(FAMILIES) as (keyof typeof FAMILIES)[]).filter((id) => id !== REFERENCE))(
    `%s matches ${REFERENCE} on every shared token`,
    (id) => {
      const tokens = FAMILIES[id][mode].tokens as Record<string, string>;
      const drifted = sharedKeys
        .filter((key) => tokens[key] !== reference[key])
        .map((key) => `${key}: ${tokens[key]} !== ${reference[key]}`);

      // Reported as a list rather than one key at a time: a real divergence is
      // usually a whole ramp, and seeing it whole is what tells you whether
      // someone re-tuned a family or fat-fingered a single value.
      expect(drifted).toEqual([]);
    }
  );

  /**
   * Guards the guard. If a rename or a refactor emptied `sharedKeys`, every
   * assertion above would pass vacuously and the whole file would become
   * decorative.
   */
  it('is actually checking a substantial number of tokens', () => {
    expect(sharedKeys.length).toBeGreaterThan(20);
  });

  /**
   * The allowlist must not outlive its entries. A token removed from the theme
   * files but left here would silently widen what is permitted to vary.
   */
  it('has no stale entries in the per-family allowlist', () => {
    const known = new Set(Object.keys(reference));
    // `text-inverse` and `text-on-status` are dark-only; absent in light is
    // expected rather than stale.
    const darkOnly = new Set(['text-inverse', 'text-on-status']);
    const stale = [...PER_FAMILY].filter((key) => !known.has(key) && !darkOnly.has(key));
    expect(stale).toEqual([]);
  });
});

/**
 * The control edge and the switch's off track are NEUTRALS (redesign spec 2.3),
 * and so are the two fills of R-27 (owner message 13, "fills before lines"), so
 * each is declared exactly twice, in the hand-authored base `:root` and `.dark`
 * blocks, and never in a family block or in the generated region.
 *
 * Two reasons this is asserted rather than assumed. A family that restated
 * `--border-control` would pass every per-family contrast assertion (it would
 * still clear 3:1 on its own ground) while quietly giving that family a control
 * edge the others do not have, which is the exact drift this file exists to
 * stop. And `check-contrast.mjs` resolves these two as hex literals, so a
 * `color-mix(...)` rewrite would turn an assertion into UNRESOLVED rather than
 * into a quiet pass: the guard here keeps the declaration where the checker
 * reads it.
 */
describe('the control edge, the off track and the fills are shared neutrals', () => {
  const raw = readFileSync(join(__dirname, 'main.css'), 'utf8');
  // Comments are blanked to spaces of the same length, so a prose mention of a
  // token cannot count as a declaration and every offset still matches `raw`.
  const css = raw.replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\n]/g, ' '));
  const generatedStart = raw.indexOf('THEMES:GENERATED:FAMILIES:START');
  const generatedEnd = raw.indexOf('THEMES:GENERATED:FAMILIES:END');

  // The dark popover's re-pointing of the strong fill and the off track is the only declaration
  // of these tokens outside the two base blocks; it is held by its own test below.
  const popoverFill = (() => {
    const from = raw.indexOf('@ws WS-TOKENS begin: popover fill');
    const to = raw.indexOf('@ws WS-TOKENS end: popover fill');
    return [from, to] as const;
  })();

  it.each(['border-control', 'control-track-off', 'background-fill', 'background-fill-strong'])(
    '--%s is declared as a hex once for light and once for dark, outside the family blocks',
    (token) => {
      const declarations = [
        ...css.matchAll(new RegExp(`(^|[\\s;{])--${token}:\\s*([^;]+);`, 'g')),
      ].filter((d) => (d.index ?? 0) < popoverFill[0] || (d.index ?? 0) > popoverFill[1]);
      expect(declarations.map((d) => d[2].trim())).toHaveLength(2);
      for (const declaration of declarations) {
        expect(declaration[2].trim()).toMatch(/^#[0-9a-f]{6}$/i);
        const at = declaration.index ?? 0;
        expect(at < generatedStart || at > generatedEnd, `--${token} sits inside a family`).toBe(
          true
        );
      }
    }
  );

  /**
   * A dark popover floats on `--background-medium` (#2c2c29), which is also the dark strong fill, so
   * a hovered field there would vanish into the popover. The rule re-points the strong step one
   * step DEEPER (#1b1b19): the rest fill is not lifted, because the placeholder ink falls under
   * 4.5:1 on any fill a visible step above that ground. The off Switch's track sinks to the same
   * well, because the page's track measured 1.05:1 on a highlighted popover row (WS-PICKERS-2).
   * Nothing else moves (main.css "popover fill"; `check-contrast.mjs` measures the rows in all
   * three families).
   */
  it('re-points only the strong fill and the off track inside a dark popover, to the page ground, after the elevation rule', () => {
    const [from, to] = popoverFill;
    expect(from, 'main.css has the popover fill block').toBeGreaterThan(-1);
    expect(to).toBeGreaterThan(from);
    const block = css.slice(from, to);
    const rule = block.match(/\.dark \.biorouter-popover-surface \{([^}]*)\}/);
    expect(rule, 'the block holds the .dark .biorouter-popover-surface rule').toBeTruthy();
    const declared = [...rule![1].matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)].map((d) => [
      d[1],
      d[2].trim(),
    ]);
    expect(declared).toEqual([
      ['--background-fill-strong', 'var(--color-neutral-900)'],
      ['--control-track-off', 'var(--color-neutral-900)'],
    ]);
    // Only custom properties: the surface's own paint is WS-PRIMITIVES' rule, which comes first.
    expect(rule![1].replace(/--[\w-]+\s*:[^;]+;/g, '').trim()).toBe('');
    const elevation = raw.indexOf('@ws WS-PRIMITIVES end: popover elevation');
    expect(elevation).toBeGreaterThan(-1);
    expect(from).toBeGreaterThan(elevation);
  });

  it('keeps the markers it measures against', () => {
    expect(generatedStart).toBeGreaterThan(0);
    expect(generatedEnd).toBeGreaterThan(generatedStart);
  });
});

/**
 * Nav icons wear their row's label ink in every family (2026-10 redesign,
 * principle 7; vision finding V1-M3). `--sidebar-icon` is a per-family token so
 * a family COULD tint its icons, and Alma Mater and Roche Limit did: teal and
 * orange on Home, New chat, Crew and every Components row, which put the accent
 * outside the zest list and gave one column two icon inks. The accent belongs to
 * the active rail and the lock badge. Parchment always passed the label ink
 * through, and Crew's sidebar (the reference design) reads the same token.
 */
describe.each(['light', 'dark'] as const)('%s: nav icons wear the label ink', (mode) => {
  it.each(Object.keys(FAMILIES) as (keyof typeof FAMILIES)[])('%s', (id) => {
    const tokens = FAMILIES[id][mode].tokens as Record<string, string>;
    expect(tokens['sidebar-icon']).toBe(tokens['sidebar-foreground']);
    expect(tokens['sidebar-icon']).not.toBe(tokens['text-accent']);
  });
});
