import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import BioRouterMessage from '../BioRouterMessage';
import type { Message } from '../../api';
import type { ChatTurnErrorData } from '../../types/turnError';
import { ChatTurnError, placeTurnError, StoredTurnErrorRetryContext } from './ChatTurnError';

vi.mock('../MarkdownContent', () => ({
  default: ({ content }: { content: string }) => <div data-testid="markdown">{content}</div>,
}));

// Live QA, 2026-11-11 (W3 10-s1-running.png): one Versa content-filter failure
// drew twice, the stored "Ran into this error" notice and then the live row
// with Retry, because a decision monitor's marker landed after the notice.
// Retry then cleared the error and sent nothing.

const FAILURE =
  'Server error: Provider content filter failed (content_filter_error). Retry the request or contact your provider administrator.';
const NOTICE = `Ran into this error: ${FAILURE}\n\nPlease retry if you think this is a transient or recoverable error. Biorouter already retried it once.`;

function row(
  id: string,
  role: 'user' | 'assistant',
  content: Message['content'],
  metadata: Message['metadata'] = { userVisible: true, agentVisible: true }
): Message {
  return { id, role, created: 1, content, metadata };
}

const prompt = row('prompt', 'user', [{ type: 'text', text: 'Read the 30 synthetic abstracts.' }]);
const notice = row('notice', 'assistant', [{ type: 'text', text: NOTICE }]);
// The marker is a user-only notice, drawn by the transcript as one line.
const marker = row(
  'marker',
  'assistant',
  [
    {
      type: 'systemNotification',
      notificationType: 'inlineMessage',
      msg: 'Monitor checked 3 times, quiet',
    },
  ],
  { userVisible: true, agentVisible: false }
);

function liveError(overrides: Partial<ChatTurnErrorData> = {}): ChatTurnErrorData {
  return {
    message: FAILURE,
    code: 'provider_failure',
    scope: 'provider',
    retryable: true,
    providerKind: 'server',
    ...overrides,
  } as ChatTurnErrorData;
}

/**
 * The transcript's tail as BaseChat composes it: every assistant row through
 * BioRouterMessage, inside the stored-notice Retry context, then the live row
 * only where `placeTurnError` says to draw it.
 */
function Transcript({
  messages,
  error,
  onRetry,
}: {
  messages: Message[];
  error: ChatTurnErrorData;
  onRetry: () => void;
}) {
  const placement = placeTurnError(error, messages);
  const stored = placement.retryOnNoticeId
    ? { messageId: placement.retryOnNoticeId, onRetry }
    : null;
  return (
    <StoredTurnErrorRetryContext.Provider value={stored}>
      {messages
        .filter((message) => message.role === 'assistant')
        .map((message) => (
          <BioRouterMessage
            key={message.id}
            sessionId="s1"
            message={message}
            messages={messages}
            toolCallNotifications={new Map()}
            onRunInTerminal={null}
            onOpenArtifact={vi.fn()}
            live
          />
        ))}
      {placement.row && <ChatTurnError error={placement.row} onRetry={onRetry} />}
    </StoredTurnErrorRetryContext.Provider>
  );
}

describe('one turn failure draws once', () => {
  it('draws the stored notice alone when a marker landed after it, and its Retry works', () => {
    const onRetry = vi.fn();
    render(
      <Transcript messages={[prompt, notice, marker]} error={liveError()} onRetry={onRetry} />
    );

    expect(screen.queryByTestId('chat-turn-error')).toBeNull();
    expect(screen.getAllByTestId('stored-turn-error')).toHaveLength(1);
    expect(screen.getAllByText('Model provider unavailable')).toHaveLength(1);
    expect(screen.getAllByRole('alert')).toHaveLength(1);

    const retry = screen.getAllByTestId('chat-turn-error-retry');
    expect(retry).toHaveLength(1);
    fireEvent.click(retry[0]);
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('draws the live row, with Retry, when no stored notice shows the failure', () => {
    const onRetry = vi.fn();
    render(<Transcript messages={[prompt]} error={liveError()} onRetry={onRetry} />);

    expect(screen.queryByTestId('stored-turn-error')).toBeNull();
    expect(screen.getAllByTestId('chat-turn-error')).toHaveLength(1);
    fireEvent.click(screen.getByTestId('chat-turn-error-retry'));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('never borrows a notice from an earlier turn', () => {
    const later = row('later', 'user', [{ type: 'text', text: 'Try again with fewer abstracts.' }]);
    const placement = placeTurnError(liveError(), [prompt, notice, later]);
    expect(placement.row).not.toBeNull();
    expect(placement.retryOnNoticeId).toBeNull();
  });

  it('shows a content-filter block once, titled by its kind, with no Retry', () => {
    const blocked =
      "Request failed: Azure's content filter blocked this reply (flagged: violence). Revise the request or contact your provider administrator.";
    const blockNotice = row('block', 'assistant', [
      {
        type: 'text',
        text: `Ran into this error: ${blocked}\n\nRetrying will not help: this one returns the same way until its cause changes.`,
      },
    ]);
    const onRetry = vi.fn();
    render(
      <Transcript
        messages={[prompt, blockNotice, marker]}
        error={liveError({ message: blocked, retryable: false, providerKind: 'policy' })}
        onRetry={onRetry}
      />
    );

    expect(screen.queryByTestId('chat-turn-error')).toBeNull();
    const stored = screen.getByTestId('stored-turn-error');
    expect(stored).toHaveTextContent('Request blocked by provider policy');
    expect(stored).toHaveTextContent(
      "Azure's content filter blocked this reply (flagged: violence)."
    );
    expect(screen.queryByTestId('chat-turn-error-retry')).toBeNull();
  });
});
