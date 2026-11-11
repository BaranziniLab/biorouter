import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SessionUsageLimitResponse } from '../../api';
import type { ModelCostRow } from '../../hooks/useCostTracking';
import { USAGE_LIMIT_COPY } from './copy';
import { CostTracker } from './CostTracker';
import { monthUsedReading } from './UsageLimitSection';
import { useSessionUsageLimit } from './useSessionUsageLimit';

/**
 * The cost card's Limit section (owner requirement 1): the limit in force for this chat and where
 * it comes from, what the chat used against it, and the writes the card makes through
 * `PUT /sessions/{id}/usage-limit` with the person's proof. The trigger stays on the line with no
 * cost to show and with Show costs off, so the section is always one click away.
 */
const mocks = vi.hoisted(() => ({
  pricing: vi.fn(),
  getLimit: vi.fn(),
  putLimit: vi.fn(),
  headers: vi.fn(async () => ({ 'X-User-Action': 'proof' })),
  browser: false,
}));
vi.mock('../../utils/pricing', () => ({ fetchModelPricing: mocks.pricing }));
vi.mock('../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({ currentModel: 'gpt-5.6-sol', currentProvider: 'versa_azure' }),
}));
vi.mock('../../api', () => ({
  getSessionUsageLimit: mocks.getLimit,
  putSessionUsageLimit: mocks.putLimit,
}));
vi.mock('../../utils/userAction', () => ({ userActionHeaders: mocks.headers }));
vi.mock('../../utils/surface', () => ({ isBrowserSurface: () => mocks.browser }));

const RATE = { input_token_cost: 0.000004, output_token_cost: 0.00002, currency: '$' };

function limitState(overrides: Partial<SessionUsageLimitResponse> = {}): SessionUsageLimitResponse {
  return {
    billingSessionId: 'chat-a',
    chatDollarLimitInert: false,
    chatOverride: null,
    defaults: { maxTokens: null, maxUsd: null },
    limit: { maxTokens: null, maxUsd: null, source: 'none' },
    monthDollarLimitInert: false,
    monthly: { maxTokens: null, maxUsd: null },
    paused: false,
    pausedByEstimate: false,
    nextRequest: null,
    reason: null,
    used: {
      chatTokens: 12_000,
      chatUsd: null,
      chatUsdComplete: true,
      month: '2026-11',
      monthTokens: 90_000,
      monthUsd: null,
      monthUsdComplete: true,
    },
    warn: false,
    ...overrides,
  };
}

const pricedRow: ModelCostRow = {
  provider: 'versa_azure',
  model: 'gpt-5.6-sol',
  inputTokens: 10_000,
  outputTokens: 2_000,
  cacheReadTokens: null,
  cacheCreationTokens: null,
  totalTokens: 12_000,
  turns: 2,
  totalCost: 1.2,
  costIsPartial: false,
};

function Harness({
  modelCostRows,
  sessionId = 'chat-a',
  onOpenUsageSettings,
}: {
  modelCostRows?: ModelCostRow[];
  sessionId?: string | null;
  onOpenUsageSettings?: () => void;
}) {
  const usageLimit = useSessionUsageLimit(sessionId);
  return (
    <>
      <button type="button" onClick={usageLimit.requestOpenCard}>
        open from notice
      </button>
      <CostTracker
        modelCostRows={modelCostRows}
        usageLimit={usageLimit}
        onOpenUsageSettings={onOpenUsageSettings}
      />
    </>
  );
}

const card = () => document.querySelector<HTMLElement>('[data-testid="cost-card"]');
const isOpen = () => card()?.getAttribute('data-state') === 'open';
const usageTrigger = () => screen.getByTestId('chat-usage');

async function openCard(trigger: HTMLElement) {
  fireEvent.click(trigger, { detail: 1 });
  await waitFor(() => expect(isOpen()).toBe(true));
  return screen.getByRole('dialog', { name: 'Cost' });
}

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
afterAll(() => vi.unstubAllGlobals());

beforeEach(() => {
  localStorage.clear();
  mocks.browser = false;
  mocks.pricing.mockReset().mockResolvedValue(RATE);
  mocks.getLimit.mockReset().mockResolvedValue({ data: limitState() });
  mocks.putLimit.mockReset();
  mocks.headers.mockClear();
});

describe('the cost item with a usage limit to manage', () => {
  it('is on the line with no cost to show, as the gauge alone, and opens a Limit section', async () => {
    render(<Harness modelCostRows={[]} />);
    await waitFor(() => expect(mocks.getLimit).toHaveBeenCalled());
    expect(screen.queryByTestId('chat-cost')).not.toBeInTheDocument();
    expect(usageTrigger()).toHaveAttribute('aria-label', USAGE_LIMIT_COPY.triggerLabel);
    expect(usageTrigger()).toHaveTextContent('');

    const dialog = await openCard(usageTrigger());
    // No cost section with nothing to price.
    expect(dialog).not.toHaveTextContent('Cost of this chat');
    const section = within(dialog).getByTestId('usage-limit-section');
    expect(section).toHaveTextContent(USAGE_LIMIT_COPY.noLimit);
    expect(section).toHaveTextContent(USAGE_LIMIT_COPY.source.none);
    expect(section).toHaveTextContent('12,000 tokens used');
    expect(within(dialog).getByRole('button', { name: USAGE_LIMIT_COPY.setLimit })).toBeVisible();
    expect(
      within(dialog).getByRole('button', { name: USAGE_LIMIT_COPY.noLimitForChatName })
    ).toBeVisible();
    // Nothing of its own to return from yet.
    expect(within(dialog).queryByRole('button', { name: USAGE_LIMIT_COPY.useDefault })).toBeNull();
  });

  it('reads the limit with the proof, and again each time the card opens', async () => {
    render(<Harness modelCostRows={[pricedRow]} />);
    await waitFor(() => expect(mocks.getLimit).toHaveBeenCalledTimes(1));
    expect(mocks.getLimit).toHaveBeenCalledWith(
      expect.objectContaining({
        path: { session_id: 'chat-a' },
        headers: { 'X-User-Action': 'proof' },
      })
    );
    await openCard(await screen.findByTestId('chat-cost'));
    await waitFor(() => expect(mocks.getLimit).toHaveBeenCalledTimes(2));
  });

  it('keeps the cost and its breakdown above the Limit section', async () => {
    mocks.getLimit.mockResolvedValue({
      data: limitState({
        limit: { maxTokens: null, maxUsd: 5, source: 'default' },
        defaults: { maxTokens: null, maxUsd: 5 },
        used: { ...limitState().used, chatUsd: 1.2 },
      }),
    });
    render(<Harness modelCostRows={[pricedRow]} />);
    const figure = await screen.findByTestId('chat-cost');
    expect(figure).toHaveTextContent('$1.20');
    const dialog = await openCard(figure);
    expect(dialog).toHaveTextContent('Cost of this chat');
    expect(within(dialog).getByRole('list', { name: 'By model' })).toBeInTheDocument();
    const section = within(dialog).getByTestId('usage-limit-section');
    await waitFor(() => expect(section).toHaveTextContent('$5.00'));
    expect(section).toHaveTextContent(USAGE_LIMIT_COPY.source.default);
    expect(section).toHaveTextContent('$1.20 of $5.00 used');
  });

  it('sets this chat’s limit in dollars for a priced model, through the client, with the proof', async () => {
    const user = userEvent.setup();
    mocks.putLimit.mockResolvedValue({
      data: limitState({
        chatOverride: { mode: 'limit', maxUsd: 5, maxTokens: null },
        limit: { maxTokens: null, maxUsd: 5, source: 'chat' },
      }),
    });
    render(<Harness modelCostRows={[pricedRow]} />);
    const dialog = await openCard(await screen.findByTestId('chat-cost'));
    await user.click(within(dialog).getByRole('button', { name: USAGE_LIMIT_COPY.setLimit }));
    const field = within(dialog).getByLabelText(USAGE_LIMIT_COPY.fieldLabel);
    await waitFor(() => expect(field).toHaveFocus());
    // A priced model starts in dollars.
    expect(within(dialog).getByRole('radio', { name: USAGE_LIMIT_COPY.dollars })).toBeChecked();
    await user.type(field, '5{Enter}');
    await waitFor(() => expect(mocks.putLimit).toHaveBeenCalledTimes(1));
    expect(mocks.putLimit).toHaveBeenCalledWith(
      expect.objectContaining({
        path: { session_id: 'chat-a' },
        headers: { 'X-User-Action': 'proof' },
        body: { mode: 'limit', maxUsd: 5 },
      })
    );
    const section = within(dialog).getByTestId('usage-limit-section');
    await waitFor(() => expect(section).toHaveTextContent(USAGE_LIMIT_COPY.source.chat));
    expect(section).toHaveTextContent('$5.00');
    expect(within(dialog).getByRole('button', { name: USAGE_LIMIT_COPY.useDefault })).toBeVisible();
    expect(
      within(dialog).getByRole('button', { name: USAGE_LIMIT_COPY.changeLimit })
    ).toBeVisible();
  });

  it('sets a token limit when the unit is switched, and refuses what is not an amount', async () => {
    const user = userEvent.setup();
    mocks.putLimit.mockResolvedValue({ data: limitState() });
    render(<Harness modelCostRows={[pricedRow]} />);
    const dialog = await openCard(await screen.findByTestId('chat-cost'));
    await user.click(within(dialog).getByRole('button', { name: USAGE_LIMIT_COPY.setLimit }));
    await user.click(within(dialog).getByRole('radio', { name: USAGE_LIMIT_COPY.tokens }));
    const field = within(dialog).getByLabelText(USAGE_LIMIT_COPY.fieldLabel);
    await user.type(field, 'lots');
    await user.click(within(dialog).getByRole('button', { name: USAGE_LIMIT_COPY.save }));
    expect(within(dialog).getByRole('alert')).toHaveTextContent(USAGE_LIMIT_COPY.invalidTokens);
    expect(field).toHaveAttribute('aria-invalid', 'true');
    expect(mocks.putLimit).not.toHaveBeenCalled();

    await user.clear(field);
    await user.type(field, '500k');
    await user.click(within(dialog).getByRole('button', { name: USAGE_LIMIT_COPY.save }));
    await waitFor(() =>
      expect(mocks.putLimit).toHaveBeenCalledWith(
        expect.objectContaining({ body: { mode: 'limit', maxTokens: 500_000 } })
      )
    );
  });

  it('writes No limit and Use default as the two other modes', async () => {
    const user = userEvent.setup();
    mocks.getLimit.mockResolvedValue({
      data: limitState({
        chatOverride: { mode: 'limit', maxTokens: 100_000, maxUsd: null },
        limit: { maxTokens: 100_000, maxUsd: null, source: 'chat' },
      }),
    });
    mocks.putLimit.mockResolvedValue({
      data: limitState({ chatOverride: { mode: 'unlimited' } }),
    });
    render(<Harness modelCostRows={[pricedRow]} />);
    const dialog = await openCard(await screen.findByTestId('chat-cost'));
    await waitFor(() =>
      expect(within(dialog).getByTestId('usage-limit-section')).toHaveTextContent(
        '12,000 of 100,000 tokens used'
      )
    );
    await user.click(
      within(dialog).getByRole('button', { name: USAGE_LIMIT_COPY.noLimitForChatName })
    );
    await waitFor(() =>
      expect(mocks.putLimit).toHaveBeenLastCalledWith(
        expect.objectContaining({ body: { mode: 'unlimited' } })
      )
    );
    await waitFor(() =>
      expect(within(dialog).getByTestId('usage-limit-section')).toHaveTextContent(
        USAGE_LIMIT_COPY.source.unlimited
      )
    );
    // The card holds focus when the pressed control goes away.
    await waitFor(() => expect(card()).toHaveFocus());

    mocks.putLimit.mockResolvedValue({ data: limitState() });
    await user.click(within(dialog).getByRole('button', { name: USAGE_LIMIT_COPY.useDefault }));
    await waitFor(() =>
      expect(mocks.putLimit).toHaveBeenLastCalledWith(
        expect.objectContaining({ body: { mode: 'inherit' } })
      )
    );
  });

  it('says a refused write could not be tied to a person', async () => {
    const user = userEvent.setup();
    mocks.putLimit.mockResolvedValue({ data: undefined, response: { status: 403 } });
    render(<Harness modelCostRows={[pricedRow]} />);
    const dialog = await openCard(await screen.findByTestId('chat-cost'));
    await user.click(
      within(dialog).getByRole('button', { name: USAGE_LIMIT_COPY.noLimitForChatName })
    );
    expect(await within(dialog).findByRole('alert')).toHaveTextContent(USAGE_LIMIT_COPY.needUser);
  });

  it('hides the spend but not the limit with Show costs off, and still offers dollars', async () => {
    const user = userEvent.setup();
    localStorage.setItem('show_pricing', 'false');
    mocks.getLimit.mockResolvedValue({
      data: limitState({
        limit: { maxTokens: 200_000, maxUsd: 5, source: 'default' },
        used: { ...limitState().used, chatUsd: 1.2 },
      }),
    });
    mocks.putLimit.mockResolvedValue({ data: limitState() });
    render(<Harness modelCostRows={[pricedRow]} />);
    // The figure is a dollar amount, so it is not drawn; the gauge stands in for it.
    expect(screen.queryByTestId('chat-cost')).not.toBeInTheDocument();
    const dialog = await openCard(await screen.findByTestId('chat-usage'));
    const section = within(dialog).getByTestId('usage-limit-section');
    await waitFor(() => expect(section).toHaveTextContent('200,000 tokens'));
    // The limit is the person's own number; what was spent is the running cost the switch hides.
    expect(section).toHaveTextContent('$5.00 · 200,000 tokens');
    expect(dialog).not.toHaveTextContent('$1.20');
    await user.click(within(dialog).getByRole('button', { name: USAGE_LIMIT_COPY.setLimit }));
    // Dollars first for a priced chat, Show costs or not.
    expect(within(dialog).getByRole('radio', { name: USAGE_LIMIT_COPY.dollars })).toBeChecked();
    await user.click(within(dialog).getByRole('radio', { name: USAGE_LIMIT_COPY.tokens }));
    await user.type(within(dialog).getByLabelText(USAGE_LIMIT_COPY.fieldLabel), '1m{Enter}');
    await waitFor(() =>
      expect(mocks.putLimit).toHaveBeenCalledWith(
        expect.objectContaining({ body: { mode: 'limit', maxTokens: 1_000_000 } })
      )
    );
  });

  it('marks the trigger and says why while a limit holds the chat', async () => {
    mocks.getLimit.mockResolvedValue({
      data: limitState({
        limit: { maxTokens: 10_000, maxUsd: null, source: 'chat' },
        chatOverride: { mode: 'limit', maxTokens: 10_000 },
        paused: true,
        reason: 'chat',
      }),
    });
    render(<Harness modelCostRows={[]} />);
    await waitFor(() => expect(usageTrigger()).toHaveAttribute('data-limit-state', 'paused'));
    expect(usageTrigger()).toHaveAttribute(
      'aria-label',
      `${USAGE_LIMIT_COPY.triggerLabel}, ${USAGE_LIMIT_COPY.triggerPaused}`
    );
    const dialog = await openCard(usageTrigger());
    expect(within(dialog).getByRole('alert')).toHaveTextContent(USAGE_LIMIT_COPY.pausedChat);
  });

  it('says the next request would pass the limit, and how much is left', async () => {
    mocks.getLimit.mockResolvedValue({
      data: limitState({
        limit: { maxTokens: null, maxUsd: 0.18, source: 'chat' },
        chatOverride: { mode: 'limit', maxUsd: 0.18 },
        used: { ...limitState().used, chatUsd: 0.1766 },
        paused: true,
        pausedByEstimate: true,
        nextRequest: { tokens: 36_000, usd: 0.0074 },
        reason: 'chat',
      }),
    });
    render(<Harness modelCostRows={[pricedRow]} />);
    const dialog = await openCard(await screen.findByTestId('chat-cost'));
    await waitFor(() =>
      expect(within(dialog).getByRole('alert')).toHaveTextContent(USAGE_LIMIT_COPY.pausedChatNext)
    );
    const section = within(dialog).getByTestId('usage-limit-section');
    // Used and limit never read alike: $0.177, not $0.18.
    expect(section).toHaveTextContent('$0.177 of $0.18 used');
    expect(section).toHaveTextContent(USAGE_LIMIT_COPY.nextRequest('$0.0074', '$0.0034'));
  });

  it('names the field by its unit, refuses a fifth decimal, and says an unpriced chat needs tokens', async () => {
    const user = userEvent.setup();
    mocks.pricing.mockResolvedValue(null);
    render(<Harness modelCostRows={[]} />);
    const dialog = await openCard(usageTrigger());
    await user.click(within(dialog).getByRole('button', { name: USAGE_LIMIT_COPY.setLimit }));
    // No price: tokens first, and dollars say why they cannot stop this chat.
    expect(within(dialog).getByRole('radio', { name: USAGE_LIMIT_COPY.tokens })).toBeChecked();
    const field = within(dialog).getByLabelText(USAGE_LIMIT_COPY.fieldLabel);
    expect(field).toHaveAccessibleName(USAGE_LIMIT_COPY.fieldNameTokens);
    await user.click(within(dialog).getByRole('radio', { name: USAGE_LIMIT_COPY.dollars }));
    expect(within(dialog).getByTestId('usage-limit-unpriced')).toHaveTextContent(
      USAGE_LIMIT_COPY.unpricedDollar
    );
    expect(field).toHaveAccessibleName(USAGE_LIMIT_COPY.fieldNameDollars);
    await user.type(field, '0.01255{Enter}');
    expect(within(dialog).getByRole('alert')).toHaveTextContent(
      USAGE_LIMIT_COPY.invalidUsdPrecision
    );
    expect(mocks.putLimit).not.toHaveBeenCalled();
  });

  it('tints the trigger at 80% of a limit', async () => {
    mocks.getLimit.mockResolvedValue({
      data: limitState({
        limit: { maxTokens: 14_000, maxUsd: null, source: 'default' },
        warn: true,
      }),
    });
    render(<Harness modelCostRows={[pricedRow]} />);
    const figure = await screen.findByTestId('chat-cost');
    await waitFor(() => expect(figure).toHaveAttribute('data-limit-state', 'warn'));
    expect(figure.getAttribute('aria-label')).toContain(USAGE_LIMIT_COPY.triggerWarn);
  });

  it('offers usage settings for the monthly limit, and no write for the chat', async () => {
    const user = userEvent.setup();
    const onOpenUsageSettings = vi.fn();
    mocks.getLimit.mockResolvedValue({
      data: limitState({
        monthly: { maxTokens: 90_000, maxUsd: null },
        paused: true,
        reason: 'month',
      }),
    });
    render(<Harness modelCostRows={[]} onOpenUsageSettings={onOpenUsageSettings} />);
    const dialog = await openCard(usageTrigger());
    await waitFor(() =>
      expect(within(dialog).getByRole('alert')).toHaveTextContent(USAGE_LIMIT_COPY.pausedMonth)
    );
    expect(within(dialog).getByTestId('usage-limit-section')).toHaveTextContent(
      'This month: 90,000 of 90,000 tokens used'
    );
    await user.click(within(dialog).getByRole('button', { name: USAGE_LIMIT_COPY.openSettings }));
    expect(onOpenUsageSettings).toHaveBeenCalledTimes(1);
  });

  it('opens and keeps the card when the composer notice asks', async () => {
    render(<Harness modelCostRows={[]} />);
    await waitFor(() => expect(mocks.getLimit).toHaveBeenCalled());
    expect(isOpen()).toBe(false);
    fireEvent.click(screen.getByRole('button', { name: 'open from notice' }));
    await waitFor(() => expect(isOpen()).toBe(true));
    expect(card()).toHaveAttribute('data-pinned', 'true');
    expect(usageTrigger()).toHaveAttribute('data-pinned', 'true');
  });

  it('closes on Escape from inside the form and hands focus back to the trigger', async () => {
    const user = userEvent.setup();
    render(<Harness modelCostRows={[]} />);
    await waitFor(() => expect(mocks.getLimit).toHaveBeenCalled());
    act(() => usageTrigger().focus());
    await user.keyboard('{Enter}');
    await waitFor(() => expect(card()).toHaveFocus());
    await user.tab();
    expect(within(card()!).getByRole('button', { name: USAGE_LIMIT_COPY.setLimit })).toHaveFocus();
    await user.keyboard('{Enter}');
    await waitFor(() =>
      expect(within(card()!).getByLabelText(USAGE_LIMIT_COPY.fieldLabel)).toHaveFocus()
    );
    await user.keyboard('{Escape}');
    await waitFor(() => expect(isOpen()).toBe(false));
    expect(usageTrigger()).toHaveFocus();
  });

  it('offers no write in a browser, nor for a subagent’s chat, and says why', async () => {
    mocks.browser = true;
    const { unmount } = render(<Harness modelCostRows={[]} />);
    let dialog = await openCard(usageTrigger());
    await waitFor(() => expect(dialog).toHaveTextContent(USAGE_LIMIT_COPY.browserNote));
    expect(within(dialog).queryByRole('button')).toBeNull();
    unmount();

    mocks.browser = false;
    mocks.getLimit.mockResolvedValue({ data: limitState({ billingSessionId: 'parent' }) });
    render(<Harness modelCostRows={[]} />);
    dialog = await openCard(usageTrigger());
    await waitFor(() => expect(dialog).toHaveTextContent(USAGE_LIMIT_COPY.parentChat));
    expect(within(dialog).queryByRole('button')).toBeNull();
  });

  it('in a chat that has not started, says when a limit can be set and reads nothing', async () => {
    render(<Harness modelCostRows={[]} sessionId="" />);
    const dialog = await openCard(usageTrigger());
    expect(dialog).toHaveTextContent(USAGE_LIMIT_COPY.noChatYet);
    expect(mocks.getLimit).not.toHaveBeenCalled();
  });
});

describe('the month line', () => {
  it('reads the month against its limit', () => {
    const state = limitState({ monthly: { maxTokens: 100_000, maxUsd: null } });
    expect(monthUsedReading(state, true)).toContain('90,000');
  });

  it('says nothing when the daemon withheld the month totals', () => {
    const base = limitState({ monthly: { maxTokens: 100_000, maxUsd: 5 } });
    const state = { ...base, used: { ...base.used, monthTokens: null, monthUsd: null } };
    expect(monthUsedReading(state, true)).toBeNull();
  });
});
