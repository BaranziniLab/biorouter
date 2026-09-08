import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import BioRouterMessage from './BioRouterMessage';
import type { Message } from '../api';

const noopOpenArtifact = vi.fn();

function shellMessage(): Message {
  return {
    id: 'm1',
    role: 'assistant',
    created: 1700000000000,
    metadata: { userVisible: true, agentVisible: true },
    content: [{ type: 'text', text: 'Try this:\n\n```bash\nls -la\n```' }],
  };
}

function view(props: {
  onRunInTerminal: ((command: string) => boolean) | null;
  isStreaming?: boolean;
}) {
  const message = shellMessage();
  return (
    <BioRouterMessage
      sessionId="s1"
      message={message}
      messages={[message]}
      toolCallNotifications={new Map()}
      onOpenArtifact={noopOpenArtifact}
      onRunInTerminal={props.onRunInTerminal}
      isStreaming={props.isStreaming ?? false}
    />
  );
}

describe('BioRouterMessage — running a shell block from the transcript', () => {
  it('offers Run on assistant prose when the chat has a terminal', async () => {
    const onRunInTerminal = vi.fn(() => true);
    render(view({ onRunInTerminal }));

    await userEvent.click(screen.getByRole('button', { name: /^run$/i }));

    expect(onRunInTerminal).toHaveBeenCalledExactlyOnceWith('ls -la');
  });

  it('offers nothing on a surface that declared it has no terminal', () => {
    render(view({ onRunInTerminal: null }));

    expect(screen.getByRole('button', { name: /^copy$/i })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^run$/i })).not.toBeInTheDocument();
  });

  it('withholds Run while the message is still streaming', () => {
    // A code fence that has not closed yet holds HALF a command, and half a
    // command is a different command — `rm -rf /tmp/build-cache` truncated
    // after `rm -rf /tmp` is the shape of the accident. Copy has always been
    // safe to offer mid-stream because it does not submit anything; Run is not,
    // so it follows onOpenArtifact's precedent and waits for the turn.
    render(view({ onRunInTerminal: vi.fn(() => true), isStreaming: true }));

    expect(screen.getByRole('button', { name: /^copy$/i })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^run$/i })).not.toBeInTheDocument();
  });
});
