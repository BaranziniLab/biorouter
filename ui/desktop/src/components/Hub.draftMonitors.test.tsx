import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

/**
 * DR-33 (design §4), on Home: the `+` menu offers the Monitors before a chat
 * exists, the dialog opens in draft mode, and Send carries the draft as the
 * start's `decisionSettings`, with the person's acceptance only while it names the
 * model new chats start on. A start the person declines keeps the message and
 * reports no failure. The real Hub and ChatInput, with the start-failure test's stubs.
 */

const { mockCreateSession, mockToastError } = vi.hoisted(() => ({
  mockCreateSession: vi.fn(),
  mockToastError: vi.fn(),
}));

vi.mock('../sessions', () => ({ createSession: mockCreateSession }));
// The draft dialog reads the catalog for the model new chats start on; one model on
// this computer, so the dialog offers no install panel.
vi.mock('../utils/sessionDecisions', async (importOriginal) => {
  const real = await importOriginal<typeof import('../utils/sessionDecisions')>();
  return {
    ...real,
    fetchDecisionCatalog: async () => ({
      models: real.parseDecisionModelEntries([
        { provider: 'ollama_decision', model: 'tev1:0.8b', tier: 'private', local: true },
      ])!,
      sessionUnavailableReason: null,
      missing: false,
    }),
  };
});
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
    currentModel: 'gpt-5.5',
    currentProvider: 'versa_azure',
    currentModelSupportsVision: false,
    currentModelSupportedInputMimeTypes: null,
  }),
}));
vi.mock('../hooks/useDiverge', () => ({
  useDiverge: () => ({ diverge: vi.fn() }),
}));
// The pickers and the footer are WS-PICKERS' modules, mocked by path; their
// insides are tested where they live.
vi.mock('./bottom_menu/toolsLists', () => ({
  useToolsLists: () => ({ shown: null, clearQueries: () => {} }),
  ToolsListPanel: () => null,
}));
// The model chip is a probe over the real effort store: these tests are about
// which scope the composer hands it (Home's draft, a tab's draft, the new
// chat's), not about how the picker draws.
vi.mock('./bottom_menu/ModelEffortChip', async () => {
  const React = await import('react');
  const store = await import('../store/reasoningEffort');
  function ModelEffortChip({ reasoningScope }: { reasoningScope: string }) {
    const effort = React.useSyncExternalStore(
      (listener) => store.subscribeToReasoningEffort(reasoningScope, listener),
      () => store.getReasoningEffort(reasoningScope)
    );
    return (
      <div>
        <button aria-label={`Reasoning effort: ${store.REASONING_EFFORT_LABELS[effort]}`} />
        {store.REASONING_EFFORTS.map((option) => (
          <button
            key={option}
            role="menuitemradio"
            aria-checked={option === effort}
            onClick={() => store.setReasoningEffort(reasoningScope, option)}
          >
            {store.REASONING_EFFORT_LABELS[option]}
          </button>
        ))}
      </div>
    );
  }
  return { ModelEffortChip };
});
// The footer draws only the model chip `ChatInput` hands it: the chip sits on
// the footer line since the one-line composer (owner, 2026-11-13).
vi.mock('./bottom_menu/ComposerFooter', () => ({
  ComposerFooter: ({ model }: { model?: React.ReactNode }) => <>{model}</>,
}));
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
import { resetComposerDraftsForTests, HOME_COMPOSER_DRAFT_KEY } from '../utils/composerDrafts';
import { resetReasoningEffortForTests } from '../store/reasoningEffort';
import {
  __resetDraftDecisionSettingsForTests,
  heldDraftDecisionSettings,
  holdDraftDecisionSettings,
} from './decisions/draftDecisionSettings';
import { PublicChatDeclinedError, parseDecisionSettings } from '../utils/sessionDecisions';

const SETTINGS = parseDecisionSettings({
  defaultModel: null,
  monitors: [
    {
      id: 'done',
      name: 'Done',
      question: { instructions: 'Is the task done?', kind: { binary: {} } },
      threshold: 0.85,
      action: 'notify',
      model: { provider: 'baseten_decision', model: 'joey-1' },
    },
  ],
})!;

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  sessionStorage.clear();
  resetReasoningEffortForTests();
  resetComposerDraftsForTests();
  __resetDraftDecisionSettingsForTests();
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

function send(text: string) {
  const composer = screen.getByPlaceholderText('Ask Biorouter anything…') as HTMLTextAreaElement;
  fireEvent.change(composer, { target: { value: text } });
  fireEvent.keyDown(composer, { key: 'Enter' });
  return composer;
}

describe('Home: decision monitors before the first message', () => {
  it('the + menu offers them, and they open in draft mode', async () => {
    render(<Hub setView={vi.fn()} />);
    await userEvent.click(screen.getByRole('button', { name: 'Add files and tools' }));
    await userEvent.click(
      await screen.findByRole('menuitem', { name: 'Decision models and monitors…' })
    );
    expect(await screen.findByTestId('monitors-draft-note')).toBeInTheDocument();
  });

  it('Send carries the draft and the acceptance made under the model new chats start on', async () => {
    holdDraftDecisionSettings(HOME_COMPOSER_DRAFT_KEY, {
      settings: SETTINGS,
      acceptedFor: 'versa_azure/gpt-5.5',
    });
    mockCreateSession.mockResolvedValueOnce({ id: 'session-1' });
    const setView = vi.fn();
    render(<Hub setView={setView} />);
    send('hello');
    await waitFor(() => expect(setView).toHaveBeenCalled());
    expect(mockCreateSession.mock.calls[0][1]).toMatchObject({
      decisionSettings: SETTINGS,
      acceptPublicDecisionModels: true,
    });
    expect(mockCreateSession.mock.calls[0][1].confirmPublicDecisionModels).toBeTypeOf('function');
    // The draft started with that chat; the next one from Home starts with none.
    expect(heldDraftDecisionSettings(HOME_COMPOSER_DRAFT_KEY)).toBeNull();
  });

  it('an acceptance made under another model is not sent: the daemon asks again', async () => {
    holdDraftDecisionSettings(HOME_COMPOSER_DRAFT_KEY, {
      settings: SETTINGS,
      acceptedFor: 'claude_code/opus',
    });
    mockCreateSession.mockResolvedValueOnce({ id: 'session-1' });
    render(<Hub setView={vi.fn()} />);
    send('hello');
    await waitFor(() => expect(mockCreateSession).toHaveBeenCalled());
    expect(mockCreateSession.mock.calls[0][1]).toMatchObject({
      decisionSettings: SETTINGS,
      acceptPublicDecisionModels: false,
    });
  });

  it('a start with no draft sends no decision settings', async () => {
    mockCreateSession.mockResolvedValueOnce({ id: 'session-1' });
    render(<Hub setView={vi.fn()} />);
    send('hello');
    await waitFor(() => expect(mockCreateSession).toHaveBeenCalled());
    expect(mockCreateSession.mock.calls[0][1]).not.toHaveProperty('decisionSettings');
  });

  it('a start the person declined keeps the message and the draft, and reports nothing', async () => {
    holdDraftDecisionSettings(HOME_COMPOSER_DRAFT_KEY, { settings: SETTINGS, acceptedFor: null });
    mockCreateSession.mockRejectedValueOnce(new PublicChatDeclinedError());
    const setView = vi.fn();
    render(<Hub setView={setView} />);
    const composer = send('keep me');
    await waitFor(() => expect(mockCreateSession).toHaveBeenCalled());
    await waitFor(() => expect(composer.value).toBe('keep me'));
    expect(mockToastError).not.toHaveBeenCalled();
    expect(setView).not.toHaveBeenCalled();
    expect(heldDraftDecisionSettings(HOME_COMPOSER_DRAFT_KEY)?.settings).toBe(SETTINGS);
  });
});
