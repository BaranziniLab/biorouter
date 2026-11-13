import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { useRef } from 'react';

/**
 * THE FOOTER CARDS' CONTRACT: the three items on the composer's footer line (the working folder,
 * the context ring and the cost) answer a hover, a click, a tap, a key, a press, a scroll and a
 * window switch in exactly the same way, and only one of their cards is open at a time. The owner
 * asked for that consistency in so many words, so it is pinned here as ONE table of behaviour
 * run over all three items: a rule that holds for two of them and not the third fails here, on the
 * row that names the odd one out.
 *
 * The blocks are numbered as the rules are in docs/desktop-ui/shared-controls.md ("Composer
 * footer cards"): 1 hover, 2 leaving and the pointer's way across (up from the trigger to the card,
 * and back down from the card to the trigger), 3 Tab, 4 click, tap, Enter and
 * Space, 5 a press inside, 6 Escape, Shift+Tab and an outside press, 7 one card at a time (every
 * ordered pair of items: a hover replaces a preview and opens nothing over a KEPT card, focus and
 * a typed key in it included, while a click, Enter, Space or a Tab still replaces it), 8 the
 * trigger's blur and a window switch (a card pinned from its trigger, or by a press inside while
 * focus was in the draft), 9 scroll, 10 the trigger's open state and its KEPT mark (hover peeks,
 * click pins: `data-pinned` on the trigger and the surface for a kept card, never for a preview,
 * gone on every close, and absent while the folder's chooser is open, #50), 11 the composer each
 * card hangs above; and, last, rule 3 where nothing tabbable follows the line (Home), so a Tab
 * moves into the card. The folder's own two rules (#50, #44) are `FolderAccessCard.test.tsx`'s. A
 * new rule goes into this table, so it holds for all three items at once; a new item on the line
 * goes into `ITEMS`. How each state LOOKS on the trigger is `footerCard.browser.test.tsx`'s.
 *
 * The real `ComposerFooter` is rendered under a composer stand-in, with all three items present,
 * exactly as `ChatInput` wires it (the composer card passed as `composerRef`), and a button on
 * either side of the line unless a row says otherwise. Only the daemon, the config store, the
 * model context and the pricing fetch are stand-ins.
 *
 * What each card SAYS is its own test's (`FolderAccessCard.test.tsx`,
 * `ContextWindowIndicator.test.tsx`, `CostTracker.render.test.tsx`), and so is what only the folder
 * does (Choose folder…, #44, #50). Where a card lands, and that the three surfaces look identical,
 * is `footerCard.browser.test.tsx`'s, because jsdom lays nothing out.
 */

const mocks = vi.hoisted(() => ({
  list: vi.fn(),
  read: vi.fn(),
  upsert: vi.fn(),
}));

vi.mock('../../api', () => ({
  listFolderAccess: mocks.list,
  revokeFolderAccess: vi.fn(async () => ({ data: { revoked: true } })),
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
}));
vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'desktop-human-proof' }),
}));
vi.mock('../../toasts', () => ({ toastError: vi.fn() }));
vi.mock('../ConfigContext', () => ({
  useConfig: () => ({ read: mocks.read, upsert: mocks.upsert }),
}));
vi.mock('../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({ currentModel: 'gpt-5.6-sol', currentProvider: 'versa_azure' }),
}));
vi.mock('../../utils/pricing', () => ({ fetchModelPricing: vi.fn(async () => null) }));

import { ComposerFooter, type ComposerFooterProps } from './ComposerFooter';
import { CONTEXT_CARD_COPY, COST_CARD_COPY, FOLDER_ACCESS_COPY, FOOTER_COPY } from './copy';
import { FOOTER_CARD_CLOSE_GRACE_MS, FOOTER_CARD_OPEN_DELAY_MS } from './useFooterCard';
import { BROWSER_SURFACE_MARKER } from '../../utils/surface';

const WORKING_DIR = '/Users/wgu/Projects/kdps';
const GRANT = {
  id: 'grant-1',
  folder: '/synthetic/results',
  access: 'read-only',
  descendants: true,
};

/** A chat with messages, a known folder, a loaded window with tokens and a cost above $0. */
const PROPS: ComposerFooterProps = {
  sessionId: 'chat-1',
  workingDir: WORKING_DIR,
  workingDirLocked: true,
  onWorkingDirChange: () => {},
  totalTokens: 92_000,
  tokenLimit: 128_000,
  isTokenLimitLoaded: true,
  onCompact: () => {},
  modelCostRows: [
    {
      provider: 'versa_azure',
      model: 'gpt-5.6-sol',
      inputTokens: 1000,
      outputTokens: 200,
      cacheReadTokens: 0,
      cacheCreationTokens: 0,
      totalTokens: 1200,
      turns: 2,
      totalCost: 0.42,
      costIsPartial: false,
    },
  ],
};

/**
 * The composer as `ChatInput` lays it out: a transcript that scrolls on its own, the composer
 * card, and the footer line under it, handed the card to hang above. A button on either side of
 * the line, so a Tab can arrive at its first item and leave its last; `after={false}` drops the
 * one after it, as on Home, where nothing tabbable follows the line and the cards, portalled to
 * the end of the page, are the next stop.
 */
function Composer({
  props = PROPS,
  after = true,
}: {
  props?: ComposerFooterProps;
  after?: boolean;
}) {
  const composer = useRef<HTMLDivElement>(null);
  return (
    <div data-testid="page">
      <div data-testid="transcript">
        <p>Transcript text</p>
      </div>
      <div ref={composer} data-testid="composer">
        <textarea aria-label="Draft" />
      </div>
      <button type="button">Before the line</button>
      <ComposerFooter {...props} composerRef={composer} />
      {after && <button type="button">After the line</button>}
    </div>
  );
}

interface Item {
  name: 'folder' | 'context' | 'cost';
  /** The trigger's `data-testid` and accessible name. */
  trigger: string;
  triggerName: string;
  /** The card's `data-testid`, accessible name and alignment. */
  card: string;
  cardName: string;
  align: 'start' | 'end';
  /** The card's first control, or null for a card that holds none (the cost's). */
  firstControl: (() => HTMLElement) | null;
  /**
   * What a press inside the card can land on: its text, and every kind of control a press there
   * handles itself (the context gauge's bar and marker take the press for a drag).
   */
  pressTargets: { name: string; get: (card: HTMLElement) => HTMLElement }[];
  /** Sets up this item's card with no control in it, or null for a card that always has one. */
  withoutControls: (() => void) | null;
}

const LABEL = {
  name: 'its section label',
  get: (card: HTMLElement) => card.querySelector<HTMLElement>('.br-footer-card__label')!,
};

/**
 * A folder the folder card writes (`FooterCardPath`), found by the absolute path it stands for:
 * its names sit in separate `<bdi>`s, so no text query finds the path whole. It throws when the
 * card writes no such folder, so a press can never land on nothing.
 */
function byPath(container: Element | Document, absolute: string): HTMLElement {
  const node = [...container.querySelectorAll<HTMLElement>('[data-path]')].find(
    (candidate) => candidate.dataset.path === absolute
  );
  if (!node) throw new Error(`the card writes no folder ${absolute}`);
  return node;
}

const ITEMS: Item[] = [
  {
    name: 'folder',
    trigger: 'dir-switcher-locked',
    triggerName: FOOTER_COPY.folderLabel(WORKING_DIR),
    card: 'folder-access-card',
    cardName: FOLDER_ACCESS_COPY.cardLabel,
    align: 'start',
    firstControl: () =>
      screen.getByRole('button', { name: FOLDER_ACCESS_COPY.revokeLabel(GRANT.folder) }),
    pressTargets: [
      LABEL,
      { name: 'a granted folder’s path', get: (card) => byPath(card, GRANT.folder) },
    ],
    // A chat with messages and no other approved folders: no Choose folder…, no Revoke.
    withoutControls: () => mocks.list.mockResolvedValue({ data: [] }),
  },
  {
    name: 'context',
    trigger: 'context-window-indicator',
    triggerName: FOOTER_COPY.contextTooltip(28, '92k', '128k'),
    card: 'context-card',
    cardName: CONTEXT_CARD_COPY.cardLabel,
    align: 'end',
    firstControl: () => screen.getByRole('slider', { name: CONTEXT_CARD_COPY.thresholdLabel }),
    pressTargets: [
      LABEL,
      {
        name: 'the gauge’s bar',
        get: (card) => within(card).getByRole('progressbar', { name: CONTEXT_CARD_COPY.barLabel }),
      },
      {
        name: 'the auto-compact marker',
        get: (card) => within(card).getByRole('slider', { name: CONTEXT_CARD_COPY.thresholdLabel }),
      },
    ],
    // The marker is always there.
    withoutControls: null,
  },
  {
    name: 'cost',
    trigger: 'chat-cost',
    triggerName: FOOTER_COPY.cost('$0.42', false),
    card: 'cost-card',
    cardName: COST_CARD_COPY.cardLabel,
    align: 'end',
    firstControl: null,
    pressTargets: [LABEL, { name: 'its total', get: (card) => within(card).getByText('$0.42') }],
    withoutControls: () => {},
  },
];

const [FOLDER] = ITEMS;

/** Every ordered pair of different items, for the one-at-a-time rule. */
const PAIRS = ITEMS.flatMap((first) =>
  ITEMS.filter((then) => then !== first).map((then) => ({ first, then }))
);

/** The pairs a single Tab or Shift+Tab moves between: the items side by side on the line. */
const NEIGHBOURS = PAIRS.filter(
  ({ first, then }) => Math.abs(ITEMS.indexOf(first) - ITEMS.indexOf(then)) === 1
);

beforeAll(() => {
  // Radix measures the anchor and the surface.
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
  );
});
afterAll(() => vi.unstubAllGlobals());

beforeEach(() => {
  mocks.list.mockReset().mockResolvedValue({ data: [GRANT] });
  mocks.read.mockReset().mockResolvedValue(null);
  mocks.upsert.mockReset().mockResolvedValue(undefined);
});

afterEach(async () => {
  // Let a grant read or a pricing fetch that is still in flight land inside act.
  await act(async () => {});
  vi.useRealTimers();
});

const trigger = (item: Item) => screen.getByTestId(item.trigger);
const surface = (item: Item) => document.querySelector<HTMLElement>(`[data-testid="${item.card}"]`);
const isOpen = (item: Item) => surface(item)?.getAttribute('data-state') === 'open';
const openCards = () => ITEMS.filter(isOpen).map((item) => item.name);

/**
 * Where the item says its card is KEPT (`data-pinned`): on the trigger, which pickers.css paints,
 * and on the surface, unstyled. Read off both, so the two can never disagree. A card that has
 * closed, or never opened, has no surface to mark.
 */
const marks = (item: Item) => ({
  trigger: trigger(item).hasAttribute('data-pinned'),
  surface: surface(item)?.hasAttribute('data-pinned') ?? false,
});
const UNMARKED = { trigger: false, surface: false };
const MARKED = { trigger: true, surface: true };

/** Settle what mounting or opening started (the pricing fetch, the grant read). */
const flush = () => act(async () => {});

/** Render the composer, with every item drawn. */
async function renderLine() {
  render(<Composer />);
  await flush();
  for (const item of ITEMS) expect(trigger(item)).toBeInTheDocument();
}

/** What a Tab onto the item comes from: the item before it on the line, or the button before it. */
function before(item: Item): HTMLElement {
  const index = ITEMS.indexOf(item);
  return index === 0
    ? screen.getByRole('button', { name: 'Before the line' })
    : trigger(ITEMS[index - 1]);
}

/**
 * Run the fake clock forward inside an async act, so what a timer started (the folder card's grant
 * read, once the card opens) lands inside it too.
 */
const advance = (ms: number) =>
  act(async () => {
    vi.advanceTimersByTime(ms);
  });

/** A mouse comes to rest on the trigger and stays past the open delay (fake timers). */
async function hoverOpen(item: Item) {
  fireEvent.pointerEnter(trigger(item), { pointerType: 'mouse' });
  await advance(FOOTER_CARD_OPEN_DELAY_MS + 10);
}

/** Real-time wait, for the userEvent cases. */
const wait = (ms: number) => act(() => new Promise((resolve) => setTimeout(resolve, ms)));

/** Focus the trigger as a program would (no Tab), press Enter, and wait for the card to take it. */
async function pinFromKeyboard(user: ReturnType<typeof userEvent.setup>, item: Item) {
  act(() => trigger(item).focus());
  await user.keyboard('{Enter}');
  await waitFor(() => expect(surface(item)).toHaveFocus());
  await flush();
}

describe.each(ITEMS)('the $name item', (item) => {
  describe('the trigger and the surface are the same kind of thing', () => {
    it('is a real button on the line’s recipe, named for its whole reading, whose card is a named dialog', async () => {
      const user = userEvent.setup();
      await renderLine();
      const button = trigger(item);
      expect(button.tagName).toBe('BUTTON');
      expect(button).toHaveAttribute('type', 'button');
      expect(button).toHaveClass('br-footline__item');
      expect(button).toHaveAccessibleName(item.triggerName);
      expect(button).toHaveAttribute('aria-haspopup', 'dialog');
      expect(button).toHaveAttribute('aria-expanded', 'false');
      expect(button).not.toHaveAttribute('aria-controls');
      expect(surface(item)).toBeNull();

      await user.click(button);
      await flush();
      const card = surface(item)!;
      expect(screen.getByRole('dialog', { name: item.cardName })).toBe(card);
      expect(card).toHaveClass('br-footer-card');
      expect(card).toHaveClass('biorouter-popover-surface');
      expect(card).toHaveAttribute('data-align', item.align);
      expect(
        card.querySelector('.br-footer-card__section > .br-footer-card__label')
      ).not.toBeNull();
    });
  });

  describe('1. hover', () => {
    it('opens after 300ms of a mouse or pen hover, not before', async () => {
      vi.useFakeTimers();
      await renderLine();
      for (const pointerType of ['mouse', 'pen']) {
        fireEvent.pointerEnter(trigger(item), { pointerType });
        await advance(FOOTER_CARD_OPEN_DELAY_MS - 10);
        expect(isOpen(item), `${pointerType}: not before the delay`).toBe(false);
        await advance(20);
        expect(isOpen(item), `${pointerType}: open after it`).toBe(true);
        fireEvent.pointerLeave(trigger(item), { pointerType });
        await advance(FOOTER_CARD_CLOSE_GRACE_MS + 10);
        expect(isOpen(item)).toBe(false);
      }
      expect(FOOTER_CARD_OPEN_DELAY_MS).toBe(300);
    });

    it('never opens on a touch hover', async () => {
      vi.useFakeTimers();
      await renderLine();
      fireEvent.pointerEnter(trigger(item), { pointerType: 'touch' });
      await advance(FOOTER_CARD_OPEN_DELAY_MS * 3);
      expect(isOpen(item)).toBe(false);
      expect(trigger(item)).toHaveAttribute('aria-expanded', 'false');
    });

    it('does not open for a pointer that leaves before the delay', async () => {
      vi.useFakeTimers();
      await renderLine();
      fireEvent.pointerEnter(trigger(item), { pointerType: 'mouse' });
      await advance(FOOTER_CARD_OPEN_DELAY_MS / 2);
      fireEvent.pointerLeave(trigger(item), { pointerType: 'mouse' });
      await advance(FOOTER_CARD_OPEN_DELAY_MS * 3);
      expect(isOpen(item)).toBe(false);
    });
  });

  describe('2. leaving', () => {
    it('closes an unpinned card 150ms after the pointer leaves the trigger', async () => {
      vi.useFakeTimers();
      await renderLine();
      await hoverOpen(item);
      fireEvent.pointerLeave(trigger(item), { pointerType: 'mouse' });
      await advance(FOOTER_CARD_CLOSE_GRACE_MS - 10);
      expect(isOpen(item)).toBe(true);
      await advance(20);
      expect(isOpen(item)).toBe(false);
      expect(FOOTER_CARD_CLOSE_GRACE_MS).toBe(150);
    });

    it('stays open while the pointer is in the card, and closes 150ms after it leaves both', async () => {
      vi.useFakeTimers();
      await renderLine();
      await hoverOpen(item);
      fireEvent.pointerLeave(trigger(item), { pointerType: 'mouse' });
      fireEvent.pointerEnter(surface(item)!, { pointerType: 'mouse' });
      await advance(FOOTER_CARD_CLOSE_GRACE_MS * 4);
      expect(isOpen(item)).toBe(true);
      fireEvent.pointerLeave(surface(item)!, { pointerType: 'mouse' });
      await advance(FOOTER_CARD_CLOSE_GRACE_MS - 10);
      expect(isOpen(item)).toBe(true);
      await advance(20);
      expect(isOpen(item)).toBe(false);
    });

    it('stays open for a pointer moving along its way to the card, and not for one off it', async () => {
      vi.useFakeTimers();
      await renderLine();
      await hoverOpen(item);
      // jsdom places nothing: give the card the box a real layout would, above the line.
      vi.spyOn(surface(item)!, 'getBoundingClientRect').mockReturnValue(
        new DOMRect(100, 100, 320, 270)
      );
      fireEvent.pointerLeave(trigger(item), { pointerType: 'mouse', clientX: 150, clientY: 560 });
      for (let step = 0; step < 6; step++) {
        await advance(FOOTER_CARD_CLOSE_GRACE_MS - 50);
        fireEvent.pointerMove(document, {
          pointerType: 'mouse',
          clientX: 152,
          clientY: 540 - step * 10,
        });
      }
      expect(isOpen(item), 'six grace periods later, still on its way').toBe(true);
      // Off its way, and still moving: a pointer outside the polygon re-arms nothing, so the grace
      // the last move on the way armed runs out WHILE the moves keep coming. (One stray move and
      // then a wait would pass even if every move re-armed the grace.)
      const strideMs = 50;
      for (let step = 0; step < 8; step++) {
        fireEvent.pointerMove(document, {
          pointerType: 'mouse',
          clientX: 800 - step * 20,
          clientY: 520,
        });
        await advance(strideMs);
        if ((step + 1) * strideMs > FOOTER_CARD_CLOSE_GRACE_MS) {
          expect(isOpen(item), `closed ${(step + 1) * strideMs}ms in, still moving`).toBe(false);
        }
      }
    });

    // The way back: the same grace, from the card down to its trigger. Without it, a person who
    // reads the card and heads back to the trigger (to click it and pin the card) loses the card
    // on the way across the composer.
    const CARD = new DOMRect(100, 100, 320, 270);
    /** Where the pointer leaves the card: its bottom edge, straight above the trigger. */
    const EXIT = { x: 160, y: CARD.bottom };
    const STRIDE_MS = 50;

    /**
     * Hover the card open, then move the pointer up into it and rest there past the grace. jsdom
     * places nothing, so the card and the trigger get the boxes a real layout would give them: the
     * card above the composer, the trigger on the line under it.
     */
    async function restInCard(): Promise<HTMLElement> {
      await hoverOpen(item);
      const card = surface(item)!;
      vi.spyOn(card, 'getBoundingClientRect').mockReturnValue(CARD);
      vi.spyOn(trigger(item), 'getBoundingClientRect').mockReturnValue(
        new DOMRect(130, 540, 60, 20)
      );
      fireEvent.pointerLeave(trigger(item), { pointerType: 'mouse', clientX: 160, clientY: 540 });
      fireEvent.pointerEnter(card, { pointerType: 'mouse' });
      await advance(FOOTER_CARD_CLOSE_GRACE_MS * 2);
      expect(isOpen(item), 'resting in the card').toBe(true);
      return card;
    }

    /** One move on the way down from the card to the trigger, `step` strides along. */
    const moveDown = (step: number) =>
      fireEvent.pointerMove(document, {
        pointerType: 'mouse',
        clientX: EXIT.x,
        clientY: EXIT.y + 15 + step * 25,
      });

    it('stays open for a pointer moving from the card back down to its trigger, and once it is there', async () => {
      vi.useFakeTimers();
      await renderLine();
      const card = await restInCard();
      fireEvent.pointerLeave(card, { pointerType: 'mouse', clientX: EXIT.x, clientY: EXIT.y });
      // A move every 50ms, for far longer than the grace: each one, on the way, re-arms it.
      const steps = 7;
      expect(steps * STRIDE_MS).toBeGreaterThan(FOOTER_CARD_CLOSE_GRACE_MS * 2);
      for (let step = 0; step < steps; step++) {
        await advance(STRIDE_MS);
        moveDown(step);
        expect(isOpen(item), `${(step + 1) * STRIDE_MS}ms on the way down`).toBe(true);
      }
      // On the trigger: the card stays for as long as the pointer does.
      fireEvent.pointerEnter(trigger(item), { pointerType: 'mouse' });
      expect(isOpen(item), 'arriving on the trigger').toBe(true);
      await advance(FOOTER_CARD_CLOSE_GRACE_MS * 4);
      expect(isOpen(item), 'resting on the trigger').toBe(true);
      // Not a card that closed on the way and that the arrival opened again after the delay.
      expect(surface(item), 'the same card, never closed').toBe(card);
    });

    it('closes for a pointer that leaves the card and strays off its way down, while it is still moving', async () => {
      vi.useFakeTimers();
      await renderLine();
      const card = await restInCard();
      fireEvent.pointerLeave(card, { pointerType: 'mouse', clientX: EXIT.x, clientY: EXIT.y });
      // A little way down first, which holds it...
      for (let step = 0; step < 2; step++) {
        await advance(STRIDE_MS);
        moveDown(step);
      }
      expect(isOpen(item), 'on its way down').toBe(true);
      // ...then off to the side, below the card and nowhere near the trigger, never stopping: the
      // grace the last move on the way armed runs out while the moves keep coming.
      for (let step = 0; step < 8; step++) {
        fireEvent.pointerMove(document, {
          pointerType: 'mouse',
          clientX: 500 + step * 20,
          clientY: CARD.bottom + 50,
        });
        await advance(STRIDE_MS);
        if ((step + 1) * STRIDE_MS > FOOTER_CARD_CLOSE_GRACE_MS) {
          expect(isOpen(item), `closed ${(step + 1) * STRIDE_MS}ms off its way, still moving`).toBe(
            false
          );
        }
      }
    });
  });

  describe('3. focus', () => {
    it('opens unpinned at once on a Tab onto the trigger, and closes on a Tab away', async () => {
      const user = userEvent.setup();
      await renderLine();
      act(() => before(item).focus());
      expect(openCards()).toEqual([]);
      await user.tab();
      expect(trigger(item)).toHaveFocus();
      expect(isOpen(item)).toBe(true);
      expect(trigger(item)).toHaveAttribute('aria-expanded', 'true');
      // A hover card does not take focus.
      expect(document.activeElement).toBe(trigger(item));
      await user.tab();
      expect(trigger(item)).not.toHaveFocus();
      expect(isOpen(item)).toBe(false);
    });

    it('stays shut when a program focuses the trigger', async () => {
      await renderLine();
      act(() => trigger(item).focus());
      await wait(20);
      expect(trigger(item)).toHaveFocus();
      expect(isOpen(item)).toBe(false);
    });
  });

  describe('4. click, tap and keyboard', () => {
    it('pins on a click, holds while the pointer wanders off, and a second click closes it', async () => {
      const user = userEvent.setup();
      await renderLine();
      await user.click(trigger(item));
      expect(isOpen(item)).toBe(true);
      await user.unhover(trigger(item));
      await wait(FOOTER_CARD_CLOSE_GRACE_MS * 2);
      expect(isOpen(item), 'pinned').toBe(true);
      await user.click(trigger(item));
      expect(isOpen(item)).toBe(false);
      expect(trigger(item)).toHaveAttribute('aria-expanded', 'false');
      await flush();
    });

    it('pins on a tap, which never hovers', async () => {
      const user = userEvent.setup();
      await renderLine();
      await user.pointer({ keys: '[TouchA]', target: trigger(item) });
      expect(isOpen(item)).toBe(true);
      await wait(FOOTER_CARD_CLOSE_GRACE_MS * 2);
      expect(isOpen(item), 'pinned').toBe(true);
      await user.pointer({ keys: '[TouchA]', target: trigger(item) });
      expect(isOpen(item)).toBe(false);
    });

    it('pins a card a hover opened instead of closing it', async () => {
      vi.useFakeTimers();
      await renderLine();
      await hoverOpen(item);
      fireEvent.click(trigger(item), { detail: 1 });
      fireEvent.pointerLeave(trigger(item), { pointerType: 'mouse' });
      await advance(FOOTER_CARD_CLOSE_GRACE_MS * 4);
      expect(isOpen(item)).toBe(true);
    });

    it.each(['{Enter}', ' '])(
      'pins on %j and moves focus onto the named card, whose next Tab reaches its first control',
      async (key) => {
        const user = userEvent.setup();
        await renderLine();
        act(() => trigger(item).focus());
        await user.keyboard(key);
        expect(isOpen(item)).toBe(true);
        await waitFor(() => expect(surface(item)).toHaveFocus());
        expect(document.activeElement).toBe(screen.getByRole('dialog', { name: item.cardName }));
        await flush();
        await user.tab();
        if (item.firstControl) {
          expect(item.firstControl()).toHaveFocus();
          expect(isOpen(item), 'pinned: Tab stays in the card').toBe(true);
        } else {
          // Nothing in the card to reach: the Tab leaves it, as a Tab away from the trigger does,
          // rather than staying on the surface and doing nothing. Where it lands is the browser's
          // (from the trigger: the next test and footerCard.browser.test.tsx).
          expect(isOpen(item)).toBe(false);
          expect(surface(item)?.contains(document.activeElement) ?? false).toBe(false);
        }
      }
    );

    it.runIf(item.withoutControls !== null)(
      'closes a card that holds no control on a Tab off its surface, and lets the Tab carry on from the trigger',
      async () => {
        item.withoutControls!();
        const user = userEvent.setup();
        await renderLine();
        await pinFromKeyboard(user, item);
        expect(surface(item)!.querySelector('button, input, [tabindex="0"]')).toBeNull();
        // Radix's focus loop cancels a Tab on a surface with nothing to wrap to. Here the card
        // hands focus to the trigger during the key and leaves the Tab alone, so the browser
        // carries on from the trigger to whatever follows it (jsdom's user-event would start from
        // the key's target instead, so the key is sent bare).
        const notCancelled = fireEvent.keyDown(surface(item)!, { key: 'Tab' });
        expect(notCancelled, 'the Tab is not swallowed').toBe(true);
        expect(trigger(item)).toHaveFocus();
        expect(isOpen(item)).toBe(false);
        await wait(20);
        expect(isOpen(item), 'the focus handed back reopens nothing').toBe(false);
      }
    );

    it.each(['{Enter}', ' '])(
      'pins a card a Tab opened on %j, and moves focus onto it',
      async (key) => {
        const user = userEvent.setup();
        await renderLine();
        act(() => before(item).focus());
        await user.tab();
        expect(isOpen(item)).toBe(true);
        await user.keyboard(key);
        expect(isOpen(item)).toBe(true);
        await waitFor(() => expect(surface(item)).toHaveFocus());
        await flush();
        // Pinned now: a pointer leaving the card no longer closes it.
        fireEvent.pointerLeave(surface(item)!, { pointerType: 'mouse' });
        await wait(FOOTER_CARD_CLOSE_GRACE_MS * 2);
        expect(isOpen(item), 'pinned').toBe(true);
      }
    );
  });

  describe('5. a press inside the card', () => {
    it('pins the card, so the pointer leaving no longer closes it', async () => {
      vi.useFakeTimers();
      await renderLine();
      await hoverOpen(item);
      const label = surface(item)!.querySelector<HTMLElement>('.br-footer-card__label')!;
      fireEvent.pointerDown(label, { pointerType: 'mouse' });
      fireEvent.pointerUp(label, { pointerType: 'mouse' });
      fireEvent.pointerLeave(trigger(item), { pointerType: 'mouse' });
      fireEvent.pointerLeave(surface(item)!, { pointerType: 'mouse' });
      await advance(FOOTER_CARD_CLOSE_GRACE_MS * 4);
      expect(isOpen(item)).toBe(true);
    });
  });

  describe('6. closing, and where focus goes', () => {
    it('closes on Escape and hands focus back to the trigger, which that does not reopen', async () => {
      const user = userEvent.setup();
      await renderLine();
      await pinFromKeyboard(user, item);
      // From the first control, or from the surface of a card that holds none (a Tab would leave
      // that one: rule 4).
      if (item.firstControl) await user.tab();
      await user.keyboard('{Escape}');
      expect(isOpen(item)).toBe(false);
      expect(trigger(item)).toHaveFocus();
      await wait(20);
      expect(isOpen(item)).toBe(false);
    });

    it('closes on Shift+Tab off the card’s surface and hands focus back to the trigger', async () => {
      const user = userEvent.setup();
      await renderLine();
      await pinFromKeyboard(user, item);
      await user.keyboard('{Shift>}{Tab}{/Shift}');
      expect(trigger(item)).toHaveFocus();
      expect(isOpen(item)).toBe(false);
      await wait(20);
      expect(isOpen(item)).toBe(false);
    });

    it('closes on an outside press and leaves focus where the press put it', async () => {
      const user = userEvent.setup();
      await renderLine();
      await pinFromKeyboard(user, item);
      const triggerFocused = vi.fn();
      trigger(item).addEventListener('focus', triggerFocused);
      await user.click(screen.getByText('Transcript text'));
      expect(isOpen(item)).toBe(false);
      expect(triggerFocused).not.toHaveBeenCalled();
      expect(trigger(item)).not.toHaveFocus();
    });

    it('treats a press on the trigger as the trigger’s own: it closes the card, focus stays', async () => {
      const user = userEvent.setup();
      await renderLine();
      await pinFromKeyboard(user, item);
      await user.click(trigger(item));
      expect(isOpen(item)).toBe(false);
      expect(trigger(item)).toHaveFocus();
    });

    it('closes on a press into the draft, which takes the focus', async () => {
      const user = userEvent.setup();
      await renderLine();
      await user.click(trigger(item));
      await user.click(screen.getByRole('textbox', { name: 'Draft' }));
      expect(isOpen(item)).toBe(false);
      expect(screen.getByRole('textbox', { name: 'Draft' })).toHaveFocus();
    });
  });

  describe('8. the trigger’s blur: a window switch, and focus that really moves', () => {
    it('keeps a pinned card when the trigger blurs with nowhere for focus to go', async () => {
      const user = userEvent.setup();
      await renderLine();
      await user.click(trigger(item));
      expect(trigger(item)).toHaveFocus();
      act(() => {
        trigger(item).blur();
        window.dispatchEvent(new FocusEvent('blur'));
      });
      await wait(20);
      expect(isOpen(item)).toBe(true);
      // And the window coming back finds it as it was.
      act(() => window.dispatchEvent(new FocusEvent('focus')));
      expect(isOpen(item)).toBe(true);
    });

    it.each(item.pressTargets)(
      'keeps a card pinned by a press on $name, with focus in the draft, through a window switch',
      async ({ get }) => {
        const user = userEvent.setup();
        await renderLine();
        const draft = screen.getByRole('textbox', { name: 'Draft' });
        await user.click(draft);
        await user.hover(trigger(item));
        await waitFor(() => expect(isOpen(item)).toBe(true));
        await flush();
        expect(draft, 'a hover leaves focus where it was').toHaveFocus();
        await user.pointer({ keys: '[MouseLeft]', target: get(surface(item)!) });
        // The press moved focus into the card, as a press anywhere in any of the three does.
        // A card whose press left focus in the draft lost it to the window switch below.
        expect(surface(item)!.contains(document.activeElement), 'focus is in the card').toBe(true);
        const focused = document.activeElement as HTMLElement;
        // The window loses focus, then comes back: Chromium blurs what had focus, then focuses it
        // again, which Radix reads as focus outside the card when that is the draft.
        act(() => {
          focused.blur();
          window.dispatchEvent(new FocusEvent('blur'));
        });
        await wait(20);
        act(() => {
          window.dispatchEvent(new FocusEvent('focus'));
          focused.focus();
        });
        await wait(20);
        expect(isOpen(item), 'the pinned card survived the window switch').toBe(true);
      }
    );

    it('still closes a pinned card when focus really moves elsewhere in the page', async () => {
      const user = userEvent.setup();
      await renderLine();
      await user.click(trigger(item));
      await user.tab();
      expect(trigger(item)).not.toHaveFocus();
      expect(isOpen(item)).toBe(false);
    });

    it('closes an unpinned card on the same blur', async () => {
      const user = userEvent.setup();
      await renderLine();
      act(() => before(item).focus());
      await user.tab();
      expect(isOpen(item)).toBe(true);
      act(() => {
        trigger(item).blur();
        window.dispatchEvent(new FocusEvent('blur'));
      });
      expect(isOpen(item)).toBe(false);
    });
  });

  describe('9. scroll', () => {
    it('closes an unpinned card on a scroll that moves the trigger, and on no other', async () => {
      vi.useFakeTimers();
      await renderLine();
      await hoverOpen(item);
      act(() => {
        fireEvent.scroll(screen.getByTestId('transcript'));
      });
      expect(isOpen(item), 'a sibling scroller').toBe(true);
      act(() => {
        fireEvent.scroll(surface(item)!);
      });
      expect(isOpen(item), 'inside the card').toBe(true);
      act(() => {
        fireEvent.scroll(screen.getByTestId('composer-footer'));
      });
      expect(isOpen(item), 'an ancestor of the trigger').toBe(false);
    });

    it('keeps a pinned card through any scroll', async () => {
      const user = userEvent.setup();
      await renderLine();
      await user.click(trigger(item));
      act(() => {
        fireEvent.scroll(screen.getByTestId('page'));
        fireEvent.scroll(window);
      });
      expect(isOpen(item)).toBe(true);
    });
  });

  describe('10. the trigger shows its card’s state', () => {
    it('says the card is open, and which card, for as long as it is: hover, Tab or pin', async () => {
      const user = userEvent.setup();
      await renderLine();
      const button = trigger(item);
      const shows = (open: boolean) => {
        expect(button).toHaveAttribute('aria-expanded', String(open));
        if (open) expect(button).toHaveAttribute('aria-controls', surface(item)!.id);
        else expect(button).not.toHaveAttribute('aria-controls');
      };
      shows(false);
      await user.hover(button);
      await waitFor(() => expect(isOpen(item)).toBe(true));
      shows(true);
      await user.unhover(button);
      await waitFor(() => expect(isOpen(item)).toBe(false));
      shows(false);
      act(() => before(item).focus());
      await user.tab();
      shows(true);
      await user.click(button);
      shows(true);
      await user.click(button);
      shows(false);
      await flush();
    });

    // HOVER PEEKS, CLICK PINS. `aria-expanded` is true for a preview and a kept card alike (above);
    // only a kept card is marked, and the mark is what the trigger paints.
    it('marks nothing while a hover or a Tab only previews the card', async () => {
      const user = userEvent.setup();
      await renderLine();
      expect(marks(item), 'at rest').toEqual(UNMARKED);
      await user.hover(trigger(item));
      await waitFor(() => expect(isOpen(item)).toBe(true));
      expect(trigger(item)).toHaveAttribute('aria-expanded', 'true');
      expect(marks(item), 'a hover preview').toEqual(UNMARKED);
      // The pointer on its way into the card, and resting there: still a preview.
      await user.unhover(trigger(item));
      await user.hover(surface(item)!);
      await wait(FOOTER_CARD_CLOSE_GRACE_MS * 2);
      expect(isOpen(item)).toBe(true);
      expect(marks(item), 'the pointer in the card').toEqual(UNMARKED);
      await user.unhover(surface(item)!);
      await waitFor(() => expect(isOpen(item)).toBe(false));
      act(() => before(item).focus());
      await user.tab();
      expect(isOpen(item)).toBe(true);
      expect(trigger(item)).toHaveAttribute('aria-expanded', 'true');
      expect(marks(item), 'a Tab preview').toEqual(UNMARKED);
      await flush();
    });

    const KEEPS: {
      how: string;
      keep: (user: ReturnType<typeof userEvent.setup>) => Promise<void>;
    }[] = [
      { how: 'a click', keep: (user) => user.click(trigger(item)) },
      {
        how: 'a tap',
        keep: (user) => user.pointer({ keys: '[TouchA]', target: trigger(item) }),
      },
      {
        how: 'Enter',
        keep: async (user) => {
          act(() => trigger(item).focus());
          await user.keyboard('{Enter}');
          await waitFor(() => expect(surface(item)).toHaveFocus());
        },
      },
      {
        how: 'Space',
        keep: async (user) => {
          act(() => trigger(item).focus());
          await user.keyboard(' ');
          await waitFor(() => expect(surface(item)).toHaveFocus());
        },
      },
      {
        how: 'a click on a hover preview',
        keep: async (user) => {
          await user.hover(trigger(item));
          await waitFor(() => expect(isOpen(item)).toBe(true));
          expect(marks(item), 'before the click').toEqual(UNMARKED);
          await user.click(trigger(item));
        },
      },
      {
        how: 'a press inside a hover preview',
        keep: async (user) => {
          await user.hover(trigger(item));
          await waitFor(() => expect(isOpen(item)).toBe(true));
          expect(marks(item), 'before the press').toEqual(UNMARKED);
          await user.pointer({ keys: '[MouseLeft]', target: LABEL.get(surface(item)!) });
        },
      },
      {
        how: 'focus entering a Tab preview',
        keep: async (user) => {
          act(() => before(item).focus());
          await user.tab();
          expect(isOpen(item)).toBe(true);
          expect(marks(item), 'before focus enters').toEqual(UNMARKED);
          act(() => surface(item)!.focus());
        },
      },
    ];

    it.each(KEEPS)(
      'marks the card kept, on the trigger and the surface alike, once $how keeps it',
      async ({ keep }) => {
        const user = userEvent.setup();
        await renderLine();
        expect(marks(item)).toEqual(UNMARKED);
        await keep(user);
        await flush();
        expect(isOpen(item)).toBe(true);
        expect(marks(item)).toEqual(MARKED);
        expect(trigger(item)).toHaveAttribute('data-pinned', 'true');
        expect(trigger(item)).toHaveAttribute('aria-expanded', 'true');
        // A disclosure, not a toggle: the mark is never a pressed state.
        expect(trigger(item)).not.toHaveAttribute('aria-pressed');
        // And it holds once the pointer has gone: a kept card ignores the pointer.
        fireEvent.pointerLeave(trigger(item), { pointerType: 'mouse' });
        fireEvent.pointerLeave(surface(item)!, { pointerType: 'mouse' });
        await wait(FOOTER_CARD_CLOSE_GRACE_MS * 2);
        expect(marks(item), 'the pointer gone').toEqual(MARKED);
      }
    );

    const CLOSES: {
      how: string;
      close: (user: ReturnType<typeof userEvent.setup>) => Promise<void>;
      /** How the card is kept first: a click, or Enter (focus in the card). */
      from: 'click' | 'keyboard';
    }[] = [
      { how: 'a second click', from: 'click', close: (user) => user.click(trigger(item)) },
      { how: 'Escape', from: 'keyboard', close: (user) => user.keyboard('{Escape}') },
      {
        how: 'Shift+Tab off its surface',
        from: 'keyboard',
        close: (user) => user.keyboard('{Shift>}{Tab}{/Shift}'),
      },
      {
        how: 'an outside press',
        from: 'click',
        close: (user) => user.click(screen.getByText('Transcript text')),
      },
      {
        how: 'a press into the draft',
        from: 'click',
        close: (user) => user.click(screen.getByRole('textbox', { name: 'Draft' })),
      },
      { how: 'focus moving elsewhere (a Tab away)', from: 'click', close: (user) => user.tab() },
    ];

    it.each(CLOSES)('drops the mark when $how closes the kept card', async ({ from, close }) => {
      const user = userEvent.setup();
      await renderLine();
      if (from === 'click') await user.click(trigger(item));
      else await pinFromKeyboard(user, item);
      await flush();
      expect(marks(item), 'kept').toEqual(MARKED);
      await close(user);
      await flush();
      expect(isOpen(item)).toBe(false);
      expect(marks(item)).toEqual(UNMARKED);
    });

    it('comes back from a hover as a preview after a kept card closed, unmarked', async () => {
      const user = userEvent.setup();
      await renderLine();
      await user.click(trigger(item));
      expect(marks(item)).toEqual(MARKED);
      await user.keyboard('{Escape}');
      expect(isOpen(item)).toBe(false);
      await user.unhover(trigger(item));
      await user.hover(trigger(item));
      await waitFor(() => expect(isOpen(item)).toBe(true));
      expect(marks(item), 'a preview again').toEqual(UNMARKED);
      await flush();
    });
  });

  describe('11. where it hangs', () => {
    // jsdom places nothing, so where the card lands is `footerCard.browser.test.tsx`'s. This pins
    // that every card is placed against the composer card the line hands it, not its own trigger.
    it('is placed against the composer card it hangs above', async () => {
      const user = userEvent.setup();
      await renderLine();
      const measured = vi.spyOn(screen.getByTestId('composer'), 'getBoundingClientRect');
      expect(measured).not.toHaveBeenCalled();
      await user.click(trigger(item));
      await flush();
      await waitFor(() => expect(measured).toHaveBeenCalled());
    });
  });
});

describe('7. one card at a time on the line', () => {
  // HOVER PEEKS, CLICK PINS: a hover never replaces a card the person KEPT. While one is kept, a
  // hover on another trigger opens nothing at all, however long it rests, and the kept card stays
  // kept, with its focus. A click, Enter, Space or a Tab on the other trigger still replaces it.
  it.each(PAIRS)(
    'opens nothing on a hover over the $then.name trigger while the $first.name card is kept, which stays kept',
    async ({ first, then }) => {
      vi.useFakeTimers();
      await renderLine();
      act(() => trigger(first).focus());
      fireEvent.click(trigger(first), { detail: 1 });
      await flush();
      expect(openCards()).toEqual([first.name]);
      fireEvent.pointerLeave(trigger(first), { pointerType: 'mouse' });
      fireEvent.pointerEnter(trigger(then), { pointerType: 'mouse' });
      await advance(FOOTER_CARD_OPEN_DELAY_MS * 3);
      expect(openCards(), 'nothing opened over the kept card').toEqual([first.name]);
      expect(trigger(then)).toHaveAttribute('aria-expanded', 'false');
      expect(marks(first)).toEqual(MARKED);
      expect(trigger(first)).toHaveFocus();
      // The pointer moving on from there changes nothing either.
      fireEvent.pointerLeave(trigger(then), { pointerType: 'mouse' });
      await advance(FOOTER_CARD_CLOSE_GRACE_MS * 4);
      expect(openCards()).toEqual([first.name]);
      expect(marks(first)).toEqual(MARKED);
    }
  );

  // (Enter's click claims the line, which also cancels every other card's waiting hover.)
  it.each(PAIRS)(
    'opens nothing for a hover on the $then.name trigger that was already waiting when Enter kept the $first.name card',
    async ({ first, then }) => {
      vi.useFakeTimers();
      await renderLine();
      fireEvent.pointerEnter(trigger(then), { pointerType: 'mouse' });
      await advance(FOOTER_CARD_OPEN_DELAY_MS / 2);
      // Enter on the focused $first trigger (a keyboard click, detail 0), with the pointer still
      // resting on $then and its open delay still running.
      act(() => trigger(first).focus());
      fireEvent.click(trigger(first), { detail: 0 });
      await flush();
      expect(openCards()).toEqual([first.name]);
      await advance(FOOTER_CARD_OPEN_DELAY_MS * 2);
      expect(openCards(), 'the waiting hover opened nothing').toEqual([first.name]);
      expect(marks(first)).toEqual(MARKED);
    }
  );

  // A card kept WITHOUT a claim, by focus entering it: nothing cancelled the other trigger's
  // waiting hover, so the hover itself must find the card kept when its delay runs out. Where the
  // ring ends the line, the Tab after it moves into its card.
  it('opens nothing for a hover that was waiting when focus moved into the previewed card beside it', async () => {
    const user = userEvent.setup();
    render(<Composer props={{ ...PROPS, modelCostRows: undefined }} after={false} />);
    await flush();
    const [folder, context] = ITEMS;
    act(() => trigger(folder).focus());
    await user.tab();
    expect(openCards(), 'a Tab previews the ring’s card').toEqual(['context']);
    expect(marks(context)).toEqual(UNMARKED);
    // The pointer comes to rest on the folder chip while the ring's card is only a preview.
    fireEvent.pointerEnter(trigger(folder), { pointerType: 'mouse' });
    await wait(FOOTER_CARD_OPEN_DELAY_MS / 3);
    // Then the Tab into the ring's card: focus entering it keeps it.
    await user.tab();
    const slider = context.firstControl!();
    expect(slider).toHaveFocus();
    expect(marks(context)).toEqual(MARKED);
    await wait(FOOTER_CARD_OPEN_DELAY_MS * 2);
    expect(openCards(), 'the waiting hover opened nothing').toEqual(['context']);
    expect(slider).toHaveFocus();
  });

  it.each(PAIRS)(
    'opens nothing on a hover over the $then.name trigger while the kept $first.name card holds focus, which stays',
    async ({ first, then }) => {
      const user = userEvent.setup();
      await renderLine();
      await pinFromKeyboard(user, first);
      // On its first control where it has one, else on its surface.
      if (first.firstControl) await user.tab();
      const focused = document.activeElement;
      expect(surface(first)!.contains(focused), 'focus is in the card').toBe(true);
      await user.hover(trigger(then));
      await wait(FOOTER_CARD_OPEN_DELAY_MS * 2);
      expect(openCards(), 'nothing opened over the kept card').toEqual([first.name]);
      expect(document.activeElement, 'focus stayed where the person put it').toBe(focused);
      await user.unhover(trigger(then));
      await wait(FOOTER_CARD_CLOSE_GRACE_MS * 2);
      expect(openCards()).toEqual([first.name]);
      expect(document.activeElement).toBe(focused);
      expect(marks(first)).toEqual(MARKED);
    }
  );

  /**
   * The case that made the rule: in a `biorouter serve` browser the folder card holds a field for
   * the approval key, and a pointer that rested on the ring or the cost for 300ms used to close
   * the card, take the typed key with it (it is cleared on close) and move focus off the field.
   */
  describe('while the folder card holds a typed approval key (a biorouter serve browser)', () => {
    beforeEach(() => {
      document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    });
    afterEach(() => {
      delete document.documentElement.dataset.biorouterSurface;
    });

    it.each(ITEMS.filter((item) => item !== FOLDER))(
      'opens nothing on a hover over the $name trigger, and the key and its focus stay',
      async (then) => {
        const user = userEvent.setup();
        await renderLine();
        await user.click(trigger(FOLDER));
        const key = await screen.findByLabelText(FOLDER_ACCESS_COPY.approvalKey);
        await user.click(key);
        await user.keyboard('typed-approval-key');
        expect(key).toHaveValue('typed-approval-key');
        await user.hover(trigger(then));
        await wait(FOOTER_CARD_OPEN_DELAY_MS * 2);
        expect(openCards()).toEqual(['folder']);
        expect(key).toHaveFocus();
        expect(key).toHaveValue('typed-approval-key');
        expect(marks(FOLDER)).toEqual(MARKED);
      }
    );
  });

  it.each(PAIRS)(
    'replaces the kept $first.name card when a click keeps the $then.name card',
    async ({ first, then }) => {
      const user = userEvent.setup();
      await renderLine();
      await user.click(trigger(first));
      expect(openCards()).toEqual([first.name]);
      await user.click(trigger(then));
      expect(openCards()).toEqual([then.name]);
      expect(marks(then)).toEqual(MARKED);
      expect(marks(first)).toEqual(UNMARKED);
      // The click put focus on its own trigger; no card took it.
      expect(trigger(then)).toHaveFocus();
    }
  );

  it.each(PAIRS.flatMap((pair) => ['Enter', 'Space'].map((key) => ({ ...pair, key }))))(
    'replaces the kept $first.name card when $key on the $then.name trigger keeps that one',
    async ({ first, then, key }) => {
      const user = userEvent.setup();
      await renderLine();
      await user.click(trigger(first));
      expect(marks(first)).toEqual(MARKED);
      // Focus reaching the other trigger, then the key on it.
      act(() => trigger(then).focus());
      await user.keyboard(key === 'Enter' ? '{Enter}' : ' ');
      await waitFor(() => expect(surface(then)).toHaveFocus());
      await flush();
      expect(openCards()).toEqual([then.name]);
      expect(marks(then)).toEqual(MARKED);
      expect(marks(first)).toEqual(UNMARKED);
    }
  );

  it.each(NEIGHBOURS)(
    'replaces the kept $first.name card when a Tab moves onto the $then.name trigger beside it, as a preview',
    async ({ first, then }) => {
      const user = userEvent.setup();
      await renderLine();
      await user.click(trigger(first));
      expect(marks(first)).toEqual(MARKED);
      const forward = ITEMS.indexOf(then) > ITEMS.indexOf(first);
      await user.tab({ shift: !forward });
      expect(trigger(then)).toHaveFocus();
      expect(openCards()).toEqual([then.name]);
      // A Tab previews: the card it opened is open and not kept.
      expect(marks(then)).toEqual(UNMARKED);
      expect(marks(first)).toEqual(UNMARKED);
      await flush();
    }
  );

  // A PREVIEW holds nothing, so a hover still replaces it.
  it.each(PAIRS)(
    'closes the $first.name card a Tab opened when a hover opens the $then.name card',
    async ({ first, then }) => {
      const user = userEvent.setup();
      await renderLine();
      act(() => before(first).focus());
      await user.tab();
      expect(openCards()).toEqual([first.name]);
      await user.hover(trigger(then));
      await waitFor(() => expect(isOpen(then)).toBe(true));
      expect(openCards()).toEqual([then.name]);
      expect(trigger(first)).toHaveFocus();
    }
  );

  it.each(PAIRS)(
    'closes the $first.name card a hover opened when a hover opens the $then.name card',
    async ({ first, then }) => {
      vi.useFakeTimers();
      await renderLine();
      await hoverOpen(first);
      fireEvent.pointerLeave(trigger(first), { pointerType: 'mouse' });
      await hoverOpen(then);
      expect(openCards()).toEqual([then.name]);
      expect(marks(then)).toEqual(UNMARKED);
    }
  );
});

/**
 * #50 for the mark: while the folder chip's native chooser is open the card is shut, so the chip
 * is not marked kept, and nothing a click or a hover did while the chooser was up marks it after.
 */
describe('10. the folder chip’s mark while its chooser is open (#50)', () => {
  const original = window.electron;
  afterEach(() => {
    window.electron = original;
  });

  it('drops the mark while the chooser is up, and nothing done then marks the chip after', async () => {
    let answer: ((value: { canceled: boolean; filePaths: string[] }) => void) | undefined;
    const electron = {
      platform: 'darwin',
      directoryChooser: vi.fn(
        () =>
          new Promise<{ canceled: boolean; filePaths: string[] }>((resolve) => {
            answer = resolve;
          })
      ),
      addRecentDir: vi.fn(),
      openDirectoryInExplorer: vi.fn(),
    };
    window.electron = electron as unknown as typeof window.electron;
    const user = userEvent.setup();
    // Home: the folder alone, unlocked and known, so its card offers Choose folder….
    render(<Composer props={{ ...PROPS, sessionId: null, workingDirLocked: false }} />);
    await flush();
    const chip = screen.getByTestId('dir-switcher');
    const card = () => document.querySelector<HTMLElement>('[data-testid="folder-access-card"]');
    await user.click(chip);
    expect(chip).toHaveAttribute('data-pinned', 'true');
    expect(card()).toHaveAttribute('data-pinned', 'true');
    await user.click(screen.getByRole('button', { name: FOLDER_ACCESS_COPY.chooseFolder }));
    expect(electron.directoryChooser).toHaveBeenCalledTimes(1);
    expect(chip).not.toHaveAttribute('data-pinned');
    expect(chip).toHaveAttribute('aria-expanded', 'false');
    // While it is up: a click, a hover.
    await user.click(chip);
    expect(chip).not.toHaveAttribute('data-pinned');
    await user.hover(chip);
    await wait(FOOTER_CARD_OPEN_DELAY_MS * 2);
    expect(chip).not.toHaveAttribute('data-pinned');
    expect(card()?.getAttribute('data-state') ?? 'absent').not.toBe('open');
    // Gone: still unmarked, and still shut.
    await act(async () => answer?.({ canceled: true, filePaths: [] }));
    await wait(FOOTER_CARD_OPEN_DELAY_MS * 2);
    expect(chip).not.toHaveAttribute('data-pinned');
    expect(card()?.getAttribute('data-state') ?? 'absent').not.toBe('open');
  });
});

/**
 * Rule 3 where nothing tabbable follows the line. The cards are portalled to the end of the page,
 * so on Home (the folder chip alone) and in a chat whose line ends with a readout, the Tab after
 * the last trigger is not "away": it lands in that trigger's card, which a Tab had opened UNPINNED.
 * Left unpinned, a pointer crossing the trigger closed it about 150ms later with focus inside,
 * dropping focus on the page; focus that enters a card pins it, as a press does.
 */
describe('3. a Tab past the last item on the line, into its card', () => {
  const LAST = [
    {
      name: 'folder',
      // Home: only the folder, unlocked and known, so its card offers Choose folder….
      props: { ...PROPS, sessionId: null, workingDirLocked: false },
      before: () => screen.getByRole('button', { name: 'Before the line' }),
      trigger: 'dir-switcher',
      card: 'folder-access-card',
      firstControl: () => screen.getByRole('button', { name: FOLDER_ACCESS_COPY.chooseFolder }),
    },
    {
      name: 'context',
      // A chat with no cost to show: the ring ends the line.
      props: { ...PROPS, modelCostRows: undefined },
      before: () => screen.getByTestId('dir-switcher-locked'),
      trigger: 'context-window-indicator',
      card: 'context-card',
      firstControl: () => screen.getByRole('slider', { name: CONTEXT_CARD_COPY.thresholdLabel }),
    },
    {
      name: 'cost',
      props: PROPS,
      before: () => screen.getByTestId('context-window-indicator'),
      trigger: 'chat-cost',
      card: 'cost-card',
      firstControl: null,
    },
  ];

  it.each(LAST)(
    'a Tab past the $name trigger pins the card it moves into, or leaves one with nothing to reach',
    async (last) => {
      const user = userEvent.setup();
      render(<Composer props={last.props} after={false} />);
      await flush();
      const button = screen.getByTestId(last.trigger);
      const card = () => document.querySelector<HTMLElement>(`[data-testid="${last.card}"]`);
      const open = () => card()?.getAttribute('data-state') === 'open';
      act(() => last.before().focus());
      await user.tab();
      expect(button).toHaveFocus();
      expect(open(), 'a Tab onto the trigger opens it').toBe(true);
      await flush();

      await user.tab();
      if (!last.firstControl) {
        // Nothing in the card to reach: the Tab leaves, and the card closes, as a Tab away does.
        expect(open()).toBe(false);
        expect(button).not.toHaveFocus();
        return;
      }
      expect(last.firstControl()).toHaveFocus();
      expect(open()).toBe(true);
      // The pointer crosses the trigger and leaves: a card that holds focus is pinned, so the
      // grace closes nothing and focus stays where the person put it.
      fireEvent.pointerEnter(button, { pointerType: 'mouse' });
      fireEvent.pointerLeave(button, { pointerType: 'mouse' });
      await wait(FOOTER_CARD_CLOSE_GRACE_MS * 2);
      expect(open(), 'held: it holds focus').toBe(true);
      expect(last.firstControl()).toHaveFocus();
      await user.keyboard('{Escape}');
      expect(open()).toBe(false);
      expect(button).toHaveFocus();
    }
  );
});
