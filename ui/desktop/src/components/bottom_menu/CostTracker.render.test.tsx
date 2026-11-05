import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useId } from 'react';
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ModelCostRow, SessionCosts } from '../../hooks/useCostTracking';
import { COST_CARD_COPY } from './copy';
import { CostTracker } from './CostTracker';
import {
  FooterCard,
  FooterCardGroup,
  FooterCardRoot,
  FooterCardSection,
  FooterCardTrigger,
  FooterCardValue,
} from './FooterCard';
import { FOOTER_CARD_OPEN_DELAY_MS, useFooterCard } from './useFooterCard';

/**
 * What the footer's cost does with what it is handed. BaseChat leans on all of
 * it (V2-R5-5): until a chat's usage ledger answers it passes NONE of the four
 * props, so the footer must draw nothing for that, and must never price the
 * bare counters on its own account. `BaseChat.costLedger.test.tsx` proves the
 * whole path through BaseChat's helpers; this file keeps the component's half
 * of the contract in the component's own suite, so a change here fails here.
 *
 * The card the figure opens is one of the footer's three (`FooterCard`): when it
 * opens and closes is `useFooterCard.test.tsx`'s, on stand-in items. What this
 * file pins is that the cost figure IS such an item, and what its card says on
 * each of the three data paths.
 */
const mocks = vi.hoisted(() => ({ pricing: vi.fn() }));
vi.mock('../../utils/pricing', () => ({ fetchModelPricing: mocks.pricing }));
vi.mock('../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({ currentModel: 'claude-opus-4-5', currentProvider: 'anthropic' }),
}));

/** The current model at $5 per million tokens, either way. */
const RATE = { input_token_cost: 0.000005, output_token_cost: 0.000005, currency: '$' };

const ledgerRow: ModelCostRow = {
  provider: 'anthropic',
  model: 'claude-opus-4-5',
  inputTokens: 100,
  outputTokens: 50,
  cacheReadTokens: null,
  cacheCreationTokens: null,
  totalTokens: 150,
  turns: 1,
  totalCost: 3.5,
  costIsPartial: false,
};

/** A model the pricing table does not know: its cost is unavailable, never $0. */
const unpricedRow: ModelCostRow = {
  provider: 'openai',
  model: 'gpt-retired',
  inputTokens: 20_000,
  outputTokens: 1_500,
  cacheReadTokens: null,
  cacheCreationTokens: null,
  totalTokens: 21_500,
  turns: 2,
  totalCost: null,
  costIsPartial: false,
};

/** Lets the footer's own price lookup land, which is when a counter would be priced. */
async function settlePricing() {
  await waitFor(() => expect(mocks.pricing).toHaveBeenCalled());
  await act(async () => {});
}

const figure = () => screen.getByTestId('chat-cost');
const costCard = () => document.querySelector<HTMLElement>('[data-testid="cost-card"]');
const isOpen = () => costCard()?.getAttribute('data-state') === 'open';

/** Every piece of the card, in order, by its part of the shared format. */
function cardShape(card: HTMLElement): string[] {
  return [...card.querySelectorAll<HTMLElement>('[class*="br-footer-card__"]')].map(
    (el) => el.className.match(/br-footer-card__[\w-]+/)?.[0] ?? ''
  );
}

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
  localStorage.clear();
  mocks.pricing.mockReset().mockResolvedValue(RATE);
});
afterEach(() => {
  vi.useRealTimers();
});

describe('CostTracker', () => {
  it('draws nothing when handed nothing to price, even for a priced model', async () => {
    render(<CostTracker />);
    await settlePricing();
    expect(screen.queryByTestId('chat-cost')).not.toBeInTheDocument();
  });

  it('draws nothing for an empty ledger and zero counters', async () => {
    render(<CostTracker inputTokens={0} outputTokens={0} sessionCosts={{}} modelCostRows={[]} />);
    await settlePricing();
    expect(screen.queryByTestId('chat-cost')).not.toBeInTheDocument();
  });

  it('prices bare counters at the current rate once there is no ledger row to read', async () => {
    // The fallback BaseChat withholds until the ledger has answered: it is only
    // the right reading when the ledger is known to hold no rows.
    render(<CostTracker inputTokens={2_000_000} outputTokens={200_000} />);
    await waitFor(() => expect(screen.getByTestId('chat-cost')).toHaveTextContent('$11.00'));
  });

  it('reads the ledger rows in preference to the counters', async () => {
    render(
      <CostTracker inputTokens={2_000_000} outputTokens={200_000} modelCostRows={[ledgerRow]} />
    );
    await waitFor(() => expect(screen.getByTestId('chat-cost')).toHaveTextContent('$3.50'));
  });

  it('draws nothing when the person has switched pricing off', async () => {
    localStorage.setItem('show_pricing', 'false');
    render(<CostTracker modelCostRows={[ledgerRow]} />);
    await act(async () => {});
    expect(screen.queryByTestId('chat-cost')).not.toBeInTheDocument();
  });
});

describe('the cost figure is a footer card trigger', () => {
  it('is a real button on the line’s recipe that names the total and says whether its card is open', async () => {
    const user = userEvent.setup();
    render(<CostTracker modelCostRows={[ledgerRow]} />);
    await waitFor(() => expect(figure()).toHaveTextContent('$3.50'));
    const button = figure();
    expect(button.tagName).toBe('BUTTON');
    expect(button).toHaveAttribute('type', 'button');
    expect(button).toHaveClass('br-footline__item');
    expect(button).toHaveAccessibleName('Chat cost $3.50');
    expect(button).toHaveAttribute('aria-haspopup', 'dialog');
    expect(button).toHaveAttribute('aria-expanded', 'false');
    expect(button).not.toHaveAttribute('aria-controls');

    await user.click(button);
    expect(button).toHaveAttribute('aria-expanded', 'true');
    expect(button).toHaveAttribute('aria-controls', costCard()!.id);
    expect(screen.getByRole('dialog', { name: COST_CARD_COPY.cardLabel })).toBe(costCard());
    // A readout on the line's right: its card lines up with the figure's right edge. Where it
    // lands under a real layout is `footerCard.browser.test.tsx`'s.
    await waitFor(() => expect(costCard()).toHaveAttribute('data-align', 'end'));
  });

  it('names a partial total as an estimate', async () => {
    render(<CostTracker modelCostRows={[ledgerRow, unpricedRow]} />);
    await waitFor(() => expect(figure()).toHaveAccessibleName('Estimated chat total $3.50'));
  });
});

describe('the cost card', () => {
  it('opens on hover with the total, its tokens, the partial note and a row per model', async () => {
    render(<CostTracker modelCostRows={[ledgerRow, unpricedRow]} />);
    await waitFor(() => expect(figure()).toHaveTextContent('$3.50'));
    vi.useFakeTimers();
    fireEvent.pointerEnter(figure(), { pointerType: 'mouse' });
    act(() => {
      vi.advanceTimersByTime(FOOTER_CARD_OPEN_DELAY_MS - 10);
    });
    expect(isOpen()).toBe(false);
    act(() => {
      vi.advanceTimersByTime(20);
    });
    expect(isOpen()).toBe(true);
    // The card replaced the dark tooltip; there is no second hover surface.
    expect(screen.queryByRole('tooltip')).not.toBeInTheDocument();

    const card = screen.getByRole('dialog', { name: 'Cost' });
    expect(card).toHaveAttribute('data-testid', 'cost-card');
    expect(card).toHaveTextContent('Cost of this chat');
    expect(within(card).getByText('$3.50')).toHaveClass('br-footer-card__value');
    expect(within(card).getByText('20,100 input tokens · 1,550 output tokens')).toHaveClass(
      'br-footer-card__detail'
    );
    expect(within(card).getByText(COST_CARD_COPY.partialNote)).toHaveClass('br-footer-card__note');

    const models = within(card).getByRole('list', { name: 'By model' });
    const rows = within(models).getAllByRole('listitem');
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent('anthropic/claude-opus-4-5');
    expect(within(rows[0]).getByText('$3.50 · 100 in · 50 out')).toHaveClass(
      'br-footer-card__detail'
    );
    expect(rows[1]).toHaveTextContent('openai/gpt-retired');
    expect(within(rows[1]).getByText('Unavailable · 20,000 in · 1,500 out')).toBeInTheDocument();
    // Rows of figures carry no action.
    expect(within(card).queryByRole('button')).not.toBeInTheDocument();
  });

  it('says nothing about an estimate when every row is priced exactly', async () => {
    const user = userEvent.setup();
    render(<CostTracker modelCostRows={[ledgerRow]} />);
    await waitFor(() => expect(figure()).toHaveTextContent('$3.50'));
    await user.click(figure());
    const card = screen.getByRole('dialog', { name: 'Cost' });
    expect(card).not.toHaveTextContent(COST_CARD_COPY.partialNote);
    expect(within(card).getByText('100 input tokens · 50 output tokens')).toBeInTheDocument();
    expect(
      within(within(card).getByRole('list', { name: 'By model' })).getAllByRole('listitem')
    ).toHaveLength(1);
  });

  it('pins on a click, outlasts the pointer leaving, and closes on a second click', async () => {
    render(<CostTracker modelCostRows={[ledgerRow]} />);
    await waitFor(() => expect(figure()).toHaveTextContent('$3.50'));
    vi.useFakeTimers();
    fireEvent.click(figure(), { detail: 1 });
    expect(isOpen()).toBe(true);
    fireEvent.pointerLeave(figure(), { pointerType: 'mouse' });
    act(() => {
      vi.advanceTimersByTime(1_000);
    });
    expect(isOpen()).toBe(true);
    fireEvent.click(figure(), { detail: 1 });
    expect(isOpen()).toBe(false);
    expect(figure()).toHaveAttribute('aria-expanded', 'false');
  });

  it('moves focus onto the card on Enter, and Escape gives it back to the figure', async () => {
    const user = userEvent.setup();
    render(<CostTracker modelCostRows={[ledgerRow]} />);
    await waitFor(() => expect(figure()).toHaveTextContent('$3.50'));
    act(() => figure().focus());
    // A program's focus opens nothing.
    expect(isOpen()).toBe(false);
    await user.keyboard('{Enter}');
    await waitFor(() => expect(costCard()).toHaveFocus());
    await user.keyboard('{Escape}');
    await waitFor(() => expect(isOpen()).toBe(false));
    expect(figure()).toHaveFocus();
  });

  it('opens on a Tab onto the figure, and closes when Tab moves on', async () => {
    const user = userEvent.setup();
    render(
      <>
        <button type="button">Before</button>
        <CostTracker modelCostRows={[ledgerRow]} />
        <button type="button">After</button>
      </>
    );
    await waitFor(() => expect(figure()).toHaveTextContent('$3.50'));
    act(() => screen.getByText('Before').focus());
    await user.tab();
    expect(figure()).toHaveFocus();
    expect(isOpen()).toBe(true);
    await user.tab();
    expect(isOpen()).toBe(false);
  });

  it('is the same card for the legacy map and the live price, without By model', async () => {
    const user = userEvent.setup();
    const legacy: SessionCosts = {
      'anthropic/claude-opus-4-5': {
        inputTokens: 100,
        outputTokens: 50,
        totalCost: 3.5,
        costIsPartial: true,
      },
    };

    const ledger = render(<CostTracker modelCostRows={[{ ...ledgerRow, costIsPartial: true }]} />);
    await waitFor(() => expect(figure()).toHaveTextContent('$3.50'));
    await user.click(figure());
    const ledgerShape = cardShape(screen.getByRole('dialog', { name: 'Cost' }));
    ledger.unmount();

    const fromMap = render(<CostTracker sessionCosts={legacy} />);
    await waitFor(() => expect(figure()).toHaveTextContent('$3.50'));
    await user.click(figure());
    const mapCard = screen.getByRole('dialog', { name: 'Cost' });
    expect(mapCard).toHaveAttribute('data-testid', 'cost-card');
    expect(within(mapCard).getByText('100 input tokens · 50 output tokens')).toBeInTheDocument();
    expect(within(mapCard).getByText(COST_CARD_COPY.partialNote)).toBeInTheDocument();
    expect(within(mapCard).queryByText(COST_CARD_COPY.byModel)).not.toBeInTheDocument();
    const mapShape = cardShape(mapCard);
    fromMap.unmount();

    render(<CostTracker inputTokens={2_000_000} outputTokens={200_000} />);
    await waitFor(() => expect(figure()).toHaveTextContent('$11.00'));
    await user.click(figure());
    const liveCard = screen.getByRole('dialog', { name: 'Cost' });
    expect(within(liveCard).getByText('$11.00')).toHaveClass('br-footer-card__value');
    expect(
      within(liveCard).getByText('2,000,000 input tokens · 200,000 output tokens')
    ).toBeInTheDocument();
    // The counters priced at the current model's rate are always an estimate.
    expect(within(liveCard).getByText(COST_CARD_COPY.partialNote)).toBeInTheDocument();
    expect(within(liveCard).queryByText(COST_CARD_COPY.byModel)).not.toBeInTheDocument();

    const totalSection = ['section', 'label', 'value', 'detail', 'note'].map(
      (part) => `br-footer-card__${part}`
    );
    expect(mapShape).toEqual(totalSection);
    expect(cardShape(liveCard)).toEqual(totalSection);
    // The ledger's card is the same first section, then By model.
    expect(ledgerShape.slice(0, totalSection.length)).toEqual(totalSection);
    expect(ledgerShape.slice(totalSection.length)).toEqual([
      'br-footer-card__section',
      'br-footer-card__label',
      'br-footer-card__list',
      'br-footer-card__row',
      'br-footer-card__row-text',
      'br-footer-card__value',
      'br-footer-card__detail',
    ]);
  });

  it('stays pinned when the ledger’s answer replaces the live price', async () => {
    const user = userEvent.setup();
    const { rerender } = render(<CostTracker inputTokens={2_000_000} outputTokens={200_000} />);
    await waitFor(() => expect(figure()).toHaveTextContent('$11.00'));
    await user.click(figure());
    expect(isOpen()).toBe(true);
    rerender(
      <CostTracker inputTokens={2_000_000} outputTokens={200_000} modelCostRows={[ledgerRow]} />
    );
    expect(figure()).toHaveTextContent('$3.50');
    expect(isOpen()).toBe(true);
    expect(screen.getByRole('list', { name: 'By model' })).toBeInTheDocument();
  });
});

/** Another card on the same line, standing in for the folder chip or the context ring. */
function OtherItem() {
  const card = useFooterCard();
  const cardId = useId();
  return (
    <FooterCardRoot card={card}>
      <FooterCardTrigger card={card} cardId={cardId} aria-label="Other" data-testid="other">
        Other
      </FooterCardTrigger>
      <FooterCard card={card} id={cardId} label="Other card" align="start" testId="other-card">
        <FooterCardSection label="Reading">
          <FooterCardValue>Other value</FooterCardValue>
        </FooterCardSection>
      </FooterCard>
    </FooterCardRoot>
  );
}

describe('one card at a time on the footer line', () => {
  const otherOpen = () =>
    document.querySelector('[data-testid="other-card"]')?.getAttribute('data-state') === 'open';

  it('closes the other card when the cost card opens, and closes the cost card when it opens', async () => {
    const user = userEvent.setup();
    render(
      <FooterCardGroup>
        <OtherItem />
        <CostTracker modelCostRows={[ledgerRow]} />
      </FooterCardGroup>
    );
    await waitFor(() => expect(figure()).toHaveTextContent('$3.50'));
    await user.click(screen.getByTestId('other'));
    expect(otherOpen()).toBe(true);

    await user.click(figure());
    expect(isOpen()).toBe(true);
    expect(otherOpen()).toBe(false);
    // Opening it never took focus from where the click left it.
    expect(figure()).toHaveFocus();

    vi.useFakeTimers();
    fireEvent.pointerEnter(screen.getByTestId('other'), { pointerType: 'mouse' });
    act(() => {
      vi.advanceTimersByTime(FOOTER_CARD_OPEN_DELAY_MS + 10);
    });
    expect(otherOpen()).toBe(true);
    expect(isOpen()).toBe(false);
  });
});
