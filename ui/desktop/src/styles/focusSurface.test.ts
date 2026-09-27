import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * D-15's focus surface, on a control that paints its own fill.
 *
 * The base block declares it once for everything —
 * `:where(a, button, …):focus-visible { background-color: var(--background-focus) }`
 * — and `:where()` is there so a component can opt out. What was not noticed is
 * that nothing has to opt out on purpose: a Tailwind `bg-*` utility lives in the
 * `utilities` cascade LAYER, and a layered utility beats every `@layer base`
 * rule outright, whatever either one's specificity is. So the `Button`
 * primitive (`bg-background-medium`, `bg-transparent`, …) and the two Knowledge
 * pickers (`bg-background-default`) had NO focus indication at all.
 *
 * Measured in Chrome on the KB selector with a real Tab press and
 * `:focus-visible` matching — before: `backgroundColor: rgb(255, 255, 255)`,
 * `outline: none`, `boxShadow: none`, border unchanged, while
 * `--background-focus` was `#e0e0dc`. After: `rgb(224, 224, 220)`, outline and
 * shadow still `none`.
 *
 * ⚠ **Asserted at the SOURCE, and it has to be** — the same reason
 * `composerFocus.test.ts` gives. jsdom has no layout engine, never runs
 * Tailwind, and does not evaluate `:focus-visible`; a component test that
 * focuses the trigger and reads `backgroundColor` sees the resting value and
 * passes whether the rule exists or not.
 *
 * ⚠ **And the fix is authored CSS with an opt-in class, not a
 * `focus-visible:bg-*` utility at the call site.** A *newly written* Tailwind
 * class can silently fail to generate (see `composerFocus.test.ts` for the
 * three spellings that were each measured absent from the stylesheet), and a
 * focus indicator must not depend on class-scanning having worked.
 */
const CSS = readFileSync(join(__dirname, 'main.css'), 'utf8');
const BUTTON = readFileSync(join(__dirname, '../components/ui/button.tsx'), 'utf8');
const KB_TRIGGER = readFileSync(
  join(__dirname, '../components/knowledge/KBSelector/KBSelectorTrigger.tsx'),
  'utf8'
);
const MODEL_PICKER = readFileSync(
  join(__dirname, '../components/knowledge/IngestPanel/IngestModelPicker.tsx'),
  'utf8'
);
const PASTE_BOX = readFileSync(
  join(__dirname, '../components/knowledge/IngestPanel/PasteTextBox.tsx'),
  'utf8'
);

function ruleBody(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = CSS.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`));
  expect(match, `no rule for ${selector}`).toBeTruthy();
  return match![1];
}

describe('the focus surface', () => {
  it('is declared as authored CSS, not left to a Tailwind variant', () => {
    expect(ruleBody('.biorouter-focus-surface:focus-visible')).toContain('var(--background-focus)');
  });

  /**
   * The rule has to sit OUTSIDE every `@layer` — that, not specificity, is what
   * lets it reach past the `utilities` layer a `bg-*` utility lives in. The
   * check is structural: count the braces of every `@layer …{` block before the
   * rule and confirm they have all closed.
   */
  it('is unlayered, which is the whole mechanism', () => {
    const index = CSS.indexOf('.biorouter-focus-surface:focus-visible');
    expect(index).toBeGreaterThan(-1);
    const before = CSS.slice(0, index);
    // Depth of nesting from `@layer name {` blocks that are still open here.
    let depth = 0;
    const layerStarts: number[] = [];
    for (let i = 0; i < before.length; i++) {
      if (before[i] === '{') {
        if (/@layer\s+[\w\s,-]*$/.test(before.slice(Math.max(0, i - 40), i))) {
          layerStarts.push(depth);
        }
        depth++;
      } else if (before[i] === '}') {
        depth--;
        if (layerStarts.length && layerStarts[layerStarts.length - 1] === depth) {
          layerStarts.pop();
        }
      }
    }
    expect(layerStarts).toHaveLength(0);
  });

  /**
   * A solid accent fill cannot take the grey: `--text-on-accent` is white, and
   * white on `--background-focus` is unreadable. §3.1's rule for a solid fill is
   * that it brightens its own token.
   */
  it('gives a solid accent fill its own step instead of the grey', () => {
    const accent = ruleBody('.biorouter-focus-surface-accent:focus-visible');
    expect(accent).toContain('var(--background-accent-hover)');
    expect(accent).not.toContain('--background-focus');
  });

  /**
   * Focus is a surface shift plus an INSET edge. The ring belongs to the
   * `prefers-contrast` block, and an outset shadow would be a ring by another
   * name — so every shadow layer here is `inset`, or the pass-through of the
   * control's own `--tw-shadow` (live QA 2026-09-24, T-16: the fill alone
   * measured 1.23–1.44:1; `focusFallback.test.ts` pins the edge itself).
   */
  it('never draws a ring', () => {
    for (const selector of [
      '.biorouter-focus-surface:focus-visible',
      '.biorouter-focus-surface-accent:focus-visible',
    ]) {
      const body = ruleBody(selector);
      expect(body).toContain('outline: none');
      expect(body).not.toMatch(/outline:\s*\d/);
    }
    for (const selector of [
      '.biorouter-focus-surface:not(input, textarea, select):focus-visible',
      '.biorouter-focus-surface-accent:focus-visible',
    ]) {
      const shadow = ruleBody(selector).match(/box-shadow:([^;]*);/)?.[1] ?? '';
      expect(shadow, selector).toMatch(/inset 0 0 0 2px/);
      const layers = shadow.split(/,(?![^(]*\))/).map((layer) => layer.trim());
      for (const layer of layers) {
        expect(layer.startsWith('inset ') || layer.startsWith('var(--tw-shadow'), layer).toBe(true);
      }
    }
  });

  it('firms a text field’s existing edge rather than adding a strip', () => {
    expect(ruleBody(':is(input, textarea, select).biorouter-focus-surface:focus')).toContain(
      'var(--border-focus)'
    );
  });

  /**
   * Every `Button` variant that writes its own fill has to opt in, or it keeps
   * the invisible focus. `link` is deliberately absent: it paints no box, so the
   * base rule still reaches it.
   */
  it('is carried by every Button variant that paints a fill', () => {
    for (const variant of ['destructive', 'outline', 'secondary', 'ghost']) {
      const line = BUTTON.match(new RegExp(`${variant}:[\\s\\S]{0,220}?',`))?.[0] ?? '';
      expect(line, variant).toContain('biorouter-focus-surface');
    }
    expect(BUTTON).toMatch(/default:[\s\S]{0,220}?biorouter-focus-surface-accent/);
    // `link` has no fill of its own, so it must NOT take the class.
    expect(BUTTON).not.toMatch(/link: 'text-text-accent[^']*biorouter-focus-surface/);
  });

  it('is carried by the Knowledge controls that write their own background', () => {
    expect(KB_TRIGGER).toContain('biorouter-focus-surface');
    expect(MODEL_PICKER).toContain('biorouter-focus-surface');
    expect(PASTE_BOX).toContain('biorouter-focus-surface');
  });
});
