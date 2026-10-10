import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import BioRouterMessage from './BioRouterMessage';
import type { Message } from '../api';

// The message hands its tool block to `ToolCallRows` (spec 3.6 P2, D18), which
// folds two or more consecutive built-in reads, lists and searches into one
// "Explored" row. `ToolCallRows.test.tsx` owns the folding rules; this pins that
// the transcript's message actually draws through it.
const noopOpenArtifact = vi.fn();

const view = (id: string, path: string) => ({
  type: 'toolRequest' as const,
  id,
  toolCall: {
    status: 'success' as const,
    value: { name: 'developer__text_editor', arguments: { command: 'view', path } },
  },
});

const done = (id: string) => ({
  type: 'toolResponse' as const,
  id,
  toolResult: {
    status: 'success' as const,
    value: { content: [{ type: 'text', text: 'ok' }], isError: false },
  },
});

function assistantWith(...ids: string[]): Message {
  return {
    id: 'assistant-1',
    role: 'assistant',
    created: 1_700_000_000,
    metadata: { userVisible: true, agentVisible: true },
    content: ids.map((id) => view(id, `/work/${id}.txt`)),
  } as Message;
}

function responses(...ids: string[]): Message {
  return {
    id: 'user-1',
    role: 'user',
    created: 1_700_000_001,
    metadata: { userVisible: true, agentVisible: true },
    content: ids.map(done),
  } as Message;
}

function renderBlock(ids: string[]) {
  const message = assistantWith(...ids);
  return render(
    <BioRouterMessage
      sessionId="s1"
      message={message}
      messages={[message, responses(...ids)]}
      messageIndex={0}
      toolCallNotifications={new Map()}
      onRunInTerminal={null}
      onOpenArtifact={noopOpenArtifact}
    />
  );
}

describe('BioRouterMessage tool block', () => {
  it('folds a run of reads in one message into one Explored row', () => {
    const { container } = renderBlock(['t1', 't2', 't3']);

    expect(screen.getByRole('button', { name: /^Explored/ })).toBeInTheDocument();
    // One entry in the block, and it is the fold, not three lines.
    expect(container.querySelectorAll('.biorouter-message-tool')).toHaveLength(1);
  });

  it('leaves a single read on its own line', () => {
    const { container } = renderBlock(['t1']);

    expect(screen.queryByRole('button', { name: /^Explored/ })).toBeNull();
    expect(container.querySelectorAll('.biorouter-message-tool')).toHaveLength(1);
  });
});
