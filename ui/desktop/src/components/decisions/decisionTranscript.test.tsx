/**
 * Decision models in the real transcript list: a decision chat's reply is one
 * card (with its text until the row loads), a turn's decision calls are ONE
 * marker line in place of their notice, and a long run of calls never becomes
 * a long run of rows (owner addendum A1).
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ProgressiveMessageList from '../ProgressiveMessageList';
import type { Message } from '../../api';
import { resetDecisionRecordCacheForTests } from './decisionChatApi';
import { wireBinaryDetail } from './decisionFixtures';

const mocks = vi.hoisted(() => ({ get: vi.fn(), headers: vi.fn() }));
vi.mock('../../api/client.gen', () => ({ client: { get: mocks.get, put: vi.fn() } }));
vi.mock('../../utils/userAction', () => ({ userActionHeaders: mocks.headers }));

beforeEach(() => {
  resetDecisionRecordCacheForTests();
  mocks.get
    .mockReset()
    .mockResolvedValue({ data: wireBinaryDetail(), error: undefined, response: { status: 200 } });
  mocks.headers.mockReset().mockResolvedValue({ 'X-User-Action': 'synthetic-proof' });
});

const userMessage: Message = {
  id: 'u1',
  role: 'user',
  created: 1_700_000_000,
  metadata: { userVisible: true, agentVisible: true },
  content: [{ type: 'text', text: 'Neuro f/u. 34F, RRMS dx 2019' }],
} as Message;

const decisionReply = {
  id: 'a1',
  role: 'assistant',
  created: 1_700_000_001,
  metadata: { userVisible: true, agentVisible: true, decisionId: 'dec-1' },
  content: [{ type: 'text', text: 'Yes · P(yes)=0.93 · tev1:0.8b' }],
} as unknown as Message;

const turnMarker = {
  id: 'n1',
  role: 'assistant',
  created: 1_700_000_002,
  metadata: {
    userVisible: true,
    agentVisible: false,
    decisionSummary: {
      turnId: 'turn-7',
      items: [
        {
          origin: 'monitor',
          originId: 'form',
          label: 'Form reached',
          count: 1800,
          passed: 0,
          failed: 1800,
          errors: 0,
          lastPassProbability: 0.31,
          fired: false,
        },
      ],
    },
  },
  content: [
    {
      type: 'systemNotification',
      notificationType: 'inlineMessage',
      msg: 'Monitors checked 1800 times this turn',
    },
  ],
} as unknown as Message;

function show(messages: Message[]) {
  const onOpenArtifact = vi.fn();
  const view = render(
    <ProgressiveMessageList
      messages={messages}
      chat={{ sessionId: 'chat-1' }}
      toolCallNotifications={new Map()}
      isUserMessage={(m: Message) => m.role === 'user'}
      onOpenArtifact={onOpenArtifact}
      onRunInTerminal={null}
    />
  );
  return { ...view, onOpenArtifact };
}

describe('decision models in the transcript', () => {
  it('draws a decision chat’s reply as one card, from its text until the row loads', async () => {
    show([userMessage, decisionReply]);
    expect(screen.getByTestId('decision-card-fallback')).toHaveTextContent('P(yes)=0.93');
    const card = await screen.findByTestId('decision-card');
    expect(card).toHaveTextContent('Yes');
    expect(screen.getAllByTestId('decision-card')).toHaveLength(1);
    expect(mocks.get).toHaveBeenCalledWith(
      expect.objectContaining({ path: { session_id: 'chat-1', decision_id: 'dec-1' } })
    );
  });

  it('draws a turn’s decision calls as one marker line in place of the notice', () => {
    const { container } = show([userMessage, turnMarker]);
    const marker = screen.getByTestId('decision-turn-marker');
    expect(marker).toHaveTextContent('Monitor checked 1,800 times, quiet');
    expect(marker).not.toHaveTextContent('Decision models');
    expect(screen.queryByText('Monitors checked 1800 times this turn')).toBeNull();
    // Two messages, two rows: 1,800 calls added one line, not 1,800.
    expect(container.querySelectorAll('[data-testid="message-container"]')).toHaveLength(2);
  });

  it('hands the marker’s history links to the chat’s artifact panel', () => {
    const { onOpenArtifact } = show([userMessage, turnMarker]);
    fireEvent.click(screen.getByRole('button', { name: 'Monitor checked 1,800 times, quiet' }));
    fireEvent.click(
      screen.getByRole('button', { name: 'Open Form reached in the decision history' })
    );
    expect(onOpenArtifact).toHaveBeenLastCalledWith(
      expect.objectContaining({
        kind: 'decisions',
        filter: { turnId: 'turn-7', origin: 'monitor', originId: 'form' },
      })
    );
    fireEvent.click(screen.getByRole('button', { name: 'Open this turn in the decision history' }));
    expect(onOpenArtifact).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: 'decisions',
        sessionId: 'chat-1',
        filter: { turnId: 'turn-7' },
      })
    );
  });

  it('leaves an ordinary notice as it was', () => {
    const plain = {
      ...turnMarker,
      id: 'n2',
      metadata: { userVisible: true, agentVisible: false },
    } as unknown as Message;
    show([userMessage, plain]);
    expect(screen.getByText('Monitors checked 1800 times this turn')).toBeInTheDocument();
    expect(screen.queryByTestId('decision-turn-marker')).toBeNull();
  });
});
