import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useId } from 'react';
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ModelCostRow, SessionCosts } from '../../hooks/useCostTracking';
import { COST_CARD_COPY, MODEL_NOT_RECORDED } from './copy';
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

  it("never prices bare counters in a decision chat, whose tokens are not the current model's", async () => {
    // Live, a local decision chat's footer priced its 144 tokens at the app's
    // current model and read "<$0.01" for a request that costs nothing.
    render(
      <CostTracker inputTokens={2_000_000} outputTokens={200_000} modelCostRows={[]} ledgerOnly />
    );
    await settlePricing();
    expect(screen.queryByTestId('chat-cost')).not.toBeInTheDocument();
  });

  it('still reads the ledger in a decision chat', async () => {
    render(
      <CostTracker
        inputTokens={2_000_000}
        outputTokens={200_000}
        modelCostRows={[ledgerRow]}
        ledgerOnly
      />
    );
    await waitFor(() => expect(screen.getByTestId('chat-cost')).toHaveTextContent('$3.50'));
  });

  it('reads the ledger rows in preference to the counters', async () => {
    render(
      <CostTracker inputTokens={2_000_000} outputTokens={200_000} modelCostRows={[ledgerRow]} />
    );
    await waitFor(() => expect(screen.getByTestId('chat-cost')).toHaveTextContent('$3.50'));
  });

  it('names the cost exact beside a local decision model', async () => {
    const local: ModelCostRow = {
      ...ledgerRow,
      provider: 'ollama_decision',
      model: 'tev1:0.8b',
      turns: 20,
      totalCost: null,
    };
    render(<CostTracker modelCostRows={[ledgerRow, local]} />);
    const cost = await screen.findByTestId('chat-cost');
    expect(cost).toHaveTextContent('$3.50');
    expect(cost).toHaveAttribute('aria-label', 'Chat cost $3.50');
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
    // Neither row recorded the cache, so the line lists input and output alone.
    expect(within(card).getByText('20,100 input · 1,550 output')).toHaveClass(
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

  it('names a ledger row with no model in sentence case in the By model list', async () => {
    const user = userEvent.setup();
    // The ledger's null model bucket: turns saved before model attribution, or from a provider
    // that reported no model. Its key token is the lowercase "unknown"; a person never reads it.
    const noModelRow: ModelCostRow = { ...unpricedRow, provider: undefined, model: undefined };
    render(<CostTracker modelCostRows={[ledgerRow, noModelRow]} />);
    await waitFor(() => expect(figure()).toHaveTextContent('$3.50'));
    await user.click(figure());
    const card = screen.getByRole('dialog', { name: COST_CARD_COPY.cardLabel });
    const rows = within(
      within(card).getByRole('list', { name: COST_CARD_COPY.byModel })
    ).getAllByRole('listitem');
    expect(rows).toHaveLength(2);
    expect(within(rows[1]).getByText(MODEL_NOT_RECORDED)).toHaveClass('br-footer-card__value');
    expect(within(rows[1]).getByText('Unavailable · 20,000 in · 1,500 out')).toHaveClass(
      'br-footer-card__detail'
    );
    expect(card).not.toHaveTextContent(/unknown/);
  });

  it('says nothing about an estimate when every row is priced exactly', async () => {
    const user = userEvent.setup();
    render(<CostTracker modelCostRows={[ledgerRow]} />);
    await waitFor(() => expect(figure()).toHaveTextContent('$3.50'));
    await user.click(figure());
    const card = screen.getByRole('dialog', { name: 'Cost' });
    expect(card).not.toHaveTextContent(COST_CARD_COPY.partialNote);
    expect(within(card).getByText('100 input · 50 output')).toBeInTheDocument();
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
    expect(within(mapCard).getByText('100 input · 50 output')).toBeInTheDocument();
    expect(within(mapCard).getByText(COST_CARD_COPY.partialNote)).toBeInTheDocument();
    expect(within(mapCard).queryByText(COST_CARD_COPY.byModel)).not.toBeInTheDocument();
    const mapShape = cardShape(mapCard);
    fromMap.unmount();

    render(<CostTracker inputTokens={2_000_000} outputTokens={200_000} />);
    await waitFor(() => expect(figure()).toHaveTextContent('$11.00'));
    await user.click(figure());
    const liveCard = screen.getByRole('dialog', { name: 'Cost' });
    expect(within(liveCard).getByText('$11.00')).toHaveClass('br-footer-card__value');
    expect(within(liveCard).getByText('2,000,000 input · 200,000 output')).toBeInTheDocument();
    // The counters hold input and output alone: the card says nothing about the cache.
    expect(liveCard).not.toHaveTextContent(/cache/);
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

  it('lists the cache the ledger recorded, so its figures add up to the chat’s billed tokens', async () => {
    const user = userEvent.setup();
    // The owner's chat: the card read "22,788 input · 382 output" beside a Tokens value of 89k,
    // and Settings > App > Usage listed the other 65,408 as cache read.
    const owners: ModelCostRow = {
      provider: 'versa_azure',
      model: 'gpt-5.5-2026-04-24',
      inputTokens: 22_788,
      outputTokens: 382,
      cacheReadTokens: 65_408,
      cacheCreationTokens: 0,
      totalTokens: 88_578,
      turns: 4,
      totalCost: 0.14,
      costIsPartial: false,
    };
    render(<CostTracker modelCostRows={[owners]} />);
    await waitFor(() => expect(figure()).toHaveTextContent('$0.14'));
    await user.click(figure());
    const card = screen.getByRole('dialog', { name: COST_CARD_COPY.cardLabel });
    // One line: input, output and one cache bucket fit the card's column.
    const total = within(card).getByText('22,788 input · 382 output · 65,408 cache read');
    expect(total).toHaveClass('br-footer-card__detail');
    expect(total.querySelector('br')).toBeNull();
    // A bucket with nothing in it says nothing.
    expect(card).not.toHaveTextContent(/cache write/);
    const [row] = within(
      within(card).getByRole('list', { name: COST_CARD_COPY.byModel })
    ).getAllByRole('listitem');
    expect(within(row).getByText('$0.14 · 22,788 in · 382 out · 65,408 cache read')).toHaveClass(
      'br-footer-card__detail'
    );
    expect(22_788 + 382 + 65_408).toBe(owners.totalTokens);
  });

  it('gives both cache buckets a second line inside the same muted detail', async () => {
    const user = userEvent.setup();
    const cachedBothWays: ModelCostRow = {
      ...ledgerRow,
      inputTokens: 1_200,
      outputTokens: 800,
      cacheReadTokens: 40_000,
      cacheCreationTokens: 9_000,
      totalTokens: 51_000,
    };
    render(<CostTracker modelCostRows={[cachedBothWays]} />);
    await waitFor(() => expect(figure()).toHaveTextContent('$3.50'));
    await user.click(figure());
    const card = screen.getByRole('dialog', { name: COST_CARD_COPY.cardLabel });

    // As they read: the no-break spaces that hold each figure to its word are spaces.
    const lines = (detail: HTMLElement) =>
      [...detail.querySelectorAll(':scope > span')].map((line) =>
        line.textContent?.replace(/\u00a0/g, ' ')
      );
    // ONE paragraph with a line break, so the second line keeps the detail's own rhythm rather
    // than the section's gap: the card's shape is the same as with one line.
    const total = within(card).getByText('1,200 input · 800 output').parentElement!;
    expect(total).toHaveClass('br-footer-card__detail');
    expect(lines(total)).toEqual([
      '1,200 input · 800 output',
      '40,000 cache read · 9,000 cache write',
    ]);
    expect(total.querySelectorAll('br')).toHaveLength(1);
    expect(card.querySelectorAll('.br-footer-card__detail')).toHaveLength(2);
    expect(cardShape(card)).toEqual([
      'br-footer-card__section',
      'br-footer-card__label',
      'br-footer-card__value',
      'br-footer-card__detail',
      'br-footer-card__section',
      'br-footer-card__label',
      'br-footer-card__list',
      'br-footer-card__row',
      'br-footer-card__row-text',
      'br-footer-card__value',
      'br-footer-card__detail',
    ]);

    const [row] = within(
      within(card).getByRole('list', { name: COST_CARD_COPY.byModel })
    ).getAllByRole('listitem');
    const rowDetail = within(row).getByText('$3.50 · 1,200 in · 800 out').parentElement!;
    expect(rowDetail).toHaveClass('br-footer-card__detail');
    expect(lines(rowDetail)).toEqual([
      '$3.50 · 1,200 in · 800 out',
      '40,000 cache read · 9,000 cache write',
    ]);
  });

  it('lists the cache buckets a legacy map’s rows carry', async () => {
    const user = userEvent.setup();
    render(
      <CostTracker
        sessionCosts={{
          'openai/gpt-5.5': {
            inputTokens: 100,
            outputTokens: 50,
            cacheReadTokens: 2_000,
            totalCost: 3.5,
          },
        }}
      />
    );
    await waitFor(() => expect(figure()).toHaveTextContent('$3.50'));
    await user.click(figure());
    const card = screen.getByRole('dialog', { name: COST_CARD_COPY.cardLabel });
    expect(within(card).getByText('100 input · 50 output · 2,000 cache read')).toHaveClass(
      'br-footer-card__detail'
    );
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

  it('closes the other card when the cost card opens, and the cost card when the other is kept, never on a hover', async () => {
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

    // Hover peeks, click pins: a hover over the other item opens nothing while the cost card is
    // kept (footerCards.contract rule 7)...
    vi.useFakeTimers();
    fireEvent.pointerEnter(screen.getByTestId('other'), { pointerType: 'mouse' });
    act(() => {
      vi.advanceTimersByTime(FOOTER_CARD_OPEN_DELAY_MS * 3);
    });
    expect(otherOpen()).toBe(false);
    expect(isOpen()).toBe(true);
    // ...and a click on it replaces it.
    fireEvent.click(screen.getByTestId('other'), { detail: 1 });
    expect(otherOpen()).toBe(true);
    expect(isOpen()).toBe(false);
  });
});
