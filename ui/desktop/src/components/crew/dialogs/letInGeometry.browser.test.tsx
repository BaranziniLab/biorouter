// @vitest-environment node
/**
 * Let in's saved-code view, measured in a real layout engine (QA Q3-35).
 *
 * jsdom lays nothing out, so `LetInDialog.test.tsx` can only say which blocks stay when the
 * joiner arrives. The defect this file exists for was geometry: the status note went from "Code
 * saved. Gina is in as soon as Gina's Crew checks in; you can close this." (two lines in the
 * 400px dialog) to "Gina joined ito-lab" (one), and the hint row under the teams went from "You
 * can add Gina to a team once Gina joins." to nothing when no team offers a choice of channels.
 * The top-anchored dialog shrank, and "Add to {team}" moved out from under the host's pointer
 * just as it enabled. Every jsdom assertion passed.
 *
 * The real `SavedCodeStatus`, `NextStepHint` and `SavedViewBody` are rendered to markup
 * (react-dom/server) and laid out in Chromium with the real `dialogs.css`, in the 480px dialog's
 * body column (QA Q4-38: Let in is the forms' width now, not 400). Tailwind does not run here, so
 * the utilities they use are written out below at `main.css`'s values — and a class the table
 * does not know fails the suite, so a new layout utility cannot slip in unmeasured.
 *
 * Round 4 added two measurements (QA Q4-38): the one-line "joined" sentence sits in the middle of
 * the room its sizer keeps, not on the first line over a blank one; and the code view's unseen
 * copy of the saved view takes exactly the saved view's room, so the footer — "Let {first} in",
 * then "Add to {team}" — does not move when the host presses it (it dropped 90px).
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { Check } from '../../icons/app-icons';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { Note } from '../../ui/note';
import type { Team } from '../crewApi';
import { letInCopy } from './copy';
import {
  nextStepReserve,
  NextStepHint,
  SavedCodeStatus,
  SavedViewBody,
  type TeamOffer,
} from './LetInDialog';
import type { ChannelChoice } from './people';

const dialogsDir = dirname(fileURLToPath(import.meta.url));

/** `ModalShell`'s `md` width (480, the forms' width), less its body's `px-4` on each side. */
const BODY = 480 - 2 * 16;

const TOKENS = `:root {
  --font-body: Arial, 'Helvetica Neue', Helvetica, ui-sans-serif, -apple-system, BlinkMacSystemFont,
    'Segoe UI', Roboto, sans-serif;
}
body { margin: 0; font: 14px/20px var(--font-body); }
button { font: inherit; padding: 0; border: 0; background: none; }
* { box-sizing: border-box; }
p { margin: 0; }`;

/**
 * The Tailwind utilities that move a box, at `main.css`'s values (spacing 4px; text-supporting
 * 12/16; `border` is Tailwind's 1px with preflight's solid style).
 */
const UTILITIES: Record<string, string> = {
  flex: 'display: flex',
  'inline-flex': 'display: inline-flex',
  'flex-col': 'flex-direction: column',
  'items-start': 'align-items: flex-start',
  'items-center': 'align-items: center',
  'justify-center': 'justify-content: center',
  'gap-0.5': 'gap: 2px',
  'gap-1': 'gap: 4px',
  'gap-1.5': 'gap: 6px',
  'gap-2': 'gap: 8px',
  'gap-3': 'gap: 12px',
  'px-3': 'padding-inline: 12px',
  'py-2.5': 'padding-block: 10px',
  'pb-1': 'padding-bottom: 4px',
  border: 'border-width: 1px; border-style: solid',
  'mt-0.5': 'margin-top: 2px',
  'h-4': 'height: 16px',
  'w-4': 'width: 16px',
  'h-6': 'height: 24px',
  'w-6': 'width: 24px',
  'shrink-0': 'flex-shrink: 0',
  'min-w-0': 'min-width: 0',
  'flex-1': 'flex: 1 1 0%',
  truncate: 'overflow: hidden; text-overflow: ellipsis; white-space: nowrap',
  'whitespace-nowrap': 'white-space: nowrap',
  'font-mono': "font-family: ui-monospace, 'SF Mono', Menlo, Consolas, monospace",
  '[overflow-wrap:anywhere]': 'overflow-wrap: anywhere',
  'text-supporting': 'font-size: 12px; line-height: 16px; font-weight: 400',
  'text-label': 'font-size: 14px; line-height: 20px; font-weight: 500',
  'text-body': 'font-size: 14px; line-height: 20px; font-weight: 400',
  // The checkbox's own parts: absolutely placed inside its 24px box, so only the box lays out.
  relative: 'position: relative',
  absolute: 'position: absolute',
  'inset-0': 'inset: 0',
  'm-0': 'margin: 0',
  'h-full': 'height: 100%',
  'w-full': 'width: 100%',
  'h-3.5': 'height: 14px',
  'w-3.5': 'width: 14px',
  'appearance-none': 'appearance: none',
  'h-0.5': 'height: 2px',
  'w-2.5': 'width: 10px',
  // A team's button: the control ladder's `sm` rung (main.css `--control-sm`).
  'h-control-sm': 'height: 28px',
};

/** Classes that only paint, or that `dialogs.css` itself lays out. */
const PAINT_ONLY = [
  /^crew-/,
  /^(bg|text-text|border-border|rounded|opacity|cursor|pointer-events|transition|peer)-?/,
  /^border-\[/,
  // The checkbox's drawn square: absolutely placed inside its box, so it moves nothing.
  /^inset-\[/,
  /^(hover|active|disabled):/,
  /^\[&(amp;)?_svg/,
  /^(biorouter-focus-surface|tint-interactive)/,
  /^lucide/,
];

const escapeClass = (name: string) => name.replace(/[.:[\]&]/g, (c) => `\\${c}`);
const UTILITY_CSS = Object.entries(UTILITIES)
  .map(([name, rule]) => `.${escapeClass(name)} { ${rule}; }`)
  .join('\n');

/** Every class the markup carries that neither the table nor the paint-only list accounts for. */
function unmeasuredClasses(html: string): string[] {
  const names = new Set<string>();
  for (const match of html.matchAll(/class="([^"]*)"/g)) {
    for (const name of match[1].split(/\s+/)) if (name) names.add(name);
  }
  return [...names].filter(
    (name) => !(name in UTILITIES) && !PAINT_ONLY.some((pattern) => pattern.test(name))
  );
}

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

interface Measured {
  /** The block's height, as the dialog's column lays it out. */
  block: number;
  /** How many lines the sentence it shows really takes: one rect per line of its text. */
  lines: number;
  /** For a status note: the room above its words, and below them, inside the note. */
  above: number;
  below: number;
  /** For a status note: how far its icon's top sits below its first line's top. */
  icon: number;
}

describe('Let in keeps its height when the joiner arrives, in a real layout engine (Q3-35)', () => {
  let browser: Browser | null = null;
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome) — skipping the Let in geometry tests. ' +
          'Run `npx playwright install chromium` to enable them.'
      );
    }
    css = readFileSync(resolve(dialogsDir, 'dialogs.css'), 'utf8');
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** Each element laid out alone in the dialog's body column. */
  async function layOut(blocks: Record<string, ReactElement>): Promise<Record<string, Measured>> {
    const html = Object.entries(blocks)
      .map(
        ([key, block]) =>
          `<section data-block="${key}" style="width: ${BODY}px; margin-bottom: 40px">` +
          renderToStaticMarkup(block) +
          '</section>'
      )
      .join('');
    expect(unmeasuredClasses(html), 'a class with no measured rule').toEqual([]);
    const tab: Page = await browser!.newPage({ viewport: { width: 800, height: 1200 } });
    try {
      await tab.setContent(
        '<!doctype html><html><head><style>' +
          TOKENS +
          UTILITY_CSS +
          css +
          '</style></head><body>' +
          html +
          '</body></html>'
      );
      return await tab.evaluate(() => {
        const out: Record<
          string,
          { block: number; lines: number; above: number; below: number; icon: number }
        > = {};
        for (const section of Array.from(document.querySelectorAll('section[data-block]'))) {
          const block = section.firstElementChild!;
          // The live note's words, or the hint's own sentence.
          const shown =
            section.querySelector('[role="status"] span') ??
            section.querySelector('.crew-reserve-shown');
          const words = document.createRange();
          if (shown) words.selectNodeContents(shown);
          const noteElement = shown?.closest('[role="status"]');
          const note = noteElement?.getBoundingClientRect();
          const text = shown && shown.textContent ? words.getBoundingClientRect() : null;
          const firstLine = shown && shown.textContent ? words.getClientRects()[0] : null;
          const icon = noteElement?.querySelector(':scope > svg')?.getBoundingClientRect();
          out[(section as HTMLElement).dataset.block!] = {
            block: block.getBoundingClientRect().height,
            lines: shown && shown.textContent ? words.getClientRects().length : 0,
            above: note && text ? text.top - note.top : 0,
            below: note && text ? note.bottom - text.bottom : 0,
            icon: icon && firstLine ? icon.top - firstLine.top : Number.NaN,
          };
        }
        return out;
      });
    } finally {
      await tab.close();
    }
  }

  for (const first of ['Gina', '@crew_gina']) {
    it(`keeps the status note as tall once ${first} joins`, async (ctx) => {
      if (!browser) return ctx.skip();
      const status = (joined: boolean) => (
        <SavedCodeStatus joined={joined} first={first} workspace="ito-lab" steady />
      );
      const measured = await layOut({ waiting: status(false), joined: status(true) });
      // The words really do shrink: two lines or more become one.
      expect(measured.waiting.lines).toBeGreaterThanOrEqual(2);
      expect(measured.joined.lines).toBe(1);
      // The note does not.
      expect(measured.joined.block).toBe(measured.waiting.block);
    });

    it(`keeps the hint row as tall once ${first} joins, even with nothing to say`, async (ctx) => {
      if (!browser) return ctx.skip();
      const reserve = nextStepReserve(first);
      const measured = await layOut({
        waiting: <NextStepHint text={letInCopy.addAfterJoin(first)} reserve={reserve} />,
        choices: <NextStepHint text={letInCopy.channelsWithTeam} reserve={reserve} />,
        nothing: <NextStepHint text={null} reserve={reserve} />,
      });
      expect(measured.waiting.lines).toBeGreaterThanOrEqual(1);
      expect(measured.nothing.lines).toBe(0);
      expect(measured.nothing.block).toBeGreaterThanOrEqual(16);
      expect(measured.choices.block).toBe(measured.waiting.block);
      expect(measured.nothing.block).toBe(measured.waiting.block);
    });
  }

  it('centres the one-line "joined" sentence in the room it keeps (QA Q4-38)', async (ctx) => {
    if (!browser) return ctx.skip();
    const measured = await layOut({
      waiting: <SavedCodeStatus joined={false} first="Jack" workspace="wong-lab" steady />,
      joined: <SavedCodeStatus joined first="Jack" workspace="wong-lab" steady />,
    });
    expect(measured.joined.lines).toBe(1);
    expect(measured.joined.block).toBe(measured.waiting.block);
    // Not on the first line over a blank second one: as much room below the words as above.
    expect(Math.abs(measured.joined.above - measured.joined.below)).toBeLessThanOrEqual(1);
    expect(measured.joined.below).toBeLessThan(measured.waiting.block / 2);
  });

  it('keeps the check icon beside the first line, where every other Note puts it', async (ctx) => {
    if (!browser) return ctx.skip();
    const measured = await layOut({
      // Any Note: the place its icon takes beside the first line.
      plain: (
        <Note icon={Check} role="status">
          <span>{letInCopy.approved('Gina')}</span>
        </Note>
      ),
      waiting: <SavedCodeStatus joined={false} first="Gina" workspace="ito-lab" steady />,
      joined: <SavedCodeStatus joined first="Gina" workspace="ito-lab" steady />,
    });
    expect(measured.plain.lines).toBeGreaterThanOrEqual(2);
    expect(measured.waiting.lines).toBeGreaterThanOrEqual(2);
    expect(Number.isFinite(measured.plain.icon)).toBe(true);
    // Two lines: the icon is beside the first, not dropped between the two (it was ~7px lower).
    expect(Math.abs(measured.waiting.icon - measured.plain.icon)).toBeLessThanOrEqual(0.5);
    // One line, centred in the room: the icon stays level with the words it marks.
    expect(measured.joined.lines).toBe(1);
    expect(Math.abs(measured.joined.icon - measured.plain.icon)).toBeLessThanOrEqual(1);
    expect(Math.abs(measured.joined.above - measured.joined.below)).toBeLessThanOrEqual(1);
  });

  it('is exactly as tall as its tallest form, the bordered "Code saved" note', async (ctx) => {
    if (!browser) return ctx.skip();
    const measured = await layOut({
      tallest: <SavedCodeStatus joined={false} first="Gina" workspace="ito-lab" steady={false} />,
      joinedAlone: <SavedCodeStatus joined first="Gina" workspace="ito-lab" steady={false} />,
      joined: <SavedCodeStatus joined first="Gina" workspace="ito-lab" steady />,
    });
    // Alone, the joined note is shorter: the height the footer used to follow up the screen.
    expect(measured.joinedAlone.block).toBeLessThan(measured.tallest.block);
    expect(measured.joined.block).toBe(measured.tallest.block);
  });

  /** A team as the snapshot projects it. */
  const team = (id: string, name: string): Team => ({
    id,
    name,
    created_by: 'person-iris',
    members: ['person-iris'],
    general_channel_id: `${id}-general`,
  });
  const choices = (id: string, others: string[]): ChannelChoice[] => [
    { id: `${id}-general`, label: '#general', always: true, checked: true },
    ...others.map((name) => ({
      id: `${id}-${name}`,
      label: `#${name}`,
      always: false,
      checked: true,
    })),
  ];
  const FINGERPRINT = '3F2A 9C1E 77B0 D4E1 0A5C 88F2 61D9 BB04';

  /** The saved view as a joiner's approval first draws it, and the code view's copy of it. */
  function bothViews(first: string, offers: TeamOffer[], steady: boolean, hint: string | null) {
    const common = {
      first,
      workspace: 'wong-lab',
      fingerprint: FINGERPRINT,
      offers,
      hint,
      holdHint: steady && offers.length > 0,
    };
    return {
      saved: (
        <SavedViewBody
          {...common}
          status={
            <SavedCodeStatus joined={false} first={first} workspace="wong-lab" steady={steady} />
          }
        />
      ),
      reserved: <SavedViewBody {...common} steady={steady} status={null} sizer />,
    };
  }

  it('reserves exactly the saved view’s room in the code view, for one team (QA Q4-38)', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const first of ['Jack', '@crew_jack']) {
      const offers: TeamOffer[] = [
        {
          team: team('team-wong', 'Wong Lab'),
          label: letInCopy.channelsIn('Wong Lab'),
          choices: choices('team-wong', ['imaging']),
          disabled: true,
          button: null,
        },
      ];
      const { saved, reserved } = bothViews(first, offers, true, letInCopy.addAfterJoin(first));
      const measured = await layOut({ saved, reserved });
      // The saved view is the taller one that used to push the footer down.
      expect(measured.saved.block).toBeGreaterThan(150);
      expect(measured.reserved.block).toBe(measured.saved.block);
    }
  });

  it('reserves exactly the saved view’s room with several teams, each with its button', async (ctx) => {
    if (!browser) return ctx.skip();
    const offers: TeamOffer[] = ['Wong Lab', 'Imaging Core'].map((name, index) => ({
      team: team(`team-${index}`, name),
      label: letInCopy.channelsIn(name),
      choices: choices(`team-${index}`, index === 0 ? ['imaging', 'methods'] : ['scope']),
      disabled: true,
      button: {
        label: letInCopy.directAddToTeam('Jack', name),
        primary: true,
        first: false,
        waiting: true,
      },
    }));
    const { saved, reserved } = bothViews('Jack', offers, true, letInCopy.addAfterJoin('Jack'));
    const measured = await layOut({ saved, reserved });
    expect(measured.reserved.block).toBe(measured.saved.block);
  });

  it('reserves the room for a device of an existing member, too', async (ctx) => {
    if (!browser) return ctx.skip();
    const offers: TeamOffer[] = [
      {
        team: team('team-wong', 'Wong Lab'),
        label: letInCopy.channelsIn('Wong Lab'),
        choices: choices('team-wong', ['imaging']).map((choice) => ({
          ...choice,
          checked: choice.always,
        })),
        disabled: false,
        button: {
          label: letInCopy.directAddToTeam('Jack', 'Wong Lab'),
          primary: false,
          first: false,
          waiting: false,
        },
      },
    ];
    const { saved, reserved } = bothViews('Jack', offers, false, letInCopy.channelsWithTeam);
    const measured = await layOut({ saved, reserved });
    expect(measured.reserved.block).toBe(measured.saved.block);
  });
});
