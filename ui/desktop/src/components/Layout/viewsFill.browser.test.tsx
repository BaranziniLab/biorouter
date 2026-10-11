// @vitest-environment node
/**
 * The Workflows, Scheduler and Built apps surfaces under the fill rule, in a real layout engine
 * (owner message 13, R-27, design.md 9.11; spec 3.10).
 *
 * jsdom runs no CSS, and every claim here is about the cascade: a custom property a band sets
 * for the controls inside it, a `::after` rule a list switches off by scope, a card whose edge is
 * transparent and whose children step back one ground, a left edge shared by two different
 * containers. So the real components (`PageHeader`, `FilterInput`, `ParameterInput`,
 * `ScheduleFacts`) are bundled with esbuild, the app's own `styles/main.css` is compiled by
 * Tailwind with the classes the bundle uses (plus `schedule.css`, which a component import would
 * have carried), and Chromium paints them. Colours are compared with probes that paint the token
 * itself, so a token value can change without this file changing.
 *
 * Every rule is proved the same way: the case strips the one declaration it guards from the
 * compiled sheet and expects the defect, so the instrument is shown to see what it claims to.
 *
 * Set `VIEWS_A_SHOTS` to a directory to also write a screenshot of each surface, light and dark.
 */
import { mkdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const SCHEDULE_CSS = readFileSync(resolve(here, '../schedule/schedule.css'), 'utf8');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));
const SHOTS = process.env.VIEWS_A_SHOTS;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { PageHeader } from './PageHeader';
import { FilterInput } from '../ui/filter-input';
import { Button } from '../ui/button';
import ParameterInput from '../parameter/ParameterInput';
import {
  DefinitionRow,
  FactRow,
  ScheduleFacts,
  ScheduleFactsSkeleton,
} from '../schedule/ScheduleFacts';

function Rows({ id, flat }: { id: string; flat: boolean }) {
  return (
    <div id={id} className={'biorouter-list-shell' + (flat ? ' br-rows-flat' : '')}>
      {[1, 2, 3].map((n) => (
        <div key={n} id={id + '-' + n} className="biorouter-list-row">
          <span>Row {n}</span>
        </div>
      ))}
    </div>
  );
}

function App() {
  const [query, setQuery] = React.useState('');
  return (
    <div style={{ width: 760, background: 'var(--background-canvas)' }}>
      <div id="band">
        <PageHeader
          title="Workflows"
          actions={<FilterInput value={query} onValueChange={setQuery} label="Filter workflows" />}
        />
      </div>
      <div style={{ padding: '8px 24px 0' }}>
        <Rows id="flat" flat />
        <Rows id="ruled" flat={false} />
        <div id="detail" style={{ marginTop: 16 }}>
          <ScheduleFacts>
            <DefinitionRow label="Runs">Every day at 2:00 AM</DefinitionRow>
            <FactRow label="Running">
              <Button id="open-chat" variant="secondary" size="sm">Open chat</Button>
            </FactRow>
          </ScheduleFacts>
          <div className="biorouter-list-row" id="run-row"><span id="run-text">Oct 10, 3:00 AM</span></div>
          <div id="skeleton" style={{ marginTop: 16 }}><ScheduleFactsSkeleton /></div>
        </div>
        <div id="param-host" style={{ marginTop: 16, paddingBottom: 24 }}>
          <ParameterInput
            parameter={{ key: 'cohort', description: 'Which cohort', input_type: 'string', requirement: 'required' }}
            onChange={() => {}}
            onDelete={() => {}}
            isExpanded
            onToggleExpanded={() => {}}
          />
        </div>
      </div>
    </div>
  );
}

createRoot(document.getElementById('root')!).render(<App />);
`;

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

describe('Workflows, Scheduler and Built apps under the fill rule (R-27)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the views fill tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      define: { 'process.env.NODE_ENV': '"production"' },
      loader: { '.css': 'empty', '.svg': 'dataurl', '.png': 'dataurl' },
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
    const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
    const compiler = await compile(
      `${readFileSync(resolve(STYLES, 'main.css'), 'utf8')}\n${SCHEDULE_CSS}`,
      { base: STYLES, onDependency() {} }
    );
    css = compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }]));
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(mode: 'light' | 'dark', sheet = css, forcedColors = false): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 760, height: 900 } });
    if (forcedColors) await tab.emulateMedia({ forcedColors: 'active' });
    await tab.setContent(
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${sheet}</style></head>` +
        `<body style="margin:0;background:var(--background-canvas)"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#param-host input');
    await tab.waitForTimeout(250);
    return tab;
  }

  /** The computed colour a probe inside `within` paints for `value` (a token, in that place). */
  const paint = (tab: Page, value: string, within = 'body') =>
    tab.evaluate(
      ([v, w]) => {
        const probe = document.createElement('div');
        probe.style.backgroundColor = v;
        document.querySelector(w)!.append(probe);
        const out = getComputedStyle(probe).backgroundColor;
        probe.remove();
        return out;
      },
      [value, within] as const
    );
  const ground = (tab: Page, selector: string) =>
    tab.evaluate((s) => getComputedStyle(document.querySelector(s)!).backgroundColor, selector);
  const afterContent = (tab: Page, selector: string) =>
    tab.evaluate((s) => getComputedStyle(document.querySelector(s)!, '::after').content, selector);
  const left = (tab: Page, selector: string) =>
    tab.evaluate((s) => document.querySelector(s)!.getBoundingClientRect().left, selector);

  /** The sheet with ONE declaration removed from the rule that holds it, to prove an instrument. */
  const without = (rule: RegExp, declaration: RegExp) => {
    const stripped = css.replace(rule, (block) => block.replace(declaration, ''));
    expect(stripped, 'the rule to strip was found').not.toBe(css);
    return stripped;
  };

  async function shot(tab: Page, mode: string) {
    if (!SHOTS) return;
    mkdirSync(SHOTS, { recursive: true });
    await tab.screenshot({ path: resolve(SHOTS, `views-fill-${mode}.png`), fullPage: true });
  }

  it('rests the band’s filter one step deeper than the fill, because the band is a recessed ground', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const strong = await paint(tab, 'var(--background-fill-strong)');
        const fill = await paint(tab, 'var(--background-fill)');
        expect(await ground(tab, '#band header'), `${mode} band ground`).toBe(
          await paint(tab, 'var(--sidebar)')
        );
        expect(
          await ground(tab, '#band .br-filter-input-field'),
          `${mode} the filter rests on the strong fill`
        ).toBe(strong);
        // There is no deeper neutral that clears 4.5:1 for the placeholder, so the field does
        // not step further under the pointer: hover equals rest.
        await tab.hover('#band .br-filter-input-field');
        await tab.waitForTimeout(250);
        expect(await ground(tab, '#band .br-filter-input-field'), `${mode} hover`).toBe(strong);
        // A band that did not set the ground would rest on the fill, 1.03:1 off its own paint.
        expect(strong).not.toBe(fill);
        await shot(tab, mode);
      } finally {
        await tab.close();
      }
    }

    const bare = await open(
      'light',
      without(/\.biorouter-page-header\[data-band\] \{[^}]*\}/, /\n\s*--br-control-ground:[^;]*;/)
    );
    try {
      expect(
        await ground(bare, '#band .br-filter-input-field'),
        'without the rule the filter falls back to the rest fill'
      ).toBe(await paint(bare, 'var(--background-fill)'));
    } finally {
      await bare.close();
    }
  }, 120_000);

  it('draws no hairline under the rows of a flat list, and keeps it under every other list, at the same height', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('light');
    try {
      expect(await afterContent(tab, '#flat-1')).toBe('none');
      expect(await afterContent(tab, '#flat-2')).toBe('none');
      expect(await afterContent(tab, '#ruled-1')).toBe('""');
      expect(await afterContent(tab, '#ruled-2')).toBe('""');
      // Nothing moves: the row keeps its transparent bottom edge, so a flat row is as tall as a ruled one.
      const heights = await tab.evaluate(() => [
        document.querySelector('#flat-1')!.getBoundingClientRect().height,
        document.querySelector('#ruled-1')!.getBoundingClientRect().height,
      ]);
      expect(heights[0]).toBe(heights[1]);
    } finally {
      await tab.close();
    }

    const bare = await open(
      'light',
      without(/\.br-rows-flat :is\([^)]*\)::after \{[^}]*\}/, /content:\s*none;/)
    );
    try {
      expect(
        await afterContent(bare, '#flat-1'),
        'without the rule the flat list draws its hairline again'
      ).toBe('""');
    } finally {
      await bare.close();
    }
  }, 120_000);

  it('gives a flat list its hairline back under forced colours, where the hover wash and every fill are gone', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('light', css, true);
    try {
      expect(await afterContent(tab, '#flat-1')).toBe('""');
      expect(await afterContent(tab, '#flat-2')).toBe('""');
      // The shared last-child rule still leaves the last row without one.
      expect(await afterContent(tab, '#flat-3')).toBe('none');
    } finally {
      await tab.close();
    }

    const bare = await open(
      'light',
      without(
        /@media \(forced-colors: active\) \{\s*\.br-rows-flat[^}]*\}\s*\}/,
        /content:\s*(?:""|'');/
      ),
      true
    );
    try {
      expect(
        await afterContent(bare, '#flat-1'),
        'without the rule a forced-colours list has no separator at all'
      ).toBe('none');
    } finally {
      await bare.close();
    }
  }, 120_000);

  it('draws the schedule facts as one filled card whose button rests on the page ground and whose text shares the run rows’ left edge', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const card = await tab.evaluate(() => {
          const el = document.querySelector('#detail .br-schedule-facts')!;
          const cs = getComputedStyle(el);
          return {
            ground: cs.backgroundColor,
            edge: cs.borderTopColor,
            edgeWidth: cs.borderTopWidth,
            radius: cs.borderTopLeftRadius,
          };
        });
        expect(card.ground, `${mode} card fill`).toBe(await paint(tab, 'var(--background-fill)'));
        expect(card.edgeWidth).toBe('1px');
        expect(card.edge, 'the edge is kept, transparent').toBe(await paint(tab, 'transparent'));
        expect(card.radius).toBe('12px');

        // A fill never sits on a fill: the secondary button steps back to the page ground and
        // still hovers on the strong fill.
        expect(await ground(tab, '#open-chat'), `${mode} button rests on the page`).toBe(
          await paint(tab, 'var(--background-default)')
        );
        await tab.hover('#open-chat');
        await tab.waitForTimeout(250);
        expect(await ground(tab, '#open-chat'), `${mode} button hover`).toBe(
          await paint(tab, 'var(--background-fill-strong)')
        );

        // One left edge for the facts and the rows (12px inset inside each).
        const label = await left(tab, '#detail .br-schedule-fact span');
        expect(label).toBeCloseTo(await left(tab, '#run-text'), 1);

        // The skeleton's bars rest on the strong fill, or they would vanish on the card.
        expect(await ground(tab, '#skeleton [data-slot="skeleton"]')).toBe(
          await paint(tab, 'var(--background-fill-strong)')
        );
      } finally {
        await tab.close();
      }
    }
  }, 120_000);

  it('draws a workflow parameter as a filled card with its fields and selects one ground back, and no rule inside', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const page = await paint(tab, 'var(--background-default)');
        const card = await tab.evaluate(() => {
          const el = document.querySelector('#param-host .parameter-input')!;
          const cs = getComputedStyle(el);
          const body = el.lastElementChild as HTMLElement;
          return {
            ground: cs.backgroundColor,
            edge: cs.borderTopColor,
            edgeWidth: cs.borderTopWidth,
            bodyRule: getComputedStyle(body).borderTopWidth,
          };
        });
        expect(card.ground, `${mode} card`).toBe(await paint(tab, 'var(--background-fill)'));
        expect(card.edgeWidth).toBe('1px');
        expect(card.edge).toBe(await paint(tab, 'transparent'));
        expect(card.bodyRule, 'no rule between the header and the details').toBe('0px');

        for (const selector of [
          '#param-host input[type="text"]',
          '#param-host select',
          '#param-host code',
        ]) {
          expect(await ground(tab, selector), `${mode} ${selector} rests on the page ground`).toBe(
            page
          );
        }
        // The select is the app's own field with the app's own chevron, not the system one.
        const select = await tab.evaluate(() => {
          const el = document.querySelector('#param-host select') as HTMLSelectElement;
          const cs = getComputedStyle(el);
          const chevron = el.parentElement!.querySelector('svg')!.getBoundingClientRect();
          const box = el.getBoundingClientRect();
          return {
            appearance: cs.appearance,
            edge: cs.borderTopColor,
            height: box.height,
            chevronSize: [chevron.width, chevron.height],
            chevronInset: box.right - chevron.right,
          };
        });
        expect(select.appearance).toBe('none');
        expect(select.edge).toBe(await paint(tab, 'transparent'));
        expect(select.height).toBe(32);
        expect(select.chevronSize).toEqual([16, 16]);
        expect(select.chevronInset).toBe(8);
      } finally {
        await tab.close();
      }
    }

    // Proof: a `bg-background-default` utility on the select (what it wore) beats the layered
    // field rule, so the fill would not be what rests there.
    const tab = await open('light');
    try {
      await tab.evaluate(() => {
        document.querySelector('#param-host select')!.classList.add('bg-background-default');
      });
      expect(await ground(tab, '#param-host select')).toBe(
        await paint(tab, 'var(--background-default)')
      );
    } finally {
      await tab.close();
    }
  }, 120_000);
});
