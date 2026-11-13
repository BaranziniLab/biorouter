import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ModelUsageRow, Session, SessionUsageLimitResponse } from '../../api';
import { useCostTracking } from '../../hooks/useCostTracking';
import { CostTracker } from './CostTracker';
import { useSessionUsageLimit } from './useSessionUsageLimit';

/**
 * 2026-11-13, found live on a Claude Haiku 5.5 chat (Versa Bedrock) with a ~93K-token first
 * message. The chat's title is a billed side call (`side:session_name`) the daemon makes after the
 * turn's last frame. The card's "Cost of this chat" read $0.0769, the one turn, while its Limit
 * line, read again on every open, counted the title's 92,745 tokens: the ledger behind the cost
 * was read when the turn ended and nothing asked again. The card now reads both on open.
 */
const mocks = vi.hoisted(() => ({
  pricing: vi.fn(),
  getUsage: vi.fn(),
  getLimit: vi.fn(),
  putLimit: vi.fn(),
  headers: vi.fn(async () => ({ 'X-User-Action': 'proof' })),
}));
vi.mock('../../utils/pricing', () => ({ fetchModelPricing: mocks.pricing }));
vi.mock('../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    currentModel: 'us.anthropic.claude-haiku-5-5',
    currentProvider: 'versa_bedrock',
  }),
}));
vi.mock('../../api', () => ({
  getSessionUsage: mocks.getUsage,
  getSessionUsageLimit: mocks.getLimit,
  putSessionUsageLimit: mocks.putLimit,
}));
vi.mock('../../utils/userAction', () => ({ userActionHeaders: mocks.headers }));
vi.mock('../../utils/surface', () => ({ isBrowserSurface: () => false }));

const turn: ModelUsageRow = {
  provider: 'versa_bedrock',
  modelId: 'us.anthropic.claude-haiku-5-5',
  inputTokens: 93_000,
  outputTokens: 400,
  cacheReadTokens: 0,
  cacheCreationTokens: 0,
  totalTokens: 93_400,
  turns: 1,
  cost: 0.0769,
  costIsPartial: false,
};
/** The same row once the title's side call landed in it: same model, same provider. */
const turnAndTitle: ModelUsageRow = {
  ...turn,
  inputTokens: turn.inputTokens + 92_745,
  outputTokens: turn.outputTokens + 6,
  totalTokens: (turn.totalTokens ?? 0) + 92_751,
  cost: 0.0769 + 0.0093,
};

function limitState(chatTokens: number): SessionUsageLimitResponse {
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
      chatTokens,
      chatUsd: null,
      chatUsdComplete: true,
      month: '2026-11',
      monthTokens: chatTokens,
      monthUsd: null,
      monthUsdComplete: true,
    },
    warn: false,
  };
}

/** A live chat: its row and its counters as the turn's last frame left them. */
const session = { id: 'chat-a', accumulated_total_tokens: 93_400 } as unknown as Session;
const liveTokens = { accumulatedInputTokens: 93_000, accumulatedOutputTokens: 400 };

/** The footer's cost item as `BaseChat` and `ChatInput` wire it. */
function Footer() {
  const { modelRows, reload } = useCostTracking({ session, liveTokens });
  const usageLimit = useSessionUsageLimit('chat-a');
  return <CostTracker modelCostRows={modelRows} reloadCost={reload} usageLimit={usageLimit} />;
}

const card = () => document.querySelector<HTMLElement>('[data-testid="cost-card"]');

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
  mocks.pricing.mockReset().mockResolvedValue(null);
  mocks.getUsage.mockReset();
  mocks.getLimit.mockReset();
  mocks.putLimit.mockReset();
});

describe('the cost card counts a side call made after the turn', () => {
  it('reads the cost again when it opens, with the limit, so both count the title', async () => {
    // At the turn's end the ledger holds the turn alone; by the time the card opens the title's
    // side call has landed in the same model's row, and the limit counts its tokens.
    mocks.getUsage
      .mockResolvedValueOnce({ data: { models: [turn] } })
      .mockResolvedValue({ data: { models: [turnAndTitle] } });
    mocks.getLimit
      .mockResolvedValueOnce({ data: limitState(93_400) })
      .mockResolvedValue({ data: limitState(186_151) });

    render(<Footer />);
    const figure = await screen.findByTestId('chat-cost');
    await waitFor(() => expect(figure).toHaveTextContent('$0.0769'));

    fireEvent.click(figure, { detail: 1 });
    await waitFor(() => expect(card()?.getAttribute('data-state')).toBe('open'));
    const dialog = screen.getByRole('dialog', { name: 'Cost' });

    await waitFor(() => expect(dialog).toHaveTextContent('$0.0862'));
    expect(screen.getByTestId('chat-cost')).toHaveTextContent('$0.0862');
    expect(within(dialog).getByTestId('usage-limit-section')).toHaveTextContent(
      '186,151 tokens used'
    );
    expect(dialog).toHaveTextContent('185,745 input');
    expect(mocks.getUsage).toHaveBeenCalledTimes(2);
    for (const call of mocks.getUsage.mock.calls) {
      expect(call[0]).toMatchObject({
        path: { session_id: 'chat-a' },
        headers: { 'X-User-Action': 'proof' },
      });
    }
  });
});
