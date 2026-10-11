import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import BioRouterMessage from './BioRouterMessage';
import type { Message } from '../api';
import { transcriptCopy } from './transcript/copy';

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
    expect(row).toHaveTextContent(`Authentication failed. Status: 401 Unauthorized. ${ADVICE}`);
    expect(screen.queryByText(/Invalid client id/)).toBeNull();
    expect(screen.getByRole('button', { name: transcriptCopy.turn.details })).toBeInTheDocument();
    // History reads it back quietly; no floating message actions on an error.
    expect(row).not.toHaveAttribute('role', 'alert');
    expect(container.querySelector('.br-message-actions-scope')).toBeNull();
  });

  it('announces a notice that arrives while the person watches', () => {
    renderMessage(assistant(`Ran into this error: ${RAW}\n\n${ADVICE}`), true);
    expect(screen.getByRole('alert')).toHaveTextContent('Model authentication failed');
  });

  it('leaves an ordinary answer to markdown', () => {
    renderMessage(assistant('Here is the answer.'));
    expect(screen.getByTestId('markdown')).toHaveTextContent('Here is the answer.');
    expect(screen.queryByTestId('stored-turn-error')).toBeNull();
  });
});
