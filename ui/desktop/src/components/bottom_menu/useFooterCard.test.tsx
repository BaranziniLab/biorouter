import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { useId, type MouseEvent } from 'react';
import {
  FooterCard,
  FooterCardAction,
  FooterCardActions,
  FooterCardGroup,
  FooterCardRoot,
  FooterCardSection,
  FooterCardTrigger,
  FooterCardValue,
} from './FooterCard';
import {
  FOOTER_CARD_OPEN_DELAY_MS,
  footerCardAnchorRect,
  isPointInPolygon,
  pointerGraceArea,
  useFooterCard,
} from './useFooterCard';

/**
 * The footer cards' primitive (`useFooterCard`, `FooterCardTrigger`, `FooterCard`), on stand-in
 * items, for what the line's behaviour table does not reach: an item's own click running before
 * the pin, Radix's focus wrap inside a card, a group of none, the anchor and grace geometry, and
 * the suppression the folder chip needs (#50).
 *
 * Every rule the three real items must share (hover, leaving, Tab, click, tap and keys, a press
 * inside, closing and focus, one card at a time, a window switch, scroll, the trigger's state) is
 * `footerCards.contract.test.tsx`'s, run over the folder, the context ring and the cost alike, so
 * it is not restated here. The folder card's own content and its chooser are
 * `FolderAccessCard.test.tsx`'s; geometry under a real pointer is `footerCard.browser.test.tsx`'s.
 */

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
afterEach(() => {
  vi.useRealTimers();
});

function TestItem({
  name,
  align = 'start',
  suppressed = false,
  onClick,
}: {
  name: string;
  align?: 'start' | 'end';
  suppressed?: boolean;
  onClick?: (event: MouseEvent<HTMLButtonElement>) => void;
}) {
  const card = useFooterCard({ suppressed });
  const cardId = useId();
  return (
    <FooterCardRoot card={card}>
      <FooterCardTrigger
        card={card}
        cardId={cardId}
        aria-label={`${name} item`}
        data-testid={`${name}-trigger`}
        onClick={onClick}
      >
        {name}
      </FooterCardTrigger>
      <FooterCard
        card={card}
        id={cardId}
        label={`${name} card`}
        align={align}
        testId={`${name}-card`}
      >
        <FooterCardSection label="Reading">
          <FooterCardValue>{`${name} value`}</FooterCardValue>
          <FooterCardActions>
            <FooterCardAction>{`${name} first`}</FooterCardAction>
            <FooterCardAction>{`${name} second`}</FooterCardAction>
          </FooterCardActions>
        </FooterCardSection>
      </FooterCard>
    </FooterCardRoot>
  );
}

/** Two items side by side, in a group (the footer line) or not. */
function Line({ grouped = true }: { grouped?: boolean }) {
  const items = (
    <div>
      <TestItem name="left" />
      <TestItem name="right" align="end" />
    </div>
  );
  return grouped ? <FooterCardGroup>{items}</FooterCardGroup> : items;
}

const trigger = (name: string) => screen.getByTestId(`${name}-trigger`);
const surface = (name: string) =>
  document.querySelector<HTMLElement>(`[data-testid="${name}-card"]`);
const isOpen = (name: string) => surface(name)?.getAttribute('data-state') === 'open';

function hoverOpen(name: string) {
  fireEvent.pointerEnter(trigger(name), { pointerType: 'mouse' });
  act(() => {
    vi.advanceTimersByTime(FOOTER_CARD_OPEN_DELAY_MS + 10);
  });
}

describe('an item’s own click', () => {
  it('lets an item’s own click run first, and a prevented click pins nothing', async () => {
    const user = userEvent.setup();
    const accelerator = vi.fn((event: MouseEvent) => event.preventDefault());
    render(<TestItem name="solo" onClick={accelerator} />);
    await user.click(trigger('solo'));
    expect(accelerator).toHaveBeenCalledTimes(1);
    expect(isOpen('solo')).toBe(false);
  });
});

describe('what the line’s contract does not reach', () => {
  async function pinFromKeyboard(user: ReturnType<typeof userEvent.setup>, name: string) {
    act(() => trigger(name).focus());
    await user.keyboard('{Enter}');
    await waitFor(() => expect(surface(name)).toHaveFocus());
  }

  it('wraps Shift+Tab from the first control inside the card, as Radix does', async () => {
    const user = userEvent.setup();
    render(<Line />);
    await pinFromKeyboard(user, 'left');
    await user.tab();
    expect(screen.getByRole('button', { name: 'left first' })).toHaveFocus();
    await user.keyboard('{Shift>}{Tab}{/Shift}');
    expect(screen.getByRole('button', { name: 'left second' })).toHaveFocus();
    expect(isOpen('left')).toBe(true);
  });

  it('closes a pinned card when Tab opens the next one', async () => {
    const user = userEvent.setup();
    render(<Line />);
    await user.click(trigger('left'));
    await user.tab();
    expect(isOpen('right')).toBe(true);
    expect(isOpen('left')).toBe(false);
  });

  it('leaves cards outside a group to themselves', () => {
    vi.useFakeTimers();
    render(<Line grouped={false} />);
    fireEvent.click(trigger('left'), { detail: 1 });
    hoverOpen('right');
    expect(isOpen('left')).toBe(true);
    expect(isOpen('right')).toBe(true);
  });
});

describe('where a card hangs', () => {
  // jsdom places nothing (and so flips every card below its trigger), so where the card lands is
  // `footerCard.browser.test.tsx`'s. That each real card is measured against the composer the line
  // hands it is the contract's (rule 11); these pin the primitive alone and its arithmetic.
  it('hangs from its trigger when nothing gives it a composer', async () => {
    const user = userEvent.setup();
    render(<TestItem name="solo" />);
    const measured = vi.spyOn(trigger('solo'), 'getBoundingClientRect');
    await user.click(trigger('solo'));
    expect(isOpen('solo')).toBe(true);
    await waitFor(() => expect(measured).toHaveBeenCalled());
  });

  it('stretches the trigger up to the composer’s top, keeping the trigger’s left and width', () => {
    const chip = new DOMRect(40, 560, 80, 20);
    const composer = new DOMRect(24, 440, 700, 110);
    const anchor = footerCardAnchorRect(chip, composer);
    expect(anchor.left).toBe(40);
    expect(anchor.width).toBe(80);
    expect(anchor.right).toBe(120);
    expect(anchor.top).toBe(440);
    expect(anchor.bottom).toBe(580);
    expect(footerCardAnchorRect(chip, null)).toBe(chip);
    expect(footerCardAnchorRect(chip, new DOMRect())).toBe(chip);
  });

  it('counts a pointer between the trigger and the card as on its way, and one beside it as not', () => {
    const exit = { x: 60, y: 560 };
    const cardBox = { top: 160, right: 360, bottom: 430, left: 40 };
    const area = pointerGraceArea(exit, cardBox);
    expect(isPointInPolygon({ x: 62, y: 500 }, area)).toBe(true);
    expect(isPointInPolygon({ x: 120, y: 440 }, area)).toBe(true);
    expect(isPointInPolygon({ x: 200, y: 300 }, area)).toBe(true);
    expect(isPointInPolygon({ x: 600, y: 520 }, area)).toBe(false);
    expect(isPointInPolygon({ x: 340, y: 550 }, area)).toBe(false);
    expect(isPointInPolygon({ x: 60, y: 600 }, area)).toBe(false);
  });
});

describe('while suppressed (#50)', () => {
  it('shows nothing, and nothing a hover or a click armed opens afterwards', () => {
    vi.useFakeTimers();
    const view = render(<TestItem name="solo" />);
    hoverOpen('solo');
    expect(isOpen('solo')).toBe(true);
    view.rerender(<TestItem name="solo" suppressed />);
    expect(isOpen('solo')).toBe(false);
    hoverOpen('solo');
    fireEvent.click(trigger('solo'), { detail: 1 });
    expect(isOpen('solo')).toBe(false);
    view.rerender(<TestItem name="solo" />);
    expect(isOpen('solo')).toBe(false);
    act(() => {
      vi.advanceTimersByTime(FOOTER_CARD_OPEN_DELAY_MS * 2);
    });
    expect(isOpen('solo')).toBe(false);
  });
});
