import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useEffect } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Message, MessageEvent, ModelUsageRow, Session, TokenState } from '../api';

/**
 * The footer's cost in a LIVE chat, on the real pieces BaseChat joins: the chat's
 * stream store (`useChatStream`), the usage hook, `footerCostInputs` and the
 * footer's `CostTracker`, wired the way BaseChat wires them
 * (`BaseChat.summaryRail.test.ts` pins that wiring at its source, since BaseChat
 * itself cannot be mounted in jsdom).
 *
 * Measured in the running app before the fix: after a new chat's first reply
 * the cost card had no By model list (the ledger read on open had no rows, so
 * the figure was the counters at the current model's rate), and after more
 * turns it stayed at $0.12 while the chat's counters moved on; only a reload
 * showed $0.18. The ledger was keyed on the `session` row, and in a live chat
 * that row is the one the chat was opened with: only the stream's `tokenState`
 * moves.
 */
const mocks = vi.hoisted(() => ({
  observeSessionEvents: vi.fn(),
  reply: vi.fn(),
  resumeAgent: vi.fn(),
  interrupt: vi.fn(),
  cancelTurn: vi.fn(),
  getSession: vi.fn(),
  listSessions: vi.fn(),
  updateFromSession: vi.fn(),
  updateSessionUserWorkflowValues: vi.fn(),
  getSessionUsage: vi.fn(),
  pricing: vi.fn(),
}));

vi.mock('../api', async (importOriginal) => {
  const { pricing: _pricing, ...api } = mocks;
  return { ...((await importOriginal()) as Record<string, unknown>), ...api };
});
vi.mock('../utils/pricing', () => ({ fetchModelPricing: mocks.pricing }));
vi.mock('../utils/userAction', async (importOriginal) => ({
  ...((await importOriginal()) as Record<string, unknown>),
  userActionHeaders: async () => ({ 'X-User-Action': 'synthetic-proof' }),
}));
vi.mock('./ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({ currentModel: 'claude-opus-4-5', currentProvider: 'anthropic' }),
}));

import { defaultChatStreamRegistry } from '../hooks/chatStreamStore';
import { useChatStream } from '../hooks/useChatStream';
import { useCostTracking } from '../hooks/useCostTracking';
import { footerCostInputs } from './BaseChat';
import { CostTracker } from './bottom_menu/CostTracker';
import { COST_CARD_COPY } from './bottom_menu/copy';

/** The chat's lifetime counters as the daemon reports them in `tokenState`. */
function counters(input: number, output: number): TokenState {
  return {
    accumulatedInputTokens: input,
    accumulatedOutputTokens: output,
    accumulatedTotalTokens: input + output,
    inputTokens: input,
    outputTokens: output,
    totalTokens: input + output,
  };
}

/** The ledger's one row for the model that served every turn. */
function ledgerRow(input: number, output: number, turns: number): ModelUsageRow {
  return {
    provider: 'anthropic',
    modelId: 'claude-opus-4-5',
    inputTokens: input,
    outputTokens: output,
    cacheReadTokens: 0,
    cacheCreationTokens: 0,
    totalTokens: input + output,
    turns,
  };
}

/**
 * The row the chat is opened with. `user_set_name` keeps the store's post-turn
 * rename poll out of the test; nothing here is about names.
 */
function openedRow(id: string, input: number | null, output: number | null): Session {
  return {
    id,
    name: 'Live cost',
    working_dir: '/tmp',
    conversation: [],
    message_count: 0,
    total_tokens: 0,
    accumulated_input_tokens: input,
    accumulated_output_tokens: output,
    accumulated_total_tokens: input === null || output === null ? null : input + output,
    created_at: '',
    updated_at: '',
    extension_data: {},
    user_set_name: true,
  } as unknown as Session;
}

function assistantText(id: string, text: string): Message {
  return {
    id,
    role: 'assistant',
    created: 1,
    content: [{ type: 'text', text }],
    metadata: { userVisible: true, agentVisible: true },
  };
}

/** A reply stream whose frames the test drives, one committed frame at a time. */
function controlledStream() {
  const events: MessageEvent[] = [];
  let wake: (() => void) | null = null;
  async function* stream() {
    for (;;) {
      if (events.length === 0) await new Promise<void>((resolve) => (wake = resolve));
      const event = events.shift();
      if (event) yield event;
    }
  }
  return {
    stream: stream(),
    push(event: MessageEvent) {
      events.push(event);
      wake?.();
      wake = null;
    },
  };
}

/** What the daemon's ledger holds right now; a turn writes it before its counters move. */
let ledgerNow: ModelUsageRow[] = [];
let mounts = 0;
let sessionSeq = 0;
const onStreamFinish = () => {};

/** BaseChat's wiring, verbatim, around the footer's cost. */
function LiveChat({ sessionId }: { sessionId: string }) {
  const { session, tokenState } = useChatStream({ sessionId, onStreamFinish });
  const { sessionCosts, modelRows, ledgerSettled } = useCostTracking({
    session,
    liveTokens: tokenState,
  });
  const footer = footerCostInputs({
    ready: !!session && ledgerSettled,
    tokenState,
    session,
    sessionCosts,
    modelRows,
  });
  useEffect(() => {
    mounts += 1;
  }, []);
  return (
    <CostTracker
      inputTokens={footer.accumulatedInputTokens}
      outputTokens={footer.accumulatedOutputTokens}
      sessionCosts={footer.sessionCosts}
      modelCostRows={footer.modelCostRows}
    />
  );
}

const usageReads = (sessionId: string) =>
  mocks.getSessionUsage.mock.calls.filter(
    (call) => (call[0] as { path: { session_id: string } }).path.session_id === sessionId
  );
const figure = () => screen.getByTestId('chat-cost');
const card = () => screen.getByRole('dialog', { name: COST_CARD_COPY.cardLabel });
const byModel = () =>
  within(within(card()).getByRole('list', { name: COST_CARD_COPY.byModel })).getAllByRole(
    'listitem'
  );

/** Open the chat; its first ledger read is the open's. */
async function openChat(row: Session) {
  mocks.resumeAgent.mockResolvedValue({ data: { session: row } });
  render(<LiveChat sessionId={row.id} />);
  await waitFor(() => expect(usageReads(row.id)).toHaveLength(1));
  return defaultChatStreamRegistry.getController(row.id);
}

/** Start a turn on a stream the test drives. */
async function startTurn(sessionId: string) {
  const turn = controlledStream();
  const sent = mocks.reply.mock.calls.length;
  mocks.reply.mockResolvedValueOnce({ stream: turn.stream });
  void defaultChatStreamRegistry.getController(sessionId).handleSubmit('Summarise the cohort.');
  await waitFor(() => expect(mocks.reply).toHaveBeenCalledTimes(sent + 1));
  return {
    /** Each streamed token is a frame carrying a copy of the counters the stream has. */
    async tokens(count: number, state: TokenState) {
      for (let token = 0; token < count; token += 1) {
        await act(async () => {
          turn.push({
            type: 'Message',
            message: assistantText(`reply-${sent}`, `word${token} `),
            token_state: state,
          } as MessageEvent);
        });
      }
    },
    async finish(state: TokenState) {
      await act(async () => {
        turn.push({ type: 'Finish', reason: 'stop', token_state: state } as MessageEvent);
      });
    },
  };
}

beforeEach(() => {
  for (const mock of Object.values(mocks)) mock.mockReset();
  mocks.getSession.mockResolvedValue({ data: null });
  mocks.listSessions.mockResolvedValue({ data: { sessions: [] } });
  mocks.updateFromSession.mockResolvedValue({ data: {} });
  mocks.updateSessionUserWorkflowValues.mockResolvedValue({ data: {} });
  mocks.getSessionUsage.mockImplementation(async () => ({ data: { models: ledgerNow } }));
  // $5 per million tokens either way: 24,000 tokens is $0.12, 36,000 is $0.18.
  mocks.pricing.mockResolvedValue({
    input_token_cost: 0.000005,
    output_token_cost: 0.000005,
    currency: '$',
  });
  ledgerNow = [];
  mounts = 0;
});

describe('the footer’s cost in a live chat', () => {
  it('shows By model after the first reply and the new total after every turn, with no reload', async () => {
    const user = userEvent.setup();
    const sid = `live-cost-new-${++sessionSeq}`;
    await openChat(openedRow(sid, null, null));

    // Turn 1: the reply streams at the counters the chat opened with, and its
    // usage lands with the turn's end.
    const first = await startTurn(sid);
    await first.tokens(10, counters(0, 0));
    ledgerNow = [ledgerRow(20_000, 4_000, 1)];
    await first.finish(counters(20_000, 4_000));
    await waitFor(() => expect(figure()).toHaveTextContent('$0.12'));
    await user.click(figure());
    await waitFor(() => expect(byModel()).toHaveLength(1));
    expect(byModel()[0]).toHaveTextContent('anthropic/claude-opus-4-5');
    expect(byModel()[0]).toHaveTextContent('$0.12 · 20,000 in · 4,000 out');

    // Turn 2, with two provider calls: each records its usage, and the card,
    // still open from turn 1, follows both.
    const second = await startTurn(sid);
    await second.tokens(10, counters(20_000, 4_000));
    ledgerNow = [ledgerRow(25_000, 5_000, 2)];
    await second.tokens(10, counters(25_000, 5_000));
    await waitFor(() => expect(figure()).toHaveTextContent('$0.15'));
    ledgerNow = [ledgerRow(30_000, 6_000, 3)];
    await second.finish(counters(30_000, 6_000));
    await waitFor(() => expect(figure()).toHaveTextContent('$0.18'));
    expect(within(card()).getByText('$0.18')).toHaveClass('br-footer-card__value');
    expect(byModel()).toHaveLength(1);
    expect(byModel()[0]).toHaveTextContent('$0.18 · 30,000 in · 6,000 out');

    expect(mounts).toBe(1);
  });

  it('moves on from the total a chat was opened with', async () => {
    const sid = `live-cost-opened-${++sessionSeq}`;
    ledgerNow = [ledgerRow(20_000, 4_000, 1)];
    await openChat(openedRow(sid, 20_000, 4_000));
    await waitFor(() => expect(figure()).toHaveTextContent('$0.12'));

    const turn = await startTurn(sid);
    await turn.tokens(5, counters(20_000, 4_000));
    ledgerNow = [ledgerRow(30_000, 6_000, 2)];
    await turn.finish(counters(30_000, 6_000));
    // It stayed at $0.12 here until a reload.
    await waitFor(() => expect(figure()).toHaveTextContent('$0.18'));
    expect(mounts).toBe(1);
  });

  it('reads the ledger once per change of the counters, never once per streamed token', async () => {
    const sid = `live-cost-tokens-${++sessionSeq}`;
    await openChat(openedRow(sid, null, null));

    const turn = await startTurn(sid);
    await turn.tokens(40, counters(0, 0));
    await act(async () => {});
    expect(usageReads(sid)).toHaveLength(1);

    // A provider call recorded its usage: the next frame carries the new counters.
    ledgerNow = [ledgerRow(20_000, 4_000, 1)];
    await turn.tokens(40, counters(20_000, 4_000));
    await waitFor(() => expect(usageReads(sid)).toHaveLength(2));
    // The turn's end reports the counters the stream already has.
    await turn.finish(counters(20_000, 4_000));
    await act(async () => {});
    expect(usageReads(sid)).toHaveLength(2);
    for (const [options] of usageReads(sid)) {
      expect(options).toMatchObject({ headers: { 'X-User-Action': 'synthetic-proof' } });
    }
  });
});
