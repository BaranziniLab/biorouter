import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  DUR,
  EASE_OUT,
  EASE_SPRING,
  MOTION,
  RESIZING_CLASSES,
  TRAVEL,
  isWindowResizing,
  motionDuration,
  prefersReducedMotion,
} from './motion';

/**
 * `styles/motion.ts` is the TypeScript mirror of the motion tokens in
 * `main.css`. WAAPI and FLIP code read the numbers from it because a `var()`
 * cannot reach an `element.animate()` timing object, so the mirror is only
 * useful while it agrees with the stylesheet. Every assertion below reads the
 * declaration out of `main.css` and compares it with the exported value, the
 * same source-level technique as `measures.test.ts` (jsdom applies no
 * stylesheet, so there is nothing else to measure).
 */
const CSS = readFileSync(join(__dirname, 'main.css'), 'utf8');

/** The CSS with comments blanked, so a token named in prose never matches. */
const CODE = CSS.replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\n]/g, ' '));

function declaration(name: string): string {
  const match = CODE.match(new RegExp(`(?:^|[\\s;{])${name}:\\s*([^;]+);`));
  expect(match, `expected a declaration of ${name} in main.css`).toBeTruthy();
  return match![1].trim();
}

function rule(selector: string, from = CODE): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = from.match(new RegExp(`(?:^|\\n)\\s*${escaped}\\s*\\{([^}]*)\\}`));
  expect(match, `expected an authored \`${selector}\` rule in main.css`).toBeTruthy();
  return match![1];
}

/** Every `@media (prefers-reduced-motion: reduce) { … }` body in the file. */
function reducedMotionBlocks(): string {
  const out: string[] = [];
  const re = /@media \(prefers-reduced-motion: reduce\)\s*\{/g;
  while (re.exec(CODE)) {
    let depth = 1;
    let i = re.lastIndex;
    for (; i < CODE.length && depth > 0; i++) {
      if (CODE[i] === '{') depth++;
      else if (CODE[i] === '}') depth--;
    }
    out.push(CODE.slice(re.lastIndex, i - 1));
  }
  return out.join('\n');
}

describe('the TS mirror equals the CSS tokens', () => {
  it.each([
    ['--dur-fast-min', DUR.fastMin],
    ['--dur-fast', DUR.fast],
    ['--dur-fast-max', DUR.fastMax],
    ['--dur-med-min', DUR.medMin],
    ['--dur-med', DUR.med],
    ['--dur-slow', DUR.slow],
  ])('%s is %ims', (token, ms) => {
    expect(declaration(token)).toBe(`${ms}ms`);
  });

  it('mirrors the one curve and the one spring', () => {
    expect(declaration('--ease-out')).toBe(EASE_OUT);
    expect(declaration('--ease-spring')).toBe(EASE_SPRING);
  });

  /** Exits are a tier faster than entrances; the ladder must stay ordered. */
  it('keeps the ladder strictly increasing', () => {
    const ladder = [DUR.fastMin, DUR.fast, DUR.fastMax, DUR.medMin, DUR.med, DUR.slow];
    expect([...ladder].sort((a, b) => a - b)).toEqual(ladder);
    expect(new Set(ladder).size).toBe(ladder.length);
  });

  it('halves the travel on the way out', () => {
    expect(TRAVEL.previewOut * 2).toBe(TRAVEL.previewIn);
    expect(MOTION.popScale).toBeLessThan(1);
    expect(MOTION.dialogScale).toBeGreaterThan(MOTION.popScale);
  });
});

describe('prefersReducedMotion()', () => {
  const original = window.matchMedia;
  afterEach(() => {
    window.matchMedia = original;
  });

  it('reads the OS setting', () => {
    window.matchMedia = vi.fn().mockReturnValue({ matches: true }) as never;
    expect(prefersReducedMotion()).toBe(true);
    expect(window.matchMedia).toHaveBeenCalledWith('(prefers-reduced-motion: reduce)');
    window.matchMedia = vi.fn().mockReturnValue({ matches: false }) as never;
    expect(prefersReducedMotion()).toBe(false);
  });

  it('answers false where matchMedia does not exist', () => {
    // @ts-expect-error simulating an environment without matchMedia
    window.matchMedia = undefined;
    expect(prefersReducedMotion()).toBe(false);
  });

  it('turns a JS duration into 0 under reduced motion unless asked to keep it', () => {
    window.matchMedia = vi.fn().mockReturnValue({ matches: true }) as never;
    expect(motionDuration('med')).toBe(0);
    expect(motionDuration('fast', { keepUnderReducedMotion: true })).toBe(DUR.fast);
    window.matchMedia = vi.fn().mockReturnValue({ matches: false }) as never;
    expect(motionDuration('med')).toBe(DUR.med);
  });
});

describe('isWindowResizing()', () => {
  afterEach(() => {
    document.body.classList.remove(...RESIZING_CLASSES);
  });

  it('is false at rest', () => {
    expect(isWindowResizing()).toBe(false);
  });

  it.each(RESIZING_CLASSES)('is true while <body> carries %s', (name) => {
    document.body.classList.add(name);
    expect(isWindowResizing()).toBe(true);
  });

  /**
   * The class names are the contract with `AppLayout` (window) and
   * `ui/sidebar.tsx` (sidebar drag). If either renames its class, this mirror
   * would silently answer false forever.
   */
  it('names the classes the layout code actually sets', () => {
    const appLayout = readFileSync(join(__dirname, '../components/Layout/AppLayout.tsx'), 'utf8');
    const sidebar = readFileSync(join(__dirname, '../components/ui/sidebar.tsx'), 'utf8');
    expect(appLayout).toContain("'biorouter-window-resizing'");
    expect(sidebar).toContain("'biorouter-sidebar-resizing'");
  });
});

describe('the resize rule', () => {
  /**
   * Layout snaps while geometry is being dragged: transitions AND animations
   * stop on every element that opts in with `data-motion-layout`. The older
   * rule beside the preview's resizing block only zeroed transitions, so a CSS
   * keyframe kept playing against the drag.
   */
  it('stops transitions and animations on [data-motion-layout] during either resize', () => {
    const match = CODE.match(
      /body\.biorouter-window-resizing \[data-motion-layout\],\s*body\.biorouter-sidebar-resizing \[data-motion-layout\]\s*\{([^}]*)\}/
    );
    expect(match, 'expected the global [data-motion-layout] resize rule').toBeTruthy();
    expect(match![1]).toMatch(/transition-duration:\s*0ms\s*!important/);
    expect(match![1]).toMatch(/animation:\s*none\s*!important/);
  });
});

describe('the shared motion classes', () => {
  it('.br-enter rises 4px and fades in over --dur-fast-max on the one curve', () => {
    expect(rule('.br-enter')).toMatch(
      /animation:\s*br-enter\s+var\(--dur-fast-max\)\s+var\(--ease-out\)/
    );
    expect(CODE).toMatch(/@keyframes br-enter\s*\{[^}]*translateY\(4px\)/);
  });

  it('.br-crossfade stacks its items in one cell, in over --dur-fast-max, out over --dur-fast', () => {
    expect(rule('.br-crossfade > .br-crossfade-item')).toMatch(/grid-area:\s*1\s*\/\s*1/);
    expect(rule(".br-crossfade-item[data-state='open']")).toMatch(
      /animation:\s*br-crossfade-in\s+var\(--dur-fast-max\)\s+var\(--ease-out\)/
    );
    expect(rule(".br-crossfade-item[data-state='closed']")).toMatch(
      /animation:\s*br-fade-out\s+var\(--dur-fast\)\s+var\(--ease-out\)/
    );
    expect(CODE).toMatch(/@keyframes br-crossfade-in\s*\{[^}]*translateY\(8px\)/);
  });

  it('.br-highlight washes --overlay-selected away over three --dur-slow periods', () => {
    expect(rule('.br-highlight')).toMatch(
      /animation:\s*br-highlight-wash\s+calc\(var\(--dur-slow\)\s*\*\s*3\)\s+var\(--ease-out\)/
    );
    expect(CODE).toMatch(/@keyframes br-highlight-wash\s*\{[^}]*var\(--overlay-selected\)/);
  });

  /**
   * Crew's contract, promoted with the classes: the global reset shortens
   * durations, but it cannot say what a moving element rests as, so every
   * animated rule restates its rest under reduced motion.
   */
  it('gives every shared class a reduced-motion rest', () => {
    const rests = reducedMotionBlocks();
    expect(rests).toMatch(/\.br-enter\s*\{[^}]*animation-name:\s*br-fade-in/);
    expect(rests).toMatch(/\.br-crossfade-item\[data-state='open'\]\s*\{[^}]*br-fade-in/);
    expect(rests).toMatch(/\.br-crossfade-item\[data-state='closed'\]\s*\{[^}]*br-fade-out/);
    expect(rests).toMatch(/\.br-highlight\s*\{[^}]*br-highlight-hold/);
  });

  /** Principle 11: never borrow a Crew class outside `crew/`. */
  it('does not reuse Crew keyframes', () => {
    for (const name of ['br-enter', 'br-crossfade-in', 'br-fade-in', 'br-fade-out']) {
      expect(CODE).toMatch(new RegExp(`@keyframes ${name}\\b`));
    }
    expect(CODE).not.toMatch(/\.br-[\w-]+\s*\{[^}]*crew-[\w-]+/);
  });
});
