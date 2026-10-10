// @vitest-environment node
/**
 * Menus, popovers, tooltips, dialogs, sheets and the scrim animate transform and opacity only
 * (principle 9; vision round 1, V1-L7).
 *
 * tw-animate's `enter` and `exit` keyframes always write `filter: blur(var(--tw-enter-blur, 0))`,
 * so every surface that used `animate-in`/`animate-out` animated `filter` too, even at a blur of
 * 0, which the animation recorder in the running app reported. The motion classes in `main.css`
 * now name the surfaces' own keyframes (`br-surface-in`, `br-surface-out`). Whether that unlayered
 * `animation-name` really beats the utility's layered `animation` shorthand is a cascade question
 * jsdom cannot answer, so the real tw-animate keyframes and a layered copy of the two utilities
 * are loaded in Chromium beside the authored blocks, and each surface's running animation is read.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
// The package's `exports` do not list the file, so it is read from the install itself.
const TW_ANIMATE = readFileSync(
  resolve(here, '../../../node_modules/tw-animate-css/dist/tw-animate.css'),
  'utf8'
);

/** tw-animate's own `enter` and `exit` keyframes, verbatim. */
function twKeyframes(): string {
  return ['enter', 'exit']
    .map((name) => {
      const match = TW_ANIMATE.match(new RegExp(`@keyframes ${name} \\{.*?\\}\\}`));
      if (!match) throw new Error(`tw-animate-css has no @keyframes ${name}`);
      return match[0];
    })
    .join('\n');
}

/** Every `@ws WS-PRIMITIVES` block of main.css, in file order. */
function primitiveBlocks(): string {
  const re =
    /\/\* @ws WS-PRIMITIVES begin: ([^*]+?) \*\/([\s\S]*?)\/\* @ws WS-PRIMITIVES end: \1 \*\//g;
  const out = Array.from(MAIN_CSS.matchAll(re), (match) => match[2]);
  if (out.length === 0) throw new Error('main.css has no @ws WS-PRIMITIVES block');
  return out.join('\n');
}

/** The tw-animate utilities the components write, in the layer Tailwind puts them in. */
const UTILITIES = `
@layer utilities {
  .animate-in { animation: enter var(--tw-duration, .15s) var(--tw-ease, ease); }
  .animate-out { animation: exit var(--tw-duration, .15s) var(--tw-ease, ease); }
  .fade-in-0 { --tw-enter-opacity: 0; }
  .fade-out-0 { --tw-exit-opacity: 0; }
}
:root { --dur-fast-min: 95ms; --dur-fast: 125ms; --dur-fast-max: 175ms; --dur-med-min: 250ms;
  --dur-med: 300ms; --ease-out: cubic-bezier(0.24, 1, 0.4, 1); }
`;

/** Each surface as the components render it: its motion class, its state and its utilities. */
const SURFACES: Array<{ name: string; className: string; state?: string }> = [
  { name: 'menu opening', className: 'br-menu-motion animate-in fade-in-0', state: 'open' },
  { name: 'menu closing', className: 'br-menu-motion animate-out fade-out-0', state: 'closed' },
  {
    name: 'tooltip opening',
    className: 'br-tooltip-motion animate-in fade-in-0',
    state: 'delayed-open',
  },
  { name: 'app tooltip', className: 'br-tooltip-motion animate-in fade-in-0' },
  {
    name: 'tooltip closing',
    className: 'br-tooltip-motion animate-out fade-out-0',
    state: 'closed',
  },
  { name: 'dialog opening', className: 'br-dialog-motion animate-in fade-in-0', state: 'open' },
  { name: 'dialog closing', className: 'br-dialog-motion animate-out fade-out-0', state: 'closed' },
  {
    name: 'scrim opening',
    className: 'biorouter-modal-overlay animate-in fade-in-0',
    state: 'open',
  },
  { name: 'sheet closing', className: 'br-sheet-motion animate-out', state: 'closed' },
];

const launchChromium = async (): Promise<Browser | null> => {
  for (const options of [{}, { channel: 'chromium' }, { channel: 'chrome' }] as const) {
    try {
      return await chromium.launch(options);
    } catch {
      // try the next build
    }
  }
  return null;
};

describe('floating surfaces animate transform and opacity only (V1-L7)', () => {
  let browser: Browser | null = null;

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) console.warn('No Chromium found: skipping the surface motion test.');
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  it('runs br-surface-in/out on every surface, with no filter in their frames', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await browser.newPage();
    try {
      await tab.setContent(
        `<!doctype html><html><head><style>${twKeyframes()}\n${UTILITIES}\n${primitiveBlocks()}</style></head><body></body></html>`
      );
      const read = (surfaces: typeof SURFACES) =>
        tab.evaluate((list) => {
          return list.map(({ name, className, state }) => {
            const el = document.createElement('div');
            el.className = className;
            if (state) el.setAttribute('data-state', state);
            document.body.appendChild(el);
            const animations = el.getAnimations() as Array<Animation & { animationName: string }>;
            const properties = new Set<string>();
            for (const animation of animations) {
              for (const frame of (animation.effect as KeyframeEffect).getKeyframes()) {
                for (const key of Object.keys(frame)) properties.add(key);
              }
            }
            return {
              name,
              animationNames: animations.map((a) => a.animationName),
              properties: [...properties],
            };
          });
        }, surfaces);

      for (const surface of await read(SURFACES)) {
        expect(surface.animationNames, `${surface.name} runs one animation`).toHaveLength(1);
        expect(surface.animationNames[0], surface.name).toMatch(/^br-surface-(in|out)$/);
        expect(surface.properties, `${surface.name} animates opacity`).toContain('opacity');
        expect(surface.properties, `${surface.name} never animates filter`).not.toContain('filter');
      }

      // The instrument: the bare utility, with no motion class, still animates `filter`.
      const [bare] = await read([{ name: 'bare utility', className: 'animate-in fade-in-0' }]);
      expect(bare.animationNames).toEqual(['enter']);
      expect(bare.properties, 'the bug this guards').toContain('filter');
    } finally {
      await tab.close();
    }
  });
});
