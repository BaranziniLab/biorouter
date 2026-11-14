import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import BioRouterMessage from './BioRouterMessage';
import type { Message } from '../api';
import { transcriptCopy } from './transcript/copy';
import { StoredTurnErrorRetryContext } from './conversation/ChatTurnError';

vi.mock('./MarkdownContent', () => ({
  default: ({ content }: { content: string }) => <div data-testid="markdown">{content}</div>,
}));

const RAW =
  'Authentication error: Authentication failed. Status: 401 Unauthorized. Response: {"error":"Invalid client id or secret"}.';
const ADVICE =
  'Retrying will not help until the key or sign-in is fixed. Once it is, send your message again: this chat reads the new credentials with its next message.';

function assistant(text: string): Message {
  return {
    id: 'm1',
    role: 'assistant',
    created: 1700000000000,
    metadata: { userVisible: true, agentVisible: true },
    content: [{ type: 'text', text }],
  };
}

function renderMessage(message: Message, live = false) {
  return render(
    <BioRouterMessage
      sessionId="s1"
      message={message}
      messages={[message]}
      toolCallNotifications={new Map()}
      onRunInTerminal={null}
      onOpenArtifact={vi.fn()}
      live={live}
    />
  );
}

// Vision V2-07: a provider failure has one look. The daemon stores its
// turn-ending notice as an assistant message; it draws as the error row (spec
// 3.6 "Errors"), never as prose holding the raw provider response.
describe('BioRouterMessage stored turn error', () => {
  it('draws the daemon notice as the error row, the raw response behind Details', () => {
    const { container } = renderMessage(assistant(`Ran into this error: ${RAW}\n\n${ADVICE}`));

    expect(screen.queryByTestId('markdown')).toBeNull();
    const row = screen.getByTestId('stored-turn-error');
    expect(row).toHaveTextContent('Model authentication failed');
    // The error's first sentence is the line; the rest folds, word for word, under Details
    // with the raw response (design.md 9.16).
    expect(row).toHaveTextContent('Authentication failed.');
    expect(row).not.toHaveTextContent('Retrying will not help');
    expect(screen.queryByText(/Invalid client id/)).toBeNull();
    const details = screen.getByRole('button', { name: transcriptCopy.turn.details });
    fireEvent.click(details);
    expect(row).toHaveTextContent(`Status: 401 Unauthorized. ${ADVICE}`);
    expect(screen.getByText(/Invalid client id/)).toBeInTheDocument();
    // History reads it back quietly; no floating message actions on an error.
    expect(row).not.toHaveAttribute('role', 'alert');
    expect(container.querySelector('.br-message-actions-scope')).toBeNull();
  });

  it('announces a notice that arrives while the person watches', () => {
    renderMessage(assistant(`Ran into this error: ${RAW}\n\n${ADVICE}`), true);
    expect(screen.getByRole('alert')).toHaveTextContent('Model authentication failed');
  });

  // When the chat's live failure is this notice, the live row is not drawn and
  // its Retry rides here, so the failure shows once and can still be retried.
  it('carries the live Retry only on the notice the chat is showing', () => {
    const onRetry = vi.fn();
    const message = assistant(`Ran into this error: ${RAW}\n\n${ADVICE}`);
    const { rerender } = render(
      <StoredTurnErrorRetryContext.Provider value={{ messageId: 'm1', onRetry }}>
        <BioRouterMessage
          sessionId="s1"
          message={message}
          messages={[message]}
          toolCallNotifications={new Map()}
          onRunInTerminal={null}
          onOpenArtifact={vi.fn()}
        />
      </StoredTurnErrorRetryContext.Provider>
    );
    fireEvent.click(screen.getByTestId('chat-turn-error-retry'));
    expect(onRetry).toHaveBeenCalledTimes(1);

    rerender(
      <StoredTurnErrorRetryContext.Provider value={{ messageId: 'another', onRetry }}>
        <BioRouterMessage
          sessionId="s1"
          message={message}
          messages={[message]}
          toolCallNotifications={new Map()}
          onRunInTerminal={null}
          onOpenArtifact={vi.fn()}
        />
      </StoredTurnErrorRetryContext.Provider>
    );
    expect(screen.queryByTestId('chat-turn-error-retry')).toBeNull();
  });

  it('leaves an ordinary answer to markdown', () => {
    renderMessage(assistant('Here is the answer.'));
    expect(screen.getByTestId('markdown')).toHaveTextContent('Here is the answer.');
    expect(screen.queryByTestId('stored-turn-error')).toBeNull();
  });
});
