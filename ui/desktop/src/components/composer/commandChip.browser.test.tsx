// @vitest-environment node
/**
 * The `/bug` command chip (#391) under the whole stylesheet, in a real layout engine, in the four
 * places it is drawn: the composer's chips row, a sent message, a queued message and the edit box;
 * and the `/bug` row of the slash menu.
 *
 * #391 wrote the chip before #394's restyle and its merge took #394's chip recipe (a 14px glyph,
 * `aria-hidden`, the 16px `.br-composer-chip-action`, the motion tokens). The claim here is the one
 * jsdom cannot check: a command chip and a reference chip read as one kind of attached object,
 * because every computed property that draws them is the same, light and dark, on the page ground
 * and inside the message bubble's tint. So each case renders a command chip beside a reference
 * chip in the same container and compares what Chromium computed for the two, rather than pinning
 * numbers that would drift with a token.
 *
 * The real pieces are bundled with esbuild: `ComposerChips` in the composer card, `MessageQueue`,
 * `ResourceRefText` (a sent message's body), `CommandChip` and `ResourceRefChip` (the edit box's
 * rail), and `MentionPopover` with its daemon reads stubbed to the daemon's command list. The
 * message bubble and the edit card are the class strings `UserMessage` writes, since mounting
 * `UserMessage` needs the Electron bridge. `styles/main.css` is compiled by Tailwind with the
 * classes the bundle uses, the composer's component CSS is appended as written.
 *
 * Set `COMMAND_CHIP_SHOTS_DIR` to also save light and dark screenshots there.
 */
import { mkdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import postcss from 'postcss';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { fontScale } from '../../../vite-plugins/fontScale.mjs';
import { COMPOSER_COPY } from './copy';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));
const SHOTS = process.env.COMMAND_CHIP_SHOTS_DIR;

/** The daemon's built-in commands as `GET /config/slash_commands` lists them. */
const DAEMON_COMMANDS = [
  { command: 'compact', help: 'Compact the chat history', command_type: 'Builtin' },
  { command: 'clear', help: 'Clear the chat history', command_type: 'Builtin' },
  {
    command: 'bug',
    help: 'Report a Biorouter bug: investigate this chat, draft an issue you approve',
    command_type: 'Builtin',
  },
];

const STUBS: { filter: RegExp; contents: string }[] = [
  {
    filter: /^\.\.\/api$/,
    contents: `
      export const getSlashCommands = async () => ({ data: { commands: ${JSON.stringify(DAEMON_COMMANDS)} } });
      export const listBases = async () => ({ data: [] });
      export const getSessionExtensions = async () => null;`,
  },
  {
    filter: /^\.\.\/utils\/userAction$/,
    contents: `export const userActionHeaders = async () => ({});`,
  },
  {
    filter: /^\.\/knowledge\/knowledgeSelection$/,
    contents: `export const readKnowledgeSelection = async () => null;`,
  },
  {
    filter: /^\.\/ConfigContext$/,
    contents: `export const useConfig = () => ({ extensionsList: [] });`,
  },
  {
    filter: /^\.\/skills\/useSkillCatalog$/,
    contents: `
      export const fetchSkillCatalog = async () => ({ generation: 0, roots: [], skills: [], bundles: [] });
      export const pickerBundles = () => [];
      export const standaloneSkills = () => [];`,
  },
];

const stubPlugin: Plugin = {
  name: 'stubs',
  setup(b) {
    for (const { filter, contents } of STUBS) {
      b.onResolve({ filter }, (args) => ({
        path: args.path,
        namespace: 'stub',
        pluginData: contents,
      }));
    }
    b.onLoad({ filter: /.*/, namespace: 'stub' }, (args) => ({
      contents: args.pluginData as string,
      loader: 'js',
    }));
    b.onLoad({ filter: /\.css$/ }, () => ({ contents: '', loader: 'js' }));
  },
};

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { composerCardClassName, composerCardSurfaceClassName } from './composerCard';
import { ComposerChips } from './ComposerChips';
import { MessageQueue } from '../MessageQueue';
import MentionPopover from '../MentionPopover';
import { CommandChip, ResourceRefChip, ResourceRefText } from '../ResourceRefChip';
import { refTag } from '../../utils/resourceRefs';
import { cn } from '../../utils';

const SKILL = { kind: 'skill', value: 'ggplot', start: 0, end: 0, raw: '' };
const SENT = '/bug the export dialog closes before the lint step ends ' + refTag('skill', 'ggplot');

function App() {
  return (
    <div id="ground" className="bg-background-default" style={{ padding: 24, width: 760 }}>
      <div style={{ display: 'grid', gap: 24 }}>
        <div id="composer" className={composerCardClassName({ isDraggingOver: false })}>
          <MessageQueue
            queuedMessages={[{ id: 'q1', content: '/bug and the retry does nothing', timestamp: 1 }]}
            onRemoveMessage={() => {}}
            onClearQueue={() => {}}
            onStopAndSend={() => {}}
            onReorderMessages={() => {}}
            onEditMessage={() => {}}
          />
          <ComposerChips
            command="bug"
            onRemoveCommand={() => {}}
            refs={[SKILL]}
            images={[]}
            files={[]}
            onRemoveRef={() => {}}
            onRemoveImage={() => {}}
            onRetryImage={() => {}}
            onRemoveFile={() => {}}
          />
          <textarea
            rows={1}
            aria-label="Message"
            placeholder="Describe what went wrong (optional)"
            className="br-composer-input block w-full resize-none border-none bg-transparent px-0 py-1.5 text-body text-text-default placeholder:text-text-muted"
          />
        </div>

        {/* UserMessage's bubble. */}
        <div className="message flex justify-end w-full">
          <div className="flex flex-col max-w-[85%] w-fit min-w-0">
            <div
              id="bubble"
              className="flex w-fit max-w-full min-w-0 self-end rounded-container bg-background-medium px-3.5 py-2.5"
            >
              <div className="min-w-0 text-body text-text-default whitespace-pre-wrap break-words">
                <ResourceRefText text={SENT} />
              </div>
            </div>
          </div>
        </div>

        {/* UserMessage's edit card. */}
        <div
          id="edit"
          className={cn('w-full rounded-container px-4 pt-3 pb-2 text-text-default', composerCardSurfaceClassName)}
        >
          <div className="mb-2 flex flex-wrap items-center gap-1.5">
            <CommandChip command="bug" onRemove={() => {}} />
            <ResourceRefChip refSpan={SKILL} onRemove={() => {}} />
          </div>
          <textarea
            rows={3}
            defaultValue="the export dialog closes before the lint step ends"
            className="block w-full resize-none border-0 bg-transparent p-0 text-body text-text-default outline-none placeholder:text-text-muted"
          />
        </div>
      </div>
      <MentionPopover
        isOpen
        onClose={() => {}}
        onSelect={() => {}}
        position={{ x: 48, y: 1060 }}
        query=""
        isSlashCommand
        selectedIndex={1}
        onSelectedIndexChange={() => {}}
      />
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

/** What Chromium computed for one chip: its box and every property that draws it. */
type ChipLook = {
  name: string;
  height: number;
  background: string;
  radius: string;
  fontSize: string;
  fontWeight: string;
  nameColor: string;
  glyph: { width: number; height: number; color: string; ariaHidden: string | null };
  action: {
    width: number;
    height: number;
    color: string;
    radius: string;
    transitionDuration: string;
  } | null;
  /** The chip's right edge stays inside its container. */
  inside: boolean;
};

const looks = (tab: Page, container: string): Promise<ChipLook[]> =>
  tab.evaluate((sel) => {
    const root = document.querySelector(sel)!;
    const box = root.getBoundingClientRect();
    return [
      ...root.querySelectorAll<HTMLElement>(
        '[data-testid="command-chip"], [data-testid="resource-ref-chip"]'
      ),
    ].map((chip) => {
      const cs = getComputedStyle(chip);
      const name = chip.querySelector<HTMLElement>(
        '[data-testid="command-chip-name"], [data-testid="resource-ref-chip-name"]'
      )!;
      const glyph = chip.querySelector('svg')!;
      const glyphBox = glyph.getBoundingClientRect();
      const action = chip.querySelector<HTMLElement>('button');
      const rect = chip.getBoundingClientRect();
      return {
        name: name.textContent ?? '',
        height: rect.height,
        background: cs.backgroundColor,
        radius: cs.borderRadius,
        fontSize: cs.fontSize,
        fontWeight: cs.fontWeight,
        nameColor: getComputedStyle(name).color,
        glyph: {
          width: glyphBox.width,
          height: glyphBox.height,
          color: getComputedStyle(glyph).color,
          ariaHidden: glyph.getAttribute('aria-hidden'),
        },
        action: action
          ? {
              width: action.getBoundingClientRect().width,
              height: action.getBoundingClientRect().height,
              color: getComputedStyle(action).color,
              radius: getComputedStyle(action).borderRadius,
              transitionDuration: getComputedStyle(action).transitionDuration,
            }
          : null,
        inside: rect.left >= box.left - 0.5 && rect.right <= box.right + 0.5,
      };
    });
  }, container);

/** The duration a probe resolves `--dur-fast-min` to, the chip action's token. */
const fastMin = (tab: Page) =>
  tab.evaluate(() => {
    const probe = document.createElement('div');
    probe.style.transition = 'color var(--dur-fast-min) ease';
    document.body.append(probe);
    const out = getComputedStyle(probe).transitionDuration;
    probe.remove();
    return out;
  });

describe('the /bug command chip under the whole stylesheet (#391 in #394)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the command chip tests.');
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
      loader: { '.svg': 'dataurl', '.png': 'dataurl' },
      plugins: [stubPlugin],
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
    const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
    const compiler = await compile(readFileSync(resolve(STYLES, 'main.css'), 'utf8'), {
      base: STYLES,
      onDependency() {},
    });
    const tailwind = compiler.build(
      new Scanner({}).scanFiles([{ content: script, extension: 'js' }])
    );
    css =
      (await postcss([fontScale()]).process(tailwind, { from: undefined })).css +
      // The component stylesheet the bundle imports (esbuild drops it), as written.
      readFileSync(resolve(here, 'composer.css'), 'utf8');
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(mode: 'light' | 'dark', deviceScaleFactor = 1): Promise<Page> {
    const tab = await browser!.newPage({
      viewport: { width: 808, height: 1100 },
      deviceScaleFactor,
    });
    await tab.setContent(
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    // The slash menu answers once its stubbed reads do.
    await tab.waitForSelector('[data-testid="mention-popover"] [role="option"]');
    // Let every colour transition settle before reading a colour.
    await tab.waitForTimeout(300);
    return tab;
  }

  /** The command chip's look equals the reference chip's beside it, but for its name. */
  const expectOneKind = (chips: ChipLook[], removable: boolean) => {
    expect(chips.map((chip) => chip.name)).toEqual([
      COMPOSER_COPY.chipCommands.bug.label,
      'ggplot',
    ]);
    const [command, reference] = chips;
    expect({ ...command, name: '' }).toEqual({ ...reference, name: '' });
    // The recipe itself, so two equally wrong chips do not pass.
    expect(command.glyph).toMatchObject({ width: 14, height: 14, ariaHidden: 'true' });
    expect(command.inside).toBe(true);
    if (removable) {
      expect(command.action).toMatchObject({ width: 16, height: 16 });
    } else {
      expect(command.action).toBeNull();
    }
  };

  for (const mode of ['light', 'dark'] as const) {
    it(`draws the command chip as a reference chip, first, in the composer's row (${mode})`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(mode);
      try {
        // The rail is `display: contents`, so the chips are measured against the row it sits in.
        const chips = await looks(tab, '[data-testid="composer-chips"]');
        expectOneKind(chips, true);
        // The action fades on `--dur-fast-min`, a live motion token, not a retired one
        // that would resolve to no transition at all.
        const duration = await fastMin(tab);
        expect(duration).not.toBe('0s');
        expect(chips[0].action!.transitionDuration).toBe(`${duration}, ${duration}`);
      } finally {
        await tab.close();
      }
    });

    it(`draws a sent /bug as the chip first, inside the bubble's tint (${mode})`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(mode);
      try {
        expectOneKind(await looks(tab, '#bubble'), false);
        // The prose follows the chip as typed, separator included, and no `/bug` is left as text.
        // What is drawn: the chips' screen-reader words are left out.
        const text = await tab.evaluate(() => {
          const copy = document.querySelector('#bubble')!.cloneNode(true) as HTMLElement;
          copy.querySelectorAll('.sr-only').forEach((node) => node.remove());
          return copy.textContent ?? '';
        });
        expect(text).not.toContain('/bug');
        expect(text).toContain(`${COMPOSER_COPY.chipCommands.bug.label} the export dialog closes`);
        // The chip shares the prose's first line rather than sitting on a line of its own.
        const [chip, line] = await tab.evaluate(() => {
          const command = document.querySelector('#bubble [data-testid="command-chip"]')!;
          const range = document.createRange();
          range.selectNodeContents(command.nextElementSibling!.firstChild!);
          const first = range.getClientRects()[0];
          const box = command.getBoundingClientRect();
          return [
            [box.top, box.bottom],
            [first.top, first.bottom],
          ];
        });
        expect(line[0]).toBeLessThan(chip[1]);
        expect(line[1]).toBeGreaterThan(chip[0]);
      } finally {
        await tab.close();
      }
    });

    it(`draws a queued /bug as the chip too (${mode})`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(mode);
      try {
        // One in the chips row, and one in the queued message above it.
        const where = await tab.evaluate(() =>
          [...document.querySelectorAll('#composer [data-testid="command-chip"]')].map((chip) =>
            chip.closest('[data-testid="composer-reference-rail"]') ? 'row' : 'queue'
          )
        );
        expect(where.sort()).toEqual(['queue', 'row']);
      } finally {
        await tab.close();
      }
    });

    it(`draws the edit box's chip as a reference chip, first, with its remove action (${mode})`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(mode);
      try {
        expectOneKind(await looks(tab, '#edit'), true);
      } finally {
        await tab.close();
      }
    });

    it(`draws the /bug row of the slash menu as every other command row (${mode})`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(mode);
      try {
        const rows = await tab.evaluate(() =>
          [
            ...document.querySelectorAll<HTMLElement>(
              '[data-testid="mention-popover"] [role="option"]'
            ),
          ].map((row) => {
            const rect = row.getBoundingClientRect();
            const glyph = row.querySelector('svg')!.getBoundingClientRect();
            const name = row.querySelector<HTMLElement>('.truncate')!;
            const badge = name.nextElementSibling as HTMLElement | null;
            const cs = getComputedStyle(row);
            return {
              name: name.textContent,
              selected: row.hasAttribute('data-selected'),
              height: rect.height,
              glyph: [glyph.left - rect.left, glyph.width, glyph.height],
              nameX: name.getBoundingClientRect().left - rect.left,
              fontSize: getComputedStyle(name).fontSize,
              nameColor: getComputedStyle(name).color,
              badge: badge?.textContent ?? null,
              radius: cs.borderRadius,
              paddingLeft: cs.paddingLeft,
            };
          })
        );
        const bug = rows.find((row) => row.name === 'bug')!;
        const clear = rows.find((row) => row.name === 'clear')!;
        expect(bug).toBeDefined();
        expect(clear).toBeDefined();
        expect({ ...bug, name: '', selected: false }).toEqual({
          ...clear,
          name: '',
          selected: false,
        });
        expect(bug.badge).toBe('Built-in');
        expect(bug.glyph.slice(1)).toEqual([16, 16]);
      } finally {
        await tab.close();
      }
    });
  }

  it('saves light and dark screenshots when asked', async (ctx) => {
    if (!browser || !SHOTS) return ctx.skip();
    mkdirSync(SHOTS, { recursive: true });
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode, 2);
      try {
        await tab.screenshot({ path: resolve(SHOTS, `command-chip-${mode}.png`), fullPage: true });
      } finally {
        await tab.close();
      }
    }
  });
});
