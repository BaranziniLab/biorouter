import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';

/**
 * F1 of the 2026-09-10 QA run, on the surface where it was measured: type into
 * the Home composer, press Enter, and have `POST /agent/start` fail. The
 * composer had already wiped itself, and the failure went to `console.error`
 * and nowhere else — no toast, no message, the text gone.
 *
 * The real Hub and the real ChatInput are rendered, because the property lives
 * in the handshake between them: ChatInput restores its box only when the
 * submit resolves `false`, and Hub decides what it resolves. Only the heavy or
 * context-hungry children are stubbed, as in `ChatInput.workingDir.test.tsx`.
 */

const { mockCreateSession, mockToastError } = vi.hoisted(() => ({
  mockCreateSession: vi.fn(),
  mockToastError: vi.fn(),
}));

vi.mock('../sessions', () => ({ createSession: mockCreateSession }));
vi.mock('../toasts', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../toasts')>()),
  toastError: mockToastError,
}));
vi.mock('./sessions/SessionsInsights', () => ({ SessionInsights: () => null }));
// The privacy-off note (H3) needs a router and two more ConfigContext hooks, and
// says nothing about starting a chat.
vi.mock('./privacy/PrivacyTiersOffNote', () => ({ PrivacyTiersOffNote: () => null }));
vi.mock('./ConfigContext', () => ({
  useConfig: () => ({
    extensionsList: [],
    getProviders: vi.fn(async () => []),
    read: vi.fn(async () => null),
  }),
}));
vi.mock('./ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: vi.fn(async () => ({ model: null, provider: null })),
    currentModel: null,
    currentProvider: null,
    currentModelSupportsVision: false,
    currentModelSupportedInputMimeTypes: null,
  }),
}));
vi.mock('../hooks/useDiverge', () => ({
  useDiverge: () => ({ diverge: vi.fn() }),
}));
vi.mock('./settings/models/bottom_bar/ModelsBottomBar', () => ({ default: () => null }));
vi.mock('./bottom_menu/BottomMenuExtensionSelection', () => ({
  BottomMenuExtensionSelection: () => null,
}));
vi.mock('./bottom_menu/BottomMenuSkillSelection', () => ({
  BottomMenuSkillSelection: () => null,
}));
vi.mock('./bottom_menu/BottomMenuKnowledgeSelection', () => ({
  BottomMenuKnowledgeSelection: () => null,
}));
vi.mock('./bottom_menu/BottomMenuReasoningEffort', () => ({
  BottomMenuReasoningEffort: () => null,
}));
vi.mock('./bottom_menu/CostTracker', () => ({ CostTracker: () => null }));
vi.mock('./MessageQueue', () => ({ default: () => null }));
vi.mock('./MentionPopover', () => {
  const MentionPopoverMock = React.forwardRef(() => null);
  MentionPopoverMock.displayName = 'MentionPopoverMock';
  return { default: MentionPopoverMock };
});
vi.mock('../api', () => ({
  getSession: vi.fn(async () => ({ data: null })),
  llamacppStatus: vi.fn(async () => ({ data: {} })),
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
}));

import Hub from './Hub';

/** What `POST /agent/start` answered on the QA run's `biorouter serve` daemon. */
const SERVE_DAEMON_REFUSAL = {
  message:
    "Switching this chat to a private model is the user's decision, not yours. The request to " +
    "switch it to 'versa_azure' did not come from the model picker, so the chat is unchanged and " +
    'still on its current model. Do not retry; the same call will be refused again.',
};

beforeEach(() => {
  vi.clearAllMocks();
  Object.assign(window, {
    appConfig: {
      get: (key: string) => (key === 'BIOROUTER_WORKING_DIR' ? '/default/workdir' : undefined),
    },
    electron: {
      directoryChooser: vi.fn(async () => ({ canceled: true, filePaths: [] })),
      addRecentDir: vi.fn(),
      logInfo: vi.fn(),
      getPathForFile: vi.fn(() => ''),
      on: vi.fn(),
      off: vi.fn(),
    },
  });
});

describe('Hub: a chat that fails to start', () => {
  it('says so in words for a person, and the composer keeps what was typed', async () => {
    mockCreateSession.mockRejectedValueOnce(SERVE_DAEMON_REFUSAL);
    const setView = vi.fn();
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
    render(<Hub setView={setView} />);

    const composer = screen.getByPlaceholderText('Ask Biorouter anything…') as HTMLTextAreaElement;
    fireEvent.change(composer, { target: { value: 'Reply with the single word ready.' } });
    fireEvent.keyDown(composer, { key: 'Enter' });

    await waitFor(() => expect(mockCreateSession).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(mockToastError).toHaveBeenCalledTimes(1));
    const notice = mockToastError.mock.calls[0][0] as { title: string; msg: string };
    expect(notice.title).toBe('Failed to start chat');
    expect(notice.msg).not.toContain('Do not retry');
    expect(notice.msg).toContain('Your message was kept.');

    await waitFor(() => expect(composer.value).toBe('Reply with the single word ready.'));
    expect(setView).not.toHaveBeenCalled();
    consoleError.mockRestore();
  });

  it('opens the chat when the start succeeds, with nothing to report', async () => {
    mockCreateSession.mockResolvedValueOnce({ id: 'session-1' });
    const setView = vi.fn();
    render(<Hub setView={setView} />);

    const composer = screen.getByPlaceholderText('Ask Biorouter anything…') as HTMLTextAreaElement;
    fireEvent.change(composer, { target: { value: 'hello' } });
    fireEvent.keyDown(composer, { key: 'Enter' });

    await waitFor(() =>
      expect(setView).toHaveBeenCalledWith(
        'pair',
        expect.objectContaining({ resumeSessionId: 'session-1', initialMessage: 'hello' })
      )
    );
    expect(mockToastError).not.toHaveBeenCalled();
  });
});
