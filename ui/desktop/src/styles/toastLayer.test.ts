import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

import { TOAST_SURFACE_CLASS_NAME } from '../components/alerts/NotificationSurface';
import { SELECT_MENU_SLOT } from '../components/ui/Select';

/**
 * Where notifications appear, and in what order they pile up.
 *
 * Both facts were reported as bugs and both are invisible to every other suite
 * in this repo, for the same reason `measures.test.ts` exists: jsdom has no
 * layout engine and never runs Tailwind, so a toast rendered in a test lands at
 * the same (non-)coordinates whatever the stylesheet says. The only assertable
 * artefacts are the declaration in main.css and the props on the container, so
 * those are what is asserted, at the source.
 *
 * The two rules, in the user's words:
 *   1. a notification always appears in the UPPER-RIGHT CORNER;
 *   2. a second, newer notification is pushed DIRECTLY BELOW the first, so the
 *      oldest sits nearest the corner and the stack grows downward.
 *
 * ⚠ Rule 2 CONTRADICTS docs/design/astryx-adoption/astryx-ui-adoption-design.md
 * §3.7 ("newest nearest the corner"). The user's instruction is what shipped;
 * the doc is what needs reconciling. Anyone arriving here from §3.7 to "fix"
 * `newestOnTop` should change the doc instead.
 */
const CSS = readFileSync(join(__dirname, 'main.css'), 'utf8');
const APP = readFileSync(join(__dirname, '../App.tsx'), 'utf8');

function declaration(name: string): string {
  const match = CSS.match(new RegExp(`--${name}:\\s*([^;]+);`));
  if (!match) throw new Error(`--${name} is not declared in main.css`);
  return match[1].trim();
}

/**
 * Resolve a token whose value is a `calc()` over pixel literals and other
 * pixel-valued tokens. Enough for the inset ladder and nothing more — the point
 * is to assert the NUMBER the corner ends up at, not to reimplement CSS.
 */
function resolvePx(name: string): number {
  const raw = declaration(name).replace(
    /var\(\s*--([a-z-]+)\s*\)/g,
    (_, ref: string) => `${resolvePx(ref)}px`
  );
  const arithmetic = raw
    .replace(/^calc\(/, '')
    .replace(/\)$/, '')
    .replace(/px\b/g, '');
  // Digits, whitespace and the four operators, and nothing else — the guard is
  // what makes evaluating the string safe, so it is deliberately total rather
  // than a "looks numeric" heuristic.
  if (!/^[\d\s.+\-*/]+$/.test(arithmetic)) {
    throw new Error(`--${name} is not a pixel expression: ${raw}`);
  }
  return Number(new Function(`return ${arithmetic};`)());
}

describe('the toast layer sits in the upper-right corner', () => {
  /**
   * The regression, exactly. `--toast-inset-top` was 144px — derived from the
   * tallest page header so a toast could never cover one — which docked the
   * layer in the middle of the content pane instead of the corner. The
   * extension-load toast floating halfway down the chat is what was reported.
   *
   * 64px is a ceiling, not a target: it is the titlebar band with room for one
   * gap and a little slack, and anything past it has stopped being a corner.
   */
  it('starts within the top-corner band, not below the page header', () => {
    expect(resolvePx('toast-inset-top')).toBeLessThanOrEqual(64);
  });

  /**
   * The floor. Electron folds `-webkit-app-region: drag` rects in DOM order,
   * and `.titlebar-drag-region` lives inside App's main tree — LATER than the
   * toast container that sits above it — so a drag rect overlapping the toast
   * eats clicks on its × and its "View details" no matter what `--z-toast`
   * says (issue #74). A toast in that band looks present and is dead.
   */
  it('clears the titlebar drag band it is measured from', () => {
    expect(resolvePx('toast-inset-top')).toBeGreaterThanOrEqual(resolvePx('titlebar-drag-height'));
    expect(declaration('toast-inset-top')).toContain('var(--titlebar-drag-height)');
  });

  /**
   * One number, one place. The drag band and the inset measured from it were
   * two independent literals; asserting the token here is what stops a change
   * to the titlebar's height from silently moving the toast layer into it.
   */
  it('reads the titlebar height from the same token the drag region does', () => {
    expect(CSS).toMatch(/\.titlebar-drag-region\s*\{[^}]*height:\s*var\(--titlebar-drag-height\)/);
  });

  it('docks the container top-right and takes its inset from the token', () => {
    expect(APP).toContain('position="top-right"');
    expect(APP).toContain("top: 'var(--toast-inset-top)'");
  });
});

describe('a newer notification lands directly below the older one', () => {
  /**
   * Spelled out rather than left to react-toastify, whose default happens to
   * agree today: without the prop the behaviour is correct BY ACCIDENT, and the
   * next reader following the design doc's "newest nearest the corner" would
   * add `newestOnTop` and undo a decision nothing was defending.
   */
  it('pins newestOnTop to false on the container', () => {
    expect(APP).toMatch(/newestOnTop=\{false\}/);
  });

  /**
   * §3.7's 12px gap between stacked toasts. It lives on the card, not the
   * container, because the container is a plain flex column — and it survives
   * react-toastify's own 16px `margin-bottom` only because `toastClassName` is
   * passed as a FUNCTION, which replaces the library's `Toastify__toast` class
   * list outright instead of appending to it. Drop the function form and the
   * vendor's unlayered rule wins over this Tailwind utility.
   */
  it('keeps the 12px stack gap on the toast card', () => {
    expect(TOAST_SURFACE_CLASS_NAME.split(/\s+/)).toContain('mb-3');
    expect(APP).toContain('toastClassName={() => TOAST_SURFACE_CLASS_NAME}');
  });
});

/**
 * Defect D3b (2026-09-13). Radix's modal sets `pointer-events: none` on <body>
 * while a dialog is open, and the toast layer inherited it: a toast drawn above
 * the dialog could not be clicked, and the press fell through to the backdrop
 * and closed the dialog. jsdom applies no stylesheet, so this is asserted at the
 * source, like everything else here; `dialog.test.tsx` covers the half that
 * decides whether a press on a toast dismisses the dialog.
 */
describe('a toast stays clickable over a modal dialog', () => {
  it('gives every toast card pointer events back', () => {
    expect(CSS).toMatch(/\.Toastify__toast-container\s*>\s*\*\s*\{[^}]*pointer-events:\s*auto\s*;/);
  });
});

/**
 * W2-PRV-16. Errors never expire, and nothing dismissed toasts on navigation, so
 * one chat's "Diverge failed" sat for minutes over the Crew details pane's
 * primary buttons on another screen. App dismisses the screen-scoped toasts
 * (`scope: 'screen'` in toasts.tsx) whenever the route or its query changes.
 * Asserted at the source, like the rest of this file: App mounts the whole
 * shell, and the behaviour of `dismissScreenToasts` itself is `toasts.test.tsx`'s.
 */
describe("a screen's toasts go when the person leaves it", () => {
  it('dismisses screen-scoped toasts on every navigation', () => {
    expect(APP).toMatch(/import \{[^}]*dismissScreenToasts[^}]*\} from '\.\/toasts'/);
    expect(APP).toMatch(/const screen = `\$\{pathname\}\$\{search\}`/);
    expect(APP).toMatch(/dismissScreenToasts\(\);\s*\}, \[screen\]\)/);
  });
});

/**
 * Spec 3.16: the toast's motion. react-toastify's default Bounce flew in from
 * 3000px with overshoots over 500ms and then collapsed over 300ms; the app
 * passes its own `cssTransition` instead, with the two keyframes authored beside
 * the Toastify block.
 */
describe('a toast slides 8px with no bounce and collapses over 125ms', () => {
  /** The body of `@keyframes name`. */
  function keyframes(name: string): string {
    const match = CSS.match(new RegExp(`@keyframes ${name}\\s*\\{([\\s\\S]*?)\\n\\}`));
    if (!match) throw new Error(`no @keyframes ${name} in main.css`);
    return match[1];
  }

  it('hands the container the custom transition, never the default Bounce', () => {
    expect(APP).toMatch(/transition=\{TOAST_TRANSITION\}/);
    expect(APP).toMatch(
      /cssTransition\(\{\s*enter: 'br-toast-in',\s*exit: 'br-toast-out',\s*collapseDuration: DUR\.fast,?\s*\}\)/
    );
    expect(APP).not.toMatch(/import \{[^}]*\bBounce\b[^}]*\} from 'react-toastify'/);
    expect(APP).not.toMatch(/transition=\{Bounce\}/);
  });

  it('enters with an 8px drop and leaves with an 8px slide, on the motion tokens', () => {
    expect(CSS).toMatch(
      /\.br-toast-in\s*\{\s*animation:\s*br-toast-in var\(--dur-fast-max\) var\(--ease-out\)/
    );
    expect(CSS).toMatch(
      /\.br-toast-out\s*\{\s*animation:\s*br-toast-out var\(--dur-fast\) var\(--ease-out\)/
    );
    expect(keyframes('br-toast-in')).toMatch(/translateY\(-8px\)/);
    expect(keyframes('br-toast-out')).toMatch(/translateX\(8px\)/);
  });

  /**
   * ⚠ The floor, again, for the frames in between. The entrance starts its
   * travel ABOVE the resting inset, so the inset minus that travel must still
   * clear the drag band, or the toast's × and "View details" are dead while it
   * animates (issue #74).
   */
  it('never enters the titlebar drag band while it animates', () => {
    const travel = Number(keyframes('br-toast-in').match(/translateY\(-(\d+)px\)/)?.[1]);
    expect(travel).toBeGreaterThan(0);
    expect(resolvePx('toast-inset-top') - travel).toBeGreaterThanOrEqual(
      resolvePx('titlebar-drag-height')
    );
  });

  it('keeps the exit an animation under reduced motion, or no toast would ever leave', () => {
    const reduced = CSS.match(
      /@media \(prefers-reduced-motion: reduce\)\s*\{\s*\.br-toast-in\s*\{[^}]*\}\s*\.br-toast-out\s*\{([^}]*)\}/
    );
    expect(reduced, 'no reduced-motion rest for the toast').not.toBeNull();
    expect(reduced![1]).toMatch(/animation-name:\s*br-toast-fade-out/);
    expect(reduced![1]).not.toMatch(/animation:\s*none/);
  });
});

/**
 * Vision V1-R7-1. A sticky toast (the extension-load report) sat over the menus that open into
 * the top-right corner: menus are at `--z-modal-dropdown` and the toast layer at `--z-toast`, so
 * the tab strip's ▾ filter field and a tab's "Open in new window" could not be pointed at. While
 * a menu or popover is open the layer drops one step below the menus. Chromium's hit test is
 * `chatGroups/toastOverMenu.browser.test.tsx`; this holds the two ends at the source: the rule
 * names the primitives' own `data-slot`s, and each primitive still emits its one.
 */
describe('a menu the person opened stands above a toast', () => {
  const SLOTS: Record<string, string[]> = {
    '../components/ui/dropdown-menu.tsx': ['dropdown-menu-content', 'dropdown-menu-sub-content'],
    '../components/ui/context-menu.tsx': ['context-menu-content'],
    '../components/ui/popover.tsx': ['popover-content'],
    // react-select's menu, named by the shared wrapper (WS-PRIMITIVES-2). The name is set in an
    // object (`'data-slot': SELECT_MENU_SLOT`), not as JSX, so its row is matched by the constant.
    '../components/ui/Select.tsx': ['select-content'],
  };

  function yieldRule(): { selector: string; body: string } {
    const match = CSS.match(/\n(body:has\([\s\S]*?\)\s*\.Toastify__toast-container)\s*\{([^}]*)\}/);
    if (!match) throw new Error('no toast yield rule in main.css');
    return { selector: match[1], body: match[2] };
  }

  it('drops the toast layer below the menus while one is open', () => {
    const { selector, body } = yieldRule();
    for (const slot of Object.values(SLOTS).flat()) {
      expect(selector).toContain(`[data-slot='${slot}']`);
    }
    expect(selector).toMatch(/\)\[data-state='open'\]\s*\)/);
    expect(body).toMatch(/z-index:\s*calc\(var\(--z-modal-dropdown\) - 1\)/);
  });

  it('keeps the yielded toast above every dialog, and the resting toast above the menus', () => {
    const z = (name: string) => Number(declaration(name));
    expect(z('z-modal-dropdown') - 1).toBeGreaterThan(z('z-modal'));
    expect(z('z-toast')).toBeGreaterThan(z('z-modal-dropdown'));
  });

  it('names data-slots the menu primitives really emit', () => {
    for (const [file, slots] of Object.entries(SLOTS)) {
      const source = readFileSync(join(__dirname, file), 'utf8');
      for (const slot of slots) {
        if (file.endsWith('Select.tsx')) {
          expect(SELECT_MENU_SLOT).toBe(slot);
          expect(source, file).toMatch(/'data-slot': SELECT_MENU_SLOT, 'data-state': 'open'/);
        } else {
          expect(source, file).toContain(`data-slot="${slot}"`);
        }
      }
    }
  });

  /**
   * WS-PRIMITIVES-2. A Select whose menu stays inside a dialog (no portal) is stacked inside the
   * dialog box's own context, which competes at `--z-modal` in the page, so the yielded toast
   * (one under the menus) still covered it. While one is open the toast steps behind the dialog.
   */
  it('steps behind a dialog while a Select inside it is open', () => {
    const match = CSS.match(
      /\nbody:has\(\.biorouter-modal-surface \[data-slot='select-content'\]\[data-state='open'\]\)\s*\.Toastify__toast-container\s*\{([^}]*)\}/
    );
    expect(match, 'no rule for a Select open inside a dialog').not.toBeNull();
    expect(match![1]).toMatch(/z-index:\s*calc\(var\(--z-modal\) - 1\)/);
    const z = (name: string) => Number(declaration(name));
    expect(z('z-modal') - 1).toBeGreaterThan(0);
    // The dialog box really is the surface the rule names.
    const dialog = readFileSync(join(__dirname, '../components/ui/dialog.tsx'), 'utf8');
    expect(dialog).toMatch(/'biorouter-modal-surface [^']*fixed[^']*z-\[var\(--z-modal\)\]/);
  });
});

/**
 * Vision V2-R7-4, checklist 15. A toast covered find in chat's count and buttons, and the
 * docked Chat summary's first rows, because all three dock in the same corner. The stack now
 * steps below find and into the summary rail's column, under its card
 * (`components/alerts/toastClearance.ts`, whose own test holds the geometry; Chromium's is
 * `alerts/toastClearance.browser.test.tsx`). This holds the ends that live elsewhere: the
 * stylesheet turns the measured move into a `translate` that leaves `top` alone, App's container
 * takes the joined column's width, and App starts the watcher.
 */
describe('a toast steps out of the way of find in chat and the Chat summary', () => {
  function rule(selector: string): string {
    const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const match = CSS.match(new RegExp(`\\n${escaped}\\s*\\{([^}]*)\\}`));
    if (!match) throw new Error(`no rule for ${selector} in main.css`);
    return match[1];
  }

  it('moves the container by a translate, only ever left and down, never by its top', () => {
    expect(rule('.Toastify__toast-container')).toMatch(
      /translate:\s*calc\(-1 \* var\(--toast-clear-x, 0px\)\) var\(--toast-clear-y, 0px\);/
    );
    // The resting corner is still the token, so the drag-band floor above still holds.
    expect(APP).toContain("top: 'var(--toast-inset-top)'");
  });

  it('glides only once placed, snaps during a resize, and not at all under reduced motion', () => {
    expect(rule('.Toastify__toast-container[data-toast-placed]')).toMatch(
      /transition:\s*translate var\(--dur-fast-max\) var\(--ease-out\);/
    );
    expect(CSS).toMatch(
      /body\.biorouter-window-resizing \.Toastify__toast-container,\s*body\.biorouter-sidebar-resizing \.Toastify__toast-container\s*\{\s*transition-duration:\s*0ms;/
    );
    expect(CSS).toMatch(
      /@media \(prefers-reduced-motion: reduce\)\s*\{\s*\.Toastify__toast-container\[data-toast-placed\]\s*\{\s*transition:\s*none;/
    );
  });

  /**
   * Final app check: stepped LEFT of the docked rail, a sticky toast stood over the person's own
   * message. In the rail's column it must also be no wider than the card, or a 420px toast
   * would hang 150px back over the conversation. Both bounds read the one variable the watcher
   * writes, and fall back to the stack's own 280 to 420 when it is unset.
   */
  it('takes the joined column width as both its min and max width', () => {
    expect(APP).toContain("minWidth: 'var(--toast-clear-width, 280px)'");
    expect(APP).toContain("maxWidth: 'var(--toast-clear-width, min(420px, calc(100vw - 32px)))'");
  });

  it('starts the watcher once, beside the container', () => {
    expect(APP).toMatch(
      /import \{ watchToastClearance \} from '\.\/components\/alerts\/toastClearance'/
    );
    expect(APP).toMatch(/useEffect\(\(\) => watchToastClearance\(\), \[\]\);/);
  });
});
