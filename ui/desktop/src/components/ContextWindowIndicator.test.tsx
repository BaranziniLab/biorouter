import React, { useId } from 'react';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  CEILING_NOTE,
  ContextWindowGauge,
  ContextWindowIndicator,
  contextWindowModelKey,
} from './ContextWindowIndicator';
import { CONTEXT_CARD_COPY, FOOTER_COPY } from './bottom_menu/copy';
import {
  FooterCard,
  FooterCardGroup,
  FooterCardRoot,
  FooterCardSection,
  FooterCardTrigger,
} from './bottom_menu/FooterCard';
import {
  FOOTER_CARD_CLOSE_GRACE_MS,
  FOOTER_CARD_OPEN_DELAY_MS,
  useFooterCard,
} from './bottom_menu/useFooterCard';

// The gauge reads and writes the auto-compact threshold through ConfigContext
// (never straight to the API — see ContextWindowIndicator.configCache.test.tsx,
// which drives the real provider and asserts the cache stays consistent).
const mocks = vi.hoisted(() => ({
  read: vi.fn(),
  upsert: vi.fn(),
}));

vi.mock('./ConfigContext', () => ({
  useConfig: () => ({ read: mocks.read, upsert: mocks.upsert }),
}));

const THRESHOLD_KEY = 'BIOROUTER_AUTO_COMPACT_THRESHOLD';

beforeAll(() => {
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
  );
});

afterAll(() => {
  vi.unstubAllGlobals();
});

afterEach(() => {
  vi.useRealTimers();
});

const ringButton = () => screen.getByTestId('context-window-indicator');
const contextCard = () => document.querySelector<HTMLElement>('[data-testid="context-card"]');
const cardOpen = () => contextCard()?.getAttribute('data-state') === 'open';

function hoverOpenRing() {
  fireEvent.pointerEnter(ringButton(), { pointerType: 'mouse' });
  act(() => {
    vi.advanceTimersByTime(FOOTER_CARD_OPEN_DELAY_MS + 10);
  });
}

/**
 * The card's body (`ContextWindowGauge`): ONE "Context window" section in the footer cards' shared
 * format, with what is left as the reading, what is used as the detail, the bar and its marker, the
 * auto-compact sentence and Compact chat. No inline "Context" label beside the bar, no tooltip on
 * the marker and no icon-only Compact any more.
 */
describe('the context card’s body', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.read.mockResolvedValue(null);
    mocks.upsert.mockResolvedValue(undefined);
  });

  const gauge = (props: Partial<React.ComponentProps<typeof ContextWindowGauge>> = {}) => (
    <ContextWindowGauge
      totalTokens={92_000}
      tokenLimit={128_000}
      isTokenLimitLoaded
      onCompact={vi.fn()}
      {...props}
    />
  );

  it('reads what is left, then what is used, then the bar, the note and Compact, in that order', () => {
    const { container } = render(gauge());
    const section = container.querySelector('section.br-footer-card__section')!;
    expect(section).not.toBeNull();
    const lines = [...section.querySelectorAll(':scope > p')].map((p) => [
      p.className,
      p.textContent,
    ]);
    expect(lines).toEqual([
      ['br-footer-card__label', 'Context window'],
      ['br-footer-card__value', '28% left'],
      ['br-footer-card__detail', '92k of 128k tokens used'],
      [
        'br-footer-card__note',
        'Biorouter compacts this chat automatically at 80% used. Drag the marker to change it.',
      ],
    ]);
    expect(CONTEXT_CARD_COPY.left(28)).toBe('28% left');
    expect(CONTEXT_CARD_COPY.used('92k', '128k')).toBe('92k of 128k tokens used');

    const bar = screen.getByRole('progressbar', { name: CONTEXT_CARD_COPY.barLabel });
    expect(bar).toHaveAttribute('aria-valuenow', '72');
    const order = [
      section.querySelector('.br-footer-card__detail')!,
      bar,
      section.querySelector('.br-footer-card__note')!,
      screen.getByRole('button', { name: CONTEXT_CARD_COPY.compact }),
    ];
    for (let i = 1; i < order.length; i++) {
      expect(order[i - 1].compareDocumentPosition(order[i])).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    }
    // No inline label beside the bar: the section's own label names it.
    expect(screen.queryByText('Context')).toBeNull();
  });

  it('gives what is left the same rounding as the ring, so the two never disagree', () => {
    // 12.5% used and 87.5% left both round up: "100 minus what is used" would read
    // "87% left" beside a ring named "88% context left".
    render(
      <ContextWindowIndicator
        totalTokens={1_000}
        tokenLimit={8_000}
        isTokenLimitLoaded
        onCompact={vi.fn()}
      />
    );
    const ring = screen.getByRole('button', { name: '88% context left · 1k of 8k' });
    fireEvent.click(ring, { detail: 1 });
    const card = screen.getByRole('dialog', { name: 'Context' });
    expect(within(card).getByText('88% left')).toBeInTheDocument();
    expect(within(card).getByRole('progressbar')).toHaveAttribute('aria-valuenow', '13');
  });

  it('names the marker as a slider whose value is said in words, with no hover tooltip', async () => {
    const user = userEvent.setup();
    mocks.read.mockResolvedValue(0.6);
    render(gauge());
    const slider = screen.getByRole('slider', { name: CONTEXT_CARD_COPY.thresholdLabel });
    await waitFor(() => expect(slider).toHaveAttribute('aria-valuenow', '60'));
    expect(slider).toHaveAttribute('aria-valuetext', 'Auto-compact at 60%');
    expect(slider).toHaveAttribute('aria-valuemin', '20');
    expect(slider).toHaveAttribute('aria-valuemax', '90');
    expect(slider).toHaveClass('br-context-gauge__marker');
    expect(slider.style.left).toBe('60%');
    // The note follows the live threshold.
    expect(screen.getByText(CONTEXT_CARD_COPY.autoCompactNote(60))).toBeInTheDocument();

    await user.hover(slider);
    await act(() => new Promise((resolve) => setTimeout(resolve, 700)));
    expect(screen.queryByRole('tooltip')).toBeNull();
    expect(document.querySelector('[data-slot="tooltip-content"]')).toBeNull();
  });

  // The daemon reads the saved value itself (`check_if_compaction_needed`): at or outside 0 and 1
  // it never compacts, and inside them it compacts at that share even where the marker, which only
  // reaches 20 to 90%, cannot sit. The card says what will happen, not where the marker is.
  it.each([0, 1, -0.2, 1.5])(
    'says automatic compaction is off for a saved %s, and the marker turns it on',
    async (saved) => {
      const user = userEvent.setup();
      // Only the threshold is saved; every other key reads as unset.
      mocks.read.mockImplementation(async (key: string) => (key === THRESHOLD_KEY ? saved : null));
      render(gauge());
      expect(await screen.findByText(CONTEXT_CARD_COPY.autoCompactOff)).toHaveClass(
        'br-footer-card__note'
      );
      expect(screen.queryByText(/compacts this chat automatically/)).toBeNull();
      const slider = screen.getByRole('slider', { name: CONTEXT_CARD_COPY.thresholdLabel });
      expect(slider).toHaveAttribute('aria-valuetext', CONTEXT_CARD_COPY.thresholdOff);
      // The marker waits at the default, where a change starts from.
      expect(slider).toHaveAttribute('aria-valuenow', '80');
      act(() => slider.focus());
      await user.keyboard('{ArrowRight}');
      expect(mocks.upsert).toHaveBeenCalledWith(THRESHOLD_KEY, 0.81, false);
      expect(screen.getByText(CONTEXT_CARD_COPY.autoCompactNote(81))).toBeInTheDocument();
      expect(slider).toHaveAttribute('aria-valuetext', 'Auto-compact at 81%');
    }
  );

  it.each([
    [0.95, 95, 90],
    [0.1, 10, 20],
    [0.9, 90, 90],
    [0.2, 20, 20],
  ])(
    'says a saved %s as %i percent, with the marker at the nearest it reaches, %i',
    async (saved, said, marker) => {
      // Only the threshold is saved; every other key reads as unset.
      mocks.read.mockImplementation(async (key: string) => (key === THRESHOLD_KEY ? saved : null));
      render(gauge());
      expect(await screen.findByText(CONTEXT_CARD_COPY.autoCompactNote(said))).toBeInTheDocument();
      const slider = screen.getByRole('slider', { name: CONTEXT_CARD_COPY.thresholdLabel });
      expect(slider).toHaveAttribute('aria-valuenow', String(marker));
      expect(slider).toHaveAttribute('aria-valuetext', `Auto-compact at ${said}%`);
    }
  );

  it.each([
    ['{ArrowRight}', 0.81],
    ['{ArrowUp}', 0.81],
    ['{ArrowLeft}', 0.79],
    ['{Shift>}{ArrowLeft}{/Shift}', 0.7],
    ['{Shift>}{ArrowRight}{/Shift}', 0.9],
    ['{Home}', 0.2],
    ['{End}', 0.9],
  ])('persists %s on the marker through the config context', async (keys, saved) => {
    const user = userEvent.setup();
    render(gauge());
    const slider = screen.getByRole('slider');
    act(() => slider.focus());
    await user.keyboard(keys);
    expect(mocks.upsert).toHaveBeenCalledTimes(1);
    expect(mocks.upsert).toHaveBeenCalledWith(THRESHOLD_KEY, saved, false);
    const pct = Math.round(saved * 100);
    expect(slider).toHaveAttribute('aria-valuetext', `Auto-compact at ${pct}%`);
    expect(screen.getByText(CONTEXT_CARD_COPY.autoCompactNote(pct))).toBeInTheDocument();
  });

  it('clamps the marker to 90% at the top and 20% at the bottom', async () => {
    const user = userEvent.setup();
    render(gauge());
    const slider = screen.getByRole('slider');
    act(() => slider.focus());
    await user.keyboard('{End}{ArrowRight}');
    expect(slider).toHaveAttribute('aria-valuenow', '90');
    await user.keyboard('{Home}{ArrowLeft}');
    expect(slider).toHaveAttribute('aria-valuenow', '20');
  });

  it('drags the marker live and saves once, on release', () => {
    const rect = vi
      .spyOn(Element.prototype, 'getBoundingClientRect')
      .mockReturnValue({ left: 0, top: 0, width: 200, height: 8 } as DOMRect);
    try {
      render(gauge());
      const bar = screen.getByRole('progressbar');
      const slider = screen.getByRole('slider');
      fireEvent.mouseDown(bar, { clientX: 100 });
      expect(slider).toHaveAttribute('aria-valuenow', '50');
      fireEvent.mouseMove(window, { clientX: 120 });
      expect(slider).toHaveAttribute('aria-valuenow', '60');
      expect(screen.getByText(CONTEXT_CARD_COPY.autoCompactNote(60))).toBeInTheDocument();
      // Past the end of the bar: clamped.
      fireEvent.mouseMove(window, { clientX: 400 });
      expect(slider).toHaveAttribute('aria-valuenow', '90');
      expect(mocks.upsert).not.toHaveBeenCalled();
      fireEvent.mouseUp(window, { clientX: 400 });
      expect(mocks.upsert).toHaveBeenCalledTimes(1);
      expect(mocks.upsert).toHaveBeenCalledWith(THRESHOLD_KEY, 0.9, false);
    } finally {
      rect.mockRestore();
    }
  });

  it('runs compaction from a text button, the footer cards’ one action', async () => {
    const user = userEvent.setup();
    const onCompact = vi.fn();
    render(gauge({ totalTokens: 24_000, tokenLimit: 1_100_000, onCompact }));
    const button = screen.getByRole('button', { name: 'Compact chat' });
    expect(button).toHaveTextContent('Compact chat');
    expect(button).toBeEnabled();
    expect(button.closest('.br-footer-card__actions')).not.toBeNull();
    expect(button.querySelector('svg')).toBeNull();
    expect(screen.queryByText(CONTEXT_CARD_COPY.nothingToCompact)).toBeNull();
    await user.click(button);
    expect(onCompact).toHaveBeenCalledTimes(1);
  });

  it('disables Compact while there is nothing to compact, and says so under it', () => {
    render(gauge({ totalTokens: 0, tokenLimit: 1_100_000 }));
    const button = screen.getByRole('button', { name: 'Compact chat' });
    expect(button).toBeDisabled();
    expect(button).not.toHaveAttribute('title');
    const note = screen.getByText('Nothing to compact yet.');
    expect(note).toHaveClass('br-footer-card__note');
    expect(button).toHaveAccessibleDescription('Nothing to compact yet.');
    expect(button.closest('.br-footer-card__actions')!.nextElementSibling).toBe(note);
  });
});

/**
 * The ring follows the footer line's one contract (`useFooterCard`, `FooterCardTrigger`): the same
 * button, hover, focus, click and close rules as the folder chip and the cost, and ONE card in
 * place of the old dark tooltip and the separate click popover.
 */
describe('the ring and its card', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.read.mockResolvedValue(null);
    mocks.upsert.mockResolvedValue(undefined);
  });

  const ring = (props: Partial<React.ComponentProps<typeof ContextWindowIndicator>> = {}) => (
    <ContextWindowIndicator
      sessionId="chat-a"
      totalTokens={92_000}
      tokenLimit={128_000}
      isTokenLimitLoaded
      onCompact={vi.fn()}
      {...props}
    />
  );

  it('is the line’s trigger: a named button that says whether its card is open', () => {
    render(ring());
    const button = ringButton();
    const line = FOOTER_COPY.contextTooltip(28, '92k', '128k');
    expect(line).toBe('28% context left · 92k of 128k');
    expect(button).toHaveAccessibleName(line);
    expect(button.tagName).toBe('BUTTON');
    expect(button).toHaveAttribute('type', 'button');
    expect(button).toHaveClass('br-footline__item');
    expect(button).toHaveAttribute('aria-haspopup', 'dialog');
    expect(button).toHaveAttribute('aria-expanded', 'false');
    expect(button).not.toHaveAttribute('aria-controls');
  });

  it('opens the context card on a hover after the delay, with no tooltip', () => {
    vi.useFakeTimers();
    render(ring());
    fireEvent.pointerEnter(ringButton(), { pointerType: 'mouse' });
    act(() => {
      vi.advanceTimersByTime(FOOTER_CARD_OPEN_DELAY_MS - 10);
    });
    expect(cardOpen()).toBe(false);
    act(() => {
      vi.advanceTimersByTime(20);
    });
    expect(cardOpen()).toBe(true);
    const card = screen.getByRole('dialog', { name: 'Context' });
    expect(card).toBe(contextCard());
    expect(card).toHaveClass('br-footer-card');
    expect(ringButton()).toHaveAttribute('aria-expanded', 'true');
    expect(ringButton()).toHaveAttribute('aria-controls', card.id);
    const body = within(card);
    expect(body.getByText('Context window')).toHaveClass('br-footer-card__label');
    expect(body.getByText('28% left')).toHaveClass('br-footer-card__value');
    expect(body.getByText('92k of 128k tokens used')).toHaveClass('br-footer-card__detail');
    expect(
      body.getByText(
        'Biorouter compacts this chat automatically at 80% used. Drag the marker to change it.'
      )
    ).toHaveClass('br-footer-card__note');
    expect(body.getByRole('progressbar', { name: 'Context window used' })).toBeInTheDocument();
    expect(body.getByRole('slider', { name: 'Auto-compact threshold' })).toHaveAttribute(
      'aria-valuetext',
      'Auto-compact at 80%'
    );
    expect(body.getByRole('button', { name: 'Compact chat' })).toBeEnabled();
    // The dark one-line tooltip is gone: the card replaced it.
    expect(screen.queryByRole('tooltip')).toBeNull();
    expect(document.querySelector('[data-slot="tooltip-content"]')).toBeNull();
  });

  it('never opens on a touch hover', () => {
    vi.useFakeTimers();
    render(ring());
    fireEvent.pointerEnter(ringButton(), { pointerType: 'touch' });
    act(() => {
      vi.advanceTimersByTime(FOOTER_CARD_OPEN_DELAY_MS * 3);
    });
    expect(cardOpen()).toBe(false);
  });

  it('closes a hover-opened card once the pointer leaves, after the grace', () => {
    vi.useFakeTimers();
    render(ring());
    hoverOpenRing();
    fireEvent.pointerLeave(ringButton(), { pointerType: 'mouse' });
    act(() => {
      vi.advanceTimersByTime(FOOTER_CARD_CLOSE_GRACE_MS + 10);
    });
    expect(cardOpen()).toBe(false);
  });

  it('pins the card on a click, holds it while the pointer wanders off, and a second click closes it', async () => {
    const user = userEvent.setup();
    render(ring());
    await user.click(ringButton());
    expect(cardOpen()).toBe(true);
    // `pickers.css` styles the open item on `aria-expanded`.
    expect(ringButton()).toHaveAttribute('aria-expanded', 'true');
    await user.unhover(ringButton());
    await act(() => new Promise((resolve) => setTimeout(resolve, FOOTER_CARD_CLOSE_GRACE_MS * 2)));
    expect(cardOpen()).toBe(true);
    // A click does not move focus into the card.
    expect(ringButton()).toHaveFocus();
    await user.click(ringButton());
    expect(cardOpen()).toBe(false);
    expect(ringButton()).toHaveAttribute('aria-expanded', 'false');
  });

  it('opens unpinned on a Tab onto the ring and closes on a Tab away', async () => {
    const user = userEvent.setup();
    render(
      <>
        <button type="button">Before</button>
        {ring()}
        <button type="button">After</button>
      </>
    );
    act(() => screen.getByRole('button', { name: 'Before' }).focus());
    await user.tab();
    expect(ringButton()).toHaveFocus();
    expect(cardOpen()).toBe(true);
    await user.tab();
    expect(screen.getByRole('button', { name: 'After' })).toHaveFocus();
    expect(cardOpen()).toBe(false);
  });

  it('pins on Enter and moves focus onto the card, whose first control is the marker', async () => {
    const user = userEvent.setup();
    render(ring());
    act(() => ringButton().focus());
    await user.keyboard('{Enter}');
    await waitFor(() => expect(contextCard()).toHaveFocus());
    expect(document.activeElement).toBe(screen.getByRole('dialog', { name: 'Context' }));
    await user.tab();
    expect(screen.getByRole('slider')).toHaveFocus();
    await user.tab();
    expect(screen.getByRole('button', { name: 'Compact chat' })).toHaveFocus();
    expect(cardOpen()).toBe(true);
  });

  it('closes on Escape and hands focus back to the ring', async () => {
    const user = userEvent.setup();
    render(ring());
    act(() => ringButton().focus());
    await user.keyboard('{Enter}');
    await waitFor(() => expect(contextCard()).toHaveFocus());
    await user.tab();
    await user.keyboard('{Escape}');
    expect(cardOpen()).toBe(false);
    expect(ringButton()).toHaveFocus();
  });

  it('runs Compact from the card, then closes it', async () => {
    const user = userEvent.setup();
    const onCompact = vi.fn();
    render(ring({ onCompact }));
    await user.click(ringButton());
    await user.click(screen.getByRole('button', { name: 'Compact chat' }));
    expect(onCompact).toHaveBeenCalledTimes(1);
    expect(cardOpen()).toBe(false);
    expect(ringButton()).toHaveAttribute('aria-expanded', 'false');
  });

  it('keeps a card the person is dragging the marker in, and saves the threshold on release', () => {
    vi.useFakeTimers();
    const rect = vi
      .spyOn(Element.prototype, 'getBoundingClientRect')
      .mockReturnValue({ left: 0, top: 0, width: 200, height: 8 } as DOMRect);
    try {
      render(ring());
      hoverOpenRing();
      const slider = screen.getByRole('slider');
      // A press in the card pins it, so the pointer leaving mid-drag closes nothing.
      fireEvent.pointerDown(slider);
      fireEvent.mouseDown(slider, { clientX: 140 });
      fireEvent.pointerLeave(contextCard()!, { pointerType: 'mouse' });
      fireEvent.mouseMove(window, { clientX: 60 });
      act(() => {
        vi.advanceTimersByTime(FOOTER_CARD_CLOSE_GRACE_MS * 4);
      });
      expect(cardOpen()).toBe(true);
      expect(slider).toHaveAttribute('aria-valuetext', 'Auto-compact at 30%');
      fireEvent.mouseUp(window);
      fireEvent.pointerUp(slider);
      expect(mocks.upsert).toHaveBeenCalledTimes(1);
      expect(mocks.upsert).toHaveBeenCalledWith(THRESHOLD_KEY, 0.3, false);
    } finally {
      rect.mockRestore();
    }
  });

  it('saves a marker key from inside the opened card', async () => {
    const user = userEvent.setup();
    render(ring());
    act(() => ringButton().focus());
    await user.keyboard('{Enter}');
    await waitFor(() => expect(contextCard()).toHaveFocus());
    await user.tab();
    await user.keyboard('{ArrowLeft}');
    expect(mocks.upsert).toHaveBeenCalledWith(THRESHOLD_KEY, 0.79, false);
    expect(screen.getByRole('slider')).toHaveAttribute('aria-valuetext', 'Auto-compact at 79%');
  });

  it('shuts its card when the ring stops being drawn, and does not reopen it when the ring returns', async () => {
    const user = userEvent.setup();
    const { rerender, container } = render(ring());
    await user.click(ringButton());
    expect(cardOpen()).toBe(true);
    // The model changed: no window for it yet, so no ring.
    rerender(ring({ isTokenLimitLoaded: false, sessionId: 'chat-b' }));
    expect(container).toBeEmptyDOMElement();
    expect(contextCard()).toBeNull();
    rerender(ring({ sessionId: 'chat-b', tokenLimit: 200_000 }));
    expect(ringButton()).toHaveAttribute('aria-expanded', 'false');
    expect(contextCard()).toBeNull();
  });

  it('draws the ring at the chip size with a 2px stroke, and no mono', () => {
    render(ring({ totalTokens: 10_000, showRemainingPercent: true }));
    const button = ringButton();
    const circles = button.querySelectorAll('circle');
    expect(circles).toHaveLength(2);
    circles.forEach((circle) => expect(circle).toHaveAttribute('stroke-width', '2'));
    expect(button.querySelector('.font-mono')).toBeNull();
  });

  it.each([
    [10_000, false],
    [64_000, false],
    [65_000, true],
    [120_000, true],
  ])('with %i of 128k used, prints the figure: %s', (used, shown) => {
    render(ring({ totalTokens: used, showRemainingPercent: true }));
    const remaining = Math.round(((128_000 - used) / 128_000) * 100);
    const figure = screen.queryByText(FOOTER_COPY.contextLeft(remaining));
    if (shown) expect(figure).toBeInTheDocument();
    else expect(figure).toBeNull();
  });

  it('never prints the figure unless asked to', () => {
    render(ring({ totalTokens: 120_000 }));
    expect(screen.queryByText(/% left$/)).toBeNull();
  });
});

/** A stand-in for the line's other items: the same trigger and surface, another card. */
function OtherItem() {
  const card = useFooterCard();
  const cardId = useId();
  return (
    <FooterCardRoot card={card}>
      <FooterCardTrigger card={card} cardId={cardId} aria-label="Other item" data-testid="other">
        Other
      </FooterCardTrigger>
      <FooterCard card={card} id={cardId} label="Other card" align="end" testId="other-card">
        <FooterCardSection label="Other">
          <p>Other value</p>
        </FooterCardSection>
      </FooterCard>
    </FooterCardRoot>
  );
}

describe('one card at a time on the footer line', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.read.mockResolvedValue(null);
    mocks.upsert.mockResolvedValue(undefined);
  });

  const otherOpen = () =>
    document.querySelector('[data-testid="other-card"]')?.getAttribute('data-state') === 'open';

  const line = (
    <FooterCardGroup>
      <OtherItem />
      <ContextWindowIndicator
        sessionId="chat-a"
        totalTokens={92_000}
        tokenLimit={128_000}
        isTokenLimitLoaded
        onCompact={vi.fn()}
      />
    </FooterCardGroup>
  );

  // Hover peeks, click pins: a hover never replaces a card a click kept (footerCards.contract
  // rule 7), and still replaces a card that is only a preview.
  it('opens nothing on a hover over the ring while another item’s card is kept, which keeps it and its focus', () => {
    vi.useFakeTimers();
    render(line);
    act(() => screen.getByTestId('other').focus());
    fireEvent.click(screen.getByTestId('other'), { detail: 1 });
    expect(otherOpen()).toBe(true);
    hoverOpenRing();
    expect(cardOpen()).toBe(false);
    expect(otherOpen()).toBe(true);
    expect(screen.getByTestId('other')).toHaveFocus();
  });

  it('closes another item’s preview when the ring’s card opens on a hover, without moving focus', () => {
    vi.useFakeTimers();
    render(line);
    act(() => screen.getByTestId('other').focus());
    fireEvent.pointerEnter(screen.getByTestId('other'), { pointerType: 'mouse' });
    act(() => {
      vi.advanceTimersByTime(FOOTER_CARD_OPEN_DELAY_MS + 10);
    });
    expect(otherOpen()).toBe(true);
    fireEvent.pointerLeave(screen.getByTestId('other'), { pointerType: 'mouse' });
    hoverOpenRing();
    expect(cardOpen()).toBe(true);
    expect(otherOpen()).toBe(false);
    expect(screen.getByTestId('other')).toHaveFocus();
  });

  it('closes the ring’s card when another item’s card is pinned', async () => {
    const user = userEvent.setup();
    render(line);
    await user.click(ringButton());
    expect(cardOpen()).toBe(true);
    await user.click(screen.getByTestId('other'));
    expect(otherOpen()).toBe(true);
    expect(cardOpen()).toBe(false);
  });
});

/**
 * F10. A context window is a property of a bound model; with no model there is
 * no window, and every figure this component could print would be about a model
 * that does not exist. Measured in onboarding: the composer's chip correctly
 * read "No model yet — choose a provider" while the gauge beside it read
 * "Context window usage. 128k of 128k tokens remaining".
 */
describe('with no model bound', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.read.mockResolvedValue(null);
    mocks.upsert.mockResolvedValue(undefined);
  });

  it('renders nothing rather than a window for a model that does not exist', () => {
    const { container } = render(
      <ContextWindowIndicator
        totalTokens={0}
        tokenLimit={0}
        isTokenLimitLoaded
        onCompact={vi.fn()}
      />
    );
    expect(container).toBeEmptyDOMElement();
    expect(screen.queryByText(/context left/)).toBeNull();
  });

  it('renders nothing in the card’s gauge either', () => {
    const { container } = render(
      <ContextWindowGauge totalTokens={0} tokenLimit={0} isTokenLimitLoaded onCompact={vi.fn()} />
    );
    expect(container).toBeEmptyDOMElement();
  });

  /**
   * The guard is on the LIMIT, not only on the loaded flag: a lookup that
   * finished and found nothing is exactly the case, and usage without a window
   * is a pair nothing can render honestly either.
   */
  it('stays hidden even when tokens have been counted', () => {
    const { container } = render(
      <ContextWindowIndicator
        totalTokens={1766}
        tokenLimit={0}
        isTokenLimitLoaded
        onCompact={vi.fn()}
      />
    );
    expect(container).toBeEmptyDOMElement();
  });

  it('still renders once a real window is known', () => {
    render(
      <ContextWindowIndicator
        totalTokens={0}
        tokenLimit={128_000}
        isTokenLimitLoaded
        onCompact={vi.fn()}
      />
    );
    expect(
      screen.getByRole('button', { name: '100% context left · 0 of 128k' })
    ).toBeInTheDocument();
  });
});

/**
 * V2-R3-5. The arc sweeps (250ms) only for a live update in the same chat. A
 * reading that arrives with the chat (the count and the window land about a
 * second after the composer mounts on an open or a tab switch), a change of
 * chat, and the window re-read after a model change all land at rest: the render
 * that moves the arc carries `data-motion-still`, which `pickers.css` turns into
 * a 0ms transition. Chromium proves the transition itself in
 * `bottom_menu/contextRingMotion.browser.test.tsx`.
 */
describe('the ring arc moves only for a live update in the same chat (V2-R3-5)', () => {
  const ring = (props: Partial<React.ComponentProps<typeof ContextWindowIndicator>>) => (
    <ContextWindowIndicator
      sessionId="chat-a"
      totalTokens={10_000}
      tokenLimit={128_000}
      isTokenLimitLoaded
      onCompact={vi.fn()}
      {...props}
    />
  );
  const arc = () => screen.getByTestId('context-ring-arc');

  it('draws its first reading at rest', () => {
    render(ring({}));
    expect(arc()).toHaveClass('br-footline__ring-arc');
    // A new element has nothing to transition from; no flag needed.
    expect(arc()).not.toHaveAttribute('data-motion-still');
  });

  it('lands the token count that arrives after an open at rest', () => {
    const { rerender } = render(ring({ totalTokens: undefined }));
    rerender(ring({ totalTokens: 92_000 }));
    expect(arc()).toHaveAttribute('data-motion-still');
  });

  it('lands the window that arrives after the count at rest', () => {
    const { rerender } = render(ring({ totalTokens: 92_000, isTokenLimitLoaded: false }));
    // Nothing is drawn against the fallback while the window loads (V2-R4-1).
    expect(screen.queryByTestId('context-ring-arc')).toBeNull();
    rerender(ring({ totalTokens: 92_000, tokenLimit: 400_000, isTokenLimitLoaded: true }));
    expect(arc()).toHaveAttribute('data-motion-still');
  });

  it('lands a re-read window at rest after holding the last one', () => {
    const { rerender } = render(ring({ totalTokens: 92_000, tokenLimit: 1_000_000 }));
    rerender(ring({ totalTokens: 92_000, tokenLimit: 1_000_000, isTokenLimitLoaded: false }));
    rerender(ring({ totalTokens: 92_000, tokenLimit: 200_000, isTokenLimitLoaded: true }));
    expect(arc()).toHaveAttribute('data-motion-still');
  });

  it('lands another chat’s reading at rest when the composer changes chat', () => {
    const { rerender } = render(ring({ totalTokens: 10_000 }));
    rerender(ring({ sessionId: 'chat-b', totalTokens: 92_000 }));
    expect(arc()).toHaveAttribute('data-motion-still');
  });

  it('sweeps for a live update in the same chat', () => {
    const { rerender } = render(ring({ totalTokens: 10_000 }));
    rerender(ring({ totalTokens: 30_000 }));
    expect(arc()).not.toHaveAttribute('data-motion-still');
  });

  it('releases the flag after the still frame, so the next live update sweeps', async () => {
    const { rerender } = render(ring({ totalTokens: undefined }));
    rerender(ring({ totalTokens: 92_000 }));
    expect(arc()).toHaveAttribute('data-motion-still');
    await waitFor(() => expect(arc()).not.toHaveAttribute('data-motion-still'));
    rerender(ring({ totalTokens: 100_000 }));
    expect(arc()).not.toHaveAttribute('data-motion-still');
  });
});

/**
 * V2-R4-1. The composer mounts with a 128k fallback in `tokenLimit` and
 * `isTokenLimitLoaded` false, and the chat's real window lands about a second
 * later. Measured on a tab switch: a 1M chat 10% full read "20% context left ·
 * 102.4k of 128k", and a 1M chat 18% full drew the empty ring and "0% left" for
 * 700ms. No ring and no figure until a window has loaded for the chat; a re-read
 * in the same chat holds the last loaded window instead of blinking out.
 */
describe('the ring draws only against a loaded window (V2-R4-1)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.read.mockResolvedValue(null);
    mocks.upsert.mockResolvedValue(undefined);
  });

  const ring = (props: Partial<React.ComponentProps<typeof ContextWindowIndicator>>) => (
    <ContextWindowIndicator
      sessionId="chat-a"
      totalTokens={180_000}
      tokenLimit={128_000}
      isTokenLimitLoaded={false}
      onCompact={vi.fn()}
      showRemainingPercent
      {...props}
    />
  );

  /** The reading the ring announces, or none when nothing is drawn. */
  const reading = () =>
    screen.queryByTestId('context-window-indicator')?.getAttribute('aria-label') ?? null;

  it('draws no ring and no figure while the window loads, then one reading', () => {
    const { container, rerender } = render(ring({}));
    // 180k of the 128k fallback would read "0% left" beside an empty ring.
    expect(container).toBeEmptyDOMElement();
    expect(screen.queryByText(/% left$/)).toBeNull();

    rerender(ring({ tokenLimit: 1_000_000, isTokenLimitLoaded: true }));
    expect(reading()).toBe(FOOTER_COPY.contextTooltip(82, '180k', '1M'));
    // 82% left is above the figure's threshold: the ring says it alone.
    expect(screen.queryByText(/% left$/)).toBeNull();
    const circumference = 2 * Math.PI * 6;
    const offset = Number(screen.getByTestId('context-ring-arc').getAttribute('stroke-dashoffset'));
    expect(offset).toBeCloseTo(circumference * 0.18, 6);
  });

  it('draws nothing while the window loads, also where the figure would print', () => {
    // 100k of the fallback is 22% left and would print "22% left".
    const { container } = render(ring({ totalTokens: 100_000 }));
    expect(container).toBeEmptyDOMElement();
  });

  it('holds the last loaded window while the same chat re-reads it', () => {
    const { rerender } = render(ring({ tokenLimit: 1_000_000, isTokenLimitLoaded: true }));
    const loaded = FOOTER_COPY.contextTooltip(82, '180k', '1M');
    expect(reading()).toBe(loaded);
    // The model changed: the composer marks the window unloaded and keeps the old
    // value until the lookup lands. The ring keeps its reading, no blink.
    rerender(ring({ tokenLimit: 1_000_000, isTokenLimitLoaded: false }));
    expect(reading()).toBe(loaded);
    // Even if the composer put its fallback back while loading, the ring holds the
    // window it loaded.
    rerender(ring({ tokenLimit: 128_000, isTokenLimitLoaded: false }));
    expect(reading()).toBe(loaded);
    rerender(ring({ tokenLimit: 200_000, isTokenLimitLoaded: true }));
    expect(reading()).toBe(FOOTER_COPY.contextTooltip(10, '180k', '200k'));
    expect(screen.getByText(FOOTER_COPY.contextLeft(10))).toBeInTheDocument();
  });

  it('holds nothing across a change of chat', () => {
    const { container, rerender } = render(
      ring({ tokenLimit: 1_000_000, isTokenLimitLoaded: true })
    );
    rerender(ring({ sessionId: 'chat-b', tokenLimit: 1_000_000, isTokenLimitLoaded: false }));
    expect(container).toBeEmptyDOMElement();
  });

  it('holds nothing when the re-read finds no model', () => {
    const { container, rerender } = render(
      ring({ tokenLimit: 1_000_000, isTokenLimitLoaded: true })
    );
    rerender(ring({ tokenLimit: 0, isTokenLimitLoaded: false }));
    expect(container).toBeEmptyDOMElement();
  });

  it('the gauge alone draws nothing against a window that has not loaded', () => {
    const { container } = render(
      <ContextWindowGauge
        totalTokens={180_000}
        tokenLimit={128_000}
        isTokenLimitLoaded={false}
        onCompact={vi.fn()}
      />
    );
    expect(container).toBeEmptyDOMElement();
  });
});

/**
 * V2-R4-1, round 5. A chat opened for the first time (a tab restored after a
 * reload, a hash or History open) loads for up to seconds, and until its row
 * lands the composer has no binding: it looks up the window of the model for NEW
 * chats, and that lookup is loaded. Measured on a 200k chat: "0 of 1.1M", then a
 * sweep to "64.6k of 1.1M", then "of 200k" five seconds later. The ring now draws
 * a window only when it was looked up for the model the chat is bound to
 * (`tokenLimitModel === boundModel`), and nothing while that model is unknown.
 */
describe('the ring draws only a window looked up for the chat’s own model (V2-R4-1, round 5)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.read.mockResolvedValue(null);
    mocks.upsert.mockResolvedValue(undefined);
  });

  const NEW_CHAT_MODEL = 'versa_azure/gpt-5.5';
  const CHAT_MODEL = 'anthropic/claude-opus-4-5';

  const ring = (props: Partial<React.ComponentProps<typeof ContextWindowIndicator>>) => (
    <ContextWindowIndicator
      sessionId="demo2"
      totalTokens={0}
      tokenLimit={1_050_000}
      isTokenLimitLoaded
      tokenLimitModel={NEW_CHAT_MODEL}
      boundModel={null}
      onCompact={vi.fn()}
      showRemainingPercent
      {...props}
    />
  );
  const reading = () =>
    screen.queryByTestId('context-window-indicator')?.getAttribute('aria-label') ?? null;

  it('draws nothing while the chat loads, then one reading at rest for its own model', () => {
    // Loading: the binding is unknown and the window loaded is the new-chat model's.
    const { container, rerender } = render(ring({}));
    expect(container).toBeEmptyDOMElement();

    // The row lands: the chat's tokens and its binding arrive in one render, while
    // the composer still holds the new-chat model's window (its re-read starts in
    // an effect after this render).
    rerender(ring({ totalTokens: 64_600, boundModel: CHAT_MODEL }));
    expect(container).toBeEmptyDOMElement();
    // The re-read starts: unloaded, the old value still in place.
    rerender(ring({ totalTokens: 64_600, boundModel: CHAT_MODEL, isTokenLimitLoaded: false }));
    expect(container).toBeEmptyDOMElement();

    // The chat's own window lands: one reading, at rest.
    rerender(
      ring({
        totalTokens: 64_600,
        boundModel: CHAT_MODEL,
        tokenLimitModel: CHAT_MODEL,
        tokenLimit: 200_000,
      })
    );
    expect(reading()).toBe(FOOTER_COPY.contextTooltip(68, '64.6k', '200k'));
    expect(screen.getByTestId('context-ring-arc')).toHaveAttribute('data-motion-still');
  });

  it('draws the loaded window at once when the chat turns out to use the new-chat model', () => {
    const { container, rerender } = render(ring({}));
    expect(container).toBeEmptyDOMElement();
    rerender(ring({ totalTokens: 110_100, boundModel: NEW_CHAT_MODEL }));
    expect(reading()).toBe(FOOTER_COPY.contextTooltip(90, '110.1k', '1.1M'));
    // The first reading after the load, so nothing to sweep from.
    expect(screen.getByTestId('context-ring-arc')).toHaveAttribute('data-motion-still');
  });

  it('holds the window across a re-read of the same model, and not across a model change', () => {
    const loaded = { totalTokens: 64_600, boundModel: CHAT_MODEL, tokenLimitModel: CHAT_MODEL };
    const { container, rerender } = render(ring({ ...loaded, tokenLimit: 200_000 }));
    const reading200k = FOOTER_COPY.contextTooltip(68, '64.6k', '200k');
    expect(reading()).toBe(reading200k);

    // The app's selection moved under this pinned chat: the lookup re-runs for the
    // same model. The ring keeps its reading, no blink.
    rerender(ring({ ...loaded, tokenLimit: 200_000, isTokenLimitLoaded: false }));
    expect(reading()).toBe(reading200k);
    rerender(ring({ ...loaded, tokenLimit: 200_000 }));
    expect(reading()).toBe(reading200k);

    // The person picks another model for this chat: the old window is not this
    // model's, loaded or held, so nothing is drawn until the new one lands.
    rerender(ring({ ...loaded, boundModel: NEW_CHAT_MODEL, tokenLimit: 200_000 }));
    expect(container).toBeEmptyDOMElement();
    rerender(
      ring({
        ...loaded,
        boundModel: NEW_CHAT_MODEL,
        tokenLimit: 200_000,
        isTokenLimitLoaded: false,
      })
    );
    expect(container).toBeEmptyDOMElement();
    rerender(
      ring({
        ...loaded,
        boundModel: NEW_CHAT_MODEL,
        tokenLimitModel: NEW_CHAT_MODEL,
        tokenLimit: 1_050_000,
      })
    );
    expect(reading()).toBe(FOOTER_COPY.contextTooltip(94, '64.6k', '1.1M'));
    expect(screen.getByTestId('context-ring-arc')).toHaveAttribute('data-motion-still');
  });

  it('drops the ring while a chat it was drawn for loads again', () => {
    const { container, rerender } = render(
      ring({ totalTokens: 64_600, boundModel: CHAT_MODEL, tokenLimitModel: CHAT_MODEL })
    );
    expect(reading()).not.toBeNull();
    rerender(ring({ totalTokens: 64_600, boundModel: null, tokenLimitModel: CHAT_MODEL }));
    expect(container).toBeEmptyDOMElement();
  });
});

describe('contextWindowModelKey', () => {
  it('names the chat’s binding when it has one, else the app’s selection', () => {
    expect(
      contextWindowModelKey({ provider: 'anthropic', model: 'claude-opus-4-5' }, 'versa_azure', 'x')
    ).toBe('anthropic/claude-opus-4-5');
    expect(contextWindowModelKey(undefined, 'versa_azure', 'gpt-5.5')).toBe('versa_azure/gpt-5.5');
    expect(contextWindowModelKey(undefined, null, undefined)).toBe('/');
  });
});

/**
 * The token ceiling (`BIOROUTER_AUTO_COMPACT_MAX_TOKENS`, 200,000 unless set): the daemon compacts
 * at the LOWER of the marker's share and the ceiling, so on a window over 250k tokens the card has
 * to say the ceiling and draw it where it falls, or it promises 80% of a 1.05M window (840k) for a
 * chat that compacts at 200k.
 */
describe('the context card shows where the token ceiling makes Biorouter compact', () => {
  const CEILING_KEY = 'BIOROUTER_AUTO_COMPACT_MAX_TOKENS';

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.upsert.mockResolvedValue(undefined);
  });

  const readWith = (values: Record<string, unknown>) =>
    mocks.read.mockImplementation(async (key: string) => values[key] ?? null);

  const gauge = (props: Partial<React.ComponentProps<typeof ContextWindowGauge>> = {}) => (
    <ContextWindowGauge
      totalTokens={150_000}
      tokenLimit={1_050_000}
      isTokenLimitLoaded
      onCompact={vi.fn()}
      {...props}
    />
  );

  it('names the default ceiling on a large window and draws it at its share', async () => {
    readWith({});
    render(gauge());
    expect(await screen.findByText(CEILING_NOTE('200k', 19))).toHaveClass('br-footer-card__note');
    const tick = screen.getByTestId('context-gauge-ceiling');
    expect(tick).toHaveAttribute('aria-label', 'Compacts at 200k tokens');
    expect(parseFloat(tick.style.left)).toBeCloseTo(19.05, 1);
    // The marker is still the threshold's own control.
    const slider = screen.getByRole('slider', { name: CONTEXT_CARD_COPY.thresholdLabel });
    expect(slider).toHaveAttribute('aria-valuenow', '80');
  });

  it('turns the bar red past the ceiling, not only past the marker', async () => {
    readWith({});
    render(gauge({ totalTokens: 210_000 }));
    await screen.findByTestId('context-gauge-ceiling');
    const bar = screen.getByRole('progressbar', { name: CONTEXT_CARD_COPY.barLabel });
    expect(bar.outerHTML).toMatch(/danger/);
  });

  it('says the marker’s share when the ceiling is above it, or set to 0', async () => {
    readWith({ [CEILING_KEY]: 0 });
    const { unmount } = render(gauge());
    expect(await screen.findByText(CONTEXT_CARD_COPY.autoCompactNote(80))).toBeInTheDocument();
    expect(screen.queryByTestId('context-gauge-ceiling')).toBeNull();
    unmount();

    readWith({});
    render(gauge({ tokenLimit: 128_000, totalTokens: 20_000 }));
    expect(await screen.findByText(CONTEXT_CARD_COPY.autoCompactNote(80))).toBeInTheDocument();
    expect(screen.queryByTestId('context-gauge-ceiling')).toBeNull();
  });

  it('follows a ceiling set in config', async () => {
    readWith({ [CEILING_KEY]: 400_000 });
    render(gauge());
    expect(await screen.findByText(CEILING_NOTE('400k', 38))).toBeInTheDocument();
  });

  it('says compaction is off when the threshold turns it off, whatever the ceiling', async () => {
    readWith({ [THRESHOLD_KEY]: 0 });
    render(gauge());
    expect(await screen.findByText(CONTEXT_CARD_COPY.autoCompactOff)).toBeInTheDocument();
    expect(screen.queryByTestId('context-gauge-ceiling')).toBeNull();
  });
});
