import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import SessionHistoryView from './SessionHistoryView';
import { SHARE_FAILED } from './copy';
import type { Message, Session } from '../../api';

/**
 * What the daemon answers when a Crew grant restricts the chat, as the generated client throws it
 * under `throwOnError`: the parsed plain-text body. Mirrored from `CREW_EXPORT_REFUSAL` in
 * `crates/biorouter/src/session/session_manager.rs`.
 */
const CREW_EXPORT_REFUSAL =
  'Crew context cannot be exported without its channel permissions. Share an authorized ' +
  'message or attachment from Crew instead.';

const mocks = vi.hoisted(() => ({
  exportSession: vi.fn(),
  createSharedSession: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock('../../api', () => ({
  exportSession: mocks.exportSession,
  declassifySession: vi.fn(),
  getSession: vi.fn(),
}));

vi.mock('../../sharedSessions', () => ({
  createSharedSession: mocks.createSharedSession,
}));

vi.mock('../../toasts', () => ({ toastError: mocks.toastError }));

vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-proof' }),
}));

vi.mock('../ProgressiveMessageList', () => ({
  default: () => <div data-testid="messages" />,
}));

vi.mock('../conversation/SearchView', () => ({
  SearchView: ({ children }: { children: ReactNode }) => <>{children}</>,
}));

vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

function message(text: string): Message {
  return {
    id: text,
    role: 'user',
    created: 1,
    content: [{ type: 'text', text }],
    metadata: { userVisible: true, agentVisible: true },
  } as Message;
}

/** The page's own copy of a Crew chat, `<crew_context>` and all. */
const onPage = message('<crew_context>channel messages</crew_context> summarize this');

function renderChat() {
  render(
    <MemoryRouter>
      <SessionHistoryView
        session={
          {
            id: 'crew-chat',
            name: 'Crew chat',
            working_dir: '/tmp',
            created_at: '2026-09-27T12:00:00Z',
            updated_at: '2026-09-27T12:00:00Z',
            message_count: 1,
            extension_data: {},
            conversation: [onPage],
          } as Session
        }
        isLoading={false}
        error={null}
        onBack={vi.fn()}
        onRetry={vi.fn()}
      />
    </MemoryRouter>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.setItem(
    'session_sharing_config',
    JSON.stringify({ enabled: true, baseUrl: 'https://share.example.test' })
  );
  mocks.createSharedSession.mockResolvedValue('share-token');
});

afterEach(() => localStorage.removeItem('session_sharing_config'));

describe('SessionHistoryView Share', () => {
  it("refuses a Crew chat as Export does: the daemon's sentence, and nothing posted", async () => {
    mocks.exportSession.mockRejectedValue(CREW_EXPORT_REFUSAL);
    renderChat();
    fireEvent.click(screen.getByRole('button', { name: 'Share' }));

    await waitFor(() =>
      expect(mocks.toastError).toHaveBeenCalledWith({
        title: SHARE_FAILED,
        msg: CREW_EXPORT_REFUSAL,
      })
    );
    expect(mocks.exportSession).toHaveBeenCalledWith({
      path: { session_id: 'crew-chat' },
      headers: { 'X-User-Action': 'test-proof' },
      throwOnError: true,
    });
    expect(mocks.createSharedSession).not.toHaveBeenCalled();
  });

  it("shares the transcript the daemon's export answers, never the page's own copy", async () => {
    const exported = message('what the export door read');
    mocks.exportSession.mockResolvedValue({
      data: JSON.stringify({ id: 'crew-chat', conversation: [exported] }),
    });
    renderChat();
    fireEvent.click(screen.getByRole('button', { name: 'Share' }));

    await waitFor(() => expect(mocks.createSharedSession).toHaveBeenCalledTimes(1));
    const [, , messages] = mocks.createSharedSession.mock.calls[0];
    expect(messages).toEqual([exported]);
    expect(JSON.stringify(messages)).not.toContain('crew_context');
  });

  it('shares nothing when the export answers something that is not a chat', async () => {
    mocks.exportSession.mockResolvedValue({ data: 'not json' });
    renderChat();
    fireEvent.click(screen.getByRole('button', { name: 'Share' }));

    await waitFor(() =>
      expect(mocks.toastError).toHaveBeenCalledWith({
        title: SHARE_FAILED,
        msg: 'The chat could not be read for sharing. Nothing was shared.',
      })
    );
    expect(mocks.createSharedSession).not.toHaveBeenCalled();
  });
});
