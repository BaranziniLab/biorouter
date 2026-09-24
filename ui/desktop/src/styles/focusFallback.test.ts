import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';

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
    expect(names).toContain(":where([role='tab']:not(.br-tab)):focus-visible");
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
   * The label underlines in the focus token; `::after` stays the selection bar.
   */
  it('underlines a focused tab’s label in the focus token', () => {
    const label = onlyRule(":where([role='tab']:not(.br-tab)):focus-visible");
    expect(isLayered(label)).toBe(false);
    const body = label.body.replace(/\s+/g, ' ');
    expect(body).toContain('text-decoration-line: underline');
    expect(body).toMatch(/text-decoration-thickness: 2px/);
    expect(body).toContain('text-decoration-color: var(--border-focus)');
    expect(body).not.toMatch(/box-shadow|background/);
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

describe('the document declares its language (T-19)', () => {
  it('sets lang on <html>', () => {
    const html = readFileSync(join(HERE, '..', '..', 'index.html'), 'utf8');
    expect(html).toMatch(/<html\s[^>]*\blang="en"/);
  });
});
