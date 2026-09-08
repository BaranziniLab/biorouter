import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * D-15's focus surface, on a TAB TRIGGER — where it must not appear.
 *
 * D-15 makes focus a surface shift: `:where(a, button, …):focus-visible` paints
 * `--background-focus`. Radix `TabsTrigger` (`components/ui/tabs.tsx`) renders a
 * `<button role="tab">` and activates on focus, so the focused tab is ALWAYS the
 * active tab — the fill therefore parked itself permanently on the active tab as
 * a grey box around its accent underline. Reported 2026-09-08 ("that weird
 * shade") against the Settings strip, which `settings/providers/ProviderCatalog`
 * reuses.
 *
 * Measured in the running app (Parchment light, `#/settings`) before the fix:
 * a mouse click on `[data-testid="settings-app-tab"]` left
 * `backgroundColor: rgba(0, 0, 0, 0)` (Chrome withholds `:focus-visible` from a
 * mouse-clicked button), but one ArrowLeft inside the strip gave
 * `backgroundColor: rgb(224, 224, 220)` — `--background-focus` = `#e0e0dc` —
 * with `:focus-visible` matching, and the winning rule read off CDP's
 * `CSS.getMatchedStylesForNode` was the D-15 block in `@layer base`. After the
 * fix both paths read `rgba(0, 0, 0, 0)`.
 *
 * ⚠ **Asserted at the SOURCE, and it has to be** — the same reason
 * `composerFocus.test.ts` and `focusSurface.test.ts` give. jsdom has no layout
 * engine, never runs Tailwind, and does not evaluate `:focus-visible`; a
 * component test that focuses a trigger and reads `backgroundColor` sees the
 * resting value and passes whether the rule exists or not.
 *
 * ⚠ **Three arms of the D-15 list match one tab trigger**, which is why the
 * exemption is a `:not([role='tab'])` around the whole list rather than the
 * removal of `[role='tab']` from it: the trigger is a `<button>`, it carries
 * `role="tab"`, and Radix's roving tabindex gives the ACTIVE trigger
 * `tabindex="0"` so it also matches `[tabindex]:not([tabindex='-1'])` — all
 * three verified on the running app. Deleting only the obvious arm is the
 * plausible wrong fix, and it changes nothing on screen.
 */
const CSS = readFileSync(join(__dirname, 'main.css'), 'utf8');
const TABS = readFileSync(join(__dirname, '../components/ui/tabs.tsx'), 'utf8');

/**
 * Every rule in the file that sets a background on a `:focus-visible` selector,
 * as `[selector, body]`. Comments are stripped first so the prose around the
 * rules — which quotes selectors — can neither satisfy nor trip an assertion.
 */
let cached: Array<[string, string]> | null = null;
function focusVisibleBackgroundRules(): Array<[string, string]> {
  if (cached) return cached;
  const withoutComments = CSS.replace(/\/\*[\s\S]*?\*\//g, '');
  const out: Array<[string, string]> = [];
  const re = /([^{}]*:focus-visible[^{}]*)\{([^}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(withoutComments))) {
    if (/background(-color)?\s*:/.test(m[2])) out.push([m[1].trim(), m[2].trim()]);
  }
  cached = out;
  return out;
}

/** The D-15 base rule: the one that paints `--background-focus` on a plain control. */
const d15 = focusVisibleBackgroundRules().find(
  ([selector, body]) =>
    body.includes('var(--background-focus)') &&
    /\bbutton\b/.test(selector) &&
    !/prefers-contrast/.test(selector)
);

describe('a tab trigger takes no focus fill', () => {
  it('still exists, and still fills everything that is not a tab', () => {
    expect(d15, 'the D-15 focus-visible fill rule is no longer recognisable').toBeTruthy();
    expect(d15![1]).toContain('var(--background-focus)');
    expect(d15![0]).toContain(":not([role='tab'])");
  });

  /**
   * The exemption has to survive the two arms that are NOT `[role='tab']`.
   * `button` and `[tabindex]:not([tabindex='-1'])` each match a Radix trigger on
   * their own, so an exemption that merely drops `[role='tab']` from the list
   * leaves the grey box exactly where it was.
   */
  it('exempts them by excluding the whole list, not by dropping one arm', () => {
    const selector = d15![0].replace(/\s+/g, '');
    expect(selector).toMatch(/\bbutton\b/);
    expect(selector).toContain("[tabindex]:not([tabindex='-1'])");
    // The `:not()` closes the alternation and sits inside the outer `:where()`,
    // so it applies to every arm rather than to one alternative within it.
    expect(selector).toMatch(/\):not\(\[role='tab'\]\)\):focus-visible$/);
  });

  /** Specificity 0 is D-15's contract: any component can still opt out. */
  it('keeps the rule at specificity 0 by nesting the :not() inside :where()', () => {
    expect(d15![0].replace(/\s+/g, '').startsWith(':where(')).toBe(true);
  });

  /**
   * The operator's requirement, stated as a property of the whole stylesheet
   * rather than of one rule. Pseudo-element rules are exempt: the underline
   * indicator below paints `::after`, which is the tab's own bar, not a box
   * around the label.
   */
  it('has no rule anywhere that paints a background on a focused tab', () => {
    const offenders = focusVisibleBackgroundRules().filter(([selector]) => {
      if (!/\[role=['"]tab['"]\]/.test(selector)) return false;
      if (/::(after|before)/.test(selector)) return false;
      // A selector that mentions tabs only in order to exclude them is fine.
      return !/:not\(\s*\[role='tab'\]\s*\)/.test(selector);
    });
    expect(offenders, `these rules fill a focused tab: ${JSON.stringify(offenders)}`).toEqual([]);
  });

  it('leaves the opt-in focus surface untouched for other controls', () => {
    const filled = focusVisibleBackgroundRules().map(([selector]) => selector);
    expect(filled.some((s) => s.includes('.biorouter-focus-surface:focus-visible'))).toBe(true);
  });
});

describe('a focused tab shows its underline firming instead', () => {
  const UNDERLINE_SELECTOR = ":where([role='tab']:not(.br-tab)):focus-visible::after";
  const LABEL_SELECTOR = ":where([role='tab']:not(.br-tab)):focus-visible";

  function body(selector: string): string {
    const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const match = CSS.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`));
    expect(match, `no rule for ${selector}`).toBeTruthy();
    return match![1];
  }

  it('thickens the accent bar and firms the label', () => {
    const bar = body(UNDERLINE_SELECTOR);
    expect(bar).toMatch(/height:\s*3px/);
    expect(bar).toContain('var(--accent-bar)');
    expect(body(LABEL_SELECTOR)).toContain('var(--text-default)');
  });

  /** D-15 forbids a ring; the ring comes back only under `prefers-contrast`. */
  it('never draws a ring and never paints a fill', () => {
    for (const selector of [UNDERLINE_SELECTOR, LABEL_SELECTOR]) {
      expect(body(selector)).not.toMatch(/box-shadow|outline:\s*\d/);
      expect(body(selector)).not.toContain('--background-focus');
    }
    expect(body(LABEL_SELECTOR)).toContain('outline: none');
  });

  /**
   * ⚠ **Unlayered, and that is the whole mechanism** — the same structural check
   * `focusSurface.test.ts` runs. The bar's height and colour come from Tailwind
   * utilities (`after:h-0.5`, `data-[state=active]:after:bg-accent-bar`), and
   * every layered utility beats every `@layer base` rule whatever the
   * specificity. Inside `@layer base` this rule would lose silently, the bar
   * would stay 2px, and nothing here would fail.
   */
  it('is unlayered, so it can reach past the utilities layer', () => {
    const index = CSS.indexOf(UNDERLINE_SELECTOR + ' {');
    expect(index).toBeGreaterThan(-1);
    const before = CSS.slice(0, index);
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
   * The underline half must not reach `.br-tab` (chat header / artifact panel /
   * dock). That component also carries `role="tab"`, draws no underline, and
   * already owns its `::after` — `.br-tab[data-dropbefore='true']::after` is the
   * drag insertion hairline, sized by `top`/`bottom` with no `height`, which a
   * `height: 3px` would over-constrain into a stub.
   */
  it('does not reach .br-tab, which owns its ::after for the drop hairline', () => {
    expect(UNDERLINE_SELECTOR).toContain(':not(.br-tab)');
    expect(CSS).toContain(".br-tab[data-dropbefore='true']::after");
  });

  /**
   * The rule matches nothing without its hook, and the hook is the `after:` bar
   * Radix's trigger draws — so the two are asserted together or they can drift
   * apart silently.
   */
  it('has its hook on the TabsTrigger primitive', () => {
    expect(TABS).toContain('TabsPrimitive.Trigger');
    expect(TABS).toContain("after:content-['']");
    expect(TABS).toContain('data-[state=active]:after:bg-accent-bar');
  });
});
