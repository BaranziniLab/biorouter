import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import BioRouterMessage from './BioRouterMessage';
import type { Message } from '../api';

const mockDivergeSession = vi.fn();
vi.mock('../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  divergeSession: (...args: unknown[]) => mockDivergeSession(...args),
}));
vi.mock('../toasts', () => ({ toastError: vi.fn() }));
vi.mock('../utils/sessionListCache', () => ({ notifySessionListChanged: vi.fn() }));
vi.mock('./MarkdownContent', () => ({
  default: ({ content }: { content: string }) => <div data-testid="markdown">{content}</div>,
}));

const noopOpenArtifact = vi.fn();

function answer(): Message {
  return {
    id: 'assistant-row-7',
    role: 'assistant',
    created: 1788560867,
    metadata: { userVisible: true, agentVisible: true },
    content: [{ type: 'text', text: 'Here is the answer.' }],
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mockDivergeSession.mockResolvedValue({
    data: { sessionId: '20260906_2', workingDir: '/w', name: 'Chat (branch 1)' },
  });
  // @ts-expect-error test shim
  window.electron = { createChatWindow: vi.fn(), createDivergedChatWindow: vi.fn() };
});

describe('Branch control anchors (issue #167)', () => {
  it('sends the message id AND its timestamp, so the daemon can cross-check them', async () => {
    // The daemon treats the id as the branch point but validates it against the
    // timestamp: an id resolving to a message OLDER than `truncateAfter` is
    // stale, and branching there is what discarded the conversation in #167.
    // That safety net only arms when both halves travel together, so the pair
    // is pinned here rather than left to the call site.
    render(
      <BioRouterMessage
        sessionId="20260906_1"
        message={answer()}
        messages={[answer()]}
        toolCallNotifications={new Map()}
        onRunInTerminal={null}
        onOpenArtifact={noopOpenArtifact}
      />
    );

    await userEvent.click(screen.getByRole('button', { name: /branch/i }));

    await waitFor(() =>
      expect(mockDivergeSession).toHaveBeenCalledWith(
        expect.objectContaining({
          path: { session_id: '20260906_1' },
          body: { truncateAfter: 1788560867, truncateAfterId: 'assistant-row-7' },
        })
      )
    );
  });
});
