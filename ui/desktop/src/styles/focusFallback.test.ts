import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';
import * as themeTokens from '../../scripts/lib/theme-tokens.mjs';

/**
 * Two focus defects from the Crew live QA round (2026-09-24), asserted at the
 * SOURCE because jsdom has no cascade layers, no `:focus-visible`, no
 * `prefers-contrast` and no forced colours — a component test that focuses a
 * control and reads its outline passes whether any of this exists or not.
 *
 * **P0-5 — the OS-requested ring could never win.** D-15 makes focus a quiet
 * surface shift and promises a real ring to anyone who asked their OS for a
 * stronger signal (`prefers-contrast: more` — macOS Increase Contrast;
 * `forced-colors: active` — Windows High Contrast). That ring was declared
 * inside `@layer base`, and every UNLAYERED declaration beats every layered
 * one whatever the specificity. The quiet-focus rules that write
 * `outline: none` are unlayered on purpose (`.biorouter-focus-surface`, the tab
 * label rule, the Crew sidebar rows and timeline), so under Increase Contrast
 * 12 of 14 Crew stops had no ring, and under forced colours — which also strip
 * the fill — focus was invisible on every Crew button, tab and menu item. Menu
 * items (`div[role=menuitem][tabindex=-1]`) were not even matched: the old
 * selector said `[tabindex]:not([tabindex='-1'])`.
 *
 * **T-16 — the default indicator was not an indicator.** The focus fill
 * measured 1.10–1.44:1 against the resting control (3:1 is owed), the selected
 * tab and the app nav's active item changed by nothing at all, and the sidebar
 * resize handle's focus was indistinguishable from rest. Each now draws a
 * neutral `--border-focus` mark that `check-contrast.mjs` holds to 3:1 against
 * every ground in all six scopes.
 */
const HERE = __dirname;
const CSS = readFileSync(join(HERE, 'main.css'), 'utf8');

type Rule = {
  selector: string;
  body: string;
  /** Offset of the rule's selector in the file. */
  index: number;
  /** Enclosing at-rule preludes, outermost first: `['@layer base']`, `['@media …']`. */
  context: string[];
};

/**
 * Every style rule in a stylesheet with the at-rules that enclose it. Comments
 * are blanked (not removed) first, so offsets still point into the real file
 * and the prose around the rules — which quotes selectors — can neither
 * satisfy nor trip an assertion.
 */
function parseRules(css: string): Rule[] {
  const src = css.replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, ' '));
  const rules: Rule[] = [];
  const stack: string[] = [];
  let segStart = 0;
  for (let i = 0; i < src.length; i++) {
    const c = src[i];
    if (c === ';') {
      segStart = i + 1;
    } else if (c === '}') {
      stack.pop();
      segStart = i + 1;
    } else if (c === '{') {
      const raw = src.slice(segStart, i);
      const prelude = raw.trim();
      if (prelude.startsWith('@')) {
        stack.push(prelude.replace(/\s+/g, ' '));
        segStart = i + 1;
        continue;
      }
      let depth = 1;
      let j = i + 1;
      for (; j < src.length && depth > 0; j++) {
        if (src[j] === '{') depth++;
        else if (src[j] === '}') depth--;
      }
      rules.push({
        selector: prelude.replace(/\s+/g, ' '),
        body: src.slice(i + 1, j - 1),
        index: segStart + (raw.length - raw.trimStart().length),
        context: [...stack],
      });
      i = j - 1;
      segStart = j;
    }
  }
  return rules;
}

const RULES = parseRules(CSS);
const isLayered = (rule: Rule) => rule.context.some((at) => at.startsWith('@layer'));
const squash = (s: string) => s.replace(/\s+/g, '');
const OS_MEDIA = '@media (prefers-contrast: more), (forced-colors: active)';

/** The one rule a selector names, failing loudly when it is gone or doubled. */
function onlyRule(selector: string): Rule {
  const found = RULES.filter((r) => r.selector === selector);
  expect(found, `expected exactly one rule for ${selector}`).toHaveLength(1);
  return found[0];
}

/** Every `.css` file under `src/` that ships in the renderer (the web bundle is generated). */
function rendererCssFiles(): string[] {
  const out: string[] = [];
  const walk = (dir: string) => {
    for (const name of readdirSync(dir)) {
      const path = join(dir, name);
      if (statSync(path).isDirectory()) {
        if (path === join(HERE, '..', 'web') || name === 'node_modules') continue;
        walk(path);
      } else if (name.endsWith('.css')) {
        out.push(path);
      }
    }
  };
  walk(join(HERE, '..'));
  return out;
}

describe('the OS-requested focus ring (P0-5)', () => {
  const ringRules = RULES.filter(
    (r) => r.context.join(' ') === OS_MEDIA && /outline:\s*2px solid var\(--ring\)/.test(r.body)
  );

  it('exists exactly once, and is not inside any cascade layer', () => {
    expect(ringRules, 'the prefers-contrast / forced-colors ring rule is gone').toHaveLength(1);
    expect(isLayered(ringRules[0])).toBe(false);
    // No copy may survive inside a layer: it is dead code there, and a reader
    // would take it for the working one.
    const layeredCopies = RULES.filter(
      (r) => isLayered(r) && r.context.some((at) => /prefers-contrast|forced-colors/.test(at))
    );
    expect(layeredCopies.map((r) => r.selector)).toEqual([]);
  });

  it('declares the ring !important, because Crew stylesheets load after this one', () => {
    const body = ringRules[0].body.replace(/\s+/g, ' ');
    expect(body).toContain('outline: 2px solid var(--ring) !important');
    expect(body).toMatch(/outline-offset: -?\d+px !important/);
    // The old block wrote `border-radius: inherit`, re-shaping the control.
    expect(body).not.toMatch(/border-radius/);
  });

  it('reaches every focusable shape, including rows that carry tabindex="-1"', () => {
    const selector = squash(ringRules[0].selector);
    for (const arm of [
      'a',
      'button',
      'input',
      'textarea',
      'select',
      'summary',
      "[role='button']",
      "[role='menuitem']",
      "[role='menuitemcheckbox']",
      "[role='menuitemradio']",
      "[role='option']",
      "[role='tab']",
      "[role='log']",
      "[role='region']",
    ]) {
      expect(selector, arm).toMatch(
        new RegExp(`[(,]${arm.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}[,)]`)
      );
    }
    // BARE `[tabindex]`: a menu item, a roving-tabindex row and a heading
    // focused programmatically all carry tabindex="-1".
    expect(selector).toContain(',[tabindex]):focus-visible');
    expect(selector).not.toContain('[tabindex]:not(');
    expect(selector).toContain("[role='menuitem'][data-highlighted]");
    // Nothing is exempted from the ring the user asked for.
    expect(selector).not.toMatch(/:not\(/);
  });

  it('comes after every unlayered rule that suppresses a focus outline', () => {
    const suppressors = RULES.filter(
      (r) =>
        !isLayered(r) &&
        r !== ringRules[0] &&
        /:focus/.test(r.selector) &&
        /(^|;|\s)outline:\s*(none|0)\b/.test(r.body)
    );
    // The known ones, so the list cannot silently be empty.
    const names = suppressors.map((r) => r.selector);
    expect(names).toContain('.biorouter-focus-surface:focus-visible');
    // A Radix tab trigger trades its outline for a label-sized ring on ::before (Q2-49).
    expect(names).toContain(":where([role='tab'][data-orientation]:not(.br-tab)):focus-visible");
    expect(names).toContain('.biorouter-sidebar-resize-handle:focus-visible');
    for (const rule of suppressors) {
      expect(rule.index, `${rule.selector} comes after the OS ring`).toBeLessThan(
        ringRules[0].index
      );
    }
  });

  it('names the user’s own focus colour under forced colours', () => {
    const forced = RULES.filter(
      (r) =>
        !isLayered(r) &&
        r.context.join(' ') === '@media (forced-colors: active)' &&
        /outline-color:\s*Highlight\s*!important/.test(r.body)
    );
    expect(forced).toHaveLength(1);
    expect(squash(forced[0].selector).split(',')).toContain(':focus-visible');
    expect(forced[0].index).toBeGreaterThan(ringRules[0].index);
  });

  it('insets the ring on rows, regions and separators so a scroller cannot clip it', () => {
    const inset = RULES.filter(
      (r) => r.context.join(' ') === OS_MEDIA && /outline-offset:\s*-2px\s*!important/.test(r.body)
    );
    expect(inset).toHaveLength(1);
    expect(inset[0].index).toBeGreaterThan(ringRules[0].index);
    const selector = squash(inset[0].selector);
    for (const role of ['menuitem', 'option', 'log', 'region', 'separator']) {
      expect(selector).toContain(`[role='${role}']`);
    }
  });

  /**
   * The ring wins by being `!important`. Two things could still beat it: a
   * LAYERED important (Tailwind's `outline-none!` — for importance, earlier
   * layers win and unlayered loses), or an unlayered important in a stylesheet
   * that loads later (every Crew `.css`). Neither may exist.
   */
  it('cannot be out-ranked by an important outline anywhere in the renderer', () => {
    const IMPORTANT_OUTLINE = /outline(-style|-width|-color|-offset)?\s*:[^;{}]*!important/;
    const OS_CONTEXTS = new Set([OS_MEDIA, '@media (forced-colors: active)']);
    const offenders: string[] = [];
    for (const file of rendererCssFiles()) {
      const rules =
        file === join(HERE, 'main.css') ? RULES : parseRules(readFileSync(file, 'utf8'));
      for (const rule of rules) {
        if (!IMPORTANT_OUTLINE.test(rule.body)) continue;
        // The OS ring itself, in main.css, is the one allowed.
        if (file === join(HERE, 'main.css') && OS_CONTEXTS.has(rule.context.join(' '))) continue;
        offenders.push(`${relative(HERE, file)}: ${rule.selector}`);
      }
    }
    expect(offenders).toEqual([]);

    const utilityOffenders: string[] = [];
    const walk = (dir: string) => {
      for (const name of readdirSync(dir)) {
        const path = join(dir, name);
        if (statSync(path).isDirectory()) {
          if (path === join(HERE, '..', 'web') || name === 'node_modules') continue;
          walk(path);
        } else if (/\.(tsx?|jsx?)$/.test(name) && !/\.test\./.test(name)) {
          const text = readFileSync(path, 'utf8');
          // Tailwind v4's important modifier is a suffix (`outline-none!`), v3's a
          // prefix (`!outline-none`), and either may follow a variant (`focus:`).
          const hit = text.match(/(^|[\s'"`:])(!outline-[\w-]+|outline-[\w-]+!)(?=[\s'"`])/);
          if (hit) utilityOffenders.push(`${relative(HERE, path)}: ${hit[2]}`);
        }
      }
    };
    walk(join(HERE, '..'));
    expect(utilityOffenders).toEqual([]);
  });
});

describe('the default focus indicator is an edge, not only a fill (T-16)', () => {
  /** The D-15 rule: the one that paints `--background-focus` on a plain control. */
  const d15 = RULES.find(
    (r) =>
      isLayered(r) &&
      /:focus-visible$/.test(r.selector) &&
      /\bbutton\b/.test(r.selector) &&
      r.body.includes('var(--background-focus)')
  );

  it('adds a 2px inset --border-focus edge to the base fill', () => {
    expect(d15, 'the D-15 base rule is unrecognisable').toBeTruthy();
    expect(d15!.body.replace(/\s+/g, ' ')).toContain(
      'box-shadow: inset 0 0 0 2px var(--border-focus)'
    );
    // Still no ring outside the OS block.
    expect(d15!.body).toMatch(/outline:\s*none/);
  });

  /**
   * Menu rows are where the fill alone measured ≈1.10:1. Radix renders them as
   * `div[role=menuitem*][tabindex=-1]`, so the `[tabindex]` arm never reaches
   * them — each role has to be named.
   */
  it('reaches every menu row and option role', () => {
    const selector = squash(d15!.selector);
    for (const role of ['menuitem', 'menuitemcheckbox', 'menuitemradio', 'option']) {
      expect(selector).toContain(`[role='${role}']`);
    }
  });

  it('takes the edge back from a text field, which is focus-visible on a click', () => {
    const field = RULES.find(
      (r) =>
        isLayered(r) &&
        /textarea/.test(r.selector) &&
        /:focus-visible/.test(r.selector) &&
        r.body.includes('var(--border-focus)')
    );
    expect(field, 'the text-field focus rule is unrecognisable').toBeTruthy();
    expect(field!.body).toMatch(/box-shadow:\s*none/);
    expect(field!.index).toBeGreaterThan(d15!.index);
  });

  it('draws the same edge on the opt-in focus surface, unlayered, never on a text field', () => {
    const surface = onlyRule('.biorouter-focus-surface:not(input, textarea, select):focus-visible');
    expect(isLayered(surface)).toBe(false);
    expect(surface.body.replace(/\s+/g, ' ')).toContain('inset 0 0 0 2px var(--border-focus)');
    // It keeps a control's own drop shadow rather than erasing it.
    expect(surface.body).toContain('var(--tw-shadow');
  });

  it('draws the accent control’s edge in its label ink, which clears 3:1 on the accent', () => {
    const accent = onlyRule('.biorouter-focus-surface-accent:focus-visible');
    expect(isLayered(accent)).toBe(false);
    expect(accent.body.replace(/\s+/g, ' ')).toContain('inset 0 0 0 2px var(--text-on-accent)');
  });

  /**
   * The selected tab is always the focused tab (activation follows focus), and
   * its bar is already painted — the audit's two captures were pixel-identical.
   * Round 1 underlined the label in the focus token; round 2 (Q2-49) read that
   * grey underline over the accent bar as "a double underline". Focus is now a
   * RING around the label in the focus token: a different shape from the
   * selection bar, and `::after` stays the selection bar.
   */
  it('rings a focused tab’s label in the focus token, not an underline', () => {
    const tab = onlyRule(":where([role='tab']:not(.br-tab)):focus-visible");
    expect(isLayered(tab)).toBe(false);
    const body = tab.body.replace(/\s+/g, ' ');
    expect(body).toContain('outline: 2px solid var(--border-focus)');
    expect(body).toContain('outline-offset: 2px');
    expect(body).toContain('border-radius: var(--radius-element)');
    expect(body).not.toMatch(/text-decoration|box-shadow|background/);
  });

  /**
   * A Radix trigger is 36px tall for a 20px label, and Crew's details pane pins
   * its tablist flush against the top of its scroller, which would clip an
   * outset outline's top edge. The same ring is drawn on `::before`, sized to
   * the label and inside the trigger's height.
   */
  it('draws a Radix trigger’s ring on ::before, sized to the label', () => {
    const radix = ":where([role='tab'][data-orientation]:not(.br-tab))";
    expect(onlyRule(`${radix}:focus-visible`).body).toMatch(/outline:\s*none/);
    const ring = onlyRule(`${radix}:focus-visible::before`);
    expect(isLayered(ring)).toBe(false);
    const body = ring.body.replace(/\s+/g, ' ');
    expect(body).toContain('border: 2px solid var(--border-focus)');
    expect(body).toContain('border-radius: var(--radius-element)');
    expect(body).toContain('position: absolute');
    expect(body).toMatch(/inset-block: calc\(50% - 14px\)/);
    // The trigger this relies on is positioned, and `::after` is its selection bar.
    const tabs = readFileSync(join(HERE, '..', 'components', 'ui', 'tabs.tsx'), 'utf8');
    expect(tabs).toMatch(/"relative inline-flex/);
    expect(tabs).not.toMatch(/before:/);
  });

  it('insets a document tab’s ring inside its pill', () => {
    expect(onlyRule(":where(.br-tab [role='tab']):focus-visible").body).toMatch(
      /outline-offset:\s*-2px/
    );
  });

  /** One ring when the OS asked for one: the outline, not the pseudo ring as well. */
  it('stands the pseudo ring down under the OS-requested ring', () => {
    const off = onlyRule("[role='tab'][data-orientation]:focus-visible::before");
    expect(off.context.join(' ')).toBe(OS_MEDIA);
    expect(off.body).toMatch(/content:\s*none/);
  });

  it('paints the focused sidebar resize handle, not a hover-grade hairline', () => {
    const handle = onlyRule('.biorouter-sidebar-resize-handle:focus-visible');
    expect(isLayered(handle)).toBe(false);
    expect(handle.body).toMatch(/background-color:\s*var\(--border-focus\)/);
    expect(onlyRule('.biorouter-sidebar-resize-handle:focus-visible::after').body).toMatch(
      /background:\s*var\(--border-focus\)/
    );
    // Focus no longer shares the hover hairline's `--border-strong`.
    const hover = RULES.find((r) => r.selector.includes('.biorouter-sidebar-resize-handle:hover'));
    expect(hover!.selector).not.toContain(':focus-visible');
  });
});

/**
 * Q2-11 and Q2-48 (live QA round 2). Forced colours repaint every background
 * as Canvas, so any shape or state that was only a FILL vanished — the checked
 * radio's dot, the selected tab's bar, a filled button's box, the tooltip's
 * box, a menu separator. And the radio's focus was drawn on its sr-only input,
 * a 1×1px clipped box. Asserted at the source: jsdom has neither forced colours
 * nor `:focus-visible`.
 */
describe('painted shapes survive forced colours, and a radio shows focus (Q2-11, Q2-48)', () => {
  const FORCED = '@media (forced-colors: active)';
  const forced = (selector: string) => {
    const found = RULES.filter(
      (r) => r.context.join(' ') === FORCED && squash(r.selector) === squash(selector)
    );
    expect(found, `expected exactly one forced-colours rule for ${selector}`).toHaveLength(1);
    expect(isLayered(found[0])).toBe(false);
    return found[0].body.replace(/\s+/g, ' ');
  };
  const source = (...path: string[]) => readFileSync(join(HERE, '..', ...path), 'utf8');

  it('rings the visible radio ring when its hidden input has keyboard focus', () => {
    // Top level, in every mode: not layered and not inside a media query.
    const rings = RULES.filter(
      (r) =>
        r.selector === "input[type='radio']:focus-visible ~ [data-radio-ring]" &&
        r.context.length === 0
    );
    expect(rings).toHaveLength(1);
    const ring = rings[0];
    const body = ring.body.replace(/\s+/g, ' ');
    expect(body).toContain('outline: 2px solid var(--ring)');
    expect(body).toContain('outline-offset: 2px');
  });

  it('redraws the radio ring and its checked dot in system colours', () => {
    expect(forced('[data-radio-ring]')).toMatch(
      /forced-color-adjust: none;.*border-color: CanvasText/
    );
    expect(forced("input[type='radio']:checked ~ [data-radio-ring]")).toContain(
      'border-color: Highlight'
    );
    expect(forced("input[type='radio']:disabled ~ [data-radio-ring]")).toContain(
      'border-color: GrayText'
    );
    expect(forced("input[type='radio']:focus-visible ~ [data-radio-ring]")).toContain(
      'outline-color: Highlight'
    );
    const dot = forced('[data-radio-dot]');
    expect(dot).toContain('forced-color-adjust: none');
    expect(dot).toContain('background-color: Highlight');
    expect(forced("input[type='radio']:disabled ~ [data-radio-dot]")).toContain(
      'background-color: GrayText'
    );
  });

  it('keeps the hooks those rules key on in CustomRadio, input first', () => {
    const radio = source('components', 'ui', 'CustomRadio.tsx');
    const input = radio.indexOf('type="radio"');
    expect(input).toBeGreaterThan(-1);
    expect(radio.indexOf('data-radio-ring=""')).toBeGreaterThan(input);
    expect(radio.indexOf('data-radio-dot=""')).toBeGreaterThan(radio.indexOf('data-radio-ring=""'));
  });

  it('paints the selected tab’s bar in Highlight, not a notch', () => {
    const bar = forced("[role='tab'][data-state='active']:not(.br-tab)::after");
    expect(bar).toContain('background-color: Highlight');
    expect(bar).toContain('forced-color-adjust: none');
  });

  it('gives every button, and the tooltip, a system edge', () => {
    expect(forced("[data-slot='button']")).toContain('border: 1px solid ButtonText');
    expect(forced("[data-slot='tooltip-content']")).toContain('border: 1px solid CanvasText');
    // The slots are the components' own; a rename would orphan the rule silently.
    expect(source('components', 'ui', 'button.tsx')).toContain('data-slot="button"');
    expect(source('components', 'ui', 'Tooltip.tsx')).toContain('data-slot="tooltip-content"');
  });

  it('paints menu and panel separators in CanvasText', () => {
    const separators = RULES.filter(
      (r) =>
        r.context.join(' ') === FORCED &&
        squash(r.selector).includes("[data-slot='dropdown-menu-separator']")
    );
    expect(separators).toHaveLength(1);
    expect(isLayered(separators[0])).toBe(false);
    const selector = squash(separators[0].selector);
    for (const slot of ['context-menu-separator', 'separator-root', 'sidebar-separator']) {
      expect(selector).toContain(`[data-slot='${slot}']`);
    }
    const body = separators[0].body.replace(/\s+/g, ' ');
    expect(body).toContain('background-color: CanvasText');
    expect(body).toContain('forced-color-adjust: none');
    expect(source('components', 'ui', 'dropdown-menu.tsx')).toContain(
      'data-slot="dropdown-menu-separator"'
    );
  });
});

/**
 * The token maths `check-contrast.mjs` runs, over the same scopes it discovers: the stylesheet's
 * families × light/dark, resolved through their `var()` chains. One implementation, two
 * consumers; `buildScopes` / `resolveHex` are not in the module's `.d.mts`, so they are typed
 * here, where they are used.
 */
type Scope = { decls: Record<string, string>; theme: Record<string, string> };
const tokens = themeTokens as unknown as {
  buildScopes(css: string): Record<string, Scope>;
  resolveHex(name: string, scope: Scope): string | null;
  contrast(a: string, b: string): number;
  blend(fillHex: string, alpha: number, groundHex: string): string;
  hexToLab(hex: string): [number, number, number];
};
const SCOPES = tokens.buildScopes(CSS);

/**
 * Q3-58, Q3-59 (live QA round 3). The unchecked radio's ring was ink at 24% — 1.6:1 on white —
 * and in forced colours every link-style button got the system button edge with no padding, so
 * the line ran through its first and last letters ("Make my connection public…", "Privacy…").
 */
describe('a radio’s ring and a link-style button hold their edges (Q3-58, Q3-59)', () => {
  const FORCED = '@media (forced-colors: active)';
  const top = (selector: string) => {
    const found = RULES.filter((r) => r.selector === selector && r.context.length === 0);
    expect(found, `expected exactly one top-level rule for ${selector}`).toHaveLength(1);
    return found[0];
  };
  const forcedRule = (selector: string) => {
    const found = RULES.filter(
      (r) => r.context.join(' ') === FORCED && squash(r.selector) === squash(selector)
    );
    expect(found, `expected exactly one forced-colours rule for ${selector}`).toHaveLength(1);
    return found[0];
  };

  it('paints the resting ring in --text-muted and the checked one in the accent edge', () => {
    const rest = top('[data-radio-ring]');
    expect(rest.body.replace(/\s+/g, ' ')).toContain('border-color: var(--text-muted)');
    expect(isLayered(rest)).toBe(false);
    const checked = top("input[type='radio']:checked ~ [data-radio-ring]");
    expect(checked.body.replace(/\s+/g, ' ')).toContain('border-color: var(--border-accent)');
    // Nothing else outside forced colours colours the ring — no heavier rule taking it back.
    const colouring = RULES.filter(
      (r) =>
        r.context.length === 0 &&
        r.selector.includes('[data-radio-ring]') &&
        /border-color/.test(r.body)
    );
    expect(colouring.map((r) => r.selector).sort()).toEqual(
      ['[data-radio-ring]', "input[type='radio']:checked ~ [data-radio-ring]"].sort()
    );
  });

  // Forced colours redraw the ring with the SAME selectors and win by coming later; a heavier
  // rest selector would out-rank `CanvasText` and paint the token under forced-color-adjust: none.
  it('lets the forced-colours ring win: same selectors, later in the file', () => {
    expect(forcedRule('[data-radio-ring]').index).toBeGreaterThan(top('[data-radio-ring]').index);
    const checked = "input[type='radio']:checked ~ [data-radio-ring]";
    expect(forcedRule(checked).index).toBeGreaterThan(top(checked).index);
  });

  it('measures at least 3:1 against every ground, in all six scopes', () => {
    const grounds = [
      '--background-app',
      '--background-canvas',
      '--background-default',
      '--background-card',
      '--background-muted',
      '--background-medium',
      '--background-well',
      '--sidebar',
    ];
    expect(Object.keys(SCOPES)).toHaveLength(6);
    const shortfalls: string[] = [];
    for (const [name, scope] of Object.entries(SCOPES)) {
      const ring = tokens.resolveHex('--text-muted', scope);
      expect(ring, `${name}: --text-muted resolves`).toBeTruthy();
      for (const ground of grounds) {
        const hex = tokens.resolveHex(ground, scope);
        expect(hex, `${name}: ${ground} resolves`).toBeTruthy();
        const ratio = tokens.contrast(ring!, hex!);
        if (ratio < 3) shortfalls.push(`${name}: ${ground} ${ratio.toFixed(2)}:1`);
      }
    }
    expect(shortfalls).toEqual([]);
    // The token it replaced, for the record: under 3:1, which is why it went.
    expect(tokens.contrast(tokens.blend('#2a2520', 0.24, '#ffffff'), '#ffffff')).toBeLessThan(3);
  });

  it('draws a link-style button as a link in forced colours: no edge through its text', () => {
    const link = forcedRule("[data-slot='button'][class~='underline-offset-4']");
    const body = link.body.replace(/\s+/g, ' ');
    expect(isLayered(link)).toBe(false);
    expect(body).toContain('border: 0 none');
    expect(body).toContain('color: LinkText');
    expect(body).toContain('text-decoration-line: underline');
    // It out-ranks the every-button edge (0,2,0 over 0,1,0) and sits after it too.
    expect(link.index).toBeGreaterThan(forcedRule("[data-slot='button']").index);
    expect(forcedRule("[data-slot='button'][class~='underline-offset-4']:disabled").body).toMatch(
      /color:\s*GrayText/
    );
    // Its focus is the OS ring every control gets: nothing here takes the outline away.
    expect(body).not.toMatch(/outline/);
  });

  // `Button` writes no variant attribute, so the rule keys on the link variant's own class token.
  // If another variant gained it, that variant would lose its edge; if the link variant lost it,
  // the edge would cut the text again.
  it('keys on a class token only the link variant carries', () => {
    const button = readFileSync(join(HERE, '..', 'components', 'ui', 'button.tsx'), 'utf8');
    const variants = button.slice(button.indexOf('variant: {'), button.indexOf('size: {'));
    const entries = [...variants.matchAll(/^\s+(\w+):\s*\n?\s*'([^']*)'/gm)].map((m) => ({
      name: m[1],
      classes: m[2].split(/\s+/),
    }));
    expect(entries.map((e) => e.name)).toEqual([
      'default',
      'destructive',
      'outline',
      'secondary',
      'ghost',
      'link',
    ]);
    for (const entry of entries) {
      expect(entry.classes.includes('underline-offset-4'), entry.name).toBe(entry.name === 'link');
    }
  });
});

/**
 * Q3-61 (live QA round 3). In dark, avatar hues 2 (orange) and 3 (amber) were the two loudest
 * fills in the set — CIELAB chroma 38 and 37 against 22–33 for the other six — and read as brown
 * and olive beside the calm indigo, teal and plum. They now sit inside the others' range, in all
 * three families. (`check-contrast.mjs` separately holds every ink to 4.5:1 and every two fills
 * ΔE00 8 apart.)
 */
describe('the dark avatar hues are equally calm (Q3-61)', () => {
  const chroma = (hex: string) => {
    const [, a, b] = tokens.hexToLab(hex);
    return Math.hypot(a, b);
  };

  it('keeps orange and amber within the chroma of the other six, in every dark scope', () => {
    const dark = Object.entries(SCOPES).filter(([name]) => name.endsWith(':dark'));
    expect(dark).toHaveLength(3);
    for (const [name, scope] of dark) {
      const fill = (n: number) => tokens.resolveHex(`--avatar-hue-${n}-bg`, scope)!;
      const others = [1, 4, 5, 6, 7, 8].map((n) => chroma(fill(n)));
      const ceiling = Math.max(...others);
      for (const n of [2, 3]) {
        expect(chroma(fill(n)), `${name}: hue ${n} (${fill(n)})`).toBeLessThanOrEqual(ceiling);
      }
    }
  });
});

describe('the document declares its language (T-19)', () => {
  it('sets lang on <html>', () => {
    const html = readFileSync(join(HERE, '..', '..', 'index.html'), 'utf8');
    expect(html).toMatch(/<html\s[^>]*\blang="en"/);
  });
});
