import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import BioRouterMessage from './BioRouterMessage';
import type { Message } from '../api';
import { transcriptCopy } from './transcript/copy';

// The transcript is no longer a display surface for artifacts — it can only hand
// one to the panel — so `onOpenArtifact` is required all the way down the chain.
const noopOpenArtifact = vi.fn();

vi.mock('./MarkdownContent', () => ({
  default: ({ content }: { content: string }) => <div data-testid="markdown">{content}</div>,
}));

function thinkingMessage(): Message {
  return {
    id: 'm1',
    role: 'assistant',
    created: 1700000000000,
    metadata: { userVisible: true, agentVisible: true },
    content: [{ type: 'text', text: '<think>weighing two options</think>Here is the answer.' }],
  };
}

function renderMessage(message: Message, isStreaming = false) {
  return render(
    <BioRouterMessage
      sessionId="s1"
      message={message}
      messages={[message]}
      toolCallNotifications={new Map()}
      isStreaming={isStreaming}
      onRunInTerminal={null}
      onOpenArtifact={noopOpenArtifact}
    />
  );
}

describe('BioRouterMessage chain-of-thought disclosure', () => {
  it("is the transcript's own row, not the browser's <details>", () => {
    const { container } = renderMessage(thinkingMessage());

    // A native <details> could not take the app's chevron, its row, its type
    // role or its motion: the marker was drawn by the user agent. It is the
    // one transcript row now (TranscriptRow), the same line a tool call is.
    expect(container.querySelector('details')).toBeNull();
    const toggle = screen.getByRole('button', { name: transcriptCopy.activity.thought });
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    expect(toggle.closest('.br-transcript-row')).not.toBeNull();
    expect(container.querySelector('[data-tool-glyph="thinking"]')).not.toBeNull();
  });

  it('keeps the chevron visible at rest and shows the thought only when asked', async () => {
    renderMessage(thinkingMessage());

    const toggle = screen.getByRole('button', { name: transcriptCopy.activity.thought });
    // Visible at rest: a marker that only appears on hover teaches nobody that
    // the row opens.
    expect(toggle.querySelector('.br-transcript-row-chevron')).not.toBeNull();

    // The thought itself is not in the DOM until it is asked for.
    expect(screen.queryByText('weighing two options')).not.toBeInTheDocument();

    await userEvent.click(toggle);

    expect(screen.getByText('weighing two options')).toBeInTheDocument();
    expect(toggle).toHaveAttribute('aria-expanded', 'true');
  });

  it('says "Thinking" and breathes while the thought is all there is', () => {
    const message: Message = {
      ...thinkingMessage(),
      content: [{ type: 'text', text: '<think>weighing two options</think>' }],
    };
    const { container } = renderMessage(message, true);

    expect(
      screen.getByRole('button', { name: transcriptCopy.activity.thinking })
    ).toBeInTheDocument();
    expect(container.querySelector('.br-transcript-row')).toHaveAttribute('data-running');
  });

  it('says "Thought", at rest, once the answer has started', () => {
    const { container } = renderMessage(thinkingMessage(), true);

    expect(
      screen.getByRole('button', { name: transcriptCopy.activity.thought })
    ).toBeInTheDocument();
    expect(container.querySelector('.br-transcript-row')).not.toHaveAttribute('data-running');
  });

  it('says "Thought", at rest, once the model has moved on to a tool call', () => {
    // The turn is still streaming (a tool is running), but the thought is
    // over: the tool row is what is working, not the thinking row.
    const message: Message = {
      ...thinkingMessage(),
      content: [
        { type: 'text', text: '<think>weighing two options</think>' },
        {
          type: 'toolRequest',
          id: 'tool-1',
          toolCall: {
            status: 'success',
            value: { name: 'developer__shell', arguments: { command: 'ls' } },
          },
        },
      ],
    } as Message;
    const { container } = renderMessage(message, true);

    expect(
      screen.getByRole('button', { name: transcriptCopy.activity.thought })
    ).toBeInTheDocument();
    expect(
      container.querySelector('[data-tool-glyph="thinking"]')?.closest('.br-transcript-row')
    ).not.toHaveAttribute('data-running');
  });

  it('times a thought it watched: "Thinking · 9s", then "Thought · 9s"', () => {
    vi.useFakeTimers();
    try {
      const thinking: Message = {
        ...thinkingMessage(),
        content: [{ type: 'text', text: '<think>weighing two options</think>' }],
      };
      const view = renderMessage(thinking, true);
      act(() => {
        vi.advanceTimersByTime(9000);
      });
      expect(screen.getByRole('button', { name: /^Thinking ?· 9s$/ })).toBeInTheDocument();

      // The answer starts: the clock stops where it was.
      view.rerender(
        <BioRouterMessage
          sessionId="s1"
          message={thinkingMessage()}
          messages={[thinkingMessage()]}
          toolCallNotifications={new Map()}
          isStreaming
          onRunInTerminal={null}
          onOpenArtifact={noopOpenArtifact}
        />
      );
      act(() => {
        vi.advanceTimersByTime(5000);
      });
      expect(screen.getByRole('button', { name: /^Thought ?· 9s$/ })).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it('gives a thought read from history no length it never measured', () => {
    renderMessage(thinkingMessage(), false);
    expect(
      screen.getByRole('button', { name: transcriptCopy.activity.thought })
    ).toBeInTheDocument();
  });
});
